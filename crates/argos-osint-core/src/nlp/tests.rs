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
fn calendar_context_excludes_months_but_preserves_people_named_after_months() {
    let source = Source::derived(
        "calendar",
        "1",
        "August Wilson met May Chen in London in August.",
    );
    let mut a = annotation(&source);
    for name in ["August Wilson", "May Chen", "August"] {
        let start = if name == "August" {
            source.prose.rfind(name).unwrap()
        } else {
            source.prose.find(name).unwrap()
        };
        a.mentions.push(Mention {
            span: Span {
                start_byte: start,
                end_byte: start + name.len(),
            },
            kind: "person".into(),
        });
    }
    let candidates = entity_candidates(&source, &a);
    assert_eq!(
        candidates
            .iter()
            .map(|c| c.original_forms[0].as_str())
            .collect::<Vec<_>>(),
        vec!["August Wilson", "May Chen"]
    );
    assert!(!calendar_month_person(
        "August spoke today",
        0,
        6,
        "August",
        std::iter::empty()
    ));
    assert!(calendar_month_person(
        "August",
        0,
        6,
        "August",
        std::iter::once((0, 6))
    ));
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

#[tokio::test]
async fn auto_complete_policy_preserves_prose_in_every_mode_and_fallback() {
    let text = "東京 😀 Alice denied the allegation.\n\nCorrection: no site opened.".repeat(80);
    for mode in [Mode::Off, Mode::Annotate, Mode::Select, Mode::Auto] {
        let mut source = Source::derived("policy-fixture", "1", &text);
        source.language_hint = Some("ja".into());
        let prepared = prepare_with_policy(
            source,
            &PreparePolicy {
                purpose: "claims".into(),
                coverage_requirement: CoverageRequirement::Complete,
                need_entities: true,
                byte_budget: 128,
            },
            "Alice",
            5,
            Arc::new(AtomicBool::new(false)),
            mode,
        )
        .await;
        assert_eq!(prepared.text, text);
        assert!(prepared.coverage.complete);
        assert_eq!(prepared.effective_action, EffectiveAction::RequiresWindows);
    }
    let text = "No names appear here; the proposal remains uncertain.";
    let source = Source::derived("fits", "1", text);
    let prepared = prepare_with_policy(
        source,
        &PreparePolicy {
            purpose: "claims".into(),
            coverage_requirement: CoverageRequirement::Optional,
            need_entities: true,
            byte_budget: 4096,
        },
        "proposal",
        4,
        Arc::new(AtomicBool::new(false)),
        Mode::Auto,
    )
    .await;
    assert_eq!(prepared.text, text);
    assert!(prepared.coverage.complete);
}
