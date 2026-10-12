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
    fn completed_modes_commit_immutable_exclusive_report_requests_without_body_or_listener() {
        use crate::investigation::insight_persist::{
            desired_generation, report_input, schedule_article_refresh,
        };
        let store = Store::memory().unwrap();
        let mut art = article();
        store.atlas_upsert_article(&art).unwrap();
        let settings = crate::provider::SettingsFile::default();
        assert!(schedule_article_refresh(&store, &art, &settings, true)
            .unwrap()
            .is_none());
        assert_eq!(
            desired_generation(&store.conn, "article", &art.id).unwrap(),
            0
        );
        let mut previous = 0;
        for mode in ReportMode::all() {
            let job = create_report_job(
                &store,
                &art,
                mode,
                &ReportScope {
                    sections: vec!["bluf".into()],
                    ..Default::default()
                },
                true,
            )
            .unwrap();
            let tasks = store.intel_report_tasks(&job.id).unwrap();
            assert!(tasks.iter().all(|t| t.task_type != "refresh_insights"));
            for element in store.intel_elements(&job.investigation_id).unwrap() {
                store
                    .update_element_assessment(
                        &element.id,
                        "unresolved",
                        "unresolved",
                        "fixture",
                        "",
                        "[]",
                    )
                    .unwrap();
            }
            store
                .upsert_report_section_markdown(
                    &job.id,
                    "bluf",
                    &format!(
                        "Complete {} report conclusion with qualified uncertainty.",
                        mode.as_str()
                    ),
                    "complete",
                    "[]",
                    "{}",
                    1,
                    "",
                )
                .unwrap();
            super::super::worker::finalize_job_state(&store, &job.id, &settings, &mut |_| {})
                .unwrap();
            assert_eq!(
                store.intel_report_job(&job.id).unwrap().unwrap().state,
                "completed"
            );
            let generation = desired_generation(&store.conn, "article", &art.id).unwrap();
            assert!(generation > previous);
            previous = generation;
            let input = report_input(&store.conn, &art.id, generation)
                .unwrap()
                .unwrap();
            let payload = serde_json::to_string(&input).unwrap();
            assert_eq!(input.surface, "recon_report");
            assert!(!input.recall_enabled);
            assert!(payload.contains(&format!("Complete {} report", mode.as_str())));
            assert!(!payload.contains(&art.title));
            assert!(!payload.contains(&art.description));
            assert!(!payload.contains(&art.url));
            assert!(input.snapshot.memory_refs.is_empty());
            assert!(input.snapshot.tool_refs.is_empty());
            schedule_article_refresh(&store, &art, &settings, true).unwrap();
            assert_eq!(
                desired_generation(&store.conn, "article", &art.id).unwrap(),
                generation
            );
            // Replayed finalization does not create another sequence/generation.
            super::super::worker::finalize_job_state(&store, &job.id, &settings, &mut |_| {})
                .unwrap();
            assert_eq!(
                desired_generation(&store.conn, "article", &art.id).unwrap(),
                generation
            );
        }
        art.title = "Changed article details cannot supersede completed report".into();
        store.atlas_upsert_article(&art).unwrap();
        schedule_article_refresh(&store, &art, &settings, true).unwrap();
        assert_eq!(
            desired_generation(&store.conn, "article", &art.id).unwrap(),
            previous
        );
        assert_eq!(
            store
                .conn
                .query_row("SELECT COUNT(*) FROM intel_report_results", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            4
        );
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
        assert!(
            insight_persist::recover_refreshes(&db, &auth, &settings, &cancel)
                .await
                .is_err()
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
    async fn scripted_empty_report_is_exclusive_clears_old_brain_and_queues_index_removal() {
        use crate::investigation::insight_persist;
        use crate::provider_attempt::mock;
        use std::sync::{atomic::AtomicBool, Arc};
        let server = mock::serve(vec![
            mock::ok_json(
                r#"{"entities":[],"claims":[],"limit_reached":false,"partial_enumeration":false}"#,
                "stop",
            ),
            mock::ok_json(r#"{"inferences":[],"links":[]}"#, "stop"),
        ])
        .await;
        let secret = mock::secret(&server.base_url);
        let mut auth = crate::secrets::AuthFile::default();
        auth.set_account(secret.clone());
        let mut settings = crate::provider::SettingsFile::default();
        let assignment = crate::provider::ModelAssignment {
            provider: secret.kind.clone(),
            model: secret.model.clone(),
            ..Default::default()
        };
        settings.defaults.claim_assessor = assignment.clone();
        settings.defaults.synthesis = assignment;
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("exclusive.db");
        let store = Store::open(&db).unwrap();
        let art = article();
        store.atlas_upsert_article(&art).unwrap();
        let old = AtlasInsightClaim {
            fingerprint: String::new(),
            entity: "geneva".into(),
            namespace: "place".into(),
            predicate: "hosts".into(),
            object: "talks".into(),
            topic: "geopolitical".into(),
            claim: "Geneva hosts earlier talks".into(),
            classification: "fact".into(),
            confidence: 0.5,
            article_id: art.id.clone(),
            source_url: art.url.clone(),
            published_at: art.published_at.clone(),
            reliability: "B".into(),
            info_credibility: 6,
            admiralty: "B6".into(),
            rsp_status: String::new(),
        };
        store
            .persist_atlas_insights(&art.run_id, &[old], &[], "", "")
            .unwrap();
        let old_memory: String = store
            .conn
            .query_row("SELECT memory_id FROM insight_claims LIMIT 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        store
            .update_memory(
                &old_memory,
                "User edited this article-owned assertion",
                "investigation",
                false,
            )
            .unwrap();
        let manual = store
            .add_memory(
                "Independently remembered Geneva context",
                "general",
                false,
                crate::brain::MemorySource {
                    app: "manual".into(),
                    conversation_id: "manual-note".into(),
                    message_id: None,
                    reference: None,
                },
            )
            .unwrap();
        let job = create_report_job(
            &store,
            &art,
            ReportMode::Verify,
            &ReportScope {
                sections: vec!["bluf".into()],
                ..Default::default()
            },
            true,
        )
        .unwrap();
        for element in store.intel_elements(&job.investigation_id).unwrap() {
            store
                .update_element_assessment(
                    &element.id,
                    "unresolved",
                    "unresolved",
                    "fixture",
                    "",
                    "[]",
                )
                .unwrap();
        }
        store
            .upsert_report_section_markdown(
                &job.id,
                "bluf",
                "Complete report contains no supported conclusions.",
                "complete",
                "[]",
                "{}",
                1,
                "",
            )
            .unwrap();
        super::super::worker::finalize_job_state(&store, &job.id, &settings, &mut |_| {}).unwrap();
        let task: String = store
            .conn
            .query_row(
                "SELECT id FROM argos_tasks WHERE operation='insight_refresh'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let output = insight_persist::run_refresh_slice(
            &db,
            &task,
            &auth,
            &settings,
            &Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(output.claims.is_empty() && output.inferences.is_empty());
        assert!(output.readiness.publication_eligible);
        assert!(store
            .atlas_claims_for_article(&art.run_id, &art.id)
            .unwrap()
            .is_empty());
        assert!(store.get_memory(&old_memory).unwrap().is_none());
        assert!(store.get_memory(&manual.id).unwrap().is_some());
        assert!(store.conn.query_row("SELECT EXISTS(SELECT 1 FROM argos_index_changes WHERE record_id=?1 AND operation='index_remove')",[&old_memory],|r|r.get::<_,bool>(0)).unwrap());
        assert_eq!(server.hits(), 2);
        for request in server.requests.lock().unwrap().iter() {
            assert!(request.contains("Complete report contains no supported conclusions"));
            assert!(!request.contains(&art.title));
            assert!(!request.contains(&art.description));
            assert!(!request.contains("Geneva hosts earlier talks"));
            assert!(!request.contains(&art.url));
        }
        // Report stays completed independently of extraction and job cleanup.
        assert_eq!(
            store.intel_report_job(&job.id).unwrap().unwrap().state,
            "completed"
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
    fn report_attempt_budget_survives_stage_updates_and_reopen() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let store = Store::open(file.path()).unwrap();
        let art = article();
        store.atlas_upsert_article(&art).unwrap();
        let job = create_report_job(
            &store,
            &art,
            ReportMode::Verify,
            &ReportScope::default(),
            false,
        )
        .unwrap();
        let task = store
            .intel_report_tasks(&job.id)
            .unwrap()
            .into_iter()
            .find(|task| task.task_type == "collect")
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE intel_report_jobs SET tool_calls_allowance=1 WHERE id=?1",
                [&job.id],
            )
            .unwrap();
        let attempt = store
            .insert_report_attempt(&job.id, job.generation, &task.id, "firecrawl_search")
            .unwrap();
        store
            .update_report_job(&job.id, "running", "collected", 0, 0, "", "")
            .unwrap();
        store
            .finish_report_attempt(&job.id, &attempt, "success")
            .unwrap();
        store
            .finish_report_attempt(&job.id, &attempt, "failed")
            .unwrap();
        assert!(store
            .insert_report_attempt(&job.id, job.generation, &task.id, "firecrawl_search")
            .is_err());
        assert!(store
            .insert_report_attempt(&job.id, job.generation + 1, &task.id, "firecrawl_search")
            .is_err());
        drop(store);
        let reopened = Store::open(file.path()).unwrap();
        let persisted = reopened.intel_report_job(&job.id).unwrap().unwrap();
        assert_eq!(persisted.tool_calls_done, 1);
        assert!(persisted.current_tool.is_empty());
        let state: String = reopened
            .conn
            .query_row(
                "SELECT state FROM intel_report_attempts WHERE id=?1",
                [&attempt],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "success");
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
            auth: crate::secrets::AuthFile::default(),
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
        assert_eq!(finished.state, "partial");
        let sections = Store::open(&db)
            .unwrap()
            .intel_report_sections(&job.id)
            .unwrap();
        assert!(sections
            .iter()
            .any(|s| s.status == "draft" && !s.markdown.is_empty()));
        assert!(events > 0);
    }
}
