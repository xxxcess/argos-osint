//! Offline batch classification contracts over varied category assignments.
use argos_osint_core::atlas::CATEGORY_IDS;
use argos_osint_core::atlas_classify::{batches, BatchAdapter, BATCH_SIZE};
use argos_osint_core::recon::model_exec::DecisionsAdapter;
use argos_osint_core::store::AtlasArticleRow;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]
    #[test]
    fn classification_order_does_not_change_receipts(categories in prop::collection::vec(0usize..7, 1..25)) {
        let articles: Vec<_> = categories.iter().enumerate().map(|(i, _)| AtlasArticleRow {
            id: format!("fixture-{i}"), title: "Offline classification fixture".into(), ..Default::default()
        }).collect();
        let rows: Vec<_> = categories.iter().enumerate().map(|(i, category)| serde_json::json!({
            "article_id": format!("fixture-{i}"), "category": CATEGORY_IDS[*category]
        })).collect();
        let first = BatchAdapter::new(articles.clone()).parse_chat(&serde_json::to_string(&rows).expect("rows"));
        let reversed: Vec<_> = rows.into_iter().rev().collect();
        let second = BatchAdapter::new(articles.clone()).parse_chat(&serde_json::to_string(&reversed).expect("reversed rows"));
        let expected: std::collections::BTreeMap<_, _> = categories.iter().enumerate()
            .map(|(i, category)| (format!("fixture-{i}"), CATEGORY_IDS[*category].to_owned()))
            .collect();
        prop_assert_eq!(first.expect("valid classifications"), expected.clone());
        prop_assert_eq!(second.expect("reordered classifications"), expected);
        let batches = batches(&articles);
        prop_assert!(batches.iter().all(|batch| !batch.is_empty() && batch.len() <= BATCH_SIZE));
        let ids: Vec<_> = batches.iter().flatten().map(|article| &article.id).collect();
        prop_assert_eq!(ids, articles.iter().map(|article| &article.id).collect::<Vec<_>>());
    }
}
