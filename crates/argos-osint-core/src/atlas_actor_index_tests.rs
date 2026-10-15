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

#[test]
fn direct_actor_sources_count_without_related_pairs_and_follow_deletion_and_reingest() {
    let store = Store::memory().unwrap();
    seed(&store, "r", "a", "Trump's office responds", &["Trump's"]);
    index_run(&store, "r", &|| false).unwrap();
    let group = store.actor_group("trump").unwrap().unwrap();
    let actor = &group.actor_ids[0];
    assert_eq!(
        store
            .actor_group_associations(&group)
            .unwrap()
            .articles
            .len(),
        1
    );
    assert_eq!(store.actor_articles(actor, None, 20).unwrap().total, 1);
    let row = store
        .brain_browse_memories(None, "")
        .unwrap()
        .into_iter()
        .find(|m| m.id == group.key())
        .unwrap();
    assert_eq!(row.category, "0 claims · 0 threads · 1 articles");
    seed(&store, "s", "b", "Trump speaks", &["Trump"]);
    index_run(&store, "s", &|| false).unwrap();
    assert_eq!(
        store
            .actor_group_associations(&store.actor_group("trump").unwrap().unwrap())
            .unwrap()
            .articles
            .len(),
        2
    );
    store.atlas_delete_article("r", "a").unwrap();
    assert_eq!(
        count(
            &store,
            "SELECT COUNT(*) FROM brain_actor_mentions WHERE origin_scope='r'"
        ),
        0
    );
    assert_eq!(
        count(
            &store,
            "SELECT COUNT(*) FROM atlas_article_index WHERE run_id='r'"
        ),
        0
    );
    assert_eq!(
        store
            .actor_group_associations(&store.actor_group("trump").unwrap().unwrap())
            .unwrap()
            .articles
            .len(),
        1
    );
    store.atlas_delete_run("s").unwrap();
    assert!(store.actor_group("trump").unwrap().is_none());
    seed(&store, "r", "a", "Trump's office responds", &["Trump's"]);
    index_run(&store, "r", &|| false).unwrap();
    assert_eq!(
        store
            .actor_group_associations(&store.actor_group("trump").unwrap().unwrap())
            .unwrap()
            .articles
            .len(),
        1
    );
}

#[test]
fn same_input_metadata_correction_replaces_mentions_and_retires_removed_actor() {
    let store = Store::memory().unwrap();
    seed(
        &store,
        "r",
        "a",
        "Jane Doe and Ann Lee meet",
        &["Jane Doe", "Ann Lee"],
    );
    index_run(&store, "r", &|| false).unwrap();
    // Same source hash, corrected NLP inventory.
    seed(&store, "r", "a", "Jane Doe and Ann Lee meet", &["Jane Doe"]);
    index_run(&store, "r", &|| false).unwrap();
    assert!(store.actor_group("ann lee").unwrap().is_none());
    assert_eq!(
        store
            .actor_group_associations(&store.actor_group("jane doe").unwrap().unwrap())
            .unwrap()
            .articles
            .len(),
        1
    );
    seed(&store, "r", "a", "Jane Doe and Ann Lee meet", &[]);
    index_run(&store, "r", &|| false).unwrap();
    assert!(store.actor_groups().unwrap().is_empty());
}

#[test]
fn related_matching_counts_possessive_aliases_once_but_keeps_direct_articles() {
    let store = Store::memory().unwrap();
    for id in ["a", "b"] {
        seed(
            &store,
            "r",
            id,
            "Trump and Trump's office responds",
            &["Trump", "Trump's"],
        );
        let mut a = store.atlas_article("r", id).unwrap().unwrap();
        a.category = "economic".into();
        store.atlas_upsert_article(&a).unwrap();
    }
    assert_eq!(
        crate::related_articles::build(&store, "r").unwrap(),
        0,
        "two spellings are one actor"
    );
    let a = store.atlas_article("r", "a").unwrap().unwrap();
    let b = store.atlas_article("r", "b").unwrap().unwrap();
    assert_eq!(
        crate::related_articles::shared_entity_mentions(&store, &a, &b)
            .unwrap()
            .len(),
        1
    );
    index_run(&store, "r", &|| false).unwrap();
    assert_eq!(
        store
            .actor_group_associations(&store.actor_group("trump").unwrap().unwrap())
            .unwrap()
            .articles
            .len(),
        2
    );
    seed(
        &store,
        "r",
        "a",
        "Trump's office joins Acme",
        &["Trump's", "Acme"],
    );
    seed(&store, "r", "b", "Trump visits Acme", &["Trump", "Acme"]);
    for id in ["a", "b"] {
        let mut a = store.atlas_article("r", id).unwrap().unwrap();
        a.category = "economic".into();
        store.atlas_upsert_article(&a).unwrap();
    }
    assert_eq!(
        crate::related_articles::build(&store, "r").unwrap(),
        1,
        "possessives match but require another distinct entity"
    );
}

#[test]
fn duplicate_url_actor_sources_preserve_exact_cycle_scope() {
    let store = Store::memory().unwrap();
    for run in ["r", "s"] {
        seed(&store, run, "same", "Ann speaks", &["Ann"]);
        index_run(&store, run, &|| false).unwrap();
    }
    let group = store.actor_group("ann").unwrap().unwrap();
    assert_eq!(
        store
            .actor_group_associations(&group)
            .unwrap()
            .articles
            .len(),
        1
    );
    for run in ["r", "s"] {
        let a = store
            .actor_group_associations_scoped(&group, Some(run))
            .unwrap();
        assert_eq!(a.articles.len(), 1);
        assert_eq!(a.articles[0].article.run_id, run);
        assert_eq!(a.articles[0].occurrences.len(), 1);
        assert_eq!(store.actor_groups_scoped(Some(run)).unwrap().len(), 1);
    }
}

#[test]
fn pruning_and_merged_survivors_keep_only_retained_actor_contributions() {
    let store = Store::memory().unwrap();
    for run in ["old", "keep"] {
        seed(&store, run, run, "Jane Doe speaks", &["Jane Doe"]);
        index_run(&store, run, &|| false).unwrap();
    }
    let group = store.actor_group("jane doe").unwrap().unwrap();
    let a = &group.actor_ids[0];
    let b = &group.actor_ids[1];
    store
        .merge_actors(b, a, &serde_json::json!({"assessed":true}))
        .unwrap();
    store
        .conn
        .execute_batch(
            "UPDATE atlas_runs SET started_at='2000-01-01',state='completed' WHERE id='old'",
        )
        .unwrap();
    store.atlas_prune_expired().unwrap();
    let group = store.actor_group("jane doe").unwrap().unwrap();
    assert_eq!(
        store
            .actor_group_associations(&group)
            .unwrap()
            .articles
            .len(),
        1
    );
    store.atlas_delete_run("keep").unwrap();
    assert!(store.actor_group("jane doe").unwrap().is_none());
    assert!(
        store.get_memory(a).unwrap().is_some(),
        "old IDs survive retirement"
    );
}
