//! Opt-in end-to-end provider smoke test using a portable configuration.
//! Retrieval is a fixed fictional article; all model boundaries are real.
//! Never writes keys, touches the user's DB, or changes the supplied profile.

use super::*;

const LIVE_TITLE: &str = "Basin Ferry suspends harbor route after hull damage";
const LIVE_DESC: &str = "Basin Ferry suspended its harbor route on Monday after inspectors found hull damage. Repairs are expected to take three weeks, and no spare vessel is available.";
const LIVE_BODY: &str = "Basin Ferry suspends harbor route after hull damage

By Morgan Reed

Basin Ferry suspended its harbor route on Monday after inspectors found hull damage in the only vessel assigned to the crossing. The operator said a crack discovered during a scheduled inspection required repairs before the vessel could return to passenger service. Inspectors have not approved a reopening date.

The operator expects repairs to take three weeks and has no spare vessel. A repair yard has agreed to inspect the damaged section this week, but the three-week estimate depends on the work revealing no additional damage. Basin Ferry said it would issue an update after the inspection and would not resume service without approval.

Commuters can use a road detour that adds 90 minutes to the journey. A replacement bus service is under discussion but has not been approved or funded. The local transport authority said it was examining passenger numbers and vehicle availability before deciding whether to provide buses. It has made no commitment to begin a service this week.

Passengers with unused tickets can obtain refunds from Basin Ferry. The authority advised commuters to allow more time for road travel until the ferry returns or a replacement service is confirmed. Residents said the crossing normally provides the shortest trip between the harbor communities. No injuries were reported during the inspection or the suspension of service.";

#[derive(Clone)]
struct LiveBackend {
    runtime: RuntimeBackend,
    related: Vec<AtlasArticleRow>,
}

/// Retain the fixed acquisition through the same Store/NLP receipt boundary
/// used after production retrieval, so canonical publication is exercised too.
fn retain_body(store: &Store, article: &AtlasArticleRow, artifact: &CleanArtifact) -> Result<()> {
    let body = store.ensure_article_body(
        &article.id,
        &article.run_id,
        &article.url,
        &article.source_domain,
        "",
    )?;
    store.commit_article_body(
        &body.id,
        &artifact.markdown,
        &artifact.content_hash,
        &artifact.quality,
        &artifact.rationale,
        &article.url,
        "configured-live-fixture",
        false,
    )?;
    let version = store
        .article_body_by_id(&body.id)?
        .unwrap()
        .body_version
        .to_string();
    let raw_hash = crate::nlp::cache::hash(LIVE_BODY);
    for transform in [
        "validated-baseline-v1",
        crate::intel_recon::clean_body::TRANSFORM,
    ] {
        let mut source = crate::nlp::Source::derived(&body.id, &version, &artifact.markdown);
        source.attempt_id = "configured-live-body".into();
        source.raw_hash = Some(raw_hash.clone());
        crate::nlp::cache::register(
            &store.conn,
            "article",
            &article.id,
            &source,
            Some(LIVE_BODY),
            transform,
        )?;
    }
    store.app_state_set(
        "article-acquisition:configured-live-body",
        &json!({"artifact_hash":raw_hash,"complete_story":true,"known_truncation":false})
            .to_string(),
    )?;
    store.app_state_set(
        &format!("intel-clean-body:{}:{version}", article.id),
        &serde_json::to_string(artifact)?,
    )?;
    Ok(())
}

impl StageBackend for LiveBackend {
    fn prepare_body<'a>(
        &'a mut self,
        _ctx: &'a StageContext,
        article: &'a AtlasArticleRow,
        _force: bool,
        _cancel: &'a Arc<AtomicBool>,
    ) -> BoxFut<'a, Result<CleanArtifact>> {
        Box::pin(async move {
            let mut artifact = clean_article(
                LIVE_BODY,
                &article.title,
                &article.description,
                &article.url,
                &[],
                None,
            )
            .await;
            artifact.acquisition_complete = true;
            artifact.sentence_index =
                Some(crate::intel_recon::sentence_index::SentenceIndex::local(
                    &artifact.markdown,
                    "configured live fixture",
                ));
            retain_body(&Store::open(&self.runtime.db_path)?, article, &artifact)?;
            Ok(artifact)
        })
    }

    fn assess<'a>(
        &'a mut self,
        ctx: &'a StageContext,
        items: Vec<crate::intel_recon::brief_fidelity::FidelityItem>,
        cancel: &'a Arc<AtomicBool>,
    ) -> BoxFut<'a, Result<HashMap<String, String>>> {
        self.runtime.assess(ctx, items, cancel)
    }

    fn classify_context<'a>(
        &mut self,
        ctx: &'a StageContext,
        batch: crate::intel_recon::brief_context::ContextBatch,
        cancel: &'a Arc<AtomicBool>,
    ) -> BoxFut<'a, Result<HashMap<String, String>>> {
        self.runtime.classify_context(ctx, batch, cancel)
    }

    fn related(&mut self, _article: &AtlasArticleRow) -> Result<Vec<AtlasArticleRow>> {
        Ok(self.related.clone())
    }

    fn generate<'a>(
        &mut self,
        request: StageRequest,
        cancel: &'a Arc<AtomicBool>,
    ) -> BoxFut<'a, Result<String>> {
        self.runtime.generate(request, cancel)
    }
}

/// Run with ARGOS_TEST_PROFILE=/absolute/path/profile.json. Optional
/// ARGOS_TEST_BRIEF_ROUTE_ROLE=summarization explicitly tests operation-only
/// routing via that role, keeping all imported role defaults and keys intact.
/// ARGOS_TEST_BRIEF_REPORT is an optional credential-free JSON report path.
/// ARGOS_TEST_BRIEF_PATCH optionally imports a routing update over the profile
/// before dispatch and verifies that the merge retains provider credentials.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "uses real configured providers; requires ARGOS_TEST_PROFILE"]
async fn configured_profile_runs_claims_inferences_and_links() {
    let profile = std::env::var("ARGOS_TEST_PROFILE").expect("ARGOS_TEST_PROFILE is required");
    let raw = std::fs::read_to_string(profile).expect("read portable configuration");
    let plan = crate::config_transfer::parse_document(&raw).expect("valid portable configuration");
    let mut snapshot = crate::config_transfer::ConfigurationSnapshot::default();
    snapshot
        .apply(&plan)
        .expect("apply configuration in memory");
    if let Ok(path) = std::env::var("ARGOS_TEST_BRIEF_PATCH") {
        let before = serde_json::to_value(&snapshot.auth).unwrap();
        let mut settings_before = serde_json::to_value(&snapshot.settings).unwrap();
        settings_before.as_object_mut().unwrap().remove("defaults");
        let quotas_before = serde_json::to_value(&snapshot.quotas).unwrap();
        let patch = std::fs::read_to_string(path).expect("read routing update");
        let patch = crate::config_transfer::parse_document(&patch).expect("valid routing update");
        snapshot.apply(&patch).expect("merge routing update");
        assert!(
            serde_json::to_value(&snapshot.auth).unwrap() == before,
            "a routing update must preserve provider credentials"
        );
        let mut settings_after = serde_json::to_value(&snapshot.settings).unwrap();
        settings_after.as_object_mut().unwrap().remove("defaults");
        assert!(
            settings_before == settings_after,
            "unrelated settings and tool credentials must survive"
        );
        assert!(
            serde_json::to_value(&snapshot.quotas).unwrap() == quotas_before,
            "quota settings must survive"
        );
    }
    // Model-only smoke: do not hand the backend any imported tool keys.
    // The pure merge checks above still cover preservation of those settings.
    let mut settings = crate::provider::SettingsFile {
        defaults: snapshot.settings.defaults,
        nlp: snapshot.settings.nlp,
        ..Default::default()
    };
    let route_role = std::env::var("ARGOS_TEST_BRIEF_ROUTE_ROLE").ok();
    if let Some(role) = route_role.as_deref() {
        assert!(crate::provider::CANONICAL_ROLE_KEYS.contains(&role));
        for operation in [
            crate::provider::OP_INFERENCES_GENERATE,
            crate::provider::OP_LINKS_GENERATE,
        ] {
            settings.defaults.operation_overrides.insert(
                operation.into(),
                crate::provider::OperationOverride::role(role),
            );
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("argos.db");
    let store = Store::open(&path).unwrap();
    store
        .conn
        .execute(
            "INSERT INTO atlas_runs(id,started_at,state) VALUES('fixture',?1,'completed')",
            [now()],
        )
        .unwrap();
    let selected = article(
        "fixture",
        "live-a1",
        LIVE_TITLE,
        LIVE_DESC,
        "https://fictional.test/ferry",
    );
    store.atlas_upsert_article(&selected).unwrap();
    let backend = LiveBackend {
        runtime: RuntimeBackend {
            db_path: path.clone(),
            keys: Default::default(),
            auth: snapshot.auth,
            settings: settings.clone(),
            user_agent: None,
        },
        related: vec![article(
            "fixture",
            "live-a2",
            "Basin Ferry closure leaves commuters facing longer road journeys",
            "Basin Ferry has no spare vessel while hull repairs are expected to take three weeks. Commuters face a 90-minute detour; a replacement bus service remains unapproved.",
            "https://fictional-other.test/ferry-detour",
        )],
    };
    for candidate in &backend.related {
        store.atlas_upsert_article(candidate).unwrap();
    }
    let job = ensure_intel_brief_pipeline(&store, &selected, &settings).unwrap();
    let started = std::time::Instant::now();
    let outcomes = drain_parallel(
        &path,
        "configured-live",
        move || Box::new(backend.clone()) as Box<dyn StageBackend + Send>,
        &Arc::new(AtomicBool::new(false)),
        &ExecConfig::default(),
        STAGE_SLOTS,
    )
    .await
    .unwrap();
    let mut attempts = store.conn.prepare(
        "SELECT o.role,a.provider,a.model,a.outcome,a.failure_category,a.elapsed_ms FROM recon_model_attempts a JOIN recon_model_operations o ON o.id=a.operation_id WHERE a.dispatched=1 ORDER BY a.started_at",
    ).unwrap();
    let attempts: Vec<Value> = attempts
        .query_map([], |r| {
            Ok(json!({
                "role": r.get::<_,String>(0)?, "provider": r.get::<_,String>(1)?,
                "model": r.get::<_,String>(2)?, "outcome": r.get::<_,String>(3)?,
                "category": r.get::<_,String>(4)?, "elapsed_ms": r.get::<_,i64>(5)?,
            }))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    let counts: HashMap<String, usize> = ["claims", "inferences", "links"]
        .into_iter()
        .map(|kind| {
            let output = stage_output(&store, &job, &format!("intel_brief_{kind}"));
            let key = if kind == "links" { "leads" } else { kind };
            (
                kind.into(),
                output
                    .ok()
                    .and_then(|v| v[key].as_array().map(Vec::len))
                    .unwrap_or(0),
            )
        })
        .collect();
    let stages = rows(&path, &job);
    let canonical =
        crate::intel_recon::brief_canonical::index_state(&store.conn, &selected.id).unwrap();
    let mut kinds = store.conn.prepare(
        "SELECT memory_kind,COUNT(*) FROM memory_metadata WHERE lifecycle='active' GROUP BY memory_kind",
    ).unwrap();
    let memory_counts: HashMap<String, usize> = kinds
        .query_map([], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as usize)))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    let report = json!({
        "route_role_override": route_role,
        "seconds": started.elapsed().as_secs_f64(),
        "stages": stages, "counts": counts, "provider_attempts": attempts,
        "canonical_publication": canonical,
        "active_memory_counts": memory_counts,
        "vector_index_exercised": false,
    });
    let report = serde_json::to_string_pretty(&report).unwrap();
    if let Ok(path) = std::env::var("ARGOS_TEST_BRIEF_REPORT") {
        std::fs::write(path, &report).unwrap();
    }
    println!("{report}");
    assert_eq!(outcomes.len(), 4, "all four stages must run");
    assert!(
        outcomes
            .iter()
            .all(|outcome| matches!(outcome, StageOutcome::Committed { .. })),
        "{outcomes:?}"
    );
    assert!(
        stages
            .iter()
            .all(|row| row.starts_with("Completed") && !row.contains("warning")),
        "{stages:?}"
    );
    assert!(
        counts.values().all(|n| *n > 0),
        "fixture must produce real claims, inferences and a link: {counts:?}"
    );
    assert_eq!(
        canonical.as_ref().map(|s| s.state.as_str()),
        Some("indexed"),
        "canonical publication must succeed, not just stage counters: {canonical:?}"
    );
    for kind in ["claim", "inference", "related"] {
        assert!(
            memory_counts.get(kind).copied().unwrap_or(0) > 0,
            "expected published {kind} memories: {memory_counts:?}"
        );
    }
    assert_eq!(
        ensure_intel_brief_pipeline(&store, &selected, &settings).unwrap(),
        job,
        "an unchanged visit reuses the completed chain"
    );
}

#[tokio::test]
async fn fixed_live_acquisition_has_complete_retained_evidence() {
    let db = db();
    let store = Store::open(&db.path).unwrap();
    let article = article(
        "fixture",
        "live-a1",
        LIVE_TITLE,
        LIVE_DESC,
        "https://fictional.test/ferry",
    );
    store.atlas_upsert_article(&article).unwrap();
    let mut artifact =
        clean_article(LIVE_BODY, LIVE_TITLE, LIVE_DESC, &article.url, &[], None).await;
    artifact.acquisition_complete = true;
    retain_body(&store, &article, &artifact).unwrap();
    let input = crate::investigation::insight_persist::article_input(
        &store,
        &article,
        0,
        "configured live fixture",
        false,
    )
    .unwrap();
    let body = input
        .evidence
        .iter()
        .find(|e| e.reference.source_id.starts_with("body:"))
        .unwrap();
    assert!(body.complete_story && !body.known_truncation);
    assert_eq!(body.text, artifact.markdown);
}
