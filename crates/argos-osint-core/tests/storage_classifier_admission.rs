//! Run ownership and admission contracts against isolated embedded storage.
#![cfg(feature = "embedded")]
use argos_osint_core::atlas::{Cursor, RunStats};
use argos_osint_core::classifier::readiness::{self, ClassifierHome, CycleRequirement};
use argos_osint_core::storage::classifier_admission::{
    self as admission, AdmissionError, RunStop, StartRunRequest,
};
use argos_osint_core::storage::open::Target;
use argos_osint_core::storage::repo::Repository;
use chrono::{DateTime, Duration, Utc};

#[tokio::test]
async fn run_claims_fence_expired_owners_and_keep_post_admission_model_optional() {
    let directory = tempfile::tempdir().unwrap();
    let repo = Repository::connect(
        &Target::Embedded {
            path: directory.path().join("storage"),
        },
        "argos",
        "classifier_run_fence",
    )
    .await
    .unwrap();
    let db_owner = admission::AdmissionOwner::new(repo.clone());
    let home = ClassifierHome::new(directory.path().join("home"));
    let cursor = serde_json::to_string(&Cursor {
        phase: 6,
        ..Cursor::default()
    })
    .unwrap();
    let stats = serde_json::to_string(&RunStats::default()).unwrap();
    argos_osint_core::storage::atlas::insert_run(repo.db(), "run", &cursor, &stats)
        .await
        .unwrap();
    let permit = readiness::preflight_cycle(&home, CycleRequirement::PostAdmissionOnly, None)
        .await
        .unwrap();
    let now: DateTime<Utc> = "2026-01-01T00:00:00Z".parse().unwrap();
    let first = admission::start_run(
        &db_owner,
        &permit,
        &StartRunRequest::fresh("run", "owner-a", &cursor, &stats, now).resume(0, 0),
    )
    .await
    .unwrap();
    let revision = admission::run_revision(&repo, "run").await.unwrap();
    assert!(matches!(
        admission::start_run(
            &db_owner,
            &permit,
            &StartRunRequest::fresh("run", "owner-b", &cursor, &stats, now)
                .resume(revision.revision, revision.work_generation)
        )
        .await,
        Err(AdmissionError::LeaseLost)
    ));
    assert!(
        !admission::heartbeat_run(&db_owner, &first, now + Duration::seconds(31))
            .await
            .unwrap()
    );
    let second = admission::start_run(
        &db_owner,
        &permit,
        &StartRunRequest::fresh(
            "run",
            "owner-b",
            &cursor,
            &stats,
            now + Duration::seconds(31),
        )
        .resume(revision.revision, revision.work_generation),
    )
    .await
    .unwrap();
    assert!(second.epoch > first.epoch);
    assert!(matches!(
        admission::save_run(
            &db_owner,
            &first,
            &cursor,
            &stats,
            now + Duration::seconds(32)
        )
        .await,
        Err(AdmissionError::LeaseLost)
    ));
    admission::save_run(
        &db_owner,
        &second,
        &cursor,
        &stats,
        now + Duration::seconds(32),
    )
    .await
    .unwrap();
    let before_invalid = argos_osint_core::storage::atlas::list_runs(repo.db())
        .await
        .unwrap();
    let invalid_cursor = serde_json::to_string(&Cursor {
        phase: 9,
        ..Cursor::default()
    })
    .unwrap();
    assert!(matches!(
        admission::save_run(
            &db_owner,
            &second,
            &invalid_cursor,
            &stats,
            now + Duration::seconds(32)
        )
        .await,
        Err(AdmissionError::InvalidRunState { .. })
    ));
    assert!(matches!(
        admission::save_run(
            &db_owner,
            &second,
            &cursor,
            "{broken",
            now + Duration::seconds(32)
        )
        .await,
        Err(AdmissionError::InvalidRunState { .. })
    ));
    let forged = RunStats {
        classifier_snapshot: Some(argos_osint_core::classifier::ModelSnapshot::new(
            "forged-model",
            "f".repeat(64),
            argos_osint_core::classifier::Provenance::synthetic_baseline("fixture", "0".repeat(64)),
        )),
        ..Default::default()
    };
    assert!(admission::save_run(
        &db_owner,
        &second,
        &cursor,
        &serde_json::to_string(&forged).unwrap(),
        now + Duration::seconds(32)
    )
    .await
    .is_err());
    let after_invalid = argos_osint_core::storage::atlas::list_runs(repo.db())
        .await
        .unwrap();
    assert_eq!(before_invalid[0].stats_json, after_invalid[0].stats_json);
    assert_eq!(before_invalid[0].cursor_json, after_invalid[0].cursor_json);
    assert_eq!(before_invalid[0].state, after_invalid[0].state);
    let terminal = admission::stop_run(
        &db_owner,
        &second,
        RunStop::Paused,
        &cursor,
        &stats,
        "paused",
        now + Duration::seconds(33),
    )
    .await
    .unwrap();
    assert!(!terminal.replayed);
    let replay = admission::stop_run(
        &db_owner,
        &second,
        RunStop::Paused,
        &cursor,
        &stats,
        "paused",
        now + Duration::seconds(34),
    )
    .await
    .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.terminal_revision, terminal.terminal_revision);
    assert!(matches!(
        admission::stop_run(
            &db_owner,
            &first,
            RunStop::Failed,
            &cursor,
            &stats,
            "stale",
            now + Duration::seconds(34)
        )
        .await,
        Err(AdmissionError::LeaseLost)
    ));
    let runs = argos_osint_core::storage::atlas::list_runs(repo.db())
        .await
        .unwrap();
    assert_eq!(runs[0].state, "paused");
    assert!(!home.home().exists());
    db_owner.shutdown().await.unwrap();
    home.shutdown().await.unwrap();
}

#[cfg(feature = "classifier")]
#[path = "fixtures/classifier/admission-support.rs"]
mod admission_fixture;

#[cfg(feature = "classifier")]
fn article(run: &str, id: &str) -> argos_osint_core::store::AtlasArticleRow {
    argos_osint_core::store::AtlasArticleRow {
        run_id: run.into(),
        id: id.into(),
        title: "Ballot patrol common".into(),
        description: "Fictional independent training example".into(),
        url: format!("https://fixture.invalid/{id}"),
        provider: "fixture".into(),
        source_domain: "fixture.invalid".into(),
        source_name: "Independent fixture".into(),
        seen_at: "2026-01-01T00:00:00Z".into(),
        ..Default::default()
    }
}

/// The real SQL path upgrades more than one keyset page, commits 63 independent
/// siblings beside a restaged row, and proves uncertain-reply idempotency with
/// original receipt bytes. No saved weights or remote provider is used.
#[cfg(feature = "classifier")]
#[tokio::test]
async fn paged_admission_preserves_siblings_replay_bytes_and_other_runs() {
    use argos_osint_core::atlas_classify::{
        self, AdmissionContext, ComputationCache, ItemClassification, MonotonicClock,
    };
    use argos_osint_core::classifier::WorkControl;
    use argos_osint_core::storage::classifier_admission::ItemCommit;
    use std::sync::Arc;

    let _serial = admission_fixture::serial().await;
    let fixture = admission_fixture::install();
    let repo = Repository::connect(
        &Target::Embedded {
            path: fixture.directory.path().join("storage"),
        },
        "argos",
        "classifier_admission_pages",
    )
    .await
    .unwrap();
    let db_owner = admission::AdmissionOwner::new(repo.clone());
    let permit =
        readiness::preflight_cycle(&fixture.home, CycleRequirement::FreshAcquisition, None)
            .await
            .unwrap();
    let pin = permit.pinned().unwrap().clone();
    let path = fixture.home.home().to_path_buf();
    let context = tokio::task::spawn_blocking(move || AdmissionContext::load(&path, pin))
        .await
        .unwrap()
        .unwrap();
    let stats = RunStats {
        classifier_snapshot: Some(context.pin.snapshot.clone()),
        ..Default::default()
    };
    let cursor = serde_json::to_string(&Cursor::default()).unwrap();
    let stats_json = serde_json::to_string(&stats).unwrap();
    let now: DateTime<Utc> = "2026-01-01T00:00:00Z".parse().unwrap();
    let run = admission::start_run(
        &db_owner,
        &permit,
        &StartRunRequest::fresh("main", "owner-a", &cursor, &stats_json, now),
    )
    .await
    .unwrap();

    // Legacy rev0/seq0 rows are inserted in one bounded driver batch. Only the
    // selected run can be upgraded, and its source hash remains unchanged.
    let mut sql = String::new();
    let mut payloads = Vec::new();
    for index in 0..530 {
        let row = article("main", &format!("article-{index:04}"));
        payloads.push(row);
        sql.push_str(&format!("CREATE type::record('atlas_admission',$key{index}) SET run_id='main',article_id=$article{index},payload_json=$payload{index},input_hash=$hash{index},source_revision='source-v1',state='pending',taxonomy_version='atlas-taxonomy-v2',created_at=$now,updated_at=$now RETURN NONE;"));
    }
    let mut query = repo.db().query(sql).bind(("now", now.to_rfc3339()));
    for (index, row) in payloads.iter().enumerate() {
        query = query
            .bind((
                format!("key{index}"),
                argos_osint_core::atlas_classify::OccurrenceKey::new("main", &row.id).record_id(),
            ))
            .bind((format!("article{index}"), row.id.clone()))
            .bind((
                format!("payload{index}"),
                serde_json::to_string(row).unwrap(),
            ))
            .bind((
                format!("hash{index}"),
                argos_osint_core::atlas_metadata::input_hash(row),
            ));
    }
    query.await.unwrap().check().unwrap();
    let other = article("other", "other-legacy");
    repo.db().query("CREATE type::record('atlas_admission',$key) SET run_id='other',article_id='other-legacy',payload_json=$payload,input_hash=$hash,source_revision='source-v1',state='pending',taxonomy_version='atlas-taxonomy-v2',created_at=$now,updated_at=$now RETURN NONE")
        .bind(("key",atlas_classify::OccurrenceKey::new("other","other-legacy").record_id()))
        .bind(("payload",serde_json::to_string(&other).unwrap())).bind(("hash",argos_osint_core::atlas_metadata::input_hash(&other))).bind(("now",now.to_rfc3339())).await.unwrap().check().unwrap();
    let mut upgraded = 0;
    loop {
        let count = admission::upgrade_legacy_page(&db_owner, &run, now)
            .await
            .unwrap();
        upgraded += count;
        if count == 0 {
            break;
        }
    }
    assert_eq!(upgraded, 530);
    let mut response = repo
        .db()
        .query("SELECT VALUE work_seq FROM ONLY type::record('atlas_admission',$key)")
        .bind((
            "key",
            atlas_classify::OccurrenceKey::new("other", "other-legacy").record_id(),
        ))
        .await
        .unwrap()
        .check()
        .unwrap();
    assert_eq!(response.take::<Option<i64>>(0).unwrap(), Some(0));

    let mut token = admission::begin_admission(&db_owner, &run, &context, false, now)
        .await
        .unwrap();
    assert_eq!(token.through_seq, 530);
    let mut cache = ComputationCache::new(Arc::new(MonotonicClock::default()));
    let control = WorkControl::new();
    let first = admission::claim_page(&db_owner, &run, &token, now)
        .await
        .unwrap();
    assert_eq!(first.inputs.len(), 64);
    let stale_id = first.inputs[0].article.id.clone();
    let mut changed = first.inputs[0].article.clone();
    changed.title = "Freight clinic common".into();
    assert!(
        admission::stage_article(&db_owner, &run, &changed, "source-v2", now)
            .await
            .unwrap()
    );
    let scored = atlas_classify::classify_page(&context, first.inputs, &mut cache, &control)
        .await
        .unwrap();
    assert_eq!(scored.outcomes.len(), 64);
    let original_bytes: Vec<_> = scored
        .outcomes
        .iter()
        .skip(1)
        .map(|item| match item {
            ItemClassification::Evaluated { receipt, .. } => {
                serde_json::to_string(receipt).unwrap()
            }
            other => panic!("real classifier unexpectedly failed: {other:?}"),
        })
        .collect();
    let committed = admission::commit_outcomes(&db_owner, &run, &scored.outcomes, now)
        .await
        .unwrap();
    assert!(matches!(committed.items[0], ItemCommit::StaleInput { .. }));
    assert_eq!(
        committed
            .items
            .iter()
            .filter(|item| matches!(item, ItemCommit::Committed { .. }))
            .count(),
        63
    );
    // Simulate losing the commit reply: resend the original values, including
    // their original evaluated_at. Exact receipt bytes are reused unchanged.
    let replay = admission::commit_outcomes(&db_owner, &run, &scored.outcomes, now)
        .await
        .unwrap();
    assert_eq!(
        replay
            .items
            .iter()
            .filter(|item| matches!(item, ItemCommit::AlreadyCommitted { .. }))
            .count(),
        63
    );
    for (item, bytes) in scored.outcomes.iter().skip(1).zip(original_bytes) {
        let ItemClassification::Evaluated { claim, .. } = item else {
            unreachable!()
        };
        let view = admission::classification_view(&repo, &claim.occurrence)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_string(view.receipt.as_ref().unwrap()).unwrap(),
            bytes
        );
    }
    token = first.next.unwrap();
    let mut settled = 63;
    let mut abandoned = None;
    loop {
        let page = admission::claim_page(&db_owner, &run, &token, now)
            .await
            .unwrap();
        let scored = atlas_classify::classify_page(&context, page.inputs, &mut cache, &control)
            .await
            .unwrap();
        let committed = if abandoned.is_none() {
            // Process loss leaves one dispatched revision behind the next
            // committed keyset position; its independent siblings can settle.
            let mut outcomes = scored.outcomes;
            abandoned = Some(outcomes.remove(0));
            admission::commit_outcomes(&db_owner, &run, &outcomes, now)
                .await
                .unwrap()
        } else {
            admission::commit_outcomes(&db_owner, &run, &scored.outcomes, now)
                .await
                .unwrap()
        };
        settled += committed
            .items
            .iter()
            .filter(|item| matches!(item, ItemCommit::Committed { .. }))
            .count();
        match page.next {
            Some(next) => token = next,
            None => break,
        }
    }
    assert_eq!(settled, 528);
    let next = admission::begin_admission(&db_owner, &run, &context, false, now)
        .await
        .unwrap();
    assert_eq!((next.after_seq, next.through_seq), (530, 531));
    let page = admission::claim_page(&db_owner, &run, &next, now)
        .await
        .unwrap();
    assert_eq!(page.inputs.len(), 1);
    assert_eq!(page.inputs[0].article.id, stale_id);
    assert_eq!(page.inputs[0].claim.source_revision, "source-v2");
    let scored = atlas_classify::classify_page(&context, page.inputs, &mut cache, &control)
        .await
        .unwrap();
    assert!(matches!(
        admission::commit_outcomes(&db_owner, &run, &scored.outcomes, now)
            .await
            .unwrap()
            .items[0],
        ItemCommit::Committed { .. }
    ));
    assert!(
        argos_osint_core::storage::atlas::admission_pending_exists(repo.db(), "main")
            .await
            .unwrap()
    );
    let later = now + Duration::seconds(31);
    assert!(matches!(
        admission::commit_outcomes(&db_owner, &run, &[abandoned.unwrap()], later)
            .await
            .unwrap()
            .items[0],
        ItemCommit::LeaseLost { .. }
    ));
    let revision = admission::run_revision(&repo, "main").await.unwrap();
    let permit = readiness::preflight_cycle(
        &fixture.home,
        CycleRequirement::AdmissionOnly,
        Some(&context.pin.snapshot),
    )
    .await
    .unwrap();
    let resumed = admission::start_run(
        &db_owner,
        &permit,
        &StartRunRequest::fresh("main", "owner-b", &cursor, &stats_json, later)
            .resume(revision.revision, revision.work_generation),
    )
    .await
    .unwrap();
    let retry = admission::begin_admission(&db_owner, &resumed, &context, true, later)
        .await
        .unwrap();
    assert_eq!(retry.after_seq, 0);
    let recovered = admission::claim_page(&db_owner, &resumed, &retry, later)
        .await
        .unwrap();
    assert_eq!(recovered.inputs.len(), 1);
    assert!(recovered.inputs[0].claim.work_seq < 128);
    let scored = atlas_classify::classify_page(&context, recovered.inputs, &mut cache, &control)
        .await
        .unwrap();
    assert!(matches!(
        admission::commit_outcomes(&db_owner, &resumed, &scored.outcomes, later)
            .await
            .unwrap()
            .items[0],
        ItemCommit::Committed { .. }
    ));
    assert!(
        !argos_osint_core::storage::atlas::admission_pending_exists(repo.db(), "main")
            .await
            .unwrap()
    );
    assert!(
        argos_osint_core::storage::atlas::admission_pending_exists(repo.db(), "other")
            .await
            .unwrap()
    );
    // A provenance-only restage after successful removal of staging must
    // advance revision and leave the original immutable receipt untouched.
    let occurrence = atlas_classify::OccurrenceKey::new("main", &payloads[1].id);
    let original = admission::classification_view(&repo, &occurrence)
        .await
        .unwrap()
        .receipt
        .unwrap();
    let original_key = original.key.record_id();
    let original_json = serde_json::to_string(&original).unwrap();
    let mut revised = payloads[1].clone();
    revised.provider = "corrected-fixture".into();
    admission::stage_article(&db_owner, &resumed, &revised, "source-v1", later)
        .await
        .unwrap();
    let activation = admission::begin_admission(&db_owner, &resumed, &context, true, later)
        .await
        .unwrap();
    let page = admission::claim_page(&db_owner, &resumed, &activation, later)
        .await
        .unwrap();
    assert_eq!(page.inputs.len(), 1);
    assert!(page.inputs[0].claim.input_revision > original.key.input_revision);
    let scored = atlas_classify::classify_page(&context, page.inputs, &mut cache, &control)
        .await
        .unwrap();
    assert_eq!(scored.metrics.computations, 0);
    assert!(matches!(
        admission::commit_outcomes(&db_owner, &resumed, &scored.outcomes, later)
            .await
            .unwrap()
            .items[0],
        ItemCommit::Committed { .. }
    ));
    let revised_view = admission::classification_view(&repo, &occurrence)
        .await
        .unwrap()
        .receipt
        .unwrap();
    assert_eq!(revised_view.provenance.provider, "corrected-fixture");
    assert_ne!(revised_view.key.record_id(), original_key);
    let mut response = repo
        .db()
        .query(
            "SELECT VALUE receipt_json FROM ONLY type::record('atlas_classification_receipt',$key)",
        )
        .bind(("key", original_key))
        .await
        .unwrap()
        .check()
        .unwrap();
    assert_eq!(
        response.take::<Option<String>>(0).unwrap().as_deref(),
        Some(original_json.as_str())
    );
    let summary = admission::run_summary(&repo, "main").await.unwrap();
    assert_eq!(summary.retained, 530);
    assert_eq!(summary.admitted + summary.rejected, 530);
    assert_eq!(summary.staged, 0);
    // Explicit language/observation provenance revokes prior claims without
    // contaminating the raw computation key or replacing historical receipts.
    let provenance_key = atlas_classify::OccurrenceKey::new("main", &payloads[2].id);
    let old_provenance = admission::classification_view(&repo, &provenance_key)
        .await
        .unwrap()
        .receipt
        .unwrap();
    let english = admission::StageProvenance::new(
        Some("en".into()),
        vec!["obs-b".into(), "obs-a".into(), "obs-a".into()],
    )
    .unwrap();
    assert!(admission::StageProvenance::new(None, vec!["x".into(); 65]).is_err());
    assert!(admission::StageProvenance::new(Some("en\u{85}".into()), vec![]).is_err());
    admission::stage_article_with_provenance(
        &db_owner,
        &resumed,
        &payloads[2],
        "source-v1",
        &english,
        later,
    )
    .await
    .unwrap();
    let activation = admission::begin_admission(&db_owner, &resumed, &context, true, later)
        .await
        .unwrap();
    let page = admission::claim_page(&db_owner, &resumed, &activation, later)
        .await
        .unwrap();
    assert_eq!(page.inputs.len(), 1);
    assert_eq!(
        page.inputs[0].provenance.observation_ids,
        ["obs-a", "obs-b"]
    );
    assert_eq!(
        page.inputs[0].claim.classifier_input_hash,
        old_provenance.key.classifier_input_hash
    );
    let old_scored = atlas_classify::classify_page(&context, page.inputs, &mut cache, &control)
        .await
        .unwrap();
    assert_eq!(old_scored.metrics.computations, 0);
    let unsupported =
        admission::StageProvenance::new(Some("fr".into()), vec!["obs-c".into()]).unwrap();
    admission::stage_article_with_provenance(
        &db_owner,
        &resumed,
        &payloads[2],
        "source-v1",
        &unsupported,
        later,
    )
    .await
    .unwrap();
    assert!(matches!(
        admission::commit_outcomes(&db_owner, &resumed, &old_scored.outcomes, later)
            .await
            .unwrap()
            .items[0],
        ItemCommit::StaleInput { .. }
    ));
    let activation = admission::begin_admission(&db_owner, &resumed, &context, true, later)
        .await
        .unwrap();
    let page = admission::claim_page(&db_owner, &resumed, &activation, later)
        .await
        .unwrap();
    assert!(page.inputs[0].claim.input_revision > old_provenance.key.input_revision);
    assert_eq!(
        page.inputs[0].claim.classifier_input_hash,
        old_provenance.key.classifier_input_hash
    );
    let scored = atlas_classify::classify_page(&context, page.inputs, &mut cache, &control)
        .await
        .unwrap();
    assert_eq!(scored.metrics.computations, 0);
    admission::commit_outcomes(&db_owner, &resumed, &scored.outcomes, later)
        .await
        .unwrap();
    let view = admission::classification_view(&repo, &provenance_key)
        .await
        .unwrap();
    assert_eq!(view.admission_state, "rejected");
    let receipt = view.receipt.unwrap();
    assert_eq!(
        receipt.receipt.rejection_reason,
        Some(argos_osint_core::classifier::RejectionReason::UnsupportedLanguage)
    );
    assert_eq!(receipt.provenance.language.as_deref(), Some("fr"));
    assert_eq!(receipt.provenance.observation_ids, ["obs-c"]);
    assert!(
        argos_osint_core::storage::atlas::article(repo.db(), "main", &payloads[2].id)
            .await
            .unwrap()
            .is_some(),
        "topic/language rejection retains historical occurrence"
    );
    let mut old = repo
        .db()
        .query(
            "SELECT VALUE receipt_json FROM ONLY type::record('atlas_classification_receipt',$key)",
        )
        .bind(("key", old_provenance.key.record_id()))
        .await
        .unwrap()
        .check()
        .unwrap();
    assert_eq!(
        old.take::<Option<String>>(0).unwrap().unwrap(),
        serde_json::to_string(&old_provenance).unwrap()
    );
    // Valid retained IDs can exceed the JS sample-ID wire limit. A cold
    // inference uses a framed digest while the durable receipt keeps the full ID.
    let long_id = "l".repeat(512);
    let mut long = article("main", &long_id);
    long.title = "Freight clinic common".into();
    admission::stage_article(&db_owner, &resumed, &long, "long-source", later)
        .await
        .unwrap();
    let activation = admission::begin_admission(&db_owner, &resumed, &context, true, later)
        .await
        .unwrap();
    let page = admission::claim_page(&db_owner, &resumed, &activation, later)
        .await
        .unwrap();
    assert_eq!(page.inputs.len(), 1);
    assert!(page.inputs[0].claim.occurrence.record_id().len() > 256);
    let mut cold = ComputationCache::default();
    let scored = atlas_classify::classify_page(&context, page.inputs, &mut cold, &control)
        .await
        .unwrap();
    assert_eq!(scored.metrics.computations, 1);
    assert!(matches!(
        admission::commit_outcomes(&db_owner, &resumed, &scored.outcomes, later)
            .await
            .unwrap()
            .items[0],
        ItemCommit::Committed { .. }
    ));
    let long_view = admission::classification_view(
        &repo,
        &atlas_classify::OccurrenceKey::new("main", &long_id),
    )
    .await
    .unwrap()
    .receipt
    .unwrap();
    assert_eq!(
        long_view.receipt.occurrence_id,
        atlas_classify::OccurrenceKey::new("main", &long_id).record_id()
    );
    assert_eq!(
        long_view.receipt.selected_domains,
        vec![
            argos_osint_core::atlas_taxonomy::DomainId::Economic,
            argos_osint_core::atlas_taxonomy::DomainId::Human
        ]
    );
    // Persisted operational retry work is distinct from ownership loss.
    for id in ["retry-busy", "retry-timeout"] {
        admission::stage_article(
            &db_owner,
            &resumed,
            &article("main", id),
            "retry-source",
            later,
        )
        .await
        .unwrap();
    }
    let activation = admission::begin_admission(&db_owner, &resumed, &context, true, later)
        .await
        .unwrap();
    let page = admission::claim_page(&db_owner, &resumed, &activation, later)
        .await
        .unwrap();
    assert_eq!(page.inputs.len(), 2);
    let outcomes: Vec<_> = page
        .inputs
        .into_iter()
        .zip([
            argos_osint_core::classifier::ClassifierError::Busy,
            argos_osint_core::classifier::ClassifierError::Timeout,
        ])
        .map(|(input, error)| ItemClassification::Retryable {
            claim: input.claim,
            error,
        })
        .collect();
    let result = admission::commit_outcomes(&db_owner, &resumed, &outcomes, later)
        .await
        .unwrap();
    assert!(matches!(&result.items[0],ItemCommit::Retryable {code,..} if code=="busy"));
    assert!(matches!(&result.items[1],ItemCommit::Retryable {code,..} if code=="timeout"));
    let summary = admission::run_summary(&repo, "main").await.unwrap();
    assert_eq!((summary.staged, summary.failed), (2, 2));
    for (id, code) in [("retry-busy", "busy"), ("retry-timeout", "timeout")] {
        let mut response=repo.db().query("SELECT VALUE state FROM ONLY type::record('atlas_admission',$key);SELECT VALUE last_error FROM ONLY type::record('atlas_admission',$key);").bind(("key",atlas_classify::OccurrenceKey::new("main",id).record_id())).await.unwrap().check().unwrap();
        assert_eq!(
            response.take::<Option<String>>(0).unwrap().as_deref(),
            Some("failed")
        );
        assert_eq!(
            response.take::<Option<String>>(1).unwrap().as_deref(),
            Some(code)
        );
    }
    // The public metadata upsert preserves independent native columns and
    // routes a corrected native source through guarded private restaging.
    admission::stop_run(
        &db_owner,
        &resumed,
        RunStop::Paused,
        &cursor,
        &stats_json,
        "before correction",
        later + Duration::seconds(1),
    )
    .await
    .unwrap();
    let retained = argos_osint_core::storage::atlas::article(repo.db(), "main", &payloads[1].id)
        .await
        .unwrap()
        .unwrap();
    let before_view = admission::classification_view(&repo, &occurrence)
        .await
        .unwrap()
        .receipt
        .unwrap();
    let mut metadata_only = retained.clone();
    metadata_only.author = "corrected author metadata".into();
    argos_osint_core::storage::atlas::upsert_article(repo.db(), &metadata_only)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_string(
            &admission::classification_view(&repo, &occurrence)
                .await
                .unwrap()
                .receipt
                .unwrap()
        )
        .unwrap(),
        serde_json::to_string(&before_view).unwrap()
    );
    let stored = argos_osint_core::storage::atlas::article(repo.db(), "main", &retained.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.author, "corrected author metadata");
    let mut corrected = metadata_only.clone();
    corrected.title = "River sensor common".into();
    argos_osint_core::storage::atlas::upsert_article(repo.db(), &corrected)
        .await
        .unwrap();
    let pending = admission::classification_view(&repo, &occurrence)
        .await
        .unwrap();
    assert_eq!(pending.admission_state, "pending");
    assert!(pending.receipt.is_none());
    assert!(!pending.legacy);
    let historical = argos_osint_core::storage::atlas::article(repo.db(), "main", &retained.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(historical.title, retained.title);
    assert_eq!(historical.categories, retained.categories);
    assert_eq!(historical.admitted_at, retained.admitted_at);
    let staged = argos_osint_core::storage::atlas::admission_row(repo.db(), "main", &retained.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(staged.article().unwrap().title, corrected.title);
    // A live actual run lease rejects an unowned correction atomically.
    let live_now = Utc::now();
    let revision = admission::run_revision(&repo, "main").await.unwrap();
    let live = admission::start_run(
        &db_owner,
        &permit,
        &StartRunRequest::fresh(
            "main",
            "live-correction-owner",
            &cursor,
            &stats_json,
            live_now,
        )
        .resume(revision.revision, revision.work_generation),
    )
    .await
    .unwrap();
    let mut blocked = corrected.clone();
    blocked.title = "A newer unowned title".into();
    assert!(
        argos_osint_core::storage::atlas::upsert_article(repo.db(), &blocked)
            .await
            .is_err()
    );
    let after = argos_osint_core::storage::atlas::admission_row(repo.db(), "main", &retained.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.payload_json, staged.payload_json);
    admission::stop_run(
        &db_owner,
        &live,
        RunStop::Paused,
        &cursor,
        &stats_json,
        "safe correction stop",
        live_now,
    )
    .await
    .unwrap();
    db_owner.shutdown().await.unwrap();
    fixture.home.shutdown().await.unwrap();
}

/// UTF-8 and JSON escaping affect transport size differently. The header
/// bounds must reject an oversized set before payload fetch, while a smaller
/// claim prefix retains every byte of valid metadata.
#[cfg(feature = "classifier")]
#[tokio::test]
async fn multibyte_escaped_metadata_is_bounded_before_fetch_and_claims_split() {
    use argos_osint_core::atlas_classify::{AdmissionContext, OccurrenceKey};
    use argos_osint_core::atlas_metadata::ArticleMetadata;
    use argos_osint_core::storage::atlas_metadata;
    let _serial = admission_fixture::serial().await;
    let fixture = admission_fixture::install();
    let repo = Repository::connect(
        &Target::Embedded {
            path: fixture.directory.path().join("storage"),
        },
        "argos",
        "classifier_metadata_wire",
    )
    .await
    .unwrap();
    let owner = admission::AdmissionOwner::new(repo.clone());
    let permit =
        readiness::preflight_cycle(&fixture.home, CycleRequirement::FreshAcquisition, None)
            .await
            .unwrap();
    let pin = permit.pinned().unwrap().clone();
    let path = fixture.home.home().to_path_buf();
    let context = tokio::task::spawn_blocking(move || AdmissionContext::load(&path, pin))
        .await
        .unwrap()
        .unwrap();
    let stats = RunStats {
        classifier_snapshot: Some(context.pin.snapshot.clone()),
        ..Default::default()
    };
    let cursor = serde_json::to_string(&Cursor::default()).unwrap();
    let stats = serde_json::to_string(&stats).unwrap();
    let now: DateTime<Utc> = "2026-01-01T00:00:00Z".parse().unwrap();
    let run = admission::start_run(
        &owner,
        &permit,
        &StartRunRequest::fresh("wire", "owner", &cursor, &stats, now),
    )
    .await
    .unwrap();
    let atom = "界\\\"\n";
    let row = article("wire", "a-large");
    let mut empty = ArticleMetadata::pending(&row);
    empty.diagnostics = vec![String::new()];
    let overhead = serde_json::to_string(&empty).unwrap().len();
    let atom_json = serde_json::to_string(atom).unwrap().len() - 2;
    let target = (admission::MAX_RESPONSE_BYTES / 2 - 4096) * 3 / 4;
    let repetitions = (target - overhead) / atom_json;
    let large = atom.repeat(repetitions);
    let expected_hash = argos_osint_core::classifier::artifact::sha256_hex(large.as_bytes());
    let mut keys = Vec::new();
    for (id, text) in [
        ("a-large", large.clone()),
        ("b-large", large),
        ("c-small", "bounded overflow".repeat(8192)),
    ] {
        let row = article("wire", id);
        admission::stage_article(&owner, &run, &row, "source", now)
            .await
            .unwrap();
        let mut metadata = ArticleMetadata::pending(&row);
        metadata.diagnostics = vec![text];
        let key = OccurrenceKey::new("wire", id);
        repo.db().query("UPSERT type::record('atlas_article_metadata',$key) SET run_id='wire',article_id=$article,input_hash=$hash,entity_state='pending',reliability_state='pending',classification_status='pending',metadata_json=$json RETURN NONE")
            .bind(("key",key.record_id())).bind(("article",id.to_owned())).bind(("hash",metadata.input_hash.clone())).bind(("json",serde_json::to_string(&metadata).unwrap())).await.unwrap().check().unwrap();
        keys.push(key);
    }
    let metrics = owner.metrics();
    let before = metrics.round_trips();
    let error = owner
        .scope(atlas_metadata::metadata_many(repo.db(), &keys))
        .await
        .unwrap_err();
    assert!(matches!(error, AdmissionError::PayloadTooLarge { .. }));
    assert_eq!(
        metrics.round_trips() - before,
        1,
        "only the bounded header query may issue for an oversized response"
    );
    let before = metrics.round_trips();
    let values = owner
        .scope(atlas_metadata::metadata_many(repo.db(), &keys[..2]))
        .await
        .unwrap();
    assert_eq!(metrics.round_trips() - before, 2);
    assert_eq!(values.len(), 2);
    for metadata in values.values() {
        assert_eq!(
            argos_osint_core::classifier::artifact::sha256_hex(metadata.diagnostics[0].as_bytes()),
            expected_hash
        );
    }
    drop(values);
    let token = admission::begin_admission(&owner, &run, &context, false, now)
        .await
        .unwrap();
    let page = admission::claim_page(&owner, &run, &token, now)
        .await
        .unwrap();
    assert_eq!(page.inputs.len(), 2);
    assert_eq!(page.next.as_ref().unwrap().after_seq, 2);
    for input in &page.inputs {
        assert_eq!(
            argos_osint_core::classifier::artifact::sha256_hex(
                input.metadata.as_ref().unwrap().diagnostics[0].as_bytes()
            ),
            expected_hash
        );
    }
    let claims: Vec<_> = page
        .inputs
        .iter()
        .map(|input| input.claim.clone())
        .collect();
    admission::release_owned(&owner, &run, &claims, now)
        .await
        .unwrap();
    drop(page);
    owner.shutdown().await.unwrap();
    fixture.home.shutdown().await.unwrap();
}

#[tokio::test]
async fn terminal_enqueue_hook_is_atomic_replayed_and_excludes_nonterminal_stops() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let directory = tempfile::tempdir().unwrap();
    let repo = Repository::connect(
        &Target::Embedded {
            path: directory.path().join("storage"),
        },
        "argos",
        "terminal_admission_hook",
    )
    .await
    .unwrap();
    repo.db()
        .query("DEFINE TABLE terminal_probe SCHEMALESS")
        .await
        .unwrap()
        .check()
        .unwrap();
    let owner = admission::AdmissionOwner::new(repo.clone());
    let home = ClassifierHome::new(directory.path().join("no-model"));
    let permit = readiness::preflight_cycle(&home, CycleRequirement::PostAdmissionOnly, None)
        .await
        .unwrap();
    let cursor = serde_json::to_string(&Cursor {
        phase: 6,
        ..Default::default()
    })
    .unwrap();
    let stats = serde_json::to_string(&RunStats::default()).unwrap();
    let now = Utc::now();
    let count = Arc::new(AtomicUsize::new(0));
    for state in [
        RunStop::Completed,
        RunStop::Partial,
        RunStop::Failed,
        RunStop::Cancelled,
    ] {
        let run = format!("terminal-{}", state.as_str());
        argos_osint_core::storage::atlas::insert_run(repo.db(), &run, &cursor, &stats)
            .await
            .unwrap();
        let first = admission::start_run(
            &owner,
            &permit,
            &StartRunRequest::fresh(&run, "terminal-owner", &cursor, &stats, now).resume(0, 0),
        )
        .await
        .unwrap();
        let untouched = count.clone();
        let pause_hook: admission::TerminalHook = Arc::new(move |_tx, _terminal, _now| {
            untouched.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { panic!("nonterminal stop dispatched terminal hook") })
        });
        admission::stop_run_with_hook(
            &owner,
            &first,
            RunStop::Paused,
            &cursor,
            &stats,
            "pause",
            now,
            Some(pause_hook),
        )
        .await
        .unwrap();
        let revision = admission::run_revision(&repo, &run).await.unwrap();
        let claim = admission::start_run(
            &owner,
            &permit,
            &StartRunRequest::fresh(&run, "terminal-owner", &cursor, &stats, now)
                .resume(revision.revision, revision.work_generation),
        )
        .await
        .unwrap();
        let failure: admission::TerminalHook = Arc::new(|tx, terminal, _now| {
            Box::pin(async move {
                tx.query("UPSERT type::record('terminal_probe',$run) SET writes=1 RETURN NONE")
                    .bind(("run", terminal.run_id))
                    .await?
                    .check()?;
                Err(argos_osint_core::storage::tx::TxError::Abort(
                    "injected_terminal_enqueue_failure".into(),
                ))
            })
        });
        assert!(admission::stop_run_with_hook(
            &owner,
            &claim,
            state,
            &cursor,
            &stats,
            "terminal",
            now,
            Some(failure)
        )
        .await
        .is_err());
        let retained = argos_osint_core::storage::atlas::list_runs(repo.db())
            .await
            .unwrap()
            .into_iter()
            .find(|row| row.id == run)
            .unwrap();
        assert_eq!(
            retained.state, "running",
            "failed enqueue must roll back terminal visibility"
        );
        let mut probe = repo
            .db()
            .query("RETURN record::exists(type::record('terminal_probe',$run))")
            .bind(("run", run.clone()))
            .await
            .unwrap()
            .check()
            .unwrap();
        assert_eq!(probe.take::<Option<bool>>(0).unwrap(), Some(false));
        let invoked = count.clone();
        let hook: admission::TerminalHook = Arc::new(move |tx, terminal, _now| {
            invoked.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                assert!(!terminal.identity.is_empty());
                tx.query("UPSERT type::record('terminal_probe',$run) SET writes=(writes ?? 0)+1 RETURN NONE").bind(("run",terminal.run_id)).await?.check()?;
                Ok(admission::TerminalEffect::new(
                    7,
                    Some("durable-calibration-job".into()),
                ))
            })
        });
        let receipt = admission::stop_run_with_hook(
            &owner,
            &claim,
            state,
            &cursor,
            &stats,
            "terminal",
            now,
            Some(hook),
        )
        .await
        .unwrap();
        assert_eq!(receipt.observation_watermark, 7);
        assert_eq!(
            receipt.calibration_job_id.as_deref(),
            Some("durable-calibration-job")
        );
        assert!(!receipt.replayed);
        let mut stored=repo.db().query("SELECT VALUE terminal_receipt_json FROM ONLY type::record('atlas_classifier_run',$run)").bind(("run",run.clone())).await.unwrap().check().unwrap();
        let bytes = stored.take::<Option<String>>(0).unwrap().unwrap();
        let forbidden: admission::TerminalHook = Arc::new(|_tx, _terminal, _now| {
            Box::pin(async { panic!("durable replay ran terminal enqueue twice") })
        });
        let replay = admission::stop_run_with_hook(
            &owner,
            &claim,
            state,
            &cursor,
            &stats,
            "changed replay note",
            now,
            Some(forbidden),
        )
        .await
        .unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.terminal_revision, receipt.terminal_revision);
        assert_eq!(replay.observation_watermark, 7);
        assert_eq!(replay.calibration_job_id, receipt.calibration_job_id);
        let mut stored=repo.db().query("SELECT VALUE terminal_receipt_json FROM ONLY type::record('atlas_classifier_run',$run);SELECT VALUE writes FROM ONLY type::record('terminal_probe',$run);").bind(("run",run)).await.unwrap().check().unwrap();
        assert_eq!(stored.take::<Option<String>>(0).unwrap().unwrap(), bytes);
        assert_eq!(stored.take::<Option<i64>>(1).unwrap(), Some(1));
    }
    assert_eq!(count.load(Ordering::SeqCst), 4);
    owner.shutdown().await.unwrap();
    home.shutdown().await.unwrap();
}

#[tokio::test]
async fn foreground_intent_is_expiring_epoch_fenced_and_scoped_to_its_repository() {
    let directory = tempfile::tempdir().unwrap();
    let repo = Repository::connect(
        &Target::Embedded {
            path: directory.path().join("storage"),
        },
        "argos",
        "foreground_handoff",
    )
    .await
    .unwrap();
    let other = Repository::connect(
        &Target::Embedded {
            path: directory.path().join("other-storage"),
        },
        "argos",
        "other_foreground_handoff",
    )
    .await
    .unwrap();
    let owner = admission::AdmissionOwner::new(repo.clone());
    let now: DateTime<Utc> = "2026-11-05T00:00:00Z".parse().unwrap();
    assert!(!admission::foreground_requested(&repo, now).await.unwrap());
    let first = admission::announce_foreground_intent(&owner, "first", now)
        .await
        .unwrap();
    assert!(admission::foreground_requested(&repo, now).await.unwrap());
    assert!(!admission::foreground_requested(&other, now).await.unwrap());
    assert!(
        admission::renew_foreground_intent(&owner, &first, now + Duration::seconds(1))
            .await
            .unwrap()
    );
    assert!(
        admission::foreground_requested(&repo, now + Duration::seconds(5))
            .await
            .unwrap()
    );
    assert!(
        !admission::foreground_requested(&repo, now + Duration::seconds(6))
            .await
            .unwrap()
    );
    assert!(
        !admission::renew_foreground_intent(&owner, &first, now + Duration::seconds(6))
            .await
            .unwrap()
    );
    let replacement =
        admission::announce_foreground_intent(&owner, "first", now + Duration::seconds(7))
            .await
            .unwrap();
    assert!(replacement.epoch() > first.epoch());
    admission::clear_foreground_intent(&owner, &first)
        .await
        .unwrap();
    assert!(
        admission::foreground_requested(&repo, now + Duration::seconds(7))
            .await
            .unwrap()
    );
    let sibling =
        admission::announce_foreground_intent(&owner, "second", now + Duration::seconds(7))
            .await
            .unwrap();
    admission::clear_foreground_intent(&owner, &replacement)
        .await
        .unwrap();
    assert!(
        admission::foreground_requested(&repo, now + Duration::seconds(7))
            .await
            .unwrap()
    );
    admission::clear_foreground_intent(&owner, &sibling)
        .await
        .unwrap();
    assert!(
        !admission::foreground_requested(&repo, now + Duration::seconds(7))
            .await
            .unwrap()
    );
    owner.shutdown().await.unwrap();
}

#[cfg(feature = "classifier")]
#[test]
fn retained_capture_keys_bind_provenance_native_revision_and_preserve_original_bytes() {
    let row = article("rü|n:one", "a文|:two");
    let english = admission::StageProvenance::new(
        Some("en".into()),
        vec!["observation-z".into(), "observation-a".into()],
    )
    .unwrap();
    let original = admission::RetainedSourceFence::new(&row, 7, &english).unwrap();
    let old_bytes = serde_json::to_vec(&original).unwrap();
    let retry: admission::RetainedSourceFence = serde_json::from_slice(&old_bytes).unwrap();
    assert_eq!(original.capture_revision(), retry.capture_revision());
    assert_eq!(
        admission::maintenance_item_key("job", &original),
        admission::maintenance_item_key("job", &retry)
    );
    let french = admission::StageProvenance::new(
        Some("fr".into()),
        vec!["observation-a".into(), "observation-z".into()],
    )
    .unwrap();
    let changed = admission::RetainedSourceFence::new(&row, 7, &french).unwrap();
    assert_eq!(
        original.classifier_input_hash,
        changed.classifier_input_hash
    );
    assert_eq!(original.source_hash, changed.source_hash);
    assert_ne!(original.capture_revision(), changed.capture_revision());
    assert_ne!(
        admission::maintenance_item_key("job", &original),
        admission::maintenance_item_key("job", &changed)
    );
    let mut new_provider = row.clone();
    new_provider.provider = "another-fixture-provider".into();
    new_provider.published_at = "2026-02-03T04:05:06Z".into();
    let changed_provider = admission::RetainedSourceFence::new(&new_provider, 7, &english).unwrap();
    assert_eq!(
        original.classifier_input_hash,
        changed_provider.classifier_input_hash
    );
    assert_eq!(original.source_hash, changed_provider.source_hash);
    assert_ne!(
        original.capture_revision(),
        changed_provider.capture_revision()
    );
    let observations =
        admission::StageProvenance::new(Some("en".into()), vec!["new-observation".into()]).unwrap();
    let changed_observations = admission::RetainedSourceFence::new(&row, 7, &observations).unwrap();
    assert_eq!(
        original.classifier_input_hash,
        changed_observations.classifier_input_hash
    );
    assert_ne!(
        original.capture_revision(),
        changed_observations.capture_revision()
    );
    let next_native = admission::RetainedSourceFence::new(&row, 8, &english).unwrap();
    assert_ne!(original.capture_revision(), next_native.capture_revision());
    assert_eq!(old_bytes, serde_json::to_vec(&original).unwrap());
    assert_eq!(original.capture_revision().len(), 64);
}

#[cfg(feature = "classifier")]
async fn insert_legacy_staging(repo: &Repository, run: &str, prefix: &str, count: usize) {
    for first in (0..count).step_by(64) {
        let rows: Vec<_> = (first..count.min(first + 64))
            .map(|index| article(run, &format!("{prefix}-{index:04}")))
            .collect();
        let mut sql = String::new();
        for index in 0..rows.len() {
            sql.push_str(&format!("CREATE type::record('atlas_admission',$key{index}) SET run_id=$run,article_id=$article{index},payload_json=$payload{index},input_hash=$hash{index},source_revision='fixture-source',state='pending',taxonomy_version='atlas-taxonomy-v2',created_at=$now,updated_at=$now RETURN NONE;"));
        }
        let mut query = repo
            .db()
            .query(sql)
            .bind(("run", run.to_owned()))
            .bind(("now", Utc::now().to_rfc3339()));
        for (index, row) in rows.iter().enumerate() {
            query = query
                .bind((
                    format!("key{index}"),
                    argos_osint_core::atlas_classify::OccurrenceKey::new(run, &row.id).record_id(),
                ))
                .bind((format!("article{index}"), row.id.clone()))
                .bind((
                    format!("payload{index}"),
                    serde_json::to_string(row).unwrap(),
                ))
                .bind((
                    format!("hash{index}"),
                    argos_osint_core::atlas_metadata::input_hash(row),
                ));
        }
        query.await.unwrap().check().unwrap();
    }
}

#[cfg(feature = "classifier")]
async fn commit_header_groups(
    owner: &admission::AdmissionOwner,
    run: &admission::RunClaim,
    context: &argos_osint_core::atlas_classify::AdmissionContext,
    claims: &[admission::AdmissionClaim],
    cache: &mut argos_osint_core::atlas_classify::ComputationCache,
    clock_offset: i64,
) {
    use argos_osint_core::storage::classifier_admission::ItemCommit;
    for claims in claims.chunks(64) {
        let now = Utc::now() + Duration::seconds(clock_offset);
        assert!(admission::heartbeat_run(owner, run, now).await.unwrap());
        let inputs = admission::bulk_inputs_current(owner, claims).await.unwrap();
        assert_eq!(inputs.len(), claims.len());
        let scored = argos_osint_core::atlas_classify::classify_page(
            context,
            inputs,
            cache,
            &argos_osint_core::classifier::WorkControl::new(),
        )
        .await
        .unwrap();
        let result = admission::commit_outcomes(
            owner,
            run,
            &scored.outcomes,
            Utc::now() + Duration::seconds(clock_offset),
        )
        .await
        .unwrap();
        assert_eq!(result.items.len(), claims.len());
        assert!(
            result
                .items
                .iter()
                .all(|result| matches!(result, ItemCommit::Committed { .. })),
            "every owned sibling must commit exactly once: {:?}",
            result.items
        );
    }
}

/// Mirrors the foreground cycle's independent lease renewal while exercising
/// real storage work; large fixture pages must not depend on host speed.
#[cfg(feature = "classifier")]
fn keep_run_alive(
    owner: admission::AdmissionOwner,
    run: admission::RunClaim,
    clock_offset: i64,
) -> (
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let (stop, mut stopped) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let mut timer =
            tokio::time::interval(std::time::Duration::from_secs(admission::HEARTBEAT_SECS));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                biased;
                _ = &mut stopped => break,
                _ = timer.tick() => {
                    assert!(admission::heartbeat_run(&owner, &run,
                        Utc::now() + Duration::seconds(clock_offset)).await.unwrap(),
                        "foreground lease must remain owned during the fixture workload");
                }
            }
        }
    });
    (stop, task)
}

/// Real 512-header streaming and 64-result commits. A second interrupted
/// activation proves that its unexamined 448 claims survive a physical reopen.
#[cfg(feature = "classifier")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn headers_stream_1025_as_512_512_1_and_reopen_recovers_unexamined_claims() {
    use argos_osint_core::atlas_classify::{AdmissionContext, ComputationCache, OccurrenceKey};
    let _serial = admission_fixture::serial().await;
    let fixture = admission_fixture::install();
    let path = fixture.directory.path().join("stream-storage");
    let target = Target::Embedded { path: path.clone() };
    let model;
    let prior_first_receipt;
    {
        let repo = Repository::connect(&target, "argos", "classifier_header_stream")
            .await
            .unwrap();
        insert_legacy_staging(&repo, "run-a", "initial", 1025).await;
        insert_legacy_staging(&repo, "run-b", "isolated", 512).await;
        let owner = admission::AdmissionOwner::new(repo.clone());
        let permit =
            readiness::preflight_cycle(&fixture.home, CycleRequirement::FreshAcquisition, None)
                .await
                .unwrap();
        let pin = permit.pinned().unwrap().clone();
        model = pin.snapshot.model_id.clone();
        let home = fixture.home.home().to_path_buf();
        let context = tokio::task::spawn_blocking(move || AdmissionContext::load(&home, pin))
            .await
            .unwrap()
            .unwrap();
        let stats = RunStats {
            classifier_snapshot: Some(context.pin.snapshot.clone()),
            ..Default::default()
        };
        let stats = serde_json::to_string(&stats).unwrap();
        let cursor = serde_json::to_string(&Cursor::default()).unwrap();
        let run = admission::start_run(
            &owner,
            &permit,
            &StartRunRequest::fresh("run-a", "before-crash", &cursor, &stats, Utc::now()),
        )
        .await
        .unwrap();
        let (stop_heartbeat, heartbeat) = keep_run_alive(owner.clone(), run.clone(), 0);
        let mut upgraded = 0;
        loop {
            let count = admission::upgrade_legacy_page(&owner, &run, Utc::now())
                .await
                .unwrap();
            upgraded += count;
            if count == 0 {
                break;
            }
            assert!(admission::heartbeat_run(&owner, &run, Utc::now())
                .await
                .unwrap());
        }
        assert_eq!(upgraded, 1025);
        let mut token = admission::begin_admission(&owner, &run, &context, false, Utc::now())
            .await
            .unwrap();
        let mut sizes = Vec::new();
        let mut cache = ComputationCache::default();
        loop {
            let page = admission::claim_header_page(&owner, &run, &token, Utc::now())
                .await
                .unwrap();
            sizes.push(page.claims.len());
            assert!(page
                .claims
                .iter()
                .all(|claim| claim.occurrence.run_id == "run-a"));
            commit_header_groups(&owner, &run, &context, &page.claims, &mut cache, 0).await;
            let Some(next) = page.next else {
                break;
            };
            token = next;
        }
        assert_eq!(sizes, [512, 512, 1]);
        let summary = admission::run_summary(&repo, "run-a").await.unwrap();
        assert_eq!(summary.staged, 0);
        assert_eq!(summary.admitted + summary.rejected, 1025);
        let mut other=repo.db().query("RETURN count(SELECT id FROM atlas_admission WHERE run_id='run-b' AND state='pending' AND work_seq=0);").await.unwrap().check().unwrap();
        assert_eq!(other.take::<Option<i64>>(0).unwrap(), Some(512));
        // A new 512-header activation crashes after its first durable subgroup.
        insert_legacy_staging(&repo, "run-a", "crash", 512).await;
        loop {
            let count = admission::upgrade_legacy_page(&owner, &run, Utc::now())
                .await
                .unwrap();
            if count == 0 {
                break;
            }
            assert!(admission::heartbeat_run(&owner, &run, Utc::now())
                .await
                .unwrap());
        }
        let token = admission::begin_admission(&owner, &run, &context, true, Utc::now())
            .await
            .unwrap();
        let page = admission::claim_header_page(&owner, &run, &token, Utc::now())
            .await
            .unwrap();
        assert_eq!(page.claims.len(), 512);
        commit_header_groups(&owner, &run, &context, &page.claims[..64], &mut cache, 0).await;
        let mut checkpoint = repo
            .db()
            .query("SELECT VALUE page_json FROM ONLY type::record('atlas_classifier_run',$run)")
            .bind(("run", "run-a"))
            .await
            .unwrap()
            .check()
            .unwrap();
        let checkpoint: admission::AdmissionPageToken =
            serde_json::from_str(&checkpoint.take::<Option<String>>(0).unwrap().unwrap()).unwrap();
        assert_eq!(
            checkpoint.after_seq, page.claims[63].work_seq,
            "durable cursor must not jump to the claimed 512-header end"
        );
        let view =
            admission::classification_view(&repo, &OccurrenceKey::new("run-a", "crash-0000"))
                .await
                .unwrap();
        prior_first_receipt = serde_json::to_vec(&view.receipt.unwrap()).unwrap();
        stop_heartbeat.send(()).unwrap();
        heartbeat.await.unwrap();
        owner.shutdown().await.unwrap();
        repo.publisher().shutdown().await;
        // No stop/release operation: these are persisted unexpired crash claims.
        drop(page);
        drop(context);
        drop(permit);
        drop(owner);
        drop(repo);
    }
    let repo = Repository::connect(&target, "argos", "classifier_header_stream")
        .await
        .unwrap();
    let owner = admission::AdmissionOwner::new(repo.clone());
    let stored = argos_osint_core::storage::atlas::list_runs(repo.db())
        .await
        .unwrap()
        .into_iter()
        .find(|run| run.id == "run-a")
        .unwrap();
    let stored_stats: RunStats = serde_json::from_str(&stored.stats_json).unwrap();
    let permit = readiness::preflight_cycle(
        &fixture.home,
        CycleRequirement::AcquisitionAndAdmission,
        stored_stats.classifier_snapshot.as_ref(),
    )
    .await
    .unwrap();
    let pin = permit.pinned().unwrap().clone();
    assert_eq!(pin.snapshot.model_id, model);
    let home = fixture.home.home().to_path_buf();
    let context = tokio::task::spawn_blocking(move || AdmissionContext::load(&home, pin))
        .await
        .unwrap()
        .unwrap();
    let revision = admission::run_revision(&repo, "run-a").await.unwrap();
    let now = Utc::now() + Duration::seconds(31);
    let run = admission::start_run(
        &owner,
        &permit,
        &StartRunRequest::fresh(
            "run-a",
            "after-crash",
            &stored.cursor_json,
            &stored.stats_json,
            now,
        )
        .resume(revision.revision, revision.work_generation),
    )
    .await
    .unwrap();
    let (stop_heartbeat, heartbeat) = keep_run_alive(owner.clone(), run.clone(), 31);
    let token = admission::begin_admission(&owner, &run, &context, true, now)
        .await
        .unwrap();
    let page = admission::claim_header_page(&owner, &run, &token, now)
        .await
        .unwrap();
    assert_eq!(page.claims.len(), 448);
    assert!(page
        .claims
        .iter()
        .all(|claim| claim.occurrence.article_id.as_str() >= "crash-0064"));
    commit_header_groups(
        &owner,
        &run,
        &context,
        &page.claims,
        &mut ComputationCache::default(),
        31,
    )
    .await;
    let view = admission::classification_view(&repo, &OccurrenceKey::new("run-a", "crash-0000"))
        .await
        .unwrap();
    assert_eq!(
        prior_first_receipt,
        serde_json::to_vec(&view.receipt.unwrap()).unwrap()
    );
    let summary = admission::run_summary(&repo, "run-a").await.unwrap();
    assert_eq!(summary.staged, 0);
    assert_eq!(summary.admitted + summary.rejected, 1537);
    let mut counts=repo.db().query("RETURN count(SELECT id FROM atlas_classification_receipt WHERE run_id='run-a');RETURN count(SELECT id FROM atlas_admission WHERE run_id='run-b' AND work_seq=0 AND state='pending');").await.unwrap().check().unwrap();
    assert_eq!(counts.take::<Option<i64>>(0).unwrap(), Some(1537));
    assert_eq!(counts.take::<Option<i64>>(1).unwrap(), Some(512));
    stop_heartbeat.send(()).unwrap();
    heartbeat.await.unwrap();
    owner.shutdown().await.unwrap();
    drop(context);
    drop(permit);
    fixture.home.shutdown().await.unwrap();
    repo.publisher().shutdown().await;
}
