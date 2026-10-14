//! Mocked acceptance coverage for Intel Recon enrichment.

#[cfg(test)]
mod acceptance {
    use crate::intel_recon::{
        body_fetch_routes, create_report_job, section_plan, seed_element_ledger,
        validate_article_body, BodyQuality, ReportMode, ReportScope,
    };
    use crate::store::{AtlasArticleClaim, AtlasArticleRow, Store};

    fn article() -> AtlasArticleRow {
        AtlasArticleRow {
            run_id: "run-acc".into(),
            id: "art-acc".into(),
            title: "Geneva talks resume".into(),
            description: "Diplomats met to discuss a ceasefire.".into(),
            url: "https://example.com/geneva".into(),
            country: "ch".into(),
            source_name: "Example".into(),
            source_domain: "example.com".into(),
            published_at: "2026-10-01T12:00:00Z".into(),
            provider: "newsapi".into(),
            temperature: 0.4,
            category: "geopolitical".into(),
            seen_at: "2026-10-01T12:00:00Z".into(),
            author: String::new(),
            image_url: String::new(),
        }
    }

    #[test]
    fn all_historical_modes_reject_new_article_report_admission() {
        let store = Store::memory().unwrap();
        let art = article();
        store.atlas_upsert_article(&art).unwrap();
        for mode in ReportMode::all() {
            for force in [false, true] {
                assert!(
                    create_report_job(&store, &art, mode, &ReportScope::default(), force)
                        .unwrap_err()
                        .to_string()
                        .contains("retired")
                );
            }
        }
        assert!(store.intel_jobs_for_article(&art.id).unwrap().is_empty());
    }

    #[tokio::test]
    async fn missing_article_details_make_zero_model_operations_in_core_and_recovery() {
        use crate::investigation::{insight_persist, insights};
        use std::sync::{atomic::AtomicBool, Arc};
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("gated.db");
        let store = Store::open(&db).unwrap();
        let art = article();
        store.atlas_upsert_article(&art).unwrap();
        let settings = crate::provider::SettingsFile::default();
        let auth = crate::secrets::AuthFile::default();
        let cancel = Arc::new(AtomicBool::new(false));
        assert!(
            insight_persist::schedule_article_refresh(&store, &art, &settings, true)
                .unwrap()
                .is_none()
        );
        let generation = insight_persist::advance_generation(
            &store.conn,
            "article",
            &art.id,
            &chrono::Utc::now().to_rfc3339(),
        )
        .unwrap();
        let mut input =
            insight_persist::article_input(&store, &art, generation, "fixture", false).unwrap();
        assert!(
            insights::run_pipeline(&mut input, &auth, &settings, &cancel, &db)
                .await
                .is_err()
        );
        insight_persist::enqueue_refresh(
            &store.conn,
            &mut input,
            &settings,
            &chrono::Utc::now().to_rfc3339(),
        )
        .unwrap();
        assert_eq!(
            insight_persist::recover_refreshes(&db, &auth, &settings, &cancel)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            store
                .conn
                .query_row("SELECT COUNT(*) FROM recon_model_operations", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            store
                .conn
                .query_row("SELECT COUNT(*) FROM memories", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn historical_report_artifact_does_not_admit_extraction_on_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("argos.db");
        let store = Store::open(&db).unwrap();
        let art = article();
        store.atlas_upsert_article(&art).unwrap();
        store.connection().execute("INSERT INTO intel_report_results(job_id,generation,article_id,completion_sequence,source_hash,details_json,completed_at) VALUES('old',1,?1,1,'hash','{}',?2)",rusqlite::params![art.id,chrono::Utc::now().to_rfc3339()]).unwrap();
        let count = crate::investigation::insight_persist::recover_refreshes(
            &db,
            &crate::secrets::AuthFile::default(),
            &crate::provider::SettingsFile::default(),
            &std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .await
        .unwrap();
        assert_eq!(count, 0);
        assert_eq!(
            store
                .connection()
                .query_row(
                    "SELECT COUNT(*) FROM argos_tasks WHERE operation='insight_refresh'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        assert_eq!(
            store
                .connection()
                .query_row("SELECT COUNT(*) FROM intel_report_results", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn body_routes_never_include_sociavault() {
        assert!(body_fetch_routes()
            .iter()
            .all(|r| !r.contains("sociavault") && *r != "sociavault_google_search"));
    }

    #[test]
    fn all_four_modes_create_distinct_section_plans() {
        assert_eq!(section_plan(ReportMode::Verify).len(), 6);
        assert_eq!(section_plan(ReportMode::Explain).len(), 6);
        assert_eq!(section_plan(ReportMode::AssessOutlook).len(), 6);
        assert_eq!(section_plan(ReportMode::FullAssessment).len(), 10);
        for mode in ReportMode::all() {
            let job_plan = section_plan(mode);
            assert_eq!(job_plan[0].key, "bluf");
        }
    }

    #[test]
    fn retired_report_admission_creates_no_section_placeholders() {
        let store = Store::memory().unwrap();
        let art = article();
        assert!(create_report_job(
            &store,
            &art,
            ReportMode::FullAssessment,
            &ReportScope::default(),
            false
        )
        .is_err());
        assert_eq!(
            store
                .connection()
                .query_row("SELECT COUNT(*) FROM intel_report_sections", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn historical_report_attempt_counts_survive_reopen() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let store = Store::open(file.path()).unwrap();
        let art = article();
        store.atlas_upsert_article(&art).unwrap();
        let inv = store
            .ensure_intel_investigation(&art.id, &art.run_id, &art.url, "{}")
            .unwrap();
        let job = store
            .insert_report_job(&inv.id, &art.id, "verify", 1, "{}", "{}", 0, 0, 24, None)
            .unwrap();
        store
            .connection()
            .execute(
                "UPDATE intel_report_jobs SET state='completed',tool_calls_done=7 WHERE id=?1",
                [&job.id],
            )
            .unwrap();
        drop(store);
        let reopened = Store::open(file.path()).unwrap();
        let retained = reopened.intel_report_job(&job.id).unwrap().unwrap();
        assert_eq!(retained.tool_calls_done, 7);
        assert_eq!(retained.state, "completed");
    }

    #[test]
    fn coverage_ledger_dispositions_every_seeded_element() {
        let store = Store::memory().unwrap();
        let inv = store
            .ensure_intel_investigation("art-acc", "run-acc", "https://example.com", "{}")
            .unwrap();
        let claim = AtlasArticleClaim {
            fingerprint: r#"["news","nato","announces","aid"]"#.into(),
            entity: "nato".into(),
            predicate: "announces".into(),
            object: "aid".into(),
            topic: "military".into(),
            classification: "fact".into(),
            confidence: 0.8,
            claim: "NATO announces aid".into(),
            source_url: "https://example.com".into(),
            published_at: "2026-10-01".into(),
            article_id: "art-acc".into(),
            reliability: "B".into(),
            info_credibility: 2,
            admiralty: "B2".into(),
            rsp_status: "gr".into(),
        };
        let rows = seed_element_ledger(&store, &inv.id, &[claim], &[]).unwrap();
        assert_eq!(rows.len(), 1);
        store
            .update_element_assessment(&rows[0].id, "assessed", "supported", "ok", "", "[]")
            .unwrap();
        let updated = store.intel_elements(&inv.id).unwrap();
        assert!(crate::intel_recon::coverage_complete(&updated));
    }

    #[test]
    fn paywall_and_snippet_fail_validation() {
        let paywall = validate_article_body(
            "Subscribe to continue reading this article.",
            "Secret",
            "https://example.com",
        );
        assert_eq!(paywall.quality, BodyQuality::Unavailable);
        let snippet = validate_article_body(
            "Result one title\nshort blurb here\nResult two title\nanother short blurb\nResult three\nmore short text\nResult four\nfinal short line",
            "Anything",
            "https://example.com",
        );
        assert_ne!(snippet.quality, BodyQuality::Complete);
    }

    #[test]
    fn atlas_prune_keeps_articles_with_intel_investigations() {
        let store = Store::memory().unwrap();
        let mut art = article();
        art.seen_at = "2000-01-01T00:00:00Z".into();
        store
            .conn
            .execute(
                "INSERT INTO atlas_runs(id,state,phase,cursor_json,stats_json,note,started_at,finished_at)
                 VALUES ('run-acc','completed',3,'','','','2000-01-01T00:00:00Z','2000-01-01T01:00:00Z')",
                [],
            )
            .unwrap();
        store.atlas_upsert_article(&art).unwrap();
        store
            .ensure_intel_investigation(&art.id, &art.run_id, &art.url, "{}")
            .unwrap();
        let removed = store.atlas_prune_expired().unwrap();
        // Run may be listed for prune attempt, but the protected article must remain.
        let kept = store.atlas_article("run-acc", "art-acc").unwrap();
        assert!(
            kept.is_some(),
            "protected article survived prune; removed={removed:?}"
        );
        let _ = removed;
    }

    #[test]
    fn report_resume_and_retry_cannot_reactivate_historical_work() {
        let store = Store::memory().unwrap();
        let art = article();
        let inv = store
            .ensure_intel_investigation(&art.id, &art.run_id, &art.url, "{}")
            .unwrap();
        let job = store
            .insert_report_job(&inv.id, &art.id, "verify", 1, "{}", "{}", 0, 0, 24, None)
            .unwrap();
        assert!(crate::intel_recon::resume_job(&store, &job.id)
            .unwrap_err()
            .to_string()
            .contains("retired"));
        assert!(crate::intel_recon::retry_failed_tasks(&store, &job.id)
            .unwrap_err()
            .to_string()
            .contains("retired"));
    }

    #[tokio::test]
    async fn report_worker_entry_rejects_execution_without_events() {
        use crate::intel_recon::JobRuntime;
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("argos.db");
        drop(Store::open(&db).unwrap());
        let art = article();
        let runtime = JobRuntime {
            stage: None,
            db_path: db,
            job_id: "old".into(),
            article_title: art.title,
            article_url: art.url,
            article_preview: art.description,
            article_published: art.published_at,
            article_domain: art.source_domain,
            run_id: art.run_id,
            keys: Default::default(),
            auth: Default::default(),
            synthesis_secret: None,
            classifier_secret: None,
            settings: Default::default(),
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        let mut events = 0;
        let result = crate::intel_recon::run_job_to_completion(runtime, |_| events += 1).await;
        assert!(result.unwrap_err().to_string().contains("retired"));
        assert_eq!(events, 0);
    }
}
