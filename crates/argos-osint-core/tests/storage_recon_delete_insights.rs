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

/// Deleting the origin investigation, with or without insights, retires its
/// runs' insight owners (generation bump, `origin investigation deleted`),
/// marks their active links `origin_deleted`, drops the thread's NLP source
/// refs, and hard-deletes doomed memories without a tombstone.
#[tokio::test]
async fn delete_thread_retires_recon_owners_and_nlp_cache() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "recon_delete_owners").await;
    for insights in [false, true] {
        let t = rt::new_thread(&repo, "doomed").await.unwrap();
        let run = format!("run-{insights}");
        repo.db()
            .query(
                "CREATE type::record('recon_run', $run) SET thread_id = $t, turn_id = 'turn',
                   state = 'completed', stage = 'done', recon_model = 'm', synthesis_model = 'm',
                   max_rounds = 1, max_calls = 1, turn_seconds = 1,
                   created_at = '2026-10-24', updated_at = '2026-10-24';
                 CREATE type::record('insight_owner_state', ['recon', $run]) SET owner_kind = 'recon',
                   owner_id = $run, desired_generation = 3, active_snapshot_id = $snap,
                   lifecycle = 'current', updated_at = '2026-10-24';
                 CREATE insight_derivation_link SET snapshot_id = $snap, left_kind = 'claim',
                   left_id = 'a', right_kind = 'claim', right_id = 'b', relation = 'supports',
                   rationale = '', assessment_json = '{}', evidence_json = '[]',
                   status = 'active', revision = 'r';
                 CREATE nlp_source_ref SET owner_kind = 'thread', owner_id = $t, source_id = 's',
                   revision = 'r', attempt_id = 'a', prose_hash = 'h', metadata_json = '{}',
                   transform_version = 'v';",
            )
            .bind(("run", run.clone()))
            .bind(("t", t.id.clone()))
            .bind(("snap", format!("snap-{insights}")))
            .await
            .unwrap()
            .check()
            .unwrap();
        let doomed = memories::add_memory(&repo, "from here", "fact", false, source(&t.id), None)
            .await
            .unwrap()
            .id;
        let gone = if insights {
            recon::delete_thread_with_insights(&repo, &t.id).await
        } else {
            recon::delete_thread(&repo, &t.id).await
        };
        assert!(gone.unwrap());
        let owner = format!("type::record('insight_owner_state', ['recon', '{run}'])");
        assert_eq!(
            n(&repo, &format!("RETURN {owner}.desired_generation")).await,
            4
        );
        assert_eq!(
            n(
                &repo,
                &format!(
                    "RETURN <int>({owner}.lifecycle = 'previous'
                       AND {owner}.last_error = 'origin investigation deleted')"
                )
            )
            .await,
            1
        );
        assert_eq!(
            n(
                &repo,
                &format!(
                    "RETURN count(SELECT id FROM insight_derivation_link
                       WHERE snapshot_id = 'snap-{insights}' AND status = 'origin_deleted')"
                )
            )
            .await,
            1
        );
        assert_eq!(
            n(
                &repo,
                &format!(
                    "RETURN count(SELECT id FROM nlp_source_ref
                       WHERE owner_kind = 'thread' AND owner_id = '{}')",
                    t.id
                )
            )
            .await,
            0
        );
        let exists = n(
            &repo,
            &format!("RETURN <int>record::exists(type::record('memory', '{doomed}'))"),
        )
        .await;
        let tombstones = n(
            &repo,
            &format!("RETURN count(SELECT id FROM memory_tombstone WHERE memory_id = '{doomed}')"),
        )
        .await;
        if insights {
            assert_eq!((exists, tombstones), (0, 0), "hard delete, no tombstone");
        } else {
            assert_eq!(exists, 1, "plain delete keeps memories");
        }
    }
}
