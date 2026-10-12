//! Classification-only cycle regressions. Historical Brain maintenance is
//! independently covered by atlas_memory's direct-helper tests.
#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use crate::atlas::{run_atlas, AtlasEvent, Cursor, RunInput, RunStats, Stop};
    use crate::store::{AtlasArticleRow, Store};

    const RUN: &str = "classification-contract-fixture";

    fn seed(path: &std::path::Path, phase: u8, category: Option<&str>, legacy: bool) {
        let store = Store::open(path).unwrap();
        let stats = RunStats {
            contract_version: if legacy {
                String::new()
            } else {
                crate::atlas_work::COLLECTION_CONTRACT.into()
            },
            ..RunStats::default()
        };
        let cursor = Cursor {
            phase,
            leg: "legacy_insights".into(),
            ..Cursor::default()
        };
        store
            .atlas_insert_run(
                RUN,
                &serde_json::to_string(&cursor).unwrap(),
                &serde_json::to_string(&stats).unwrap(),
            )
            .unwrap();
        store.atlas_set_state(RUN, "paused", "", false).unwrap();
        if let Some(category) = category {
            store
                .atlas_upsert_article(&AtlasArticleRow {
                    run_id: RUN.into(),
                    id: "article-1".into(),
                    title: "Acme opens Lisbon office".into(),
                    description: "Jane Doe announces the opening".into(),
                    url: "https://news.example/a".into(),
                    category: category.into(),
                    country: "pt".into(),
                    source_name: "Desk".into(),
                    source_domain: "news.example".into(),
                    published_at: String::new(),
                    provider: "newsapi".into(),
                    temperature: 1.0,
                    seen_at: String::new(),
                    author: String::new(),
                    image_url: String::new(),
                })
                .unwrap();
        }
    }

    async fn resume(path: &std::path::Path) -> (Stop, Vec<AtlasEvent>) {
        let mut events = Vec::new();
        let pause = AtomicBool::new(false);
        let keys = crate::osint::ProviderKeys::default();
        let stop = run_atlas(
            RunInput {
                db_path: path,
                pause: &pause,
                keys: &keys,
                user_agent: "Argos offline fixture",
                pace: false,
                resume: true,
                feed: &[],
                classifier: None,
                synthesizer: None,
                synthesizer_fallbacks: &[],
                run_id: Some(RUN),
            },
            |event| events.push(event),
            |_call| {
                Box::pin(async {
                    panic!("retained classification/native resume must not fetch provider data")
                })
            },
        )
        .await
        .unwrap();
        (stop, events)
    }

    fn assert_no_cycle_reasoning(store: &Store, events: &[AtlasEvent]) {
        assert!(store.list_memories().unwrap().is_empty());
        for table in [
            "argos_memory_index_state",
            "argos_index_changes",
            "insight_snapshots",
            "insight_claims",
        ] {
            let count: i64 = store
                .conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0, "cycle created {table} obligations");
        }
        let model_calls: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM telemetry_events WHERE event_type IN ('model_operation','model_attempt')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(model_calls, 0, "native resume called a model role");
        assert!(!events.iter().any(|e| matches!(
            e,
            AtlasEvent::MemoryProgress { .. } | AtlasEvent::MemoriesChanged { .. }
        )));
    }

    #[tokio::test]
    async fn legacy_phase4_and_phase5_resume_without_synthesis_or_brain_publication() {
        for phase in [4, 5] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("argos.db");
            seed(&path, phase, Some("economic"), true);
            let (stop, events) = resume(&path).await;
            assert_eq!(stop, Stop::Finished);
            let store = Store::open(&path).unwrap();
            let run = store.atlas_latest_run().unwrap().unwrap();
            assert_eq!(run.state, "completed");
            let stats: RunStats = serde_json::from_str(&run.stats_json).unwrap();
            assert_eq!(
                stats.contract_version,
                crate::atlas_work::COLLECTION_CONTRACT
            );
            assert_eq!(stats.coverage.classified, 1);
            assert_eq!(
                store.atlas_list_articles(RUN).unwrap()[0].category,
                "economic"
            );
            assert_no_cycle_reasoning(&store, &events);
        }
    }

    #[tokio::test]
    async fn empty_cycle_finishes_without_any_model_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("argos.db");
        seed(&path, 3, None, false);
        let (stop, events) = resume(&path).await;
        assert_eq!(stop, Stop::Finished);
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store.atlas_latest_run().unwrap().unwrap().state,
            "completed"
        );
        assert_no_cycle_reasoning(&store, &events);
    }

    #[tokio::test]
    async fn missing_classifier_blocks_required_rows_without_faking_unk_success() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("argos.db");
        seed(&path, 3, Some("unk"), false);
        let (_, events) = resume(&path).await;
        let store = Store::open(&path).unwrap();
        assert_eq!(store.atlas_latest_run().unwrap().unwrap().state, "blocked");
        assert_eq!(
            store
                .atlas_metadata(RUN, "article-1")
                .unwrap()
                .unwrap()
                .classification_status,
            "blocked"
        );
        assert_no_cycle_reasoning(&store, &events);
    }

    #[tokio::test]
    async fn ambiguous_legacy_unk_is_preserved_and_flagged_without_reclassification() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("argos.db");
        seed(&path, 4, Some("unk"), true);
        let (_, events) = resume(&path).await;
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store
                .atlas_metadata(RUN, "article-1")
                .unwrap()
                .unwrap()
                .classification_status,
            "legacy_unknown"
        );
        assert_no_cycle_reasoning(&store, &events);
    }

    #[test]
    fn historical_maintenance_rejects_new_contract_and_startup_does_not_repair() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("argos.db");
        seed(&path, 5, None, false);
        let store = Store::open(&path).unwrap();
        assert!(crate::atlas_memory::repair_run(&store, RUN, None).is_err());
        assert!(crate::atlas_memory::ensure_historical_contract(&store, RUN).is_err());
        assert!(!crate::atlas_memory::spawn_startup_reconciliation(path));
        let repairs: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM argos_jobs WHERE kind='atlas_memory_repair'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(repairs, 0);
    }
}
