//! Real metadata Link generation, semantic decisions and canonical publication.
#![cfg(feature = "surreal")]

#[path = "support/actor_catalog.rs"]
mod actors;
#[path = "support/current_admission.rs"]
mod current_admission;
mod support;

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::Result;
use argos_osint_core::intel_recon::brief_fidelity::FidelityItem;
use argos_osint_core::intel_recon::brief_pipeline::BriefInputs;
use argos_osint_core::intel_recon::brief_stages::{
    self, BoxFut, ExecConfig, StageBackend, StageContext, StageOutcome, StageRequest,
};
use argos_osint_core::intel_recon::clean_body::CleanArtifact;
use argos_osint_core::memory_metadata::MemoryPayload;
use argos_osint_core::nlp::cache::hash;
use argos_osint_core::nlp::Source;
use argos_osint_core::storage::pipeline_inputs::{app_state_set, put_article_body, register};
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::storage::{atlas, brief, brief_publish, brief_stages as stages};
use argos_osint_core::store::AtlasArticleRow;
use serde_json::{json, Value};
use support::Server;
use surrealdb::types::RecordId;

fn article() -> AtlasArticleRow {
    AtlasArticleRow {
        run_id: "run1".into(),
        id: "a1".into(),
        title: "Port strike widens".into(),
        description: "Dockworkers walk out across the coast.".into(),
        url: "https://example.test/a1".into(),
        country: "US".into(),
        source_name: "Example".into(),
        source_domain: "example.test".into(),
        published_at: "2026-10-22T00:00:00Z".into(),
        provider: "p".into(),
        temperature: 0.0,
        category: "labor".into(),
        seen_at: "2026-10-22T01:00:00Z".into(),
        author: String::new(),
        image_url: String::new(),
        categories: Vec::new(),
        admitted_at: String::new(),
    }
}

struct Fixture {
    repo: Repository,
    body: Value,
    claims: Value,
    inferences: Value,
    source: AtlasArticleRow,
}

impl Fixture {
    async fn new(repo: Repository) -> Self {
        let mut a = article();
        a.title = "Port closure reroutes coastal shipping traffic.".into();
        a.description = "The closure has redirected ships to another harbor.".into();
        atlas::upsert_article(repo.db(), &a).await.unwrap();
        // A complete retained body acquisition for a1.
        let sentence = "Port closure reroutes coastal shipping traffic as dockworkers walk out across the coast. ";
        let prose = sentence.repeat(12);
        let prose = prose.trim_end().to_string();
        let raw = format!("<html>{prose}</html>");
        put_article_body(
            &repo,
            &argos_osint_core::intel_recon::ArticleBodyRow {
                id: "b1".into(),
                article_id: "a1".into(),
                run_id: "run1".into(),
                original_url: a.url.clone(),
                resolved_url: a.url.clone(),
                source_domain: a.source_domain.clone(),
                source_name: a.source_name.clone(),
                body_markdown: prose.clone(),
                content_hash: hash(&prose),
                quality: "complete".into(),
                quality_rationale: String::new(),
                body_version: 1,
                fetch_tool: "fetch".into(),
                fetched_at: "2026-10-22T02:00:00Z".into(),
                retry_after: String::new(),
                state: "ready".into(),
                created_at: "2026-10-22T02:00:00Z".into(),
                updated_at: "2026-10-22T02:00:00Z".into(),
            },
        )
        .await
        .unwrap();
        let source = Source {
            source_id: "b1".into(),
            revision: "1".into(),
            attempt_id: "att1".into(),
            raw_hash: Some(hash(&raw)),
            prose_hash: hash(&prose),
            prose: prose.clone(),
            language_hint: None,
            published_at: None,
            retrieved_at: "2026-10-22T02:00:00Z".into(),
        };
        register(
            &repo,
            "article",
            "a1",
            &source,
            Some(&raw),
            "validated-baseline-v1",
        )
        .await
        .unwrap();
        app_state_set(
        &repo,
        "article-acquisition:att1",
        &json!({"artifact_hash": hash(&raw), "complete_story": true, "known_truncation": false})
            .to_string(),
    )
    .await
    .unwrap();

        register(
            &repo,
            "article",
            "a1",
            &source,
            Some(&raw),
            argos_osint_core::intel_recon::clean_body::TRANSFORM,
        )
        .await
        .unwrap();
        let body = json!({"markdown":prose, "content_hash":hash(&prose)});
        let claims = json!({"claims":[{"id":"t1","fidelity":"faithful","subject_id":"e1","predicate":"reroutes","object":"coastal shipping traffic","text":a.title,"evidence":[{"id":"t1","start_byte":0,"end_byte":a.title.len()}]}]});
        let sentence = brief_stages::sentences("b", &prose).remove(0);
        let inferences = json!({"body_hash":hash(&prose),"inferences":[{"id":"i1","fidelity":"faithful","text":"Port closure redirects coastal shipping traffic.","support":"supported","inference_kind":"implication","domains":[],"conditions":[],"limitations":["Only one retained source story"],"premise_claim_ids":["t1"],"body_evidence_ids":[sentence.id],"counterevidence_ids":[],"counterevidence":[],"why":"The retained story describes the redirected ships.","evidence":[{"id":sentence.id,"start_byte":sentence.start_byte,"end_byte":sentence.end_byte}]}]});
        Self {
            repo,
            body,
            claims,
            inferences,
            source: a,
        }
    }
    async fn target(&self, id: &str, published: &str) -> AtlasArticleRow {
        let mut target = self.source.clone();
        target.id = id.into();
        target.run_id = "later-run".into();
        target.title = "Shipping traffic shifts after port closure.".into();
        target.description = "Ships use a different harbor while the port is closed.".into();
        target.url = format!("https://independent.test/{id}");
        target.source_domain = "independent.test".into();
        target.source_name = "Another publisher".into();
        target.published_at = published.into();
        atlas::upsert_article(self.repo.db(), &target)
            .await
            .unwrap();
        target
    }
    async fn run(&self, revision: &str, backend: &mut Backend) -> String {
        let inputs = BriefInputs {
            article_id: self.source.id.clone(),
            input_revision: revision.into(),
            title: self.source.title.clone(),
            metadata: brief_stages::metadata_snapshot(&self.source),
            role_snapshot: serde_json::to_string(
                &argos_osint_core::provider::SettingsFile::default().defaults,
            )
            .unwrap(),
            ..Default::default()
        };
        let chain = stages::ensure(&self.repo, &inputs).await.unwrap();
        // The fixture supplies completed prerequisite receipts; Links itself is
        // claimed, executed, assessed and published through the real worker.
        for (op, output) in [
            ("intel_brief_body", &self.body),
            ("intel_brief_claims", &self.claims),
            ("intel_brief_inferences", &self.inferences),
        ] {
            let id = stages::task_id(&chain.job_id, op);
            self.repo.db().query("UPDATE $task SET state='completed', output_hash=$hash; UPSERT $output SET task=$task, root=$job, stage=$stage, output_hash=$hash, output_json=$json, committed_at=time::now();")
                .bind(("task",RecordId::new("task",id.clone())))
                .bind(("output",RecordId::new("stage_output",id)))
                .bind(("job",RecordId::new("job",chain.job_id.clone())))
                .bind(("stage",op.to_string())).bind(("hash",hash(&output.to_string()))).bind(("json",output.to_string()))
                .await.unwrap().check().unwrap();
        }
        for op in ["intel_brief_claims", "intel_brief_inferences"] {
            let prepared = brief_publish::prepare(
                &self.repo,
                &self.source.id,
                &self.body,
                &self.claims,
                &self.inferences,
                op,
            )
            .await
            .unwrap();
            assert!(prepared.is_ok(), "{prepared:?}");
            let state = brief_publish::publish_stage(
                &self.repo,
                &self.source.id,
                &chain.job_id,
                op,
                &prepared,
                op,
            )
            .await
            .unwrap();
            assert_eq!(state.state, "indexed", "{state:?}");
            brief::put_stage_receipt(&self.repo, &chain.job_id, op, &state)
                .await
                .unwrap();
            brief::put_index_state(&self.repo, &self.source.id, &state)
                .await
                .unwrap();
        }
        let config = ExecConfig::default();
        let (leased, claimed) = stages::claim(&self.repo, "curated-fixture", &config)
            .await
            .unwrap()
            .expect("Links is ready");
        assert_eq!(claimed.operation, "intel_brief_links");
        let outcome = stages::run_claimed(
            &self.repo,
            leased,
            claimed,
            backend,
            &Arc::new(AtomicBool::new(false)),
            &config,
        )
        .await
        .unwrap();
        let expected_disposition = if revision == "failed-clock" {
            "failed"
        } else {
            "committed"
        };
        assert!(
            matches!(
                (&outcome, expected_disposition),
                (StageOutcome::Committed { .. }, "committed")
                    | (StageOutcome::Failed { .. }, "failed")
            ),
            "{outcome:?}"
        );
        let mut timing = self.repo.db().query("SELECT payload_json FROM telemetry_event WHERE job_id=$job AND mode='intel_brief_links'")
            .bind(("job",chain.job_id.clone())).await.unwrap().check().unwrap();
        let rows: Vec<Value> = timing.take(0).unwrap();
        assert_eq!(
            rows.len(),
            1,
            "real Surreal execution records one measured disposition"
        );
        let timing: Value =
            serde_json::from_str(rows[0]["payload_json"].as_str().unwrap()).unwrap();
        assert_eq!(timing["outcome"], expected_disposition);
        assert!(timing["local_us"].is_u64());
        assert!(timing["persistence_us"].is_u64());
        assert!(timing["queue_us"].is_null() && timing["rss_bytes"].is_null());
        chain.job_id
    }
    async fn links(&self) -> Vec<(String, String, MemoryPayload)> {
        let mut result=self.repo.db().query("SELECT meta::id(id) AS id, lifecycle, payload_json FROM memory WHERE memory_kind='link' ORDER BY id").await.unwrap().check().unwrap();
        let rows: Vec<Value> = result.take(0).unwrap();
        rows.into_iter()
            .map(|row| {
                (
                    row["id"].as_str().unwrap().to_string(),
                    row["lifecycle"].as_str().unwrap().to_string(),
                    serde_json::from_str(row["payload_json"].as_str().unwrap()).unwrap(),
                )
            })
            .collect()
    }
}

struct Backend {
    target: AtlasArticleRow,
    kind: &'static str,
    assessed: usize,
    mutate: Option<Repository>,
}
impl StageBackend for Backend {
    fn prepare_body<'a>(
        &'a mut self,
        _: &'a StageContext,
        _: &'a AtlasArticleRow,
        _: bool,
        _: &'a Arc<AtomicBool>,
    ) -> BoxFut<'a, Result<CleanArtifact>> {
        Box::pin(async { anyhow::bail!("fixture must not dispatch Body") })
    }
    fn related(&mut self, _: &AtlasArticleRow) -> Result<Vec<AtlasArticleRow>> {
        Ok(vec![self.target.clone()])
    }
    fn assess<'a>(
        &'a mut self,
        _: &'a StageContext,
        items: Vec<FidelityItem>,
        _: &'a Arc<AtomicBool>,
    ) -> BoxFut<'a, Result<HashMap<String, String>>> {
        self.assessed += items.len();
        Box::pin(async move {
            assert!(items.iter().all(|item| item.kind == "curated_link"));
            if let Some(repo) = self.mutate.take() {
                let mut changed = self.target.clone();
                changed.title = "Corrected: the earlier closure report was withdrawn.".into();
                atlas::upsert_article(repo.db(), &changed).await?;
            }
            Ok(items
                .into_iter()
                .map(|item| (item.id, "faithful".into()))
                .collect())
        })
    }
    fn generate<'a>(
        &mut self,
        request: StageRequest,
        _: &'a Arc<AtomicBool>,
    ) -> BoxFut<'a, Result<String>> {
        if self.kind == "provider-failure-fixture" {
            return Box::pin(async {
                anyhow::bail!("offline provider fixture failed before a complete answer")
            });
        }
        let raw=json!({"leads":[{"article_id":"r1","inference_id":"i1","metadata_evidence_ids":["r1.t1"],"why":"The port closure redirects shipping traffic through another harbor.","verification_needed":"Inspect the independent full story before corroboration.","proposed_relation":self.kind,"chronology_basis":"publication","conditions":["If the reported port closure continues"],"limitations":["Target evidence is metadata only"]}]}).to_string();
        Box::pin(async move {
            use argos_osint_core::intel_recon::brief_timing::{self, Phase};
            let clock =
                brief_timing::current_clock().expect("real storage stage installs its own clock");
            tokio::spawn(brief_timing::with_clock(clock, async move {
                brief_timing::timed(Phase::Validation, async move {
                    (request.validator)(&raw).map_err(anyhow::Error::msg)?;
                    Ok(raw)
                })
                .await
            }))
            .await?
        })
    }
}

#[tokio::test]
async fn metadata_curator_publishes_four_kinds_but_never_claims_independent_support() {
    let Some(server) = Server::start() else {
        return;
    };
    for (index, kind, published, expected) in [
        (0, "contextualizes", "2026-10-23T00:00:00Z", true),
        (1, "precedes", "2026-10-23T00:00:00Z", true),
        (2, "updates", "2026-10-21T00:00:00Z", true),
        (3, "potential_impact", "2026-10-23T00:00:00Z", true),
        (4, "supports", "2026-10-23T00:00:00Z", false),
        (5, "contradicts", "2026-10-23T00:00:00Z", false),
        (6, "precedes", "2026-10-22T00:00:00Z", false),
        (7, "precedes", "2026-10-21T00:00:00Z", false),
        (8, "updates", "", false),
    ] {
        let fixture = Fixture::new(server.repo("argos", &format!("curated_{index}")).await).await;
        let target = fixture.target("target", published).await;
        let mut backend = Backend {
            target,
            kind,
            assessed: 0,
            mutate: None,
        };
        fixture.run("first", &mut backend).await;
        let links = fixture.links().await;
        assert_eq!(links.len(), usize::from(expected), "{kind}: {links:?}");
        assert_eq!(
            backend.assessed,
            usize::from(expected),
            "deterministic abstentions must not dispatch semantic validation"
        );
        if let Some((_, lifecycle, MemoryPayload::Link(link))) = links.first() {
            assert_eq!(lifecycle, "active");
            assert_eq!(link.relation, kind);
            let receipt = link.validation.as_ref().expect("current typed receipt");
            receipt.validate_structure().unwrap();
            assert!(!receipt.context_revision.is_empty());
            assert!(
                receipt
                    .evidence
                    .iter()
                    .any(|reference| reference.origin_root.is_empty()),
                "metadata cannot manufacture an independent acquisition root"
            );
            if kind == "potential_impact" {
                assert!(!receipt.conditions.is_empty());
                assert!(!receipt.limitations.is_empty());
            }
        }
    }
}

#[tokio::test]
async fn a_candidate_changed_during_semantic_validation_cannot_replace_a_live_confirmed_sibling() {
    let Some(server) = Server::start() else {
        return;
    };
    let fixture = Fixture::new(server.repo("argos", "curated_stale").await).await;
    let target = fixture.target("first", "2026-10-23T00:00:00Z").await;
    let mut backend = Backend {
        target,
        kind: "contextualizes",
        assessed: 0,
        mutate: None,
    };
    fixture.run("first", &mut backend).await;
    let original = fixture.links().await;
    assert_eq!(original.len(), 1, "{original:?}");
    let mut target = fixture.target("second", "2026-10-23T00:00:00Z").await;
    // This distinct assertion requires a new semantic decision. Identical
    // passages would correctly reuse the first decision before the race hook.
    target.title = "Port closure redirects shipping traffic toward an inland harbor.".into();
    atlas::upsert_article(fixture.repo.db(), &target)
        .await
        .unwrap();
    let mut backend = Backend {
        target,
        kind: "contextualizes",
        assessed: 0,
        mutate: Some(fixture.repo.clone()),
    };
    fixture.run("second", &mut backend).await;
    assert_eq!(
        backend.assessed, 1,
        "the correction occurs after actual semantic dispatch"
    );
    let current = fixture.links().await;
    assert_eq!(
        current, original,
        "stale new metadata must not publish or destroy the still-live confirmed sibling"
    );
}

#[tokio::test]
async fn real_failed_stage_records_its_partial_clock_without_publishing_a_link() {
    let Some(server) = Server::start() else {
        return;
    };
    let fixture = Fixture::new(server.repo("argos", "curated_failed_clock").await).await;
    let target = fixture.target("target", "2026-10-23T00:00:00Z").await;
    let mut backend = Backend {
        target,
        kind: "provider-failure-fixture",
        assessed: 0,
        mutate: None,
    };
    fixture.run("failed-clock", &mut backend).await;
    assert!(fixture.links().await.is_empty());
    assert_eq!(backend.assessed, 0);
}

#[tokio::test]
async fn concurrent_new_ready_body_fences_new_publication_without_retiring_historical_confirmed_link(
) {
    use argos_osint_core::storage::tx::{run_tx, RetryPolicy};
    use argos_osint_core::storage::{derivations, insight, live_evidence};
    use std::sync::atomic::Ordering;
    use tokio::sync::Notify;
    let Some(server) = Server::start() else {
        return;
    };
    let fixture = Fixture::new(server.repo("argos", "curated_body_phantom").await).await;
    let target = fixture
        .target("historical-sibling", "2026-10-23T00:00:00Z")
        .await;
    let mut backend = Backend {
        target,
        kind: "contextualizes",
        assessed: 0,
        mutate: None,
    };
    fixture.run("first", &mut backend).await;
    let original = fixture.links().await;
    assert_eq!(original.len(), 1);
    let prepared = Arc::new(
        brief_publish::prepare(
            &fixture.repo,
            &fixture.source.id,
            &fixture.body,
            &fixture.claims,
            &fixture.inferences,
            "intel_brief_inferences",
        )
        .await
        .unwrap()
        .unwrap(),
    );
    let mut epoch = fixture
        .repo
        .db()
        .query("RETURN type::record('atlas_body_input_guard','a1').rev")
        .await
        .unwrap()
        .check()
        .unwrap();
    let initial_epoch = epoch.take::<Option<i64>>(0).unwrap().unwrap();
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let first = Arc::new(AtomicBool::new(true));
    let publisher = {
        let (repo, prepared, entered, release, first) = (
            fixture.repo.clone(),
            prepared.clone(),
            entered.clone(),
            release.clone(),
            first.clone(),
        );
        tokio::spawn(async move {
            run_tx(repo.db(), RetryPolicy::default(), move |tx| {
                let (prepared, entered, release, first) = (
                    prepared.clone(),
                    entered.clone(),
                    release.clone(),
                    first.clone(),
                );
                async move {
                    let mut input = prepared.input.clone();
                    let mut output = prepared.output.clone();
                    let generation =
                        insight::advance_generation(&tx, "article", "a1", "2026-11-01T00:00:00Z")
                            .await?;
                    input.snapshot.generation = u32::try_from(generation).map_err(|_| {
                        argos_osint_core::storage::tx::TxError::Abort(
                            "fixture_generation_overflow".into(),
                        )
                    })?;
                    input.snapshot.key = "concurrent-body-publication".into();
                    for inference in &mut output.inferences {
                        inference.snapshot_key = input.snapshot.key.clone();
                    }
                    let snapshot = input.snapshot.clone();
                    let strong = live_evidence::publication_step(snapshot.clone());
                    let paused: insight::SnapshotStep = Arc::new(move |tx, key| {
                        let (strong, entered, release, first) = (
                            strong.clone(),
                            entered.clone(),
                            release.clone(),
                            first.clone(),
                        );
                        Box::pin(async move {
                            strong(tx, key).await?;
                            if first.swap(false, Ordering::AcqRel) {
                                entered.notify_one();
                                release.notified().await;
                            }
                            Ok(())
                        })
                    });
                    let write = derivations::derivation_step(
                        Arc::new(input),
                        Arc::new(output),
                        Some("inference"),
                        derivations::DerivationContext {
                            save_insights: true,
                            recon_thread: None,
                        },
                    );
                    insight::publish_snapshot_on(
                        &tx,
                        &snapshot,
                        &snapshot.key,
                        "active",
                        "2026-11-01T00:00:00Z",
                        &paused,
                        &write,
                    )
                    .await
                }
            })
            .await
        })
    };
    // The publisher has already checked the old ready head and written the
    // epoch guard, but has not committed its canonical snapshot/receipts.
    if tokio::time::timeout(std::time::Duration::from_secs(10), entered.notified())
        .await
        .is_err()
    {
        release.notify_one();
        panic!(
            "old head was not admitted by the real publisher: {:?}",
            publisher.await.unwrap()
        );
    }
    async fn refresh_body(
        repo: &Repository,
        committed: &std::sync::atomic::AtomicU64,
    ) -> Result<()> {
        let prose =
            "Updated port closure report redirects shipping traffic through another harbor. "
                .repeat(12);
        let raw = format!("<html>{prose}</html>");
        put_article_body(
            repo,
            &argos_osint_core::intel_recon::ArticleBodyRow {
                id: "newer-ready-body".into(),
                article_id: "a1".into(),
                run_id: "run1".into(),
                original_url: "https://example.test/a1".into(),
                resolved_url: "https://example.test/a1".into(),
                source_domain: "example.test".into(),
                source_name: "Example".into(),
                body_markdown: prose.clone(),
                content_hash: hash(&prose),
                quality: "complete".into(),
                quality_rationale: String::new(),
                body_version: 2,
                fetch_tool: "offline-refresh".into(),
                fetched_at: "2026-11-01T01:00:00Z".into(),
                retry_after: String::new(),
                state: "ready".into(),
                created_at: "2026-11-01T01:00:00Z".into(),
                updated_at: "2026-11-01T01:00:00Z".into(),
            },
        )
        .await?;
        committed.fetch_add(1, Ordering::AcqRel);
        let mut source = Source::derived("newer-ready-body", "2", &prose);
        source.attempt_id = "acquisition-refresh".into();
        source.raw_hash = Some(hash(&raw));
        register(
            repo,
            "article",
            "a1",
            &source,
            Some(&raw),
            "validated-baseline-v1",
        )
        .await?;
        committed.fetch_add(1, Ordering::AcqRel);
        register(
            repo,
            "article",
            "a1",
            &source,
            Some(&raw),
            argos_osint_core::intel_recon::clean_body::TRANSFORM,
        )
        .await?;
        committed.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }
    let committed = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let writer_entered = Arc::new(Notify::new());
    let writer = {
        let (repo, committed, writer_entered) = (
            fixture.repo.clone(),
            committed.clone(),
            writer_entered.clone(),
        );
        tokio::spawn(async move {
            writer_entered.notify_one();
            refresh_body(&repo, &committed).await
        })
    };
    writer_entered.notified().await;
    // Both real operations overlap. The database may serialize either one
    // first; the common guard must prevent an unvalidated phantom outcome.
    release.notify_one();
    let publication = publisher.await.unwrap();
    match writer.await.unwrap() {
        Ok(()) => {}
        Err(error)
            if matches!(
                error.downcast_ref::<argos_osint_core::storage::tx::TxError>(),
                Some(argos_osint_core::storage::tx::TxError::Contention { .. })
            ) =>
        {
            // This is an explicit fresh producer invocation after a reported
            // bounded contention result, not a hidden retry of uncertain work.
            refresh_body(&fixture.repo, &committed).await.unwrap();
        }
        Err(error) => panic!("unexpected producer failure: {error:#}"),
    }
    if let Err(error) = &publication {
        assert!(
            matches!(
                error,
                argos_osint_core::storage::tx::TxError::Contention { .. }
                    | argos_osint_core::storage::tx::TxError::Abort(_)
            ),
            "unexpected publisher failure: {error}"
        );
    }
    let mut epoch = fixture
        .repo
        .db()
        .query("RETURN type::record('atlas_body_input_guard','a1').rev")
        .await
        .unwrap()
        .check()
        .unwrap();
    assert_eq!(
        epoch.take::<Option<i64>>(0).unwrap().unwrap(),
        initial_epoch
            + committed.load(Ordering::Acquire) as i64
            + if publication
                .as_ref()
                .is_ok_and(|snapshot| snapshot.value.is_some())
            {
                1_i64
            } else {
                0_i64
            },
        "every successful producer and new publication joins the same operational epoch"
    );
    let retained = prepared.input.snapshot.clone();
    run_tx(fixture.repo.db(), RetryPolicy::default(), move |tx| {
        let retained = retained.clone();
        async move { live_evidence::validate_in(&tx, &retained).await }
    })
    .await
    .unwrap();
    let current = prepared.input.snapshot.clone();
    assert!(
        run_tx(fixture.repo.db(), RetryPolicy::default(), move |tx| {
            let current = current.clone();
            async move { live_evidence::validate_for_publication_in(&tx, &current).await }
        })
        .await
        .is_err()
    );
    let mut before = fixture
        .repo
        .db()
        .query("RETURN count(SELECT id FROM insight_snapshot)")
        .await
        .unwrap()
        .check()
        .unwrap();
    let before = before.take::<Option<i64>>(0).unwrap();
    let refused = brief_publish::publish_stage(
        &fixture.repo,
        &fixture.source.id,
        "new-head-first-publication",
        "intel_brief_inferences",
        &Ok((*prepared).clone()),
        "new-head-first-old-packet",
    )
    .await
    .unwrap();
    assert_eq!(refused.state, "failed");
    assert!(
        refused.reason.contains("current body acquisition changed"),
        "{refused:?}"
    );
    let mut after = fixture
        .repo
        .db()
        .query("RETURN count(SELECT id FROM insight_snapshot)")
        .await
        .unwrap()
        .check()
        .unwrap();
    assert_eq!(
        after.take::<Option<i64>>(0).unwrap(),
        before,
        "new-head-first refusal writes no snapshot/receipt"
    );
    assert_eq!(
        fixture.links().await,
        original,
        "a new acquisition alone does not retire a valid historical confirmed sibling"
    );
    if publication.is_err() {
        let mut receipt=fixture.repo.db().query("SELECT VALUE input_key FROM insight_snapshot WHERE input_key='concurrent-body-publication'").await.unwrap().check().unwrap();
        assert!(
            receipt.take::<Vec<String>>(0).unwrap().is_empty(),
            "a conflicted new publication writes no stale snapshot or receipt"
        );
    }
}

/// Offline fixture timings exercise actual scheduling, validation, checkpoint
/// and canonical publication. Scripted decisions are not quality evidence or
/// a measurement of remote model latency.
struct PipelineBenchmarkBackend {
    body: Value,
    target: AtlasArticleRow,
    calls: Arc<std::sync::Mutex<std::collections::BTreeMap<String, u64>>>,
}

impl PipelineBenchmarkBackend {
    fn count(&self, operation: &str) {
        *self
            .calls
            .lock()
            .unwrap()
            .entry(operation.into())
            .or_default() += 1;
    }
}

impl StageBackend for PipelineBenchmarkBackend {
    fn prepare_body<'a>(
        &'a mut self,
        _: &'a StageContext,
        _: &'a AtlasArticleRow,
        _: bool,
        _: &'a Arc<AtomicBool>,
    ) -> BoxFut<'a, Result<CleanArtifact>> {
        self.count("retained_body");
        let markdown = self.body["markdown"].as_str().unwrap().to_owned();
        Box::pin(async move {
            Ok(CleanArtifact {
                content_hash: hash(&markdown),
                baseline_hash: hash(&markdown),
                spans: vec![argos_osint_core::intel_recon::clean_body::SpanMap {
                    baseline_start: 0,
                    baseline_end: markdown.len(),
                    clean_start: 0,
                }],
                markdown,
                quality: "complete".into(),
                rationale: "Retained offline fixture acquisition".into(),
                removed: vec![],
                diagnostics: vec![],
                reviewed: 0,
                batches: 0,
                acquisition_complete: true,
                sentence_index: None,
            })
        })
    }

    fn assess<'a>(
        &'a mut self,
        _: &'a StageContext,
        items: Vec<FidelityItem>,
        _: &'a Arc<AtomicBool>,
    ) -> BoxFut<'a, Result<HashMap<String, String>>> {
        self.count("scripted_fidelity_batch");
        Box::pin(async move {
            Ok(items
                .into_iter()
                .map(|item| (item.id, "faithful".into()))
                .collect())
        })
    }

    fn classify_context<'a>(
        &mut self,
        _: &'a StageContext,
        batch: argos_osint_core::intel_recon::brief_context::ContextBatch,
        _: &'a Arc<AtomicBool>,
    ) -> BoxFut<'a, Result<HashMap<String, String>>> {
        self.count("scripted_context_batch");
        Box::pin(async move {
            Ok(batch
                .candidates
                .into_iter()
                .map(|item| (item.id, "support".into()))
                .collect())
        })
    }

    fn related(&mut self, _: &AtlasArticleRow) -> Result<Vec<AtlasArticleRow>> {
        self.count("fixture_related_lookup");
        Ok(vec![self.target.clone()])
    }

    fn generate<'a>(
        &mut self,
        request: StageRequest,
        _: &'a Arc<AtomicBool>,
    ) -> BoxFut<'a, Result<String>> {
        self.count(&request.stage);
        Box::pin(async move {
            let input: Value = serde_json::from_str(&request.user)?;
            let output = match request.stage.as_str() {
                "intel_brief_claims" => {
                    let evidence = input["TITLE"][0]["id"].as_str().ok_or_else(|| {
                        anyhow::anyhow!("benchmark claims request must offer a title sentence")
                    })?;
                    json!({"claims":[{
                        "id":"c1","text":"Port closure reroutes coastal shipping traffic.",
                        "subject_id":"e1","predicate":"reroutes","object":"coastal shipping traffic",
                        "attributed_to":"Example","modality":"asserted","polarity":"positive","time":"",
                        "evidence_ids":[evidence]}],"limit_reached":false,"empty_reason":""})
                }
                "intel_brief_inferences" => {
                    // Current Synthesis sees one claim and its selected context,
                    // rather than the older whole-article CLAIMS/body arrays.
                    let premise = input["CLAIM"]["id"].as_str().ok_or_else(|| {
                        anyhow::anyhow!("benchmark inference request must offer one claim")
                    })?;
                    let evidence = input["CONTEXT_SENTENCES"]
                        .as_array()
                        .and_then(|sentences| {
                            sentences
                                .iter()
                                .find(|sentence| sentence["relation"] == "support")
                        })
                        .and_then(|sentence| sentence["id"].as_str())
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "benchmark inference request must offer a supporting body sentence"
                            )
                        })?;
                    json!({"inferences":[{"id":"i1",
                        "text":"Continued port closure could redirect coastal shipping traffic.",
                        "premise_claim_ids":[premise],"body_evidence_ids":[evidence],
                        "support":"conditional","inference_kind":"conditional_outlook","domains":[],
                        "conditions":["The reported port closure continues"],
                        "why":"The retained account describes shipping rerouted by the closure.",
                        "limitations":["One fictional retained source story"],"counterevidence_ids":[]}],"empty_reason":""})
                }
                "intel_brief_links" => {
                    anyhow::ensure!(
                        input["INFERENCES"]
                            .as_array()
                            .is_some_and(|items| items.len() == 1),
                        "benchmark Links request must offer the published inference"
                    );
                    anyhow::ensure!(
                        input["CANDIDATES"].is_array(),
                        "benchmark Links request must carry its bounded candidate array"
                    );
                    json!({"leads":[],"empty_reason":"No independently verified corroboration in fixture"})
                }
                other => anyhow::bail!("unexpected benchmark generation {other}"),
            };
            let raw = output.to_string();
            (request.validator)(&raw).map_err(anyhow::Error::msg)?;
            Ok(raw)
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "manual offline Focus stage and repeat-visit benchmark"]
async fn offline_fixed_focus_benchmark() {
    use std::time::Instant;

    // The publisher runs on background tasks, so a thread-local dispatcher
    // would miss its original transaction error. This ignored exact-process
    // benchmark installs only a bounded publisher-warning collector.
    struct PublicationWarnings(Arc<std::sync::Mutex<Vec<String>>>);
    struct MessageVisitor(String);
    impl tracing::field::Visit for MessageVisitor {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            if field.name() == "message" {
                self.0 = format!("{value:?}").chars().take(2048).collect();
            }
        }
    }
    impl tracing::Subscriber for PublicationWarnings {
        fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
            metadata.is_event()
                && *metadata.level() == tracing::Level::WARN
                && metadata.target() == "argos_osint_core::storage::publisher"
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            let mut visitor = MessageVisitor(String::new());
            event.record(&mut visitor);
            let mut warnings = self.0.lock().unwrap();
            if warnings.len() < 16 {
                warnings.push(visitor.0);
            }
        }
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }
    let publication_warnings = Arc::new(std::sync::Mutex::new(Vec::new()));
    tracing::subscriber::set_global_default(PublicationWarnings(publication_warnings.clone()))
        .expect("Focus benchmark requires its isolated exact-test process");

    let server = Server::start().expect("required isolated embedded harness");
    let repo = server.repo("argos", "focus_fixed_performance").await;
    let mut fixture = Fixture::new(repo.clone()).await;
    // Establish the real native publication prerequisites before timing. The
    // unmatched catalog adds no actors to the fixed source/body workload.
    actors::seed_actor_catalog(&repo, &["Unmatched benchmark catalog actor"]).await;
    current_admission::admit(&repo, &mut fixture.source, "Ballot patrol common").await;
    let mut target = fixture.target("r1", "2026-10-23T00:00:00Z").await;
    current_admission::admit(&repo, &mut target, "Ballot patrol common").await;
    for article in [&fixture.source, &target] {
        let metadata = argos_osint_core::atlas_metadata::collect(
            article,
            None,
            Arc::new(AtomicBool::new(false)),
        )
        .await;
        assert_eq!(
            metadata.entity_state,
            argos_osint_core::atlas_metadata::MetadataState::Complete
        );
        assert!(
            argos_osint_core::storage::atlas_metadata::save_metadata(repo.db(), &metadata)
                .await
                .unwrap()
        );
    }
    // The retained cleaned body needs its actual native NLP receipt before
    // the actor publisher can consume it. Registration alone stores no features.
    let body_source = Source::derived("body:b1", "1", fixture.body["markdown"].as_str().unwrap());
    let body_context = body_source.context();
    let annotation = argos_osint_core::nlp::Service::shared()
        .annotate(
            body_source.clone(),
            body_context.clone(),
            Arc::new(AtomicBool::new(false)),
        )
        .await;
    assert_eq!(annotation.status, argos_osint_core::nlp::Status::Complete);
    assert!(annotation.validate(&body_source));
    argos_osint_core::storage::nlp_cache::put(&repo, &body_source, &body_context, &annotation)
        .await
        .unwrap();
    let calls = Arc::new(std::sync::Mutex::new(std::collections::BTreeMap::new()));
    let mut samples = Vec::new();
    for index in 0..6 {
        let inputs = BriefInputs {
            article_id: fixture.source.id.clone(),
            input_revision: format!("focus-fixed-performance-{index}"),
            title: fixture.source.title.clone(),
            metadata: brief_stages::metadata_snapshot(&fixture.source),
            role_snapshot: serde_json::to_string(
                &argos_osint_core::provider::SettingsFile::default().defaults,
            )
            .unwrap(),
            ..Default::default()
        };
        let started = Instant::now();
        let chain = stages::ensure(&repo, &inputs).await.unwrap();
        assert!(chain.created);
        let admitted_us = started.elapsed().as_micros();
        let ran = stages::drain_parallel(
            &repo,
            "focus-benchmark",
            || {
                Box::new(PipelineBenchmarkBackend {
                    body: fixture.body.clone(),
                    target: target.clone(),
                    calls: calls.clone(),
                })
            },
            &Arc::new(AtomicBool::new(false)),
            &ExecConfig::default(),
            2,
        )
        .await;
        if let Err(error) = &ran {
            eprintln!(
                "ARGOS_FOCUS_FAILURE_CONTEXT {}",
                json!({"iteration":index,"error":error.to_string(),"publisher_warnings":*publication_warnings.lock().unwrap()})
            );
            // Failure-only, bounded identities/hashes/status: never body or article prose.
            let mut diagnostic = repo.db().query(
                "SELECT record::id(id) AS body_id,body_version,state,content_hash,updated_at FROM article_body WHERE article_id=$article ORDER BY body_version DESC,updated_at DESC LIMIT 8;
                 SELECT source_id,revision,attempt_id,raw_hash,prose_hash,transform_version FROM nlp_source_ref WHERE owner_kind='article' AND owner_id=$article ORDER BY registered_at DESC LIMIT 16;
                 SELECT record::id(id) AS feature_key,prose_hash,crypto::sha256(annotations_json) AS annotation_sha256,IF bytes::len(type::bytes(annotations_json))<=131072 {annotations_json} ELSE {NONE} AS annotations_json FROM nlp_feature WHERE prose_hash IN (SELECT VALUE prose_hash FROM nlp_source_ref WHERE owner_kind='article' AND owner_id=$article LIMIT 16) LIMIT 16;
                 SELECT stage,state,last_error,output_revision,crypto::sha256(checkpoint_json) AS checkpoint_sha256,IF bytes::len(type::bytes(disposition_json))<=1024 {disposition_json} ELSE {NONE} AS disposition_json,IF bytes::len(type::bytes(captured_json))<=16384 {captured_json} ELSE {NONE} AS captured_json FROM atlas_classification_derivative WHERE run_id=$run AND article_id=$article ORDER BY stage LIMIT 8;
                 SELECT record::id(id) AS task_id,operation,state,input_hash,output_hash,error_category FROM task WHERE job=$job ORDER BY operation LIMIT 8;
                 SELECT record::id(job) AS job_id,generation,state,input_revision FROM brief_run WHERE article_id=$article ORDER BY generation DESC LIMIT 8;
                 SELECT record::id(id) AS key,crypto::sha256(value) AS value_sha256,IF bytes::len(type::bytes(value))<=4096 {value} ELSE {NONE} AS bounded_value FROM app_state WHERE string::starts_with(record::id(id),$clean_prefix) OR string::starts_with(record::id(id),'article-acquisition:') LIMIT 16;
                 SELECT record::id(id) AS artifact_hash,crypto::sha256(text) AS actual_hash,byte_len,string::trim(text)!='' AS nonempty FROM nlp_artifact WHERE record::id(id) IN (SELECT VALUE prose_hash FROM nlp_source_ref WHERE owner_kind='article' AND owner_id=$article LIMIT 16) LIMIT 16;"
            ).bind(("article",fixture.source.id.clone())).bind(("run",fixture.source.run_id.clone()))
                .bind(("job",RecordId::new("job",chain.job_id.clone())))
                .bind(("clean_prefix",format!("intel-clean-body:{}:",fixture.source.id)))
                .await.unwrap().check().unwrap();
            let mut snapshots = serde_json::Map::new();
            for (slot, label) in [
                "body_heads",
                "source_refs",
                "body_candidate_cache",
                "derivatives",
                "tasks",
                "brief_runs",
                "clean_and_acquisition_cache",
                "artifact_checks",
            ]
            .into_iter()
            .enumerate()
            {
                let mut rows: Vec<Value> = diagnostic.take(slot).unwrap();
                for row in &mut rows {
                    if let Some(object) = row.as_object_mut() {
                        if let Some(raw) = object.remove("annotations_json") {
                            let parsed = raw
                                .as_str()
                                .and_then(|value| serde_json::from_str::<Value>(value).ok())
                                .unwrap_or(Value::Null);
                            object.insert("annotation_identity".into(),json!({"schema_version":parsed["schema_version"],"feature_key":parsed["feature_key"],"prose_hash":parsed["prose_hash"],"status":parsed["status"]}));
                        }
                        if let Some(raw) = object.remove("bounded_value") {
                            let parsed = raw
                                .as_str()
                                .and_then(|value| serde_json::from_str::<Value>(value).ok())
                                .unwrap_or(Value::Null);
                            object.insert("cache_status".into(),json!({"complete_story":parsed["complete_story"],"known_truncation":parsed["known_truncation"],"content_hash":parsed["content_hash"],"baseline_hash":parsed["baseline_hash"],"quality":parsed["quality"],"acquisition_complete":parsed["acquisition_complete"]}));
                        }
                        if let Some(raw) = object.remove("captured_json") {
                            let parsed = raw
                                .as_str()
                                .and_then(|value| serde_json::from_str::<Value>(value).ok())
                                .unwrap_or(Value::Null);
                            object.insert("captured_identity".into(),json!({"body":parsed["body"],"metadata_sha256":parsed["metadata_sha256"],"admission_state":parsed["admission_state"],"source_hash":parsed["source"]["source_hash"],"classifier_input_hash":parsed["source"]["classifier_input_hash"]}));
                        }
                    }
                }
                snapshots.insert(label.into(), json!(rows));
            }
            eprintln!(
                "ARGOS_FOCUS_FAILURE {}",
                json!({"iteration":index,"error":error.to_string(),"primed_feature_key":annotation.feature_key,"primed_prose_hash":annotation.prose_hash,"actual":snapshots})
            );
        }
        let ran = ran.unwrap();
        assert_eq!(ran, 4);
        let total_us = started.elapsed().as_micros();
        let mut response = repo.db().query("SELECT mode,outcome,metric_ms,payload_json FROM telemetry_event WHERE event_type='intel_stage' AND job_id=$job ORDER BY mode")
            .bind(("job",chain.job_id.clone())).await.unwrap().check().unwrap();
        let rows: Vec<Value> = response.take(0).unwrap();
        assert_eq!(rows.len(), 4, "each actual stage must emit timing");
        assert!(
            rows.iter().all(|row| row["outcome"] == "committed"),
            "{rows:?}"
        );
        let published = brief::published_brief(&repo, &fixture.source.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(published.1["claims"].as_array().unwrap().len(), 1);
        assert_eq!(published.2["inferences"].as_array().unwrap().len(), 1);
        let before = calls.lock().unwrap().clone();
        let revisit = Instant::now();
        let cached = stages::ensure(&repo, &inputs).await.unwrap();
        let revisit_us = revisit.elapsed().as_micros();
        assert_eq!(cached.job_id, chain.job_id);
        assert!(!cached.created);
        assert_eq!(
            *calls.lock().unwrap(),
            before,
            "repeat visit cannot dispatch models"
        );
        samples.push(json!({"index":index,"first_process_use":index==0,"admission_us":admitted_us,"total_us":total_us,"revisit_us":revisit_us,"actual_stage_events":rows}));
    }
    println!(
        "ARGOS_FOCUS_RESULT {}",
        json!({"schema":"argos-fixed-focus-performance-v1",
        "body_sha256":fixture.body["content_hash"],"metadata_sha256":hash(&brief_stages::metadata_snapshot(&fixture.source).to_string()),
        "claims_per_generation":1,"inferences_per_generation":1,"scripted_calls":*calls.lock().unwrap(),"samples":samples,
        "measurement_scope":"Actual scheduling, validation, checkpoints and publication; retained body, precomputed real native NLP receipts and scripted decisions; setup excluded; no real-provider latency or model-quality claim",
        "unchanged_visit":"attaches to completed generation without model dispatch"})
    );
}
