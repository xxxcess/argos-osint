//! Recon threads, messages and Brain edge tables on SurrealDB (migration
//! phase 4a): the delete cascades that replace the legacy store's foreign keys, the
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

#[tokio::test]
async fn recon_threads_and_messages_match_the_legacy_store() {
    use argos_osint_core::storage::recon_threads as rt;
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "recon_threads").await;
    let blank = rt::new_thread(&repo, "  ").await.unwrap();
    assert_eq!(blank.title, argos_osint_core::recon::PLACEHOLDER_TITLE);
    let t = rt::new_thread(&repo, " example.org ").await.unwrap();
    assert_eq!(t.title, "example.org");

    // Messages from an import (explicit sequences) are never overtaken.
    recon::put_message(&repo, &message("m-old", &t.id, 3))
        .await
        .unwrap();
    let a = rt::add_message(&repo, &t.id, "user", "who?", None)
        .await
        .unwrap();
    let b = rt::add_message(&repo, &t.id, "assistant", "them", Some("run-1"))
        .await
        .unwrap();
    assert_eq!((a.sequence, b.sequence), (4, 5));
    let other = rt::add_message(&repo, &blank.id, "user", "hi", None)
        .await
        .unwrap();
    assert_eq!(other.sequence, 1, "sequences are per thread");
    let listed = rt::list_messages(&repo, &t.id).await.unwrap();
    assert_eq!(
        listed.iter().map(|m| m.sequence).collect::<Vec<_>>(),
        vec![3, 4, 5]
    );
    assert_eq!(listed[2].run_id.as_deref(), Some("run-1"));
    assert!(rt::add_message(&repo, "thread-missing", "user", "x", None)
        .await
        .is_err());

    // Rename, draft, recall toggle; title or entity search.
    assert!(rt::rename_thread(&repo, &t.id, "Port scan").await.unwrap());
    assert!(rt::rename_thread(&repo, &t.id, " ").await.is_err());
    assert!(!rt::rename_thread(&repo, "nope", "x").await.unwrap());
    rt::save_draft(&repo, &t.id, "draft", 7).await.unwrap();
    assert!(rt::set_recall_insights(&repo, &t.id, true).await.unwrap());
    let got = recon::get_thread(&repo, &t.id).await.unwrap().unwrap();
    assert_eq!(
        (
            got.title.as_str(),
            got.draft.as_str(),
            got.scroll,
            got.recall_insights
        ),
        ("Port scan", "draft", 7, true)
    );
    rt::link_entity(&repo, &t.id, "domain", "Example.org", Some("call-1"))
        .await
        .unwrap();
    rt::link_entity(&repo, &t.id, "domain", "Example.org", Some("call-1"))
        .await
        .unwrap();
    rt::link_entity(&repo, &t.id, "email", "a@example.org", None)
        .await
        .unwrap();
    assert_eq!(
        rt::thread_entities(&repo, &t.id).await.unwrap(),
        vec![
            ("domain".to_string(), "Example.org".to_string()),
            ("email".to_string(), "a@example.org".to_string())
        ]
    );
    let ids =
        |v: Vec<argos_osint_core::recon::Thread>| v.into_iter().map(|t| t.id).collect::<Vec<_>>();
    assert_eq!(
        ids(rt::list_threads(&repo, "port").await.unwrap()),
        vec![t.id.clone()]
    );
    assert_eq!(
        ids(rt::list_threads(&repo, "example.ORG").await.unwrap()),
        vec![t.id.clone()]
    );
    assert_eq!(
        ids(rt::list_threads(&repo, "").await.unwrap()),
        vec![t.id.clone(), blank.id.clone()],
        "newest update first"
    );

    // Last thread follows selection and forgets deleted threads.
    assert_eq!(rt::last_thread(&repo).await.unwrap(), None);
    rt::select_thread(&repo, &t.id).await.unwrap();
    assert_eq!(rt::last_thread(&repo).await.unwrap(), Some(t.id.clone()));
    assert!(rt::select_thread(&repo, "nope").await.is_err());
    assert!(recon::delete_thread(&repo, &t.id).await.unwrap());
    assert_eq!(rt::last_thread(&repo).await.unwrap(), None);
    assert_eq!(
        count(&repo, "RETURN count(SELECT * FROM recon_thread_entity)").await,
        0,
        "entity links cascade"
    );
}

#[tokio::test]
async fn concurrent_appends_get_distinct_sequences() {
    use argos_osint_core::storage::recon_threads as rt;
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "recon_seq").await;
    let t = rt::new_thread(&repo, "busy").await.unwrap();
    let mut tasks = Vec::new();
    for i in 0..6 {
        let (repo, id) = (repo.clone(), t.id.clone());
        tasks.push(tokio::spawn(async move {
            rt::add_message(&repo, &id, "user", &format!("q{i}"), None)
                .await
                .unwrap()
                .sequence
        }));
    }
    let mut seqs = Vec::new();
    for t in tasks {
        seqs.push(t.await.unwrap());
    }
    seqs.sort();
    assert_eq!(seqs, (1..=6).collect::<Vec<_>>());
}

#[tokio::test]
async fn tool_preferences_default_and_persist() {
    use argos_osint_core::storage::recon_threads as rt;
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "recon_tools").await;
    assert!(rt::tool_enabled(&repo, "hunter_person_enrichment")
        .await
        .unwrap());
    assert!(!rt::tool_enabled(&repo, "firecrawl_crawl").await.unwrap());
    rt::set_tool_enabled(&repo, "hunter_person_enrichment", false)
        .await
        .unwrap();
    rt::set_tool_enabled(&repo, "firecrawl_crawl", true)
        .await
        .unwrap();
    assert!(!rt::tool_enabled(&repo, "hunter_person_enrichment")
        .await
        .unwrap());
    assert!(rt::tool_enabled(&repo, "firecrawl_crawl").await.unwrap());
    assert!(rt::set_tool_enabled(&repo, "no_such_tool", true)
        .await
        .is_err());
}
