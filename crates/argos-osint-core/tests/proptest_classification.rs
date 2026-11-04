//! Offline batch classification contracts over varied category assignments.
use argos_osint_core::atlas::{rank_question_id, CATEGORY_IDS, RANKED_PICKS};
use argos_osint_core::atlas_classify::{batches, BatchAdapter, BATCH_SIZE};
use argos_osint_core::recon::model_exec::DecisionsAdapter;
use argos_osint_core::store::AtlasArticleRow;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]
    #[test]
    fn classification_order_does_not_change_receipts(categories in prop::collection::vec(0usize..7, 1..=BATCH_SIZE)) {
        // One documented batch: `parse_chat` consumes a single response whose
        // ranked answers (BATCH_SIZE * RANKED_PICKS blocks) must stay under
        // the parser's documented block bound; `batches` (asserted below)
        // cuts larger inputs at BATCH_SIZE in production.
        let articles: Vec<_> = categories.iter().enumerate().map(|(i, _)| AtlasArticleRow {
            id: format!("fixture-{i}"), title: "Offline classification fixture".into(), ..Default::default()
        }).collect();
        // The first two picks are distinct fitting categories; later ranks stop.
        let picks = |category: &usize| {
            let first = CATEGORY_IDS[*category];
            let second = CATEGORY_IDS[(*category + 1) % (CATEGORY_IDS.len() - 1)];
            vec![first, second, "done"]
        };
        let blocks: Vec<String> = categories.iter().enumerate().map(|(i, category)| {
            let mut text = String::new();
            let picked = picks(category);
            for rank in 1..=RANKED_PICKS {
                let label = picked.get(rank - 1).copied().unwrap_or("done");
                text.push_str(&format!(
                    "## Decision\nQuestion: {}\nChoice: {label}\nEnd\n",
                    rank_question_id(&format!("fixture-{i}"), rank)
                ));
            }
            text
        }).collect();
        let raw = |reversed: bool| {
            let mut text = String::from("argos-text-v1\n");
            let ordered: Vec<&String> = if reversed {
                blocks.iter().rev().collect()
            } else {
                blocks.iter().collect()
            };
            for block in ordered {
                text.push_str(block);
            }
            text
        };
        let expected: std::collections::BTreeMap<_, _> = categories.iter().enumerate()
            .map(|(i, category)| {
                let picked = picks(category);
                (format!("fixture-{i}"), format!("{},{}", picked[0], picked[1]))
            })
            .collect();
        let first = BatchAdapter::new(articles.clone()).parse_chat(raw(false).as_str());
        let second = BatchAdapter::new(articles.clone()).parse_chat(raw(true).as_str());
        prop_assert_eq!(first.expect("valid classifications"), expected.clone());
        prop_assert_eq!(second.expect("reordered classifications"), expected);
        let batches = batches(&articles);
        prop_assert!(batches.iter().all(|batch| !batch.is_empty() && batch.len() <= BATCH_SIZE));
        let ids: Vec<_> = batches.iter().flatten().map(|article| &article.id).collect();
        prop_assert_eq!(ids, articles.iter().map(|article| &article.id).collect::<Vec<_>>());
    }
}
