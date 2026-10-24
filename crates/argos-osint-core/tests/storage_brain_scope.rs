//! The Brain scope pool on SurrealDB (`storage::brain_scope`): one task per
//! payload, a drain that writes the deterministic inference and links over
//! the actor's topology, and a rerun that leaves the links unchanged.
#![cfg(feature = "surreal")]

mod support;

use std::sync::atomic::AtomicBool;

use argos_osint_core::brain::MemorySource;
use argos_osint_core::brain_actors::BindRequest;
use argos_osint_core::brain_scope::ScopeAnalysisPayload;
use argos_osint_core::brain_topology::{CycleScope, TopologyOptions, TopologySeed};
use argos_osint_core::memory_metadata::ActorSubtype;
use argos_osint_core::storage::tx::{run_tx, RetryPolicy};
use argos_osint_core::storage::{actor_bind, bootstrap, brain_scope, memories, recon};
use serde_json::json;
use support::*;

fn source() -> MemorySource {
    MemorySource {
        app: "recon".into(),
        conversation_id: "t1".into(),
        message_id: None,
        reference: None,
    }
}

async fn count(repo: &argos_osint_core::storage::repo::Repository, sql: &str) -> i64 {
    let mut r = repo.db().query(sql).await.unwrap().check().unwrap();
    r.take::<Option<i64>>(0usize).unwrap().unwrap_or(0)
}

#[tokio::test]
async fn scope_tasks_dedupe_and_drain_writes_inference_and_links() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "brain_scope").await;
    bootstrap::ensure_schema(repo.db()).await.unwrap();
    let actor = run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
        let req = BindRequest {
            name: "Jane Doe",
            scope: "s",
            subtype: ActorSubtype::Person,
            identifiers: &[],
            external: None,
            provenance: json!({}),
        };
        let now = chrono::Utc::now().to_rfc3339();
        Ok(actor_bind::bind_in(&tx, &req, &now).await?.0.actor_id)
    })
    .await
    .unwrap()
    .value;
    for text in ["Jane Doe bought Widgets", "Jane Doe sued Gadgets"] {
        let m = memories::add_memory(&repo, text, "fact", false, source(), None)
            .await
            .unwrap();
        recon::put_actor_ref(&repo, &m.id, &actor, "subject")
            .await
            .unwrap();
    }
    let payload = ScopeAnalysisPayload {
        seed: TopologySeed::Actor(actor.clone()),
        cycle_scope: CycleScope::AllRetained,
        depth: 1,
        options: TopologyOptions::default(),
    };
    let job = brain_scope::enqueue(&repo, &payload).await.unwrap();
    assert_eq!(brain_scope::enqueue(&repo, &payload).await.unwrap(), job);
    assert_eq!(
        count(
            &repo,
            "RETURN count(SELECT 1 FROM task WHERE pool = 'brain_scope')"
        )
        .await,
        1
    );

    let stop = AtomicBool::new(false);
    assert_eq!(brain_scope::drain(&repo, "w1", &stop).await.unwrap(), 1);
    assert_eq!(
        count(
            &repo,
            "RETURN count(SELECT 1 FROM task WHERE pool = 'brain_scope' AND state = 'completed')"
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &repo,
            "RETURN count(SELECT 1 FROM memory WHERE memory_kind = 'inference')"
        )
        .await,
        1
    );
    let links = count(
        &repo,
        "RETURN count(SELECT 1 FROM memory WHERE memory_kind = 'link')",
    )
    .await;
    assert!(links >= 2, "links {links}");
    assert_eq!(brain_scope::drain(&repo, "w1", &stop).await.unwrap(), 0);

    let again = brain_scope::analyze(&repo, &payload, None).await.unwrap();
    assert_eq!(again.links_created, 0, "{again:?}");
}
