use super::*;
use crate::atlas_metadata::{ArticleMention, ArticleMetadata, MetadataState};
use crate::store::AtlasArticleRow;

pub(crate) fn seed(store: &Store, run: &str, id: &str, title: &str, names: &[&str]) {
    let _ = store.atlas_insert_run(run, "{}", "{}");
    let a = AtlasArticleRow {
        run_id: run.into(),
        id: id.into(),
        title: title.into(),
        url: format!("https://x.test/{id}"),
        source_name: "X".into(),
        source_domain: "x.test".into(),
        published_at: "2026-10-12".into(),
        provider: "fixture".into(),
        category: "unk".into(),
        ..Default::default()
    };
    store.atlas_upsert_article(&a).unwrap();
    let mut m = ArticleMetadata::pending(&a);
    m.entity_state = MetadataState::Complete;
    for n in names {
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
fn phase6_receipts_every_occurrence_including_zero_actor_in_bounded_batches() {
    let store = Store::memory().unwrap();
    // 250 occurrences → three ≤100-occurrence transactions.
    for i in 0..250 {
        if i % 5 == 0 {
            seed(&store, "r", &format!("a{i:03}"), "Weather is mild", &[]);
        } else {
            seed(
                &store,
                "r",
                &format!("a{i:03}"),
                "Jane Doe speaks",
                &["Jane Doe"],
            );
        }
    }
    let report = index_run(&store, "r", &|| false).unwrap();
    assert!(report.complete(), "{report:?}");
    assert_eq!(
        (report.total, report.indexed, report.zero_actor),
        (250, 250, 50)
    );
    assert_eq!(
        count(&store, "SELECT COUNT(*) FROM atlas_article_index"),
        250
    );
    // Receipts also satisfy the historical backfill (no double indexing).
    let cov = store.brain_backfill_coverage().unwrap();
    let atlas = cov.iter().find(|c| c.source == "atlas_articles").unwrap();
    assert_eq!((atlas.done, atlas.total), (250, 250));
    let again = index_run(&store, "r", &|| false).unwrap();
    assert_eq!((again.reused, again.indexed), (250, 0));
}

#[test]
fn failed_index_write_blocks_completion_and_resume_retries_only_missing() {
    let store = Store::memory().unwrap();
    seed(&store, "r", "good", "Jane Doe speaks", &["Jane Doe"]);
    seed(&store, "r", "bad", "John Roe speaks", &["John Roe"]);
    store
        .conn
        .execute_batch(
            "CREATE TRIGGER fail_bad BEFORE INSERT ON brain_actor_mentions WHEN NEW.origin_id='bad'
             BEGIN SELECT RAISE(ABORT,'injected'); END;",
        )
        .unwrap();
    let report = index_run(&store, "r", &|| false).unwrap();
    assert!(!report.complete());
    assert_eq!((report.indexed, report.failed), (1, 1));
    // The failed unit left nothing behind (atomic per occurrence).
    assert_eq!(
        count(
            &store,
            "SELECT COUNT(*) FROM atlas_article_index WHERE article_id='bad'"
        ),
        0
    );
    assert_eq!(
        count(
            &store,
            "SELECT COUNT(*) FROM brain_actors WHERE display_name='John Roe'"
        ),
        0
    );
    store.conn.execute_batch("DROP TRIGGER fail_bad").unwrap();
    let resumed = index_run(&store, "r", &|| false).unwrap();
    assert!(resumed.complete());
    assert_eq!((resumed.reused, resumed.indexed), (1, 1));
}

#[test]
fn stop_before_a_batch_writes_nothing_and_revision_change_reindexes() {
    let store = Store::memory().unwrap();
    seed(&store, "r", "a", "Jane Doe speaks", &["Jane Doe"]);
    let stopped = index_run(&store, "r", &|| true).unwrap();
    assert!(stopped.stopped && !stopped.complete());
    assert_eq!(count(&store, "SELECT COUNT(*) FROM atlas_article_index"), 0);
    assert!(index_run(&store, "r", &|| false).unwrap().complete());
    // Input revision change (re-extracted metadata) invalidates the receipt.
    seed(
        &store,
        "r",
        "a",
        "Jane Doe and Ann Lee speak",
        &["Jane Doe", "Ann Lee"],
    );
    let re = index_run(&store, "r", &|| false).unwrap();
    assert_eq!((re.reused, re.indexed), (0, 1));
    // A tampered checksum is not a valid receipt.
    store
        .conn
        .execute_batch("UPDATE atlas_article_index SET index_checksum='x'")
        .unwrap();
    assert_eq!(index_run(&store, "r", &|| false).unwrap().indexed, 1);
}

#[test]
fn collection_contracts_are_recognised_centrally() {
    use crate::atlas_work::*;
    assert!(is_collection_contract(COLLECTION_CONTRACT_V1));
    assert!(is_collection_contract(COLLECTION_CONTRACT));
    assert_eq!(COLLECTION_CONTRACT, "atlas-collection-v2");
    assert!(!is_collection_contract(""));
    assert!(!is_collection_contract("atlas-memory-v3"));
    assert_eq!(stage_label(STAGE_ACTOR_ARTICLES), "actor_articles");
    assert_eq!(stage_unit(STAGE_ACTOR_ARTICLES), "article");
    assert_eq!(crate::atlas::phase_title(6), "Index actors and articles");
    assert_eq!(crate::atlas::phase_title(5), "Related article leads");
}
