//! Brain browse associations on SurrealDB (migration phase 4a): Actor-group
//! claim/thread/article counts from typed claim actors (through redirects),
//! actor refs, direct mentions and active Links; claim associations; and
//! article origins from live derivations and insight sources. Both modes via
//! `scripts/test_surreal.py`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::brain::MemorySource;
use argos_osint_core::memory_metadata::{
    ActorRef, ClaimPayload, IdentityState, MemoryKind, MemoryPayload,
};
use argos_osint_core::recon::Thread;
use argos_osint_core::related_articles::ArticleOrigin;
use argos_osint_core::storage::recon::{self, Endpoint};
use argos_osint_core::storage::{atlas, browse_assoc, memories};
use argos_osint_core::store::AtlasArticleRow;
use support::*;

fn source() -> MemorySource {
    MemorySource {
        app: "cli".into(),
        conversation_id: "c1".into(),
        message_id: None,
        reference: None,
    }
}

fn thread(id: &str, title: &str) -> Thread {
    Thread {
        id: id.into(),
        title: title.into(),
        created_at: "2026-10-23T00:00:00Z".into(),
        updated_at: "2026-10-23T00:00:00Z".into(),
        draft: String::new(),
        scroll: 0,
        recall_insights: true,
    }
}

fn article(id: &str, url: &str) -> AtlasArticleRow {
    AtlasArticleRow {
        run_id: "r1".into(),
        id: id.into(),
        title: format!("Title {id}"),
        url: url.into(),
        category: "unk".into(),
        seen_at: format!("2026-10-23T00:00:0{}Z", id.len()),
        ..Default::default()
    }
}

#[tokio::test]
async fn actor_groups_count_claims_threads_and_articles() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "browse_assoc").await;
    let db = repo.db().clone();
    atlas::insert_run(&db, "r1", "{}", "{}").await.unwrap();
    atlas::upsert_article(&db, &article("a1", "https://ex.com/x"))
        .await
        .unwrap();
    atlas::upsert_article(&db, &article("a22", "https://ex.com/y"))
        .await
        .unwrap();
    recon::put_thread(&repo, &thread("t1", "Thread One"))
        .await
        .unwrap();
    recon::put_thread(&repo, &thread("t2", "Thread Two"))
        .await
        .unwrap();

    // Actors: act1 and act2 share a name; act3 redirects to act1.
    db.query(
        "CREATE memory:act1 SET text = 'Acme Corp', memory_kind = 'actor';
         CREATE memory:act2 SET text = 'ACME corp', memory_kind = 'actor';
         CREATE memory:act3 SET text = 'Acme', memory_kind = 'actor', lifecycle = 'redirected';
         CREATE brain_actor:act1 SET display_name = 'Acme Corp', created_at = '', updated_at = '';
         CREATE brain_actor:act2 SET display_name = 'ACME corp', created_at = '', updated_at = '';
         CREATE brain_actor:act3 SET display_name = 'Acme', created_at = '', updated_at = '';
         CREATE brain_actor_redirect:act3 SET to_actor = 'act1', created_at = '';
         CREATE memory:l1 SET text = 'link', memory_kind = 'link';
         CREATE memory:l2 SET text = 'old link', memory_kind = 'link', lifecycle = 'retired';
         CREATE brain_actor_mention SET actor_id = 'act1', origin_kind = 'thread',
           origin_id = 't2', created_at = '';
         CREATE brain_actor_mention SET actor_id = 'act2', origin_kind = 'article',
           origin_scope = 'r1', origin_id = 'a22', origin_revision = 'stale', created_at = '';",
    )
    .await
    .unwrap()
    .check()
    .unwrap();

    let c1 = memories::add_memory(&repo, "Acme bought Widgets", "fact", false, source(), None)
        .await
        .unwrap();
    let c2 = memories::add_memory(&repo, "Acme sued Gadgets", "fact", false, source(), None)
        .await
        .unwrap();
    let c3 = memories::add_memory(&repo, "Unrelated claim", "fact", false, source(), None)
        .await
        .unwrap();
    let payload = serde_json::to_string(&MemoryPayload::Claim(ClaimPayload {
        actors: vec![ActorRef {
            actor_id: "act3".into(),
            role: "subject".into(),
            binding_state: IdentityState::default(),
        }],
        ..Default::default()
    }))
    .unwrap();
    db.query(
        "UPDATE type::record('memory', $c1) SET payload_json = $p RETURN NONE;
         CREATE insight_claim:fp1 SET memory_id = $c1, entity_id = 'e', predicate = 'p',
           object_value = 'o', topic = 't', classification = 'fact', confidence = 0.5,
           created_at = '', updated_at = '';
         CREATE insight_source SET fingerprint = 'fp1', thread_id = 't1', answer_id = 'x',
           call_id = 'c';
         CREATE insight_claim:fp2 SET memory_id = $c2, entity_id = 'e', predicate = 'p',
           object_value = 'o', topic = 't', classification = 'fact', confidence = 0.5,
           created_at = '', updated_at = '';
         CREATE insight_source SET fingerprint = 'fp2', run_id = 'r1', answer_id = 'x',
           call_id = 'article:a1', source_url = 'https://ex.com/x#top';
         CREATE insight_source SET fingerprint = 'fp2', thread_id = 't2', answer_id = 'x',
           call_id = 'gone', deleted_origin = true;
         CREATE type::record('insight_snapshot', 's1') SET owner_kind = 'article', owner_id = 'a22',
           generation = 1, input_key = 'k', inputs_json = '{}', entities_json = '[]',
           coverage_json = '{}', readiness_json = '{}', contract_version = 'c',
           policy_version = 'p', created_at = '1';
         CREATE type::record('insight_derivation', 'd1') SET snapshot_id = 's1', memory_id = $c3,
           insight_kind = 'claim', fingerprint = 'f', item_json = '{}', assessment_json = '{}',
           status = 'assessed', revision = '1';
         CREATE type::record('insight_derivation', 'd2') SET snapshot_id = 's1', memory_id = $c3,
           insight_kind = 'claim', fingerprint = 'f', item_json = '{}', assessment_json = '{}',
           status = 'retired', revision = '1';
         CREATE insight_derivation_evidence SET derivation_id = 'd2', source_id = 'article:a1',
           source_revision = 'r0', memory_id = $c3, evidence_json = '{}';",
    )
    .bind(("c1", c1.id.clone()))
    .bind(("c2", c2.id.clone()))
    .bind(("c3", c3.id.clone()))
    .bind(("p", payload))
    .await
    .unwrap()
    .check()
    .unwrap();
    recon::put_actor_ref(&repo, &c2.id, "act2", "subject")
        .await
        .unwrap();
    // An active Link act1 -> a1 counts; a retired one does not.
    for (link, art) in [("l1", "a1"), ("l2", "a22")] {
        let side = |side, kind, r, o| Endpoint {
            memory_id: link,
            side,
            endpoint_kind: kind,
            ref_id: r,
            origin: o,
            revision: "",
        };
        recon::put_endpoint(&repo, &side("source", "memory", "act1", ""))
            .await
            .unwrap();
        recon::put_endpoint(&repo, &side("target", "article", art, "r1"))
            .await
            .unwrap();
    }

    // Article origins: a live derivation owned by an article (any run);
    // the retired one and its evidence are ignored.
    assert_eq!(
        browse_assoc::article_origins_for_memory(&repo, &c3.id)
            .await
            .unwrap(),
        vec![ArticleOrigin {
            run_id: "r1".into(),
            article_id: "a22".into()
        }]
    );
    assert_eq!(
        browse_assoc::article_origins_for_memory(&repo, &c2.id)
            .await
            .unwrap(),
        vec![ArticleOrigin {
            run_id: "r1".into(),
            article_id: "a1".into()
        }]
    );

    let c1a = browse_assoc::claim_associations(&repo, &c1.id)
        .await
        .unwrap();
    assert_eq!(c1a.threads, vec![("t1".into(), "Thread One".into())]);
    assert_eq!(c1a.actors.len(), 1, "act3 resolves into the Acme group");
    assert_eq!(c1a.actors[0].name_key, "acme corp");
    assert!(c1a.articles.is_empty());
    let c2a = browse_assoc::claim_associations(&repo, &c2.id)
        .await
        .unwrap();
    assert!(c2a.threads.is_empty(), "a deleted origin is not a thread");
    assert_eq!(c2a.articles.len(), 1);
    assert_eq!(c2a.articles[0].article.id, "a1");

    let group = c1a.actors[0].clone();
    let ga = browse_assoc::actor_group_associations(&repo, &group)
        .await
        .unwrap();
    let mut claims: Vec<_> = ga.claims.iter().map(|m| m.id.clone()).collect();
    claims.sort();
    let mut want = vec![c1.id.clone(), c2.id.clone()];
    want.sort();
    assert_eq!(claims, want, "typed (via redirect) and actor-ref claims");
    assert_eq!(
        ga.threads,
        vec![
            ("t1".into(), "Thread One".into()),
            ("t2".into(), "Thread Two".into())
        ]
    );
    // a1 through the Link and c2's source; a22's mention is stale and its
    // Link retired.
    assert_eq!(
        ga.articles
            .iter()
            .map(|a| a.article.id.as_str())
            .collect::<Vec<_>>(),
        vec!["a1"]
    );

    let rows = browse_assoc::brain_browse_memories(&repo, None, "")
        .await
        .unwrap();
    let g = rows
        .iter()
        .find(|m| m.id == "actor-group:acme corp")
        .expect("group row");
    assert_eq!(g.category, "2 claims · 2 threads · 1 articles");
    assert_eq!(g.source.app, "brain-actor-group");
    assert_eq!(
        rows.iter()
            .filter(|m| m.id.starts_with("actor-group:"))
            .count(),
        1
    );
    assert_eq!(rows.len(), 4, "three claims and one group");
    let only_claims = browse_assoc::brain_browse_memories(&repo, Some(MemoryKind::Claim), "sued")
        .await
        .unwrap();
    assert_eq!(only_claims, vec![c2]);
}
