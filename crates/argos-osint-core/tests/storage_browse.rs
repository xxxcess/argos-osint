//! Brain browse, get and delete on the async repository (migration phase
//! 4), plus the lexical fallback of recall when embedding is off or
//! failing. Both modes via `scripts/test_surreal.py`.
#![cfg(feature = "surreal")]

mod support;

use std::sync::Arc;

use argos_osint_core::brain::MemorySource;
use argos_osint_core::brain_browse::BrowseRow;
use argos_osint_core::memory_metadata::MemoryKind;
use argos_osint_core::storage::vectors::{self, Embedder};
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

fn ok_embedder() -> Embedder {
    Embedder {
        model: "fake".into(),
        embed: Arc::new(|texts: Vec<String>| {
            Ok(texts
                .iter()
                .map(|t| {
                    let mut v = vec![0.0f32; 384];
                    v[t.len() % 384] = 1.0;
                    v
                })
                .collect())
        }),
    }
}

async fn count(repo: &argos_osint_core::storage::repo::Repository, sql: &str) -> i64 {
    let mut r = repo.db().query(sql).await.unwrap().check().unwrap();
    let n: Option<i64> = r.take(0usize).unwrap();
    n.unwrap_or(0)
}

#[tokio::test]
async fn browse_get_and_delete_through_the_repository() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "browse").await;
    let add = |text: &'static str, pinned: bool| {
        let repo = repo.clone();
        async move {
            memories::add_memory(&repo, text, "fact", pinned, source(), None)
                .await
                .unwrap()
        }
    };
    let a = add("Acme ships via Rotterdam", false).await;
    let b = add("Pinned harbor note", true).await;
    let inf = add("Acme probably expands", false).await;
    let old = add("Archived Acme claim", false).await;
    repo.db()
        .query(
            "UPDATE $i SET memory_kind = 'inference' RETURN NONE;
             UPDATE $o SET lifecycle = 'archived' RETURN NONE;
             FOR $k IN ['a1', 'a2', 'a3', 'a4'] {
               CREATE type::record('memory', $k) SET text = $k, memory_kind = 'actor', source_json = '{}' RETURN NONE;
             };
             UPDATE memory:a4 SET lifecycle = 'archived' RETURN NONE;
             CREATE brain_actor:a1 SET display_name = 'ACME Corp', created_at = '', updated_at = '' RETURN NONE;
             CREATE brain_actor:a2 SET display_name = 'Acme  corp.', created_at = '', updated_at = '' RETURN NONE;
             CREATE brain_actor:a3 SET display_name = 'Acme old', created_at = '', updated_at = '' RETURN NONE;
             CREATE brain_actor:a4 SET display_name = 'Gone Ltd', created_at = '', updated_at = '' RETURN NONE;
             CREATE brain_actor_redirect:a3 SET to_actor = 'a2', created_at = '' RETURN NONE;",
        )
        .bind(("i", surrealdb::types::RecordId::new("memory", inf.id.clone())))
        .bind(("o", surrealdb::types::RecordId::new("memory", old.id.clone())))
        .await
        .unwrap()
        .check()
        .unwrap();

    let rows = browse::brain_browse(&repo, None, "").await.unwrap();
    let claims: Vec<_> = rows
        .iter()
        .filter_map(|r| match r {
            BrowseRow::Claim(m) => Some(m.id.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        claims,
        vec![b.id.clone(), a.id.clone()],
        "active claims, pinned first"
    );
    let groups = browse::actor_groups(&repo).await.unwrap();
    assert_eq!(
        groups.len(),
        1,
        "a4 is archived, a3 is redirected: {groups:?}"
    );
    assert_eq!(groups[0].name_key, "acme corp");
    assert_eq!(groups[0].actor_ids, vec!["a1", "a2", "a3"]);

    let acme = browse::brain_browse(&repo, None, "ACME").await.unwrap();
    assert_eq!(acme.len(), 2, "one claim and the group");
    let only_actors = browse::brain_browse(&repo, Some(MemoryKind::Actor), "")
        .await
        .unwrap();
    assert!(matches!(only_actors.as_slice(), [BrowseRow::ActorGroup(_)]));
    assert_eq!(
        browse::brain_browse(&repo, Some(MemoryKind::Claim), "")
            .await
            .unwrap()
            .len(),
        2
    );

    assert_eq!(
        browse::get_memory(&repo, &a.id).await.unwrap(),
        Some(a.clone())
    );
    assert_eq!(
        browse::get_memory(&repo, &old.id)
            .await
            .unwrap()
            .map(|m| m.id),
        Some(old.id.clone()),
        "get ignores lifecycle"
    );
    assert!(browse::get_memory(&repo, "nope").await.unwrap().is_none());

    vectors::reindex(&repo, &ok_embedder()).await.unwrap();
    assert!(browse::delete_memory(&repo, &a.id).await.unwrap());
    assert!(browse::get_memory(&repo, &a.id).await.unwrap().is_none());
    assert!(
        !browse::delete_memory(&repo, &a.id).await.unwrap(),
        "second delete is a no-op"
    );
    assert!(!browse::delete_memory(&repo, "nope").await.unwrap());
    assert_eq!(
        count(
            &repo,
            &format!(
                "RETURN count(SELECT id FROM memory_tombstone WHERE memory_id = '{}')",
                a.id
            )
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &repo,
            &format!(
                "RETURN count(SELECT id FROM memory_embeddings WHERE key = '{}')",
                a.id
            )
        )
        .await,
        0
    );
}

#[tokio::test]
async fn recall_falls_back_to_lexical_when_embedding_is_off_or_failing() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "lexical_fallback").await;
    let m = memories::add_memory(
        &repo,
        "Convoy refuels at Gdansk",
        "fact",
        false,
        source(),
        None,
    )
    .await
    .unwrap();
    memories::add_memory(
        &repo,
        "Unrelated weather note",
        "fact",
        false,
        source(),
        None,
    )
    .await
    .unwrap();
    let first = |hits: Vec<memories::ScoredMemory>| hits.first().map(|h| h.memory.id.clone());

    // Embedding disabled, before and after a generation is served.
    assert_eq!(
        first(
            memories::recall(&repo, "where does the convoy refuel", 5, None)
                .await
                .unwrap()
        ),
        Some(m.id.clone())
    );
    vectors::reindex(&repo, &ok_embedder()).await.unwrap();
    assert_eq!(
        first(
            memories::recall(&repo, "where does the convoy refuel", 5, None)
                .await
                .unwrap()
        ),
        Some(m.id.clone())
    );

    // Embedding failing for the served model.
    let failing = Embedder {
        model: "fake".into(),
        embed: Arc::new(|_| anyhow::bail!("injected embedding failure")),
    };
    assert_eq!(
        first(
            memories::recall(&repo, "where does the convoy refuel", 5, Some(&failing))
                .await
                .unwrap()
        ),
        Some(m.id.clone())
    );
}
