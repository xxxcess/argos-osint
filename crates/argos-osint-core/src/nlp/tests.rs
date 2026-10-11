use super::*;
fn annotation(source: &Source) -> Annotation {
    let mut a = Annotation::terminal(source, &source.context(), Status::Complete);
    a.sentences = spans::paragraphs(&source.prose)
        .iter()
        .map(|r| Sentence {
            span: *r,
            id: cache::hash(&format!(
                "{}:{}:{}",
                source.prose_hash, r.start_byte, r.end_byte
            )),
        })
        .collect();
    a
}
#[test]
fn late_correction_selection_and_gold_spans() {
    let mut text =
        "Acme announced a project. Alice Smith attributed the announcement to Acme.\n\n".to_owned();
    for _ in 0..800 {
        text.push_str("Background page content about unrelated matters without identities.\n\n");
    }
    let tail="Correction: Alice Smith denied Acme's $1.2 billion estimate, not $2 billion.\n\nThe report remains uncertain pending confirmation.";
    assert!(text.len() > 48000);
    text.push_str(tail);
    let source = Source::derived("gold", "1", &text);
    let a = annotation(&source);
    let selection = select(&a, &source, "body-claims", "Acme", 2000).unwrap();
    let selected = selection.text(&source).unwrap();
    assert!(selected.contains(tail));
    assert!(selected.contains("attributed"));
    assert!(selection.serialized_bytes < text.len() / 10);
    assert!(!selection.omitted_sentence_ids.is_empty());
    assert!(selection.contains(&source, tail));
    let wrong = Source::derived("other", "1", &text.replace("Acme", "Other"));
    assert!(!selection.contains(&wrong, tail));
    assert!(!selection.contains(&source, "unseen paragraph"));
    assert_eq!(
        select(&a, &source, "body-claims", "Acme", 10).unwrap_err(),
        SelectionError::BudgetExceeded
    );
    eprintln!(
        "gold source_bytes={} selected_bytes={} omitted_ids={}",
        text.len(),
        selection.serialized_bytes,
        selection.omitted_sentence_ids.len()
    );
}
#[test]
fn quote_unicode_and_no_entities_preserve_exact_units() {
    let text="😀中e\u{301} opening\r\n\r\nAlice said “There is doubt.\r\n\r\nThis is a correction.” No entity is required.\r\n\r\nEnd.";
    let s = Source::derived("gold", "1", text);
    let a = annotation(&s);
    let v = select(&a, &s, "test", "doubt", 1000).unwrap();
    assert_eq!(v.text(&s).unwrap(), text);
    let mut incomplete = a.clone();
    incomplete.status = Status::Timeout;
    assert_eq!(
        select(&incomplete, &s, "test", "", 1000).unwrap_err(),
        SelectionError::Incomplete
    );
    let quoted = "Opening story.\n\nAdjacent context.\n\nUnrelated background.\n\nAlice said “First qualification.\n\nThe needle belongs to this quotation.\n\nClosing qualification.”\n\nClosing context.\n\n".to_owned()
        + &"Unrelated filler.\n\n".repeat(100);
    let s = Source::derived("quoted", "1", &quoted);
    let selection = select(&annotation(&s), &s, "test", "needle", 600).unwrap();
    let selected = selection.text(&s).unwrap();
    assert!(selected.contains("Alice said “First qualification.\n\nThe needle belongs to this quotation.\n\nClosing qualification.”"));
    assert!(!selection.omitted_sentence_ids.is_empty());
}
#[test]
fn canonical_keys_context_and_gc() {
    let store = crate::store::Store::memory().unwrap();
    let conn = &store.conn;
    cache::migrate(conn).unwrap();
    cache::migrate(conn).unwrap();
    let mut s = Source::derived("shared", "1", "Alice Smith said tomorrow.");
    let ctx = s.context();
    let a = annotation(&s);
    cache::register(conn, "article", "one", &s, None, "historical-v1").unwrap();
    cache::register(conn, "article", "two", &s, None, "historical-v1").unwrap();
    cache::put(conn, &s, &ctx, &a).unwrap();
    assert!(cache::get(conn, &s, &ctx).unwrap().is_some());
    let mut changed = ctx.clone();
    changed.reference_date = "2000-01-01".into();
    assert_ne!(
        cache::feature_key(&s, &ctx),
        cache::feature_key(&s, &changed)
    );
    s.revision = "2".into();
    assert_eq!(cache::feature_key(&s, &ctx), a.feature_key);
    cache::remove_owner(conn, "article", "one").unwrap();
    assert!(cache::get(conn, &s, &ctx).unwrap().is_some());
    cache::remove_owner(conn, "article", "two").unwrap();
    assert!(cache::get(conn, &s, &ctx).unwrap().is_none());
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM nlp_artifacts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        cache::key(&serde_json::json!({"a":1,"b":[2,3]})),
        cache::key(&serde_json::json!({"b":[2,3],"a":1}))
    );
}
#[test]
fn byte_admission_and_disconnection() {
    let (sender, receiver) = mpsc::channel(1);
    let service = Service {
        sender,
        bytes: Arc::new(AtomicUsize::new(QUEUE_BYTES)),
        readiness: Ok(()),
    };
    let s = Source::derived("test", "1", "x");
    assert!(matches!(service.permit(&s), Err(Status::Busy)));
    service.bytes.store(0, Ordering::Release);
    let permit = service.permit(&s).unwrap();
    assert_eq!(service.bytes.load(Ordering::Acquire), 1);
    drop(permit);
    assert_eq!(service.bytes.load(Ordering::Acquire), 0);
    drop(receiver);
    #[cfg(feature = "nlp")]
    assert!(matches!(
        service.try_submit(s.clone(), s.context(), Arc::new(AtomicBool::new(false))),
        Err(Status::Failed)
    ));
    let large = Source::derived("test", "1", &"x".repeat(MAX_SOURCE + 1));
    assert!(matches!(service.permit(&large), Err(Status::TooLarge)));
}
#[cfg(feature = "nlp")]
#[tokio::test]
async fn runtime_date_validation_and_cache_cancellation() {
    let source = Source::derived(
        "dates",
        "1",
        "Tomorrow, January 34th 2020 and 01/02/2020 were discussed. $1.2 billion, not $2 billion.",
    );
    let ctx = ParseContext {
        reference_date: "2026-10-11".into(),
        ..Default::default()
    };
    let worker = Service::shared();
    let begin = std::time::Instant::now();
    let a = worker
        .annotate(
            source.clone(),
            ctx.clone(),
            Arc::new(AtomicBool::new(false)),
        )
        .await;
    assert_eq!(a.status, Status::Complete, "{:?}", a.flags);
    assert!(a
        .date_candidates
        .iter()
        .all(|d| d.span.slice(&source.prose).is_some()));
    for d in &a.date_candidates {
        let text = d.span.slice(&source.prose).unwrap();
        if text.contains("34") || text.contains("01/02") {
            assert!(d.normalized.is_none(), "{d:?}");
        }
    }
    let cold = begin.elapsed();
    eprintln!(
        "parser queue_wait_ms={} parser_ms={} output_bytes={}",
        a.metrics.queue_wait_ms, a.metrics.parser_ms, a.metrics.result_bytes
    );
    let begin = std::time::Instant::now();
    let b = worker
        .annotate(
            source.clone(),
            ctx.clone(),
            Arc::new(AtomicBool::new(false)),
        )
        .await;
    assert_eq!(a.feature_key, b.feature_key);
    let warm = begin.elapsed();
    let cancelled = worker
        .annotate(source.clone(), ctx, Arc::new(AtomicBool::new(true)))
        .await;
    assert_eq!(cancelled.status, Status::Cancelled);
    eprintln!(
        "parser fixture_bytes={} parse_ms={} memory_cache_ms={}",
        source.prose.len(),
        cold.as_millis(),
        warm.as_millis()
    );
}

#[cfg(feature = "nlp")]
#[tokio::test]
async fn real_gold_material_evidence_and_safe_language_fallback() {
    let fixtures: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/gold.json")).unwrap();
    for fixture in fixtures.as_array().unwrap() {
        let mut s = Source::derived(
            fixture["id"].as_str().unwrap(),
            "gold-v1",
            fixture["text"].as_str().unwrap(),
        );
        s.language_hint = Some(fixture["language"].as_str().unwrap().into());
        let a = Service::shared()
            .annotate(s.clone(), s.context(), Arc::new(AtomicBool::new(false)))
            .await;
        let selected =
            if s.language_hint.as_deref() == Some("zh") || fixture["safe_fallback"] == true {
                assert!(matches!(a.status, Status::Unsupported | Status::Failed));
                assert!(select(&a, &s, "gold", "", 1024).is_err());
                let prepared = prepare_mode(
                    s.clone(),
                    "gold",
                    "",
                    1024,
                    1024,
                    Arc::new(AtomicBool::new(false)),
                    Mode::Select,
                )
                .await;
                assert!(prepared.selection.is_none());
                assert!(prepared.diagnostic.is_some());
                prepared.text
            } else {
                assert_eq!(a.status, Status::Complete, "{} {:?}", s.source_id, a.flags);
                select(&a, &s, "gold", fixture["focus"].as_str().unwrap(), 4096)
                    .unwrap()
                    .text(&s)
                    .unwrap()
            };
        for evidence in fixture["material"].as_array().unwrap() {
            assert!(
                selected.contains(evidence.as_str().unwrap()),
                "{}",
                s.source_id
            );
        }
    }
}

#[test]
fn upgrade_schema_27_preserves_unrelated_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("upgrade.db");
    {
        let store = crate::store::Store::open(&path).unwrap();
        store.conn.execute_batch("CREATE TABLE unrelated_nlp_test(value TEXT); INSERT INTO unrelated_nlp_test VALUES('preserved'); DROP TABLE nlp_artifacts; DROP TABLE nlp_source_refs; DROP TABLE nlp_features; PRAGMA user_version=27;").unwrap();
    }
    let store = crate::store::Store::open(&path).unwrap();
    cache::migrate(&store.conn).unwrap();
    assert_eq!(
        store
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        28
    );
    assert_eq!(
        store
            .conn
            .query_row("SELECT value FROM unrelated_nlp_test", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "preserved"
    );
}
