//! Live-evidence validation inside the publishing transaction on SurrealDB
//! (P3 item 2, part 3). Run through `python3 scripts/test_surreal.py --mode both`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::investigation::insights::{EvidenceLocation, EvidenceRef, EvidenceSnapshot};
use argos_osint_core::nlp::cache::hash;
use argos_osint_core::storage::live_evidence::{step, validate_in};
use argos_osint_core::storage::tx::{run_tx, RetryPolicy};
use argos_osint_core::store::memory_revision;
use support::*;
use surrealdb::engine::any::Any;
use surrealdb::Surreal;

fn reference(source_id: &str, revision: &str, artifact: &str) -> EvidenceRef {
    EvidenceRef {
        source_id: source_id.into(),
        revision: revision.into(),
        artifact_hash: hash(artifact),
        acquisition_id: String::new(),
        origin_root: String::new(),
        url: String::new(),
        published_at: None,
        retrieved_at: String::new(),
        location: EvidenceLocation::JsonPointer(String::new()),
    }
}

async fn validate(db: &Surreal<Any>, snapshot: &EvidenceSnapshot) -> Result<(), String> {
    let snapshot = snapshot.clone();
    run_tx(db, RetryPolicy::default(), |tx| {
        let snapshot = snapshot.clone();
        async move { validate_in(&tx, &snapshot).await }
    })
    .await
    .map(|_| ())
    .map_err(|e| e.to_string())
}

async fn sql(db: &Surreal<Any>, q: &str) {
    db.query(q).await.unwrap().check().unwrap();
}

#[tokio::test]
async fn live_evidence_is_checked_in_the_transaction() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = server.root("argos", "live_evidence").await;
    let (title, description, body, memo) = ("Title", "Desc", "Body text", "Premise");
    for text in [format!("{title}\n{description}"), body.to_string()] {
        db.query(
            "CREATE type::record('nlp_artifact', $h) SET text = $t, byte_len = 1, created_at = ''",
        )
        .bind(("h", hash(&text)))
        .bind(("t", text))
        .await
        .unwrap()
        .check()
        .unwrap();
    }
    sql(
        &db,
        "CREATE atlas_article SET run_id = 'r1', article_id = 'a1', title = 'Title',
           description = 'Desc', url = 'https://x/a1', country = '', source_name = '',
           source_domain = '', published_at = '2026-01-01', provider = '', temperature = 0.0,
           seen_at = '2026-01-02';
         CREATE article_body:b1 SET article_id = 'a1', run_id = 'r1', original_url = '',
           resolved_url = '', source_domain = '', source_name = '', body_markdown = 'Body text',
           content_hash = '', quality = '', quality_rationale = '', body_version = 3,
           fetch_tool = '', fetched_at = '', retry_after = '', state = 'ready',
           created_at = '', updated_at = '';
         CREATE memory:m1 SET text = 'Premise';",
    )
    .await;
    let mut article = reference(
        "article:a1",
        &hash(&format!("{title}\n{description}")),
        &format!("{title}\n{description}"),
    );
    article.url = "https://x/a1".into();
    article.published_at = Some("2026-01-01".into());
    let snapshot = EvidenceSnapshot {
        owner_kind: "article".into(),
        owner_id: "a1".into(),
        source_refs: vec![article, reference("body:b1", "3", body)],
        memory_refs: vec![("m1".into(), memory_revision(memo))],
        generation: 1,
        entities: vec![],
        tool_refs: vec![],
        coverage_by_stage: vec![],
        assessed_claim_refs: vec![],
        identity_bindings: vec![],
        memory_claims: vec![],
        key: "k".into(),
    };
    validate(&db, &snapshot).await.unwrap();
    // The step form validates the captured snapshot.
    let s = step(snapshot.clone());
    run_tx(&db, RetryPolicy::default(), |tx| {
        let s = s.clone();
        async move { s(tx, String::new()).await }
    })
    .await
    .unwrap();

    let fails = |sql_text: &'static str, message: &'static str| {
        let (db, snapshot) = (db.clone(), snapshot.clone());
        async move {
            sql(&db, sql_text).await;
            let err = validate(&db, &snapshot).await.unwrap_err();
            assert!(err.contains(message), "{sql_text}: {err}");
        }
    };
    fails(
        "UPDATE memory:m1 SET text = 'edited'",
        "memory premise revision changed",
    )
    .await;
    fails(
        "UPDATE memory:m1 SET text = 'Premise'; UPDATE article_body:b1 SET body_version = 4",
        "body evidence revision changed",
    )
    .await;
    fails(
        "UPDATE article_body:b1 SET body_version = 3; UPDATE atlas_article SET title = 'New'",
        "article metadata revision changed",
    )
    .await;
    fails(
        "UPDATE atlas_article SET title = 'Title'; DELETE nlp_artifact",
        "retained source artifact unavailable",
    )
    .await;
    fails("DELETE atlas_article", "article owner no longer exists").await;

    let mut recon = snapshot.clone();
    recon.owner_kind = "recon".into();
    assert!(validate(&db, &recon)
        .await
        .unwrap_err()
        .contains("isn't available on SurrealDB yet"));
}
