use super::*;
use crate::atlas_metadata::{ArticleMention, ArticleMetadata, MetadataState};
use crate::store::AtlasArticleRow;
use rusqlite::params;

fn article(run: &str, id: &str, title: &str) -> AtlasArticleRow {
    AtlasArticleRow {
        run_id: run.into(),
        id: id.into(),
        title: title.into(),
        description: String::new(),
        url: format!("https://x.test/{id}"),
        source_name: "X".into(),
        source_domain: "x.test".into(),
        published_at: "2026-10-12".into(),
        provider: "fixture".into(),
        category: "unk".into(),
        ..Default::default()
    }
}

fn seed(store: &Store, run: &str, id: &str, title: &str, names: &[&str]) {
    let _ = store.atlas_insert_run(run, "{}", "{}");
    let title = if names.is_empty() {
        title.to_string()
    } else {
        format!("{title} with Acme")
    };
    let a = article(run, id, &title);
    store.atlas_upsert_article(&a).unwrap();
    let mut m = ArticleMetadata::pending(&a);
    m.entity_state = MetadataState::Complete;
    let mut names = names.to_vec();
    if !names.is_empty() {
        names.push("Acme");
    }
    for n in &names {
        let start = title.find(n).unwrap();
        m.mentions.push(ArticleMention {
            type_hint: "person".into(),
            comparison_key: n.to_lowercase(),
            field: "title".into(),
            start_byte: start,
            end_byte: start + n.len(),
            original_text: (*n).into(),
        });
    }
    assert!(store.atlas_save_metadata(&m).unwrap());
}

fn count(store: &Store, sql: &str) -> i64 {
    store.conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

#[test]
fn cross_cycle_backfill_indexes_actors_links_and_bounded_candidates_idempotently() {
    let store = Store::memory().unwrap();
    seed(&store, "rA", "a1", "Jane Doe visits Kyiv", &["Jane Doe"]);
    seed(&store, "rB", "b1", "Jane Doe resigns", &["Jane Doe"]);
    // Same URL-based article retained in two cycles.
    seed(&store, "rB", "a1", "Jane Doe visits Kyiv", &["Jane Doe"]);
    seed(&store, "rB", "z0", "Weather is mild", &[]);
    let report = store.brain_backfill(1000).unwrap();
    assert!(report.complete, "{report:?}");
    let atlas = report
        .coverage
        .iter()
        .find(|c| c.source == "atlas_articles")
        .unwrap();
    assert_eq!((atlas.done, atlas.total), (4, 4));
    // a1 is one article (same scope) in both cycles → one actor; b1 another.
    assert_eq!(count(&store, "SELECT COUNT(*) FROM brain_actors"), 4);
    assert_eq!(
        count(&store, "SELECT COUNT(*) FROM brain_actor_mentions"),
        6
    );
    assert_eq!(
        count(
            &store,
            "SELECT COUNT(*) FROM memory_metadata WHERE memory_kind='link'"
        ),
        6,
        "one observed mention Link per actor occurrence"
    );
    let related = count(
        &store,
        "SELECT COUNT(*) FROM memory_metadata WHERE memory_kind='related'",
    );
    assert!(
        related >= 1,
        "tentative same-name Related between a1 and b1"
    );
    let basis: String = store
        .conn
        .query_row(
            "SELECT payload_json FROM memory_metadata WHERE memory_kind='related' LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(basis.contains("identity unconfirmed"));
    let zero: String = store
        .conn
        .query_row(
            "SELECT coverage FROM brain_backfill_receipts WHERE unit_id='rB/z0'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(zero, "zero actors");
    // Restart / repeat: nothing new, no duplicates.
    let before = count(&store, "SELECT COUNT(*) FROM memories");
    let again = store.brain_backfill(1000).unwrap();
    assert_eq!(again.processed, 0);
    assert_eq!(count(&store, "SELECT COUNT(*) FROM memories"), before);
}

#[test]
fn bounded_batches_resume_and_revised_text_reindexes_only_that_occurrence() {
    let store = Store::memory().unwrap();
    for i in 0..5 {
        seed(
            &store,
            "rA",
            &format!("a{i}"),
            &format!("Ann Lee item {i}"),
            &["Ann Lee"],
        );
    }
    let first = store.brain_backfill(2).unwrap();
    assert_eq!(first.processed, 2);
    assert!(!first.complete);
    let rest = store.brain_backfill(1000).unwrap();
    assert_eq!(rest.processed, 3);
    assert!(rest.complete);
    // Revised title: old mention/receipt invalidated, only a0 reindexed.
    seed(&store, "rA", "a0", "Ann Lee revised headline", &["Ann Lee"]);
    let re = store.brain_backfill(1000).unwrap();
    assert_eq!(re.processed, 1);
    let live: i64 = count(
        &store,
        "SELECT COUNT(*) FROM brain_actor_mentions WHERE origin_id='a0'",
    );
    assert_eq!(
        live, 2,
        "each current entity replaces its stale revision mention"
    );
}

#[test]
fn deleting_one_cycle_preserves_the_other_and_last_origin_retires() {
    let store = Store::memory().unwrap();
    seed(&store, "rA", "a1", "Jane Doe visits Kyiv", &["Jane Doe"]);
    seed(&store, "rB", "a1", "Jane Doe visits Kyiv", &["Jane Doe"]);
    store.brain_backfill(1000).unwrap();
    let actor: String = store
        .conn
        .query_row("SELECT actor_id FROM brain_actors", [], |r| r.get(0))
        .unwrap();
    store.atlas_delete_run("rA").unwrap();
    let state = || -> String {
        store
            .conn
            .query_row(
                "SELECT lifecycle FROM memory_metadata WHERE memory_id=?1",
                [&actor],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert_eq!(state(), "active");
    assert_eq!(
        crate::brain_actors::mentions(&store.conn, &actor)
            .unwrap()
            .len(),
        1
    );
    store.atlas_delete_run("rB").unwrap();
    assert_eq!(state(), "retired");
}

#[test]
fn legacy_entities_claims_links_and_leads_migrate_to_canonical_records() {
    let store = Store::memory().unwrap();
    let c = &store.conn;
    c.execute_batch(
        "INSERT INTO recon_entities(id,kind,namespace,canonical,label) VALUES('e1','username','github','ada_osint','Ada');",
    )
    .unwrap();
    let m = store
        .add_memory(
            "Ada maintains the repo",
            "fact",
            false,
            crate::brain::MemorySource {
                app: "recon".into(),
                conversation_id: "t1".into(),
                message_id: None,
                reference: None,
            },
        )
        .unwrap();
    c.execute(
        "INSERT INTO insight_claims(fingerprint,memory_id,entity_id,predicate,object_value,topic,classification,confidence,created_at,updated_at)
         VALUES('fp1',?1,'Ada','maintains','repo','t','news',0.9,'x','x')",
        [&m.id],
    )
    .unwrap();
    seed(&store, "rA", "a1", "Ada speaks", &["Ada"]);
    seed(&store, "rA", "a2", "Ada again", &["Ada"]);
    c.execute(
        "INSERT INTO intel_inference_article_leads(id,article_id,source_generation,job_id,inference_id,inference_text,candidate_article_id,candidate_run_id,candidate_metadata_revision,interpretation,verification_needed,citations_json,created_at)
         VALUES('lead1','a1',1,'j','i1','txt','a2','rA','rev','may corroborate','check dates','[]','x')",
        [],
    )
    .unwrap();
    let report = store.brain_backfill(1000).unwrap();
    assert!(report.complete, "{report:?}");
    let refs = count(
        &store,
        "SELECT COUNT(*) FROM brain_memory_actor_refs WHERE role='subject'",
    );
    assert_eq!(refs, 1, "claim subject bound through Brain");
    assert_eq!(
        count(&store, "SELECT COUNT(*) FROM brain_actor_external_refs WHERE scope='recon_entities' AND legacy_id='e1'"),
        1
    );
    let lead: String = store
        .conn
        .query_row("SELECT memory_id FROM brain_legacy_map WHERE legacy_table='intel_inference_article_leads'", [], |r| r.get(0))
        .unwrap();
    match store.memory_payload(&lead).unwrap().unwrap() {
        crate::memory_metadata::MemoryPayload::Related(r) => {
            assert!(r.basis.contains("potential corroboration"));
            assert_eq!(r.verification_gap, "check dates");
        }
        other => panic!("lead must be Related, never a support Link: {other:?}"),
    }
    // Claim memory ID unchanged.
    assert!(store.get_memory(&m.id).unwrap().is_some());
}

#[test]
fn derivation_links_become_canonical_link_memories() {
    let store = Store::memory().unwrap();
    let c = &store.conn;
    let src = |t: &str| crate::brain::MemorySource {
        app: "article".into(),
        conversation_id: t.into(),
        message_id: None,
        reference: None,
    };
    let m1 = store
        .add_memory("Claim one", "fact", false, src("o"))
        .unwrap()
        .id;
    let m2 = store
        .add_memory("Inference two", "fact", false, src("o"))
        .unwrap()
        .id;
    c.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
    for (id, kind) in [(&m1, "claim"), (&m2, "inference")] {
        c.execute(
            "INSERT OR REPLACE INTO memory_metadata(memory_id,memory_kind) VALUES(?1,?2)",
            params![id, kind],
        )
        .unwrap();
    }
    c.execute(
        "INSERT INTO insight_derivations(id,snapshot_id,memory_id,insight_kind,fingerprint,item_json,assessment_json,status,revision)
         VALUES('s:c1','s',?1,'claim','f1','{}','{}','assessed','r'),('s:i1','s',?2,'inference','f2','{}','{}','assessed','r')",
        params![m1, m2],
    )
    .unwrap();
    c.execute(
        "INSERT INTO insight_derivation_links(id,snapshot_id,left_kind,left_id,right_kind,right_id,relation,rationale,assessment_json,evidence_json,status,revision)
         VALUES('s:l1','s','claim','c1','inference','i1','supports','because','{}','[]','assessed','r1'),
               ('s:l2','s','claim','c1','inference','i1','weird','x','{}','[]','assessed','r1')",
        [],
    )
    .unwrap();
    let report = store.brain_backfill(1000).unwrap();
    assert!(report.complete, "{report:?}");
    let link: String = c
        .query_row("SELECT memory_id FROM brain_legacy_map WHERE legacy_table='insight_derivation_links' AND legacy_id='s:l1'", [], |r| r.get(0))
        .unwrap();
    match store.memory_payload(&link).unwrap().unwrap() {
        crate::memory_metadata::MemoryPayload::Link(l) => {
            assert_eq!(l.relation, "supports");
            assert_eq!(
                l.source,
                EndpointRef::Memory {
                    memory_id: m1.clone(),
                    kind: MemoryKind::Claim
                }
            );
        }
        other => panic!("{other:?}"),
    }
    let unmapped: String = c
        .query_row(
            "SELECT coverage FROM brain_backfill_receipts WHERE unit_id='s:l2'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(unmapped.contains("unmapped relation"));
}
