//! Mocked acceptance coverage for Intel Recon enrichment.

#[cfg(test)]
mod acceptance {
    use crate::intel_recon::{
        body_fetch_routes, create_report_job, section_plan, seed_element_ledger,
        validate_article_body, BodyQuality, ReportMode, ReportScope,
    };
    use crate::store::{AtlasArticleClaim, AtlasArticleRow, AtlasInsightClaim, Store};

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
    fn create_report_places_section_placeholders_immediately() {
        let store = Store::memory().unwrap();
        let art = article();
        store.atlas_upsert_article(&art).unwrap();
        let job = create_report_job(
            &store,
            &art,
            ReportMode::FullAssessment,
            &ReportScope::default(),
            false,
        )
        .unwrap();
        let sections = store.intel_report_sections(&job.id).unwrap();
        assert_eq!(sections.len(), 10);
        assert!(sections.iter().all(|s| s.status == "waiting"));
        assert_eq!(job.sections_total, 10);
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
    fn active_job_is_reused_instead_of_duplicated() {
        let store = Store::memory().unwrap();
        let art = article();
        store.atlas_upsert_article(&art).unwrap();
        let first = create_report_job(
            &store,
            &art,
            ReportMode::Verify,
            &ReportScope::default(),
            false,
        )
        .unwrap();
        let second = create_report_job(
            &store,
            &art,
            ReportMode::Verify,
            &ReportScope::default(),
            false,
        )
        .unwrap();
        assert_eq!(first.id, second.id);
        let forced = create_report_job(
            &store,
            &art,
            ReportMode::Verify,
            &ReportScope::default(),
            true,
        )
        .unwrap();
        assert_ne!(first.id, forced.id);
        assert_eq!(forced.revision, 2);
    }

    #[tokio::test]
    async fn verify_job_reaches_terminal_state_with_deterministic_synthesis() {
        use crate::intel_recon::{JobRuntime, ReportMode, ReportScope};
        use crate::osint::ProviderKeys;
        use crate::provider::SettingsFile;
        use std::sync::atomic::AtomicBool;
        use std::sync::Arc;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let db = dir.path().join("argos.db");
        let store = Store::open(&db).unwrap();
        let art = article();
        store.atlas_upsert_article(&art).unwrap();
        // Seed a usable body so acquire_body is a cache hit.
        let body = store
            .ensure_article_body(
                &art.id,
                &art.run_id,
                &art.url,
                &art.source_domain,
                &art.source_name,
            )
            .unwrap();
        store
            .commit_article_body(
                &body.id,
                "# Geneva talks resume\n\nDiplomats from France and Germany met today to discuss the ceasefire proposal after overnight shelling near the border crossing. Officials said progress was limited but talks will continue next week in the same venue.",
                "hash",
                "complete",
                "fixture",
                &art.url,
                "fixture",
                false,
            )
            .unwrap();
        let claim = AtlasInsightClaim {
            fingerprint: r#"["news","diplomats","discuss","ceasefire"]"#.into(),
            entity: "diplomats".into(),
            namespace: "news".into(),
            predicate: "discuss".into(),
            object: "ceasefire".into(),
            topic: "geopolitical".into(),
            claim: "Diplomats discuss ceasefire".into(),
            classification: "fact".into(),
            confidence: 0.7,
            article_id: art.id.clone(),
            source_url: art.url.clone(),
            published_at: art.published_at.clone(),
            reliability: "B".into(),
            info_credibility: 2,
            admiralty: "B2".into(),
            rsp_status: "gr".into(),
        };
        store
            .persist_atlas_insights(&art.run_id, &[claim], &[], "", "")
            .unwrap();

        let job = create_report_job(
            &store,
            &art,
            ReportMode::Verify,
            &ReportScope::default(),
            false,
        )
        .unwrap();
        let runtime = JobRuntime {
            db_path: db.clone(),
            job_id: job.id.clone(),
            article_title: art.title.clone(),
            article_url: art.url.clone(),
            article_preview: art.description.clone(),
            article_published: art.published_at.clone(),
            article_domain: art.source_domain.clone(),
            run_id: art.run_id.clone(),
            keys: ProviderKeys::default(),
            synthesis_secret: None,
            classifier_secret: None,
            settings: SettingsFile::default(),
            cancel: Arc::new(AtomicBool::new(false)),
        };
        let mut events = 0u32;
        crate::intel_recon::run_job_to_completion(runtime, |_| {
            events += 1;
        })
        .await
        .unwrap();
        let finished = Store::open(&db)
            .unwrap()
            .intel_report_job(&job.id)
            .unwrap()
            .unwrap();
        assert!(
            matches!(finished.state.as_str(), "completed" | "partial"),
            "state={}",
            finished.state
        );
        assert!(finished.sections_done > 0);
        let sections = Store::open(&db)
            .unwrap()
            .intel_report_sections(&job.id)
            .unwrap();
        assert!(sections
            .iter()
            .any(|s| s.status == "complete" && !s.markdown.is_empty()));
        assert!(events > 0);
    }
}
