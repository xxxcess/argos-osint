//! Recon `delete_thread` with its insights on SurrealDB
//! (`storage::recon::delete_thread_with_insights`, `ReconStore::delete_thread(_, true)`):
//! memories this thread alone sourced go, shared claims keep their other
//! sources. Both modes via `scripts/test_surreal.py`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::brain::MemorySource;
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::storage::{memories, recon, recon_threads as rt};
use support::*;

fn source(conversation: &str) -> MemorySource {
    MemorySource {
        app: "recon".into(),
        conversation_id: conversation.into(),
        message_id: None,
        reference: None,
    }
}

async fn n(repo: &Repository, sql: &str) -> i64 {
    let mut r = repo.db().query(sql).await.unwrap().check().unwrap();
    r.take::<Option<i64>>(0usize).unwrap().unwrap_or(-1)
}

async fn claim(repo: &Repository, fp: &str, memory: &str, threads: &[&str]) {
    repo.db()
        .query(
            "CREATE type::record('insight_claim', $fp) SET memory_id = $m, entity_id = 'e',
               predicate = 'p', object_value = 'o', topic = 't', classification = 'fact',
               confidence = 0.9, created_at = '2026-10-24', updated_at = '2026-10-24';
             FOR $t IN $threads {
               CREATE insight_source SET fingerprint = $fp, thread_id = $t, run_id = 'run',
                 answer_id = 'answer-' + $t, call_id = 'call-' + $t;
             };",
        )
        .bind(("fp", fp.to_string()))
        .bind(("m", memory.to_string()))
        .bind((
            "threads",
            threads.iter().map(|t| t.to_string()).collect::<Vec<_>>(),
        ))
        .await
        .unwrap()
        .check()
        .unwrap();
}

#[tokio::test]
async fn delete_thread_with_insights_removes_what_it_alone_sourced() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "recon_delete_insights").await;
    let t = rt::new_thread(&repo, "doomed").await.unwrap();
    let keep = rt::new_thread(&repo, "kept").await.unwrap();
    let add = |text: &'static str, conv: String| {
        let repo = &repo;
        async move {
            memories::add_memory(repo, text, "fact", false, source(&conv), None)
                .await
                .unwrap()
                .id
        }
    };
    let only = add("only this thread", "other".into()).await;
    let shared = add("both threads", "other".into()).await;
    let conv = add("from this conversation", t.id.clone()).await;
    claim(&repo, "fp-only", &only, &[&t.id]).await;
    claim(&repo, "fp-shared", &shared, &[&t.id, &keep.id]).await;

    assert!(recon::delete_thread_with_insights(&repo, &t.id)
        .await
        .unwrap());
    let exists = |id: &str| format!("RETURN <int>record::exists(type::record('memory', '{id}'))");
    assert_eq!(n(&repo, &exists(&only)).await, 0);
    assert_eq!(n(&repo, &exists(&conv)).await, 0);
    assert_eq!(n(&repo, &exists(&shared)).await, 1);
    assert_eq!(
        n(&repo, "RETURN <int>record::exists(insight_claim:⟨fp-only⟩)").await,
        0
    );
    assert_eq!(
        n(
            &repo,
            "RETURN count(SELECT id FROM insight_source WHERE fingerprint = 'fp-shared')"
        )
        .await,
        1,
        "the other thread's source stays"
    );
    assert_eq!(
        n(
            &repo,
            &format!(
                "RETURN count(SELECT id FROM insight_source WHERE thread_id = '{}')",
                t.id
            )
        )
        .await,
        0
    );
    assert!(recon::get_thread(&repo, &t.id).await.unwrap().is_none());
    assert!(!recon::delete_thread_with_insights(&repo, &t.id)
        .await
        .unwrap());
}
