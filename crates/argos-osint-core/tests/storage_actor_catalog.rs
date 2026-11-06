//! Offline catalog seed/retry and article association contracts.
#![cfg(feature = "surreal")]
#[path = "support/current_admission.rs"]
mod current_admission;
mod support;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};

use argos_osint_core::actor_catalog::Actor;
use argos_osint_core::atlas_actor_index::surreal;
use argos_osint_core::storage::tx::{run_tx, RetryPolicy};
use argos_osint_core::storage::{actor_bind, actor_catalog};
use argos_osint_core::store::AtlasArticleRow;

fn actors(count: usize) -> Vec<Actor> {
    (0..count)
        .map(|i| Actor {
            id: format!("fixture-actor-{i}"),
            source_id: format!("fixture:{i}"),
            name: format!("Fixture Union {i}"),
            subtype: "organization".into(),
            aliases: vec![
                format!("Fixture Union {i}"),
                format!("Harbor Alias {i}"),
                "Shared Alias".into(),
            ],
            metadata: BTreeMap::from([("license".into(), "fixture-only".into())]),
        })
        .collect()
}

#[tokio::test]
async fn unseeded_article_extraction_does_not_create_provisional_actors() {
    let Some(server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("catalog", "unseeded").await;
    run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
        let req = argos_osint_core::brain_actors::BindRequest {
            name: "Unknown Person",
            subtype: argos_osint_core::memory_metadata::ActorSubtype::Person,
            scope: "article:fixture",
            identifiers: &[],
            external: None,
            provenance: serde_json::json!({}),
        };
        assert!(
            actor_bind::bind_optional_in(&tx, &req, "2026-11-02T00:00:00Z")
                .await?
                .is_none()
        );
        Ok(())
    })
    .await
    .unwrap();
    let mut result = repo
        .db()
        .query("RETURN count(SELECT id FROM brain_actor)")
        .await
        .unwrap()
        .check()
        .unwrap();
    assert_eq!(result.take::<Option<usize>>(0).unwrap(), Some(0));
}

#[tokio::test]
async fn partial_seed_retries_without_duplicates_and_only_complete_receipts_hide_action() {
    let Some(server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("catalog", "retry").await;
    assert!(
        actor_catalog::status(repo.db())
            .await
            .expect("initial status")
            .needs_seed
    );
    let cancel = AtomicBool::new(false);
    let result = actor_catalog::seed_actors(&repo, actors(129), "fixture-v1", &cancel, |p| {
        if p.processed == 128 {
            cancel.store(true, Ordering::Release);
        }
    })
    .await;
    assert!(result.is_err());
    assert!(
        actor_catalog::status(repo.db())
            .await
            .expect("partial status")
            .needs_seed
    );
    let partial_article = AtlasArticleRow {
        title: "Harbor Alias 0".into(),
        ..Default::default()
    };
    assert!(actor_catalog::article_actors(&repo, &partial_article)
        .await
        .expect("partial catalog article lookup")
        .expect("catalog remains enabled")
        .is_empty());
    cancel.store(false, Ordering::Release);
    let progress = actor_catalog::seed_actors(&repo, actors(129), "fixture-v1", &cancel, |_| {})
        .await
        .expect("retry seed");
    assert!(progress.complete);
    assert_eq!(progress.processed, 129);
    assert!(
        !actor_catalog::status(repo.db())
            .await
            .expect("complete status")
            .needs_seed
    );
    actor_catalog::seed_actors(&repo, actors(129), "fixture-v1", &cancel, |_| {})
        .await
        .expect("repeat seed");
    let mut r = repo.db().query("RETURN count(SELECT id FROM brain_actor); RETURN count(SELECT id FROM memory WHERE memory_kind = 'actor'); RETURN count(SELECT id FROM brain_actor_alias);").await.expect("counts").check().expect("valid counts");
    assert_eq!(r.take::<Option<usize>>(0).expect("actors"), Some(129));
    assert_eq!(r.take::<Option<usize>>(1).expect("memories"), Some(129));
    assert_eq!(r.take::<Option<usize>>(2).expect("aliases"), Some(387));
    repo.db()
        .query("DELETE brain_actor")
        .await
        .expect("empty actors")
        .check()
        .expect("delete");
    assert!(
        actor_catalog::status(repo.db())
            .await
            .expect("empty status")
            .needs_seed
    );
}

#[tokio::test]
async fn articles_match_catalog_aliases_without_nlp_and_never_create_unknown_identities() {
    let Some(server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("catalog", "matching").await;
    actor_catalog::seed_actors(
        &repo,
        actors(2),
        "fixture-v1",
        &AtomicBool::new(false),
        |_| {},
    )
    .await
    .expect("seed");
    let mut article = AtlasArticleRow {
        id: "article".into(),
        run_id: "run".into(),
        title: "Harbor Alias 0 meets Unknown Person and Shared Alias".into(),
        ..Default::default()
    };
    argos_osint_core::storage::atlas::insert_run(repo.db(), "run", "{}", "{}")
        .await
        .expect("run");
    current_admission::admit(&repo, &mut article, "Ballot patrol common").await;
    let matched = actor_catalog::article_actors(&repo, &article)
        .await
        .expect("catalog projection")
        .expect("catalog enabled");
    assert_eq!(matched.len(), 1);
    assert_eq!(matched[0].id, "fixture-actor-0");
    // A catalog actor can open its dossier before any article indexing exists.
    let detail = argos_osint_core::intel_service::brief(&repo, &article)
        .await
        .expect("unindexed brief");
    assert_eq!(
        detail["actors"][0]["topology_seed"],
        serde_json::json!({"Actor":"fixture-actor-0"})
    );
    let mut mentions = repo
        .db()
        .query("RETURN count(SELECT id FROM brain_actor_mention)")
        .await
        .expect("mention count")
        .check()
        .expect("valid count");
    assert_eq!(
        mentions.take::<Option<usize>>(0).expect("count"),
        Some(0),
        "reading the dossier action must not index the article"
    );
    surreal::index_occurrence(&repo, &article, None, "revision", "2026-11-01T00:00:00Z")
        .await
        .expect("index without NLP");
    let dossier = argos_osint_core::storage::actor_dossier::read(&repo, "fixture-actor-0")
        .await
        .expect("actor dossier")
        .expect("actor exists");
    if dossier.loaded_briefs != 1 {
        let current = argos_osint_core::storage::atlas::current_actor_mentions(
            &repo,
            &["fixture-actor-0".into()],
            &argos_osint_core::atlas_classify::OccurrenceKey::new("probe", "none"),
            100,
            None,
        )
        .await
        .unwrap();
        let mut raw = repo.db().query("SELECT origin_revision,native_input_revision,publication_batch FROM brain_actor_mention; SELECT input_revision,publication_batch,index_checksum FROM atlas_article_index; SELECT record::id(id) AS id,lifecycle,lifecycle_ingest_batch,memory_kind FROM memory WHERE memory_kind INSIDE ['link','related'];").await.unwrap().check().unwrap();
        let mentions: Vec<serde_json::Value> = raw.take(0).unwrap();
        let indexes: Vec<serde_json::Value> = raw.take(1).unwrap();
        let links: Vec<serde_json::Value> = raw.take(2).unwrap();
        eprintln!("native current={} expected_article_revision={} mentions={mentions:?} indexes={indexes:?} links={links:?}",current.len(),argos_osint_core::atlas_metadata::input_hash(&article));
    }
    assert_eq!(
        dossier.loaded_briefs, 1,
        "indexing checksum must not masquerade as an article revision: {dossier:#?}"
    );
    assert_eq!(
        dossier.briefs[0].node.revision,
        argos_osint_core::atlas_metadata::input_hash(&article)
    );
    let graph = argos_osint_core::storage::topology::topology_snapshot(
        &repo,
        &argos_osint_core::brain_topology::TopologySeed::Article {
            run_id: article.run_id.clone(),
            article_id: article.id.clone(),
            revision: argos_osint_core::atlas_metadata::input_hash(&article),
            originating_memory: None,
        },
        &Default::default(),
    )
    .await
    .expect("article topology");
    assert!(
        graph.nodes.iter().any(|n| n.id == "fixture-actor-0"),
        "article-to-actor direction must use the same revision"
    );
    // Homes indexed by the older contract recover through the normal drainer.
    repo.db().query("UPDATE atlas_article_index SET index_checksum = 'old-contract'; UPDATE brain_actor_mention SET origin_revision = 'old-enrichment-checksum'").await.expect("old fixture").check().expect("old receipt");
    let repaired = argos_osint_core::storage::brain_backfill::backfill(&repo, 100)
        .await
        .expect("bounded repair");
    assert_eq!(repaired.processed, 1);
    assert_eq!(repaired.failed, 0);
    let repaired = argos_osint_core::storage::actor_dossier::read(&repo, "fixture-actor-0")
        .await
        .expect("repaired dossier")
        .expect("actor");
    assert_eq!(repaired.loaded_briefs, 1);
    // Actual article changes still invalidate the recorded mention.
    let mut revised = article.clone();
    revised.title = "Revised title without the actor".into();
    // Native source replacement is published only after real local admission.
    current_admission::admit(&repo, &mut revised, "Ballot patrol common").await;
    let stale = argos_osint_core::storage::actor_dossier::read(&repo, "fixture-actor-0")
        .await
        .expect("stale read")
        .expect("actor");
    assert_eq!(stale.loaded_briefs, 0);
    current_admission::admit(&repo, &mut article, "Ballot patrol common").await;
    let mut r = repo.db().query("SELECT VALUE actor_id FROM brain_actor_mention; RETURN count(SELECT id FROM brain_actor);").await.expect("mentions").check().expect("valid mentions");
    assert_eq!(
        r.take::<Vec<String>>(0).expect("actors"),
        vec!["fixture-actor-0"]
    );
    assert_eq!(r.take::<Option<usize>>(1).expect("count"), Some(2));
    run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
        assert_eq!(
            actor_catalog::lookup_in(&tx, "HARBOR ALIAS 0").await?,
            Some("fixture-actor-0".into())
        );
        assert!(actor_catalog::lookup_in(&tx, "Shared Alias")
            .await?
            .is_none());
        let req = argos_osint_core::brain_actors::BindRequest {
            name: "Unknown Person",
            subtype: argos_osint_core::memory_metadata::ActorSubtype::Person,
            scope: "article:fixture",
            identifiers: &[],
            external: None,
            provenance: serde_json::json!({}),
        };
        assert!(
            actor_bind::bind_optional_in(&tx, &req, "2026-11-01T00:00:00Z")
                .await?
                .is_none()
        );
        Ok(())
    })
    .await
    .expect("binding without new actors");
    run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
        argos_osint_core::storage::lifecycle::retire_origin_in(&tx, "article", "run").await?;
        Ok(())
    })
    .await
    .expect("retire article origin");
    let mut r = repo
        .db()
        .query("SELECT VALUE lifecycle FROM ONLY type::record('memory', 'fixture-actor-0')")
        .await
        .expect("actor lifecycle")
        .check()
        .expect("read lifecycle");
    assert_eq!(
        r.take::<Option<String>>(0).expect("lifecycle"),
        Some("active".into())
    );
    assert!(
        argos_osint_core::storage::actor_dedup::candidates(&repo, 10)
            .await
            .expect("identity candidates")
            .is_empty()
    );
}

#[tokio::test]
async fn retention_updates_actor_associations_and_preserves_other_occurrences() {
    let Some(server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("catalog", "retention").await;
    actor_catalog::seed_actors(
        &repo,
        actors(1),
        "retention-v1",
        &AtomicBool::new(false),
        |_| {},
    )
    .await
    .expect("seed");
    for run in ["old", "current"] {
        argos_osint_core::storage::atlas::insert_run(repo.db(), run, "{}", "{}")
            .await
            .expect("run");
    }
    for (run, id) in [
        ("old", "expired"),
        ("old", "protected"),
        ("current", "retained"),
    ] {
        let mut article = AtlasArticleRow {
            run_id: run.into(),
            id: id.into(),
            title: "Fixture Union 0 speaks".into(),
            ..Default::default()
        };
        current_admission::admit(&repo, &mut article, "Ballot patrol common").await;
        surreal::index_occurrence(
            &repo,
            &article,
            None,
            "enrichment-checksum",
            "2026-11-02T00:00:00Z",
        )
        .await
        .expect("index");
    }
    repo.db().query("UPDATE atlas_run:old SET state = 'done', started_at = '2026-01-01T00:00:00Z'; UPDATE atlas_run:current SET state = 'done'").await.unwrap().check().unwrap();
    argos_osint_core::storage::intel::ensure_investigation(
        repo.db(),
        "protected",
        "old",
        "https://example.test/protected",
        "{}",
    )
    .await
    .expect("protect");
    for mid in ["shared", "expired-only", "protected-only"] {
        repo.db().query("CREATE type::record('memory', $m) SET text = $m, memory_kind = 'claim', source_json = $source; CREATE type::record('insight_claim', $m) SET memory_id = $m, entity_id = 'fixture', predicate = 'reports', object_value = $m, topic = 'fixture', classification = 'fact', confidence = 0.5, created_at = '', updated_at = ''; CREATE type::record('brain_memory_actor_ref', $m) SET memory_id = $m, actor_id = 'fixture-actor-0', role = 'subject';")
            .bind(("m", mid.to_string()))
            .bind(("source", serde_json::json!({"app":"atlas","conversation_id":"old","message_id":null,"reference":null}).to_string()))
            .await.unwrap().check().unwrap();
    }
    for (key, mid, run, article) in [
        ("s1", "shared", "old", "expired"),
        ("s2", "shared", "current", "retained"),
        ("s3", "expired-only", "old", "expired"),
        ("s4", "protected-only", "old", "protected"),
    ] {
        repo.db().query("CREATE type::record('insight_source', $key) SET fingerprint = $mid, run_id = $run, answer_id = $answer, call_id = $article")
            .bind(("key", key.to_string())).bind(("mid", mid.to_string())).bind(("run", run.to_string()))
            .bind(("answer", format!("atlas-{run}"))).bind(("article", article.to_string())).await.unwrap().check().unwrap();
    }
    let before = argos_osint_core::storage::actor_dossier::read(&repo, "fixture-actor-0")
        .await
        .unwrap()
        .unwrap();
    assert_eq!((before.loaded_briefs, before.loaded_claims), (3, 3));
    argos_osint_core::storage::atlas_delete::prune_expired(repo.publisher(), chrono::Utc::now())
        .await
        .expect("automatic retention");
    let pruned = argos_osint_core::storage::actor_dossier::read(&repo, "fixture-actor-0")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (pruned.loaded_briefs, pruned.loaded_claims),
        (2, 2),
        "protected and independently sourced associations survive"
    );
    assert!(!pruned.claims.iter().any(|a| a.node.id == "expired-only"));
    argos_osint_core::storage::atlas_delete::delete_article(
        repo.publisher(),
        "current",
        "retained",
    )
    .await
    .unwrap();
    let one = argos_osint_core::storage::actor_dossier::read(&repo, "fixture-actor-0")
        .await
        .unwrap()
        .unwrap();
    assert_eq!((one.loaded_briefs, one.loaded_claims), (1, 1));
    argos_osint_core::storage::atlas_delete::delete_article(repo.publisher(), "old", "protected")
        .await
        .unwrap();
    let none = argos_osint_core::storage::actor_dossier::read(&repo, "fixture-actor-0")
        .await
        .unwrap()
        .unwrap();
    assert_eq!((none.loaded_briefs, none.loaded_claims), (0, 0));
    let mut rows = repo
        .db()
        .query(
            "RETURN count(SELECT id FROM brain_actor_mention WHERE actor_id = 'fixture-actor-0')",
        )
        .await
        .unwrap()
        .check()
        .unwrap();
    assert_eq!(rows.take::<Option<usize>>(0).unwrap(), Some(0));
}

#[tokio::test]
async fn cleaned_body_actors_append_bind_insights_and_fence_changed_acquisitions() {
    use argos_osint_core::investigation::insights::{
        Assessment, Inference, PipelineOutput, SourceClaim,
    };
    use argos_osint_core::nlp::Source;
    use argos_osint_core::storage::insight::{
        advance_generation, publish_snapshot_on, SnapshotStep,
    };
    use argos_osint_core::storage::pipeline_inputs;
    use std::sync::Arc;
    let Some(server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("catalog", "clean_body").await;
    let mut catalog = actors(3);
    catalog[1].aliases.push("Microsoft".into());
    actor_catalog::seed_actors(&repo, catalog, "body-v1", &AtomicBool::new(false), |_| {})
        .await
        .unwrap();
    argos_osint_core::storage::atlas::insert_run(repo.db(), "run", "{}", "{}")
        .await
        .unwrap();
    let mut article = AtlasArticleRow {
        id: "body-article".into(),
        run_id: "run".into(),
        title: "Fixture Union 0 speaks".into(),
        description: "Officials report a new agreement.".into(),
        ..Default::default()
    };
    current_admission::admit(&repo, &mut article, "Ballot patrol common").await;
    argos_osint_core::storage::atlas::insert_run(repo.db(), "other-run", "{}", "{}")
        .await
        .unwrap();
    let mut other = AtlasArticleRow {
        run_id: "other-run".into(),
        ..article.clone()
    };
    current_admission::admit(&repo, &mut other, "Ballot patrol common").await;
    assert_eq!(
        argos_osint_core::atlas_metadata::input_hash(&article),
        argos_osint_core::atlas_metadata::input_hash(&other),
        "same-article occurrences share unchanged metadata and body acquisition"
    );
    let brief_settings = argos_osint_core::provider::SettingsFile::default();
    let brief_auth = argos_osint_core::secrets::AuthFile::default();
    let metadata_context_before =
        argos_osint_core::storage::brief_stages::inputs_for_article_with_auth(
            &repo,
            &article,
            &brief_settings,
            &brief_auth,
        )
        .await
        .unwrap();
    let metadata_chain =
        argos_osint_core::storage::brief_stages::ensure(&repo, &metadata_context_before)
            .await
            .unwrap();
    let initial = argos_osint_core::intel_service::brief(&repo, &article)
        .await
        .unwrap();
    assert_eq!(initial["actors"][0]["id"], "fixture-actor-0");
    assert_eq!(initial["actors_body_pending"], true);
    let mut empty_article = AtlasArticleRow {
        id: "empty-actors".into(),
        run_id: "run".into(),
        title: "An ordinary report".into(),
        description: "Officials meet.".into(),
        ..Default::default()
    };
    current_admission::admit(&repo, &mut empty_article, "Ballot patrol common").await;
    let empty = argos_osint_core::intel_service::brief(&repo, &empty_article)
        .await
        .unwrap();
    assert!(empty["actors"].as_array().unwrap().is_empty());
    assert_eq!(
        empty["actors_pending"], true,
        "no false empty before body cleanup"
    );
    let body = argos_osint_core::storage::article_body::ensure(
        &repo,
        &article.id,
        &article.run_id,
        "https://example.test",
        "example.test",
        "Fixture",
    )
    .await
    .unwrap();
    let raw =
        "Advertisement: Fixture Union 2. é Fixture Union 1 negotiated; Fixture Union 0 replied.";
    let clean = "é Microsoft negotiated; Fixture Union 0 replied. Barack Obama spoke.";
    argos_osint_core::storage::article_body::commit(
        &repo,
        &body.id,
        raw,
        &argos_osint_core::nlp::cache::hash(raw),
        "complete",
        "fixture",
        "https://example.test",
        "fixture",
        false,
    )
    .await
    .unwrap();
    let mut baseline = Source::derived(&body.id, "1", raw);
    baseline.attempt_id = "acq-1".into();
    baseline.raw_hash = Some(argos_osint_core::nlp::cache::hash(raw));
    pipeline_inputs::register(
        &repo,
        "article",
        &article.id,
        &baseline,
        Some(raw),
        "validated-baseline-v1",
    )
    .await
    .unwrap();
    let unclean = actor_catalog::article_actors(&repo, &article)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        unclean.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
        vec!["fixture-actor-0"],
        "raw and baseline text must not supply body actors"
    );
    let mut cleaned = Source::derived(&body.id, "1", clean);
    cleaned.attempt_id = "acq-1".into();
    cleaned.raw_hash = baseline.raw_hash.clone();
    pipeline_inputs::register(
        &repo,
        "article",
        &article.id,
        &cleaned,
        Some(raw),
        argos_osint_core::intel_recon::clean_body::TRANSFORM,
    )
    .await
    .unwrap();
    let matched = actor_catalog::article_actors(&repo, &article)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        matched.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
        vec!["fixture-actor-0", "fixture-actor-1"],
        "bundled NLP resolves an organization alias; unknown people create no catalog actor"
    );
    let metadata_context_after =
        argos_osint_core::storage::brief_stages::inputs_for_article_with_auth(
            &repo,
            &article,
            &brief_settings,
            &brief_auth,
        )
        .await
        .unwrap();
    assert_eq!(
        metadata_context_before.input_revision, metadata_context_after.input_revision,
        "body completion cannot invalidate its own metadata root generation"
    );
    assert_eq!(
        argos_osint_core::storage::brief_stages::ensure(&repo, &metadata_context_after)
            .await
            .unwrap()
            .job_id,
        metadata_chain.job_id,
        "a body cache head before Body task commit cannot supersede the pending chain"
    );
    assert_eq!(
        metadata_context_before.metadata["domain_context"],
        metadata_context_after.metadata["domain_context"]
    );
    assert_eq!(
        metadata_context_after.metadata["actors"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "body-only actor never reaches metadata Claims roots"
    );
    for actor in &matched {
        for mention in actor.mentions.iter().filter(|m| m.field == "body") {
            assert_eq!(
                &clean[mention.start_byte..mention.end_byte],
                mention.original_text
            );
        }
    }
    let available = argos_osint_core::intel_service::brief(&repo, &article)
        .await
        .unwrap();
    assert_eq!(available["actors"][0]["id"], "fixture-actor-0");
    assert_eq!(available["actors"][1]["id"], "fixture-actor-1");
    assert_eq!(
        available["actors"][1]["topology_seed"],
        serde_json::json!({"Actor":"fixture-actor-1"})
    );
    assert_eq!(
        available["actors_body_pending"], true,
        "retained clean-body actors stay visible while the new Body stage is queued"
    );
    let mut count = repo
        .db()
        .query("RETURN count(SELECT id FROM brain_actor)")
        .await
        .unwrap()
        .check()
        .unwrap();
    assert_eq!(
        count.take::<Option<usize>>(0).unwrap(),
        Some(3),
        "unknown NLP people must not create catalog or provisional actors"
    );
    surreal::index_article_occurrences(&repo, &article.id)
        .await
        .unwrap();
    let dossier = argos_osint_core::storage::actor_dossier::read(&repo, "fixture-actor-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(dossier.loaded_briefs, 2);
    assert!(dossier.briefs[0]
        .relationship
        .receipts
        .iter()
        .any(|r| r.contains(&cleaned.prose_hash)));
    let mut input = pipeline_inputs::article_input(&repo, &article, 1, "fixture")
        .await
        .unwrap();
    let entity = input
        .snapshot
        .entities
        .iter()
        .find(|e| e.id.ends_with(":actor:fixture-actor-1"))
        .unwrap()
        .clone();
    let reference = input
        .evidence
        .iter()
        .find(|e| e.reference.source_id.starts_with("body:"))
        .unwrap()
        .reference
        .clone();
    assert!(argos_osint_core::storage::actor_bind::referenced(
        &entity,
        "",
        std::slice::from_ref(&reference),
        |_| true
    ));
    let mut wrong_source = reference.clone();
    wrong_source.source_id = format!("article:{}", article.id);
    assert!(
        !argos_osint_core::storage::actor_bind::referenced(&entity, "", &[wrong_source], |_| true),
        "body byte spans cannot bind metadata evidence"
    );
    let assessment = Assessment {
        fidelity: "faithful".into(),
        threshold_passed: true,
        ..Default::default()
    };
    let claim = SourceClaim {
        support_receipts: vec![],
        id: "clean-claim".into(),
        subject_ref: entity.id.clone(),
        predicate: "negotiated".into(),
        object: "agreement".into(),
        text: "Fixture Union 1 negotiated an agreement.".into(),
        polarity: "positive".into(),
        modality: "asserted".into(),
        attributed_to: String::new(),
        temporal_scope: String::new(),
        evidence_refs: vec![reference.clone()],
        extraction_scope: "clean-body-fixture".into(),
        assessment: assessment.clone(),
    };
    let inference = Inference {
        inference_kind: None,
        domains: Vec::new(),
        support_state: String::new(),
        conditions: Vec::new(),
        id: "clean-inference".into(),
        text: "The agreement may advance talks.".into(),
        entity_refs: vec![entity.id],
        premise_claim_ids: vec![],
        evidence_refs: vec![reference],
        rationale: "Retained clean-body evidence".into(),
        limitations: vec![],
        assessment,
        snapshot_key: input.snapshot.key.clone(),
    };
    let output = PipelineOutput {
        entities: input.snapshot.entities.clone(),
        claims: vec![claim],
        inferences: vec![inference],
        ..Default::default()
    };
    let generation = run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
        advance_generation(&tx, "article", "body-article", "now").await
    })
    .await
    .unwrap()
    .value;
    input.snapshot.generation = generation as u32;
    let input = Arc::new(input);
    let publish = argos_osint_core::storage::derivations::derivation_step(
        input.clone(),
        Arc::new(output),
        None,
        argos_osint_core::storage::derivations::DerivationContext {
            save_insights: true,
            recon_thread: None,
        },
    );
    let validate: SnapshotStep = Arc::new(|_, _| Box::pin(async { Ok(()) }));
    run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let (input, publish, validate) = (input.clone(), publish.clone(), validate.clone());
        async move {
            publish_snapshot_on(
                &tx,
                &input.snapshot,
                &input.snapshot.key,
                "active",
                "2026-11-02T00:00:00Z",
                &validate,
                &publish,
            )
            .await
        }
    })
    .await
    .unwrap();
    let associated = argos_osint_core::storage::actor_dossier::read(&repo, "fixture-actor-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (associated.loaded_claims, associated.loaded_inferences),
        (1, 1)
    );
    assert_eq!(
        associated.inferences[0]
            .relationship
            .actor_reference
            .as_ref()
            .unwrap()
            .role,
        "mentioned"
    );
    // A new acquisition with no current cleanup must not reuse old body mentions.
    baseline.attempt_id = "acq-2".into();
    pipeline_inputs::register(
        &repo,
        "article",
        &article.id,
        &baseline,
        Some(raw),
        "validated-baseline-v1",
    )
    .await
    .unwrap();
    let stale = argos_osint_core::storage::actor_dossier::read(&repo, "fixture-actor-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stale.loaded_briefs, 0);
    let metadata = argos_osint_core::storage::actor_dossier::read(&repo, "fixture-actor-0")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        metadata.loaded_briefs, 0,
        "an unvalidated body acquisition invalidates the whole native generation"
    );
    cleaned.attempt_id = "acq-2".into();
    pipeline_inputs::register(
        &repo,
        "article",
        &article.id,
        &cleaned,
        Some(raw),
        argos_osint_core::intel_recon::clean_body::TRANSFORM,
    )
    .await
    .unwrap();
    surreal::index_article_occurrences(&repo, &article.id)
        .await
        .unwrap();
    for actor in ["fixture-actor-0", "fixture-actor-1"] {
        let restored = argos_osint_core::storage::actor_dossier::read(&repo, actor)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            restored.loaded_briefs, 2,
            "current source validation and native refresh restore both occurrences"
        );
    }
    argos_osint_core::storage::atlas_delete::delete_article(repo.publisher(), "run", &article.id)
        .await
        .unwrap();
    let shared = argos_osint_core::storage::actor_dossier::read(&repo, "fixture-actor-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            shared.loaded_briefs,
            shared.loaded_claims,
            shared.loaded_inferences
        ),
        (1, 1, 1),
        "deleting one occurrence cannot erase another occurrence's clean artifact or insights"
    );
    argos_osint_core::storage::atlas_delete::delete_article(
        repo.publisher(),
        "other-run",
        &article.id,
    )
    .await
    .unwrap();
    let deleted = argos_osint_core::storage::actor_dossier::read(&repo, "fixture-actor-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            deleted.loaded_briefs,
            deleted.loaded_claims,
            deleted.loaded_inferences
        ),
        (0, 0, 0)
    );
}

#[tokio::test]
async fn catalog_dossier_recovers_unique_legacy_actor_associations_without_other_article_actors() {
    let Some(server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("catalog", "legacy_dossier").await;
    actor_catalog::seed_actors(
        &repo,
        actors(2),
        "legacy-fixture",
        &AtomicBool::new(false),
        |_| {},
    )
    .await
    .unwrap();
    repo.db().query("CREATE brain_actor:legacy SET display_name = 'Fixture Union 0', subtype = 'organization', scope_key = 'article:old', identity_state = 'provisional', revision = 1, created_at = '', updated_at = '';
        CREATE brain_actor_alias:legacy SET actor_id = 'legacy', alias = 'Fixture Union 0', normalized = 'fixture union 0', alias_type = 'name', provenance_json = '{}';
        CREATE memory:legacy_claim SET text = 'Legacy actor claim', memory_kind = 'claim', source_json = '{}';
        CREATE insight_claim:legacy_claim SET memory_id = 'legacy_claim', entity_id = 'legacy', predicate = 'reports', object_value = 'a', topic = 'fixture', classification = 'fact', confidence = 0.5, created_at = '', updated_at = '';
        CREATE brain_memory_actor_ref:legacy SET memory_id = 'legacy_claim', actor_id = 'legacy', role = 'subject';
        CREATE memory:other_claim SET text = 'Other actor claim', memory_kind = 'claim', source_json = '{}';
        CREATE insight_claim:other_claim SET memory_id = 'other_claim', entity_id = 'other', predicate = 'reports', object_value = 'b', topic = 'fixture', classification = 'fact', confidence = 0.5, created_at = '', updated_at = '';
        CREATE brain_memory_actor_ref:other SET memory_id = 'other_claim', actor_id = 'fixture-actor-1', role = 'subject';").await.unwrap().check().unwrap();
    let dossier = argos_osint_core::storage::actor_dossier::read(&repo, "fixture-actor-0")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(dossier.loaded_claims, 1);
    assert_eq!(dossier.claims[0].node.id, "legacy_claim");
    let other = argos_osint_core::storage::actor_dossier::read(&repo, "fixture-actor-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(other.loaded_claims, 1);
    assert_eq!(other.claims[0].node.id, "other_claim");
    // An explicit identity redirect wins over a historical name match.
    repo.db().query("CREATE brain_actor_redirect:legacy SET to_actor = 'fixture-actor-1', receipt_json = '{}', created_at = ''").await.unwrap().check().unwrap();
    let redirected = argos_osint_core::storage::actor_dossier::read(&repo, "fixture-actor-0")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(redirected.loaded_claims, 0);
}

#[tokio::test]
async fn curated_title_alias_resolves_for_an_existing_seed_and_dossier_mentions() {
    let Some(server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("catalog", "trump_title").await;
    let actor = Actor {
        id: "donald".into(),
        source_id: "os:Q22686".into(),
        name: "Donald Trump".into(),
        subtype: "person".into(),
        aliases: vec!["Donald Trump".into()],
        metadata: BTreeMap::new(),
    };
    actor_catalog::seed_actors(
        &repo,
        vec![actor],
        "old-seed",
        &AtomicBool::new(false),
        |_| {},
    )
    .await
    .unwrap();
    let mut article = AtlasArticleRow {
        id: "title".into(),
        run_id: "r".into(),
        title: "trump considering using nuclear weapons against iran - tucker carlson".into(),
        ..Default::default()
    };
    argos_osint_core::storage::atlas::insert_run(repo.db(), "r", "{}", "{}")
        .await
        .unwrap();
    current_admission::admit(&repo, &mut article, "Ballot patrol common").await;
    let brief = argos_osint_core::intel_service::brief(&repo, &article)
        .await
        .unwrap();
    assert_eq!(brief["actors"][0]["display_name"], "Donald Trump");
    run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
        assert_eq!(
            actor_catalog::lookup_in(&tx, "Trump").await?,
            Some("donald".into())
        );
        Ok(())
    })
    .await
    .unwrap();
    surreal::index_occurrence(&repo, &article, None, "title-v1", "2026-11-02T00:00:00Z")
        .await
        .unwrap();
    let dossier = argos_osint_core::storage::actor_dossier::read(&repo, "donald")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(dossier.loaded_briefs, 1);
}
