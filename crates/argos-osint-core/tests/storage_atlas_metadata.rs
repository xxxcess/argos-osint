//! Atlas article metadata tests (migration phase 3, step 4) against a real
//! SurrealDB 3.3.4 disk server or the embedded RocksDB engine (see
//! `support`). Run through `python3 scripts/test_surreal.py --mode both`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::atlas_metadata::{input_hash, ArticleMention, ArticleMetadata};
use argos_osint_core::storage::{atlas, atlas_metadata};
use argos_osint_core::store::AtlasArticleRow;
use support::*;
use surrealdb::engine::any::Any;
use surrealdb::Surreal;

async fn fresh(server: &Server, name: &str) -> (Surreal<Any>, AtlasArticleRow) {
    let db = server.root("argos", name).await;
    atlas::apply_schema(&db).await.unwrap();
    atlas_metadata::apply_schema(&db).await.unwrap();
    let row = AtlasArticleRow {
        run_id: "r".into(),
        id: "a".into(),
        title: "Jane Doe meets Acme".into(),
        description: "Acme hires Jane Doe.".into(),
        category: "unk".into(),
        seen_at: "1".into(),
        ..Default::default()
    };
    atlas::upsert_article(&db, &row).await.unwrap();
    (db, row)
}

fn mention(field: &str, start: usize, end: usize, text: &str) -> ArticleMention {
    ArticleMention {
        type_hint: "person".into(),
        comparison_key: text.to_lowercase(),
        field: field.into(),
        start_byte: start,
        end_byte: end,
        original_text: text.into(),
    }
}

async fn mentions(db: &Surreal<Any>) -> Vec<String> {
    let mut r = db
        .query("SELECT VALUE original_text FROM atlas_article_mention ORDER BY field, start_byte")
        .await
        .unwrap();
    r.take(0).unwrap()
}

#[tokio::test]
async fn metadata_saves_valid_mentions_and_keeps_classification() {
    let Some(server) = Server::start() else {
        return;
    };
    let (db, row) = fresh(&server, "meta_save").await;
    let mut m = ArticleMetadata::pending(&row);
    m.mentions = vec![
        mention("title", 0, 8, "Jane Doe"),
        mention("description", 11, 19, "Jane Doe"),
        mention("title", 0, 8, "Jane Doe"),
        mention("title", 1, 3, "bad"),
        mention("url", 0, 1, "x"),
    ];
    assert!(atlas_metadata::save_metadata(&db, &m).await.unwrap());
    assert_eq!(mentions(&db).await, vec!["Jane Doe", "Jane Doe"]);
    let hash = input_hash(&row);
    assert!(atlas_metadata::set_classification(
        &db,
        "r",
        "a",
        &hash,
        "classified",
        r#"{"ok":true}"#,
        Some("conflict")
    )
    .await
    .unwrap());
    // A same-input save keeps the classification it did not capture.
    m.mentions.truncate(1);
    assert!(atlas_metadata::save_metadata(&db, &m).await.unwrap());
    let stored = atlas_metadata::metadata(&db, "r", "a")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.classification_status, "classified");
    assert_eq!(
        stored.classification_receipt,
        Some(serde_json::json!({"ok": true}))
    );
    assert_eq!(mentions(&db).await, vec!["Jane Doe"]);
    let art = atlas::article(&db, "r", "a").await.unwrap().unwrap();
    assert_eq!(art.category, "conflict");
}

#[tokio::test]
async fn stale_or_missing_articles_are_rejected() {
    let Some(server) = Server::start() else {
        return;
    };
    let (db, row) = fresh(&server, "meta_stale").await;
    let mut m = ArticleMetadata::pending(&row);
    m.input_hash = "old".into();
    assert!(!atlas_metadata::save_metadata(&db, &m).await.unwrap());
    assert!(
        !atlas_metadata::set_classification(&db, "r", "a", "old", "x", "", None)
            .await
            .unwrap()
    );
    assert!(
        !atlas_metadata::set_classification(&db, "r", "nope", "old", "x", "", None)
            .await
            .unwrap()
    );
    assert!(atlas_metadata::metadata(&db, "r", "a")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn deleting_an_article_cascades_to_its_metadata() {
    let Some(server) = Server::start() else {
        return;
    };
    let (db, row) = fresh(&server, "meta_cascade").await;
    let mut m = ArticleMetadata::pending(&row);
    m.mentions = vec![mention("title", 0, 8, "Jane Doe")];
    assert!(atlas_metadata::save_metadata(&db, &m).await.unwrap());
    db.query(
        "CREATE atlas_related_article SET run_id = 'r', left_id = 'a', right_id = 'b', left_revision = '',
           right_revision = '', shared_json = '[]', match_version = '1', named_count = 1, shared_count = 1, rarity = 0.5;
         CREATE atlas_related_article SET run_id = 'r', left_id = '0', right_id = 'a', left_revision = '',
           right_revision = '', shared_json = '[]', match_version = '1', named_count = 1, shared_count = 1, rarity = 0.5;
         DELETE type::record('atlas_article', $key);",
    )
    .bind(("key", argos_osint_core::storage::codec::encode(&["r", "a"])))
    .await
    .unwrap()
    .check()
    .unwrap();
    let mut r = db
        .query("RETURN count(SELECT id FROM atlas_article_metadata) + count(SELECT id FROM atlas_article_mention) + count(SELECT id FROM atlas_related_article)")
        .await
        .unwrap();
    assert_eq!(r.take::<Option<i64>>(0).unwrap(), Some(0));
}

#[tokio::test]
async fn article_mentions_use_their_index() {
    let Some(server) = Server::start() else {
        return;
    };
    let (db, _) = fresh(&server, "meta_plan").await;
    let mut r = db
        .query(format!(
            "{} EXPLAIN FULL",
            atlas_metadata::ARTICLE_MENTIONS_SURQL
        ))
        .bind(("run", "r".to_string()))
        .bind(("article", "a".to_string()))
        .await
        .unwrap()
        .check()
        .unwrap();
    let p = format!("{:?}", r.take::<surrealdb::types::Value>(0).unwrap());
    assert!(p.contains("atlas_mentions_article"), "{p}");
    assert!(!p.contains("Iterate Table"), "{p}");
}
