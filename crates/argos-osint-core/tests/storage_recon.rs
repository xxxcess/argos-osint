//! Recon threads, messages and Brain edge tables on SurrealDB (migration
//! phase 4a): the delete cascades that replace SQLite's foreign keys, the
//! parent assertions, and `answer_memories`. Both modes via
//! `scripts/test_surreal.py`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::brain::MemorySource;
use argos_osint_core::recon::{Message, Thread};
use argos_osint_core::storage::recon::{self, Endpoint};
use argos_osint_core::storage::{browse, memories};
use support::*;

fn source() -> MemorySource {
    MemorySource {
        app: "cli".into(),
        conversation_id: "c1".into(),
        message_id: None,
        reference: None,
    }
}

fn thread(id: &str) -> Thread {
    Thread {
        id: id.into(),
        title: format!("T {id}"),
        created_at: "2026-10-23T00:00:00Z".into(),
        updated_at: "2026-10-23T00:00:00Z".into(),
        draft: String::new(),
        scroll: 0,
        recall_insights: true,
    }
}

fn message(id: &str, thread: &str, seq: i64) -> Message {
    Message {
        id: id.into(),
        thread_id: thread.into(),
        sequence: seq,
        role: "assistant".into(),
        content: format!("answer {id}"),
        run_id: None,
        created_at: "2026-10-23T00:00:00Z".into(),
    }
}

fn e<'a>(side: &'a str, m: &'a str) -> Endpoint<'a> {
    Endpoint {
        memory_id: m,
        side,
        endpoint_kind: "memory",
        ref_id: "x",
        origin: "",
        revision: "",
    }
}

async fn count(repo: &argos_osint_core::storage::repo::Repository, sql: &str) -> i64 {
    let mut r = repo.db().query(sql).await.unwrap().check().unwrap();
    let n: Option<i64> = r.take(0usize).unwrap();
    n.unwrap_or(0)
}

#[tokio::test]
async fn thread_delete_cascades_and_leaves_no_orphans() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "recon").await;
    let a = memories::add_memory(&repo, "Claim A", "fact", false, source(), None)
        .await
        .unwrap();
    let b = memories::add_memory(&repo, "Claim B", "fact", false, source(), None)
        .await
        .unwrap();
    for t in ["t1", "t2"] {
        recon::put_thread(&repo, &thread(t)).await.unwrap();
    }
    assert_eq!(
        recon::get_thread(&repo, "t1").await.unwrap(),
        Some(thread("t1"))
    );
    recon::put_message(&repo, &message("m1", "t1", 1))
        .await
        .unwrap();
    recon::put_message(&repo, &message("m2", "t1", 2))
        .await
        .unwrap();
    recon::put_message(&repo, &message("m3", "t2", 1))
        .await
        .unwrap();
    assert!(
        recon::put_message(&repo, &message("mx", "t1", 1))
            .await
            .is_err(),
        "sequence is unique per thread"
    );
    assert!(
        recon::put_message(&repo, &message("my", "nope", 1))
            .await
            .is_err(),
        "a message needs its thread"
    );
    recon::link_message_memories(&repo, "m1", &[b.id.clone(), a.id.clone()])
        .await
        .unwrap();
    recon::link_message_memories(&repo, "m2", std::slice::from_ref(&a.id))
        .await
        .unwrap();
    recon::link_message_memories(&repo, "m3", std::slice::from_ref(&a.id))
        .await
        .unwrap();
    assert!(
        recon::link_message_memories(&repo, "nope", std::slice::from_ref(&a.id))
            .await
            .is_err()
    );

    // Endpoints and actor refs, and their memory cascade.
    recon::put_endpoint(&repo, &e("source", &b.id))
        .await
        .unwrap();
    recon::put_endpoint(&repo, &e("target", &b.id))
        .await
        .unwrap();
    recon::put_endpoint(&repo, &e("source", &a.id))
        .await
        .unwrap();
    assert!(recon::put_endpoint(&repo, &e("source", "nope"))
        .await
        .is_err());
    assert!(recon::put_endpoint(&repo, &e("middle", &a.id))
        .await
        .is_err());
    recon::put_actor_ref(&repo, &b.id, "actor-1", "subject")
        .await
        .unwrap();
    recon::put_actor_ref(&repo, &a.id, "actor-1", "subject")
        .await
        .unwrap();

    let answers = recon::answer_memories(&repo, "t1").await.unwrap();
    assert_eq!(
        answers["m1"]
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        vec![b.id.as_str(), a.id.as_str()],
        "cited order"
    );
    assert_eq!(answers["m1"][1], a);
    assert_eq!(answers.len(), 2);

    assert!(browse::delete_memory(&repo, &b.id).await.unwrap());
    let answers = recon::answer_memories(&repo, "t1").await.unwrap();
    assert_eq!(
        answers["m1"][0].category, "missing",
        "a deleted memory reads as gone"
    );
    let mb = format!("'{}'", b.id);
    assert_eq!(
        count(
            &repo,
            &format!("RETURN count(SELECT id FROM brain_memory_endpoint WHERE memory_id = {mb})")
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &repo,
            &format!("RETURN count(SELECT id FROM brain_memory_actor_ref WHERE memory_id = {mb})")
        )
        .await,
        0
    );
    assert_eq!(
        count(&repo, "RETURN count(SELECT id FROM brain_memory_endpoint)").await,
        1
    );

    // Thread delete: messages and links go; the other thread is untouched.
    repo.db()
        .query("CREATE brain_actor_mention SET actor_id = 'actor-1', origin_kind = 'thread', origin_id = 't1', created_at = '' RETURN NONE")
        .await
        .unwrap()
        .check()
        .unwrap();
    assert!(recon::delete_thread(&repo, "t1").await.unwrap());
    assert!(!recon::delete_thread(&repo, "t1").await.unwrap());
    assert!(recon::get_thread(&repo, "t1").await.unwrap().is_none());
    for (what, sql) in [
        ("messages", "RETURN count(SELECT id FROM recon_message WHERE thread_id = 't1')"),
        ("links", "RETURN count(SELECT id FROM recon_message_memory WHERE message_id INSIDE ['m1', 'm2'])"),
        ("mentions", "RETURN count(SELECT id FROM brain_actor_mention WHERE origin_id = 't1')"),
        (
            "orphan messages",
            "RETURN count(SELECT id FROM recon_message WHERE !record::exists(type::record('recon_thread', thread_id)))",
        ),
        (
            "orphan links",
            "RETURN count(SELECT id FROM recon_message_memory WHERE !record::exists(type::record('recon_message', message_id)))",
        ),
        (
            "orphan endpoints",
            "RETURN count(SELECT id FROM brain_memory_endpoint WHERE !record::exists(type::record('memory', memory_id)))",
        ),
        (
            "orphan actor refs",
            "RETURN count(SELECT id FROM brain_memory_actor_ref WHERE !record::exists(type::record('memory', memory_id)))",
        ),
    ] {
        assert_eq!(count(&repo, sql).await, 0, "{what}");
    }
    assert_eq!(
        recon::answer_memories(&repo, "t2").await.unwrap()["m3"],
        vec![a]
    );
    assert!(recon::answer_memories(&repo, "t1")
        .await
        .unwrap()
        .is_empty());
}
