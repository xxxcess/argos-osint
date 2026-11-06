//! Immutable brief reuse and explicit summary generation on isolated storage.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::brain_neighborhood_brief::{self as brief, BriefInput};
use argos_osint_core::brain_neighborhood_brief_exec::Delivery;
use argos_osint_core::brain_topology::{
    CycleScope, TopologyEdge, TopologyEdgeKind, TopologyNode, TopologyNodeKind, TopologySeed,
    TopologySnapshot,
};
use argos_osint_core::intel_recon::brief_pipeline::BriefInputs;
use argos_osint_core::storage::{brief_stages, neighborhood_brief};
use std::collections::BTreeMap;
use support::*;

#[tokio::test]
async fn navigation_is_read_only_and_explicit_generation_is_single_flight() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "summary_explicit").await;
    let node = |id: &str, root: bool| TopologyNode {
        id: id.into(),
        kind: TopologyNodeKind::Claim,
        label: "A retained evidence assertion".into(),
        detail: String::new(),
        subtype: String::new(),
        status: "recorded".into(),
        is_root: root,
        article_id: String::new(),
        run_id: String::new(),
        published_at: String::new(),
        cycle: None,
        member_ids: Vec::new(),
        evidence_count: 0,
        passages: Vec::new(),
        min_hops: usize::from(!root),
        revision: "metadata-v1".into(),
        predecessor: None,
        topology_seed: None,
    };
    let snapshot = TopologySnapshot {
        root_id: "c1".into(),
        root_label: "Retained assertion".into(),
        root_kind: Some(TopologyNodeKind::Claim),
        nodes: vec![node("c1", true), node("c2", false)],
        edges: vec![TopologyEdge {
            id: "premise-1".into(),
            from: "c1".into(),
            to: "c2".into(),
            kind: TopologyEdgeKind::Premise,
            label: "premise".into(),
            status: "recorded".into(),
            member_ids: Vec::new(),
            receipts: Vec::new(),
            rationale: String::new(),
            actor_reference: None,
        }],
        discovered_edges: 1,
        total_edges: 1,
        discovered_nodes: 2,
        total_nodes: 2,
        exploration_exhausted: true,
        ..Default::default()
    };
    let seed = TopologySeed::Claim("c1".into());
    let input = BriefInput {
        snapshot: &snapshot,
        seed: &seed,
        depth: 1,
        cycle: &CycleScope::AllRetained,
        payloads: BTreeMap::new(),
        payload_reads_skipped: 0,
    };
    let fallback = brief::build(&input);
    let packet = brief::build_input(&input, &fallback.basis_key);
    assert!(packet.worth_generating());
    for _ in 0..3 {
        assert!(matches!(
            neighborhood_brief::publish(&repo, &packet, &fallback, "test-route-v1")
                .await
                .unwrap()
                .1,
            Delivery::Idle
        ));
    }
    let mut response = repo
        .db()
        .query("RETURN count(SELECT id FROM task);")
        .await
        .unwrap()
        .check()
        .unwrap();
    let before: Option<i64> = response.take(0).unwrap();
    assert_eq!(before.unwrap_or(0), 0, "navigation cannot create tasks");
    let (request, state) =
        neighborhood_brief::request_generation(&repo, &packet, &fallback, "test-route-v1")
            .await
            .unwrap();
    assert!(matches!(state, Delivery::Pending));
    let (replayed, state) =
        neighborhood_brief::request_generation(&repo, &packet, &fallback, "test-route-v1")
            .await
            .unwrap();
    assert_eq!(request.cache_key(), replayed.cache_key());
    assert!(matches!(state, Delivery::Pending));
    assert!(matches!(
        neighborhood_brief::publish(&repo, &packet, &fallback, "test-route-v1")
            .await
            .unwrap()
            .1,
        Delivery::Pending
    ));
    let mut response = repo
        .db()
        .query("RETURN count(SELECT id FROM task);")
        .await
        .unwrap()
        .check()
        .unwrap();
    let after: Option<i64> = response.take(0).unwrap();
    assert_eq!(
        after,
        Some(1),
        "one explicit identity produces one queue task"
    );
    // A row under the correct lookup key is not enough: the delivered content
    // must still refer to the frozen evidence basis and packet references.
    for foreign_ref in [false, true] {
        let mut poisoned = fallback.clone();
        if foreign_ref {
            poisoned.references.push(brief::BasisRef {
                n: poisoned.references.len() + 1,
                target: brief::BasisTarget::Node("outside-current-packet".into()),
                label: "foreign".into(),
            });
        } else {
            poisoned.basis_key = "stale-neighborhood-basis".into();
        }
        let stored =
            argos_osint_core::storage::summary_flush::load_request(&repo, &request.cache_key())
                .await
                .unwrap()
                .expect("explicit generation persists its frozen request");
        let mut cached = stored
            .deterministic
            .as_deref()
            .expect("public stored request retains its actual deterministic fallback")
            .clone();
        cached.content = serde_json::to_string(&poisoned).unwrap();
        cached.fallback = false;
        cached.source_hashes = request
            .sources
            .iter()
            .map(|source| source.hash.clone())
            .collect();
        argos_osint_core::storage::summary_flush::cache_put(&repo, &request, &cached)
            .await
            .unwrap();
        repo.db().query("UPDATE task SET state='completed',output_hash='generated:fixture' WHERE operation='topology_neighborhood' RETURN NONE").await.unwrap().check().unwrap();
        assert!(
            matches!(
                neighborhood_brief::publish(&repo, &packet, &fallback, "test-route-v1")
                    .await
                    .unwrap()
                    .1,
                Delivery::Failed(_)
            ),
            "invalid cached generation keeps deterministic fallback and truthful failure"
        );
        let (_, delivery) =
            neighborhood_brief::request_generation(&repo, &packet, &fallback, "test-route-v1")
                .await
                .unwrap();
        assert!(
            matches!(delivery, Delivery::Pending),
            "explicit current-basis retry replaces invalid result and requeues the same identity"
        );
        let mut count = repo
            .db()
            .query("RETURN count(SELECT id FROM task);")
            .await
            .unwrap()
            .check()
            .unwrap();
        assert_eq!(
            count.take::<Option<i64>>(0).unwrap(),
            Some(1),
            "retry cannot create a duplicate task"
        );
    }
    // Exercise the real read/cache, explicit worker and failure publication
    // phases with an isolated bounded observation sink (no global subscriber).
    let observations = argos_osint_core::intel_recon::brief_timing::ObservationSink::default();
    argos_osint_core::intel_recon::brief_timing::with_observations(observations.clone(), async {
        let _ = brief::build(&input);
        let _ = brief::build_input(&input, &fallback.basis_key);
        neighborhood_brief::publish(&repo, &packet, &fallback, "test-route-v1")
            .await
            .unwrap();
        let claimed = argos_osint_core::storage::jobs::claim_next(
            repo.db(),
            "summary",
            "timing-owner",
            60,
            chrono::Utc::now(),
            "timing-summary-claim",
        )
        .await
        .unwrap()
        .value
        .expect("explicit retry is claimable");
        let outcome = neighborhood_brief::run_claimed(&repo, &claimed, &request, None, 60)
            .await
            .unwrap();
        assert!(!outcome.generated && !outcome.stale);
        let frozen =
            argos_osint_core::storage::summary_flush::load_request(&repo, &request.cache_key())
                .await
                .unwrap()
                .expect("timed generation uses the actual persisted request/fallback");
        assert!(frozen.deterministic.is_some());
        let runtime = argos_osint_core::recon::model_exec::RoleRuntime::new(
            Default::default(),
            Default::default(),
        );
        assert!(argos_osint_core::brain_neighborhood_brief_exec::generate(
            &runtime,
            None,
            "offline-summary-timing",
            None,
            &frozen,
            &std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false))
        )
        .await
        .is_err());
    })
    .await;
    {
        let observations = observations.lock().unwrap();
        assert_eq!(observations.omitted, 0);
        for phase in [
            "neighborhood_fallback",
            "neighborhood_packet",
            "neighborhood_request",
            "neighborhood_cache",
            "neighborhood_publication",
            "neighborhood_provider",
            "topology_neighborhood",
        ] {
            assert!(
                observations
                    .events
                    .iter()
                    .any(|event| event["operation"] == phase),
                "actual phase {phase} was not observed: {:?}",
                observations.events
            );
        }
        assert!(observations
            .events
            .iter()
            .any(|event| event["operation"] == "neighborhood_publication"
                && event["measurement"]["committed"] == true));
        assert!(
            !serde_json::to_string(&*observations)
                .unwrap()
                .contains("A retained evidence assertion"),
            "operational observations never contain source prose"
        );
    }
    assert!(
        matches!(
            neighborhood_brief::publish(&repo, &packet, &fallback, "changed-route-v2")
                .await
                .unwrap()
                .1,
            Delivery::Idle
        ),
        "a changed route requires a new explicit action"
    );
}

#[tokio::test]
async fn semantic_changes_admit_one_generation_and_reload_reopens_without_requeue() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "brief_revision").await;
    let mut inputs = BriefInputs {
        article_id: "article-a".into(),
        input_revision: "source-domain-prompt-route-v1".into(),
        stage_revisions: vec![
            "body-v1".into(),
            "claims-v1".into(),
            "inferences-v1".into(),
            "links-v1".into(),
        ],
        ..Default::default()
    };
    let first = brief_stages::ensure(&repo, &inputs).await.unwrap();
    let again = brief_stages::ensure(&repo, &inputs).await.unwrap();
    assert_eq!(first.job_id, again.job_id);
    assert!(!again.created);
    inputs.input_revision = "source-secondary-domain-prompt-route-v2".into();
    let (left, right) = tokio::join!(
        brief_stages::ensure(&repo, &inputs),
        brief_stages::ensure(&repo, &inputs)
    );
    let (left, right) = (left.unwrap(), right.unwrap());
    assert_eq!(
        left.job_id, right.job_id,
        "concurrent semantic ensure is single flight"
    );
    assert_ne!(left.job_id, first.job_id);
    let base = inputs.input_revision.clone();
    inputs.input_revision.push_str(":reload:explicit-action");
    let reload = brief_stages::ensure(&repo, &inputs).await.unwrap();
    assert_ne!(reload.job_id, left.job_id);
    inputs.input_revision = base;
    let reopen = brief_stages::ensure(&repo, &inputs).await.unwrap();
    assert_eq!(reopen.job_id, reload.job_id);
    assert!(!reopen.created);
}

#[tokio::test]
async fn durable_body_heads_a_b_a_change_only_joined_stages_and_preserve_pending_self_completion() {
    use argos_osint_core::storage::pipeline_inputs;
    use argos_osint_core::store::AtlasArticleRow;
    use serde_json::json;
    use surrealdb::types::RecordId;
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "body_transition").await;
    let article = AtlasArticleRow {
        id: "body-refresh-article".into(),
        run_id: "retained".into(),
        title: "Retained metadata remains identical".into(),
        description: "The metadata assertion is independent of cleaned body revisions.".into(),
        ..Default::default()
    };
    argos_osint_core::storage::atlas::upsert_article(repo.db(), &article)
        .await
        .unwrap();
    let settings = argos_osint_core::provider::SettingsFile::default();
    let auth = argos_osint_core::secrets::AuthFile::default();
    let initial = brief_stages::inputs_for_article_with_auth(&repo, &article, &settings, &auth)
        .await
        .unwrap();
    let chain_a = brief_stages::ensure(&repo, &initial).await.unwrap();
    async fn head(
        repo: &argos_osint_core::storage::repo::Repository,
        article: &AtlasArticleRow,
        version: i64,
        text: &str,
    ) {
        let hash = argos_osint_core::nlp::cache::hash(text);
        pipeline_inputs::put_article_body(
            repo,
            &argos_osint_core::intel_recon::ArticleBodyRow {
                id: format!("body-{version}"),
                article_id: article.id.clone(),
                run_id: article.run_id.clone(),
                original_url: String::new(),
                resolved_url: String::new(),
                source_domain: String::new(),
                source_name: String::new(),
                body_markdown: text.into(),
                content_hash: hash.clone(),
                quality: "complete".into(),
                quality_rationale: String::new(),
                body_version: version,
                fetch_tool: "offline-fixture".into(),
                fetched_at: "2026-11-01T00:00:00Z".into(),
                retry_after: String::new(),
                state: "ready".into(),
                created_at: "2026-11-01T00:00:00Z".into(),
                updated_at: "2026-11-01T00:00:00Z".into(),
            },
        )
        .await
        .unwrap();
        let artifact = json!({"markdown":text,"content_hash":hash,"baseline_hash":hash,"quality":"complete","rationale":"complete retained fixture","removed":[],"spans":[],"diagnostics":[],"reviewed":0,"batches":0,"acquisition_complete":true});
        pipeline_inputs::app_state_set(
            repo,
            &format!("intel-clean-body:{}:{version}", article.id),
            &artifact.to_string(),
        )
        .await
        .unwrap();
    }
    async fn completed(repo: &argos_osint_core::storage::repo::Repository, job: &str, hash: &str) {
        let task = brief_stages::task_id(job, "intel_brief_body");
        repo.db().query("UPDATE $task SET state='completed',output_hash=$hash; UPSERT $output SET task=$task,root=$root,stage='intel_brief_body',output_hash=$hash,output_json=$json,committed_at=time::now() RETURN NONE")
            .bind(("task",RecordId::new("task",task.clone()))).bind(("output",RecordId::new("stage_output",task))).bind(("root",RecordId::new("job",job.to_string()))).bind(("hash",hash.to_string())).bind(("json",json!({"content_hash":hash}).to_string())).await.unwrap().check().unwrap();
    }
    head(&repo, &article, 1, "The first complete retained body A.").await;
    let pending = brief_stages::inputs_for_article_with_auth(&repo, &article, &settings, &auth)
        .await
        .unwrap();
    assert_eq!(
        pending.input_revision, initial.input_revision,
        "own cache head before receipt cannot churn its pending chain"
    );
    assert_eq!(
        brief_stages::ensure(&repo, &pending).await.unwrap().job_id,
        chain_a.job_id
    );
    completed(
        &repo,
        &chain_a.job_id,
        &argos_osint_core::nlp::cache::hash("The first complete retained body A."),
    )
    .await;
    let a = brief_stages::inputs_for_article_with_auth(&repo, &article, &settings, &auth)
        .await
        .unwrap();
    assert_eq!(a.input_revision, initial.input_revision);
    head(
        &repo,
        &article,
        2,
        "The replacement complete retained body B.",
    )
    .await;
    let b = brief_stages::inputs_for_article_with_auth(&repo, &article, &settings, &auth)
        .await
        .unwrap();
    assert_ne!(a.input_revision, b.input_revision);
    assert_eq!(
        a.stage_revisions[1], b.stage_revisions[1],
        "Claims remain metadata-only"
    );
    assert_ne!(
        a.stage_revisions[2], b.stage_revisions[2],
        "Inference join follows body revision"
    );
    let chain_b = brief_stages::ensure(&repo, &b).await.unwrap();
    completed(
        &repo,
        &chain_b.job_id,
        &argos_osint_core::nlp::cache::hash("The replacement complete retained body B."),
    )
    .await;
    head(&repo, &article, 3, "The first complete retained body A.").await;
    let restored = brief_stages::inputs_for_article_with_auth(&repo, &article, &settings, &auth)
        .await
        .unwrap();
    assert_ne!(
        restored.input_revision, a.input_revision,
        "A→B→A is a new observed transition, never stale generation reuse"
    );
    assert_ne!(restored.input_revision, b.input_revision);
    assert_eq!(restored.stage_revisions[1], a.stage_revisions[1]);
    let chain_c = brief_stages::ensure(&repo, &restored).await.unwrap();
    assert_ne!(chain_c.job_id, chain_a.job_id);
    assert_ne!(chain_c.job_id, chain_b.job_id);
}

#[tokio::test]
async fn retained_legacy_economic_history_never_becomes_current_domain_context() {
    use argos_osint_core::intelligence_contracts::DomainPacket;
    use argos_osint_core::storage::atlas;
    use argos_osint_core::store::AtlasArticleRow;
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "legacy_economic_packet").await;
    let article = AtlasArticleRow {
        id: "legacy-economic".into(),
        run_id: "retained-history".into(),
        title: "The retained Economic bulletin stays readable".into(),
        description: "Original legacy metadata has no authoritative current admission receipt."
            .into(),
        category: "economic".into(),
        categories: vec!["economic".into()],
        ..Default::default()
    };
    atlas::upsert_article(repo.db(), &article).await.unwrap();
    let retained = atlas::article(repo.db(), &article.run_id, &article.id)
        .await
        .unwrap()
        .expect("history remains directly readable");
    assert_eq!(retained.category, "economic");
    assert_eq!(retained.categories, ["economic"]);
    let inputs = brief_stages::inputs_for_article_with_auth(
        &repo,
        &retained,
        &argos_osint_core::provider::SettingsFile::default(),
        &argos_osint_core::secrets::AuthFile::default(),
    )
    .await
    .unwrap();
    let packet: DomainPacket =
        serde_json::from_value(inputs.metadata["domain_context"].clone()).unwrap();
    packet.validate().unwrap();
    assert!(packet.domains.is_empty());
    assert!(packet.admission.is_none());
    assert_eq!(
        packet.unavailable_reason.as_deref(),
        Some("no_current_admission_receipt"),
        "legacy label collision must remain explicit unavailable context"
    );
    assert_eq!(inputs.metadata["title"], retained.title);
    let mut response = repo
        .db()
        .query("RETURN count(SELECT id FROM task);")
        .await
        .unwrap()
        .check()
        .unwrap();
    assert_eq!(
        response.take::<Option<i64>>(0).unwrap(),
        Some(0),
        "reading legacy Focus context does not queue generation"
    );
}
