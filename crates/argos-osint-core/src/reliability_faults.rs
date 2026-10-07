//! Spec §19 fault-injection / reliability matrix (offline).
//!
//! Covers mock providers, clocks, retries, cache invalidation, lease fencing,
//! concurrent insight replace serialization, stream interruption partial
//! preserve, and shared rate-limit cooldown — without network access.

#[cfg(test)]
mod tests {
    use crate::provider_request::execute_with_retries;
    use crate::recon::clocks::ClockSet;
    use crate::summarization::{
        cache_get, cache_put, deterministic_follow_up, SummarizationMode, SummaryRequest,
        SummarySource,
    };
    use crate::tasks::{
        can_retry, claim_next, enqueue_job, enqueue_task, migrate_tables, note_shared_rate_limit,
        renew_lease, AdmissionGuard, ErrorCategory, NewJob, NewTask, OperationKind,
        ProviderAdmission, TaskState,
    };
    use rusqlite::Connection;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    #[tokio::test]
    async fn mock_provider_summarization_retries_then_gives_up() {
        let hits = AtomicU32::new(0);
        let out: crate::provider_request::AttemptOutcome<()> =
            execute_with_retries("fault-summ", OperationKind::Summarization, || async {
                hits.fetch_add(1, Ordering::SeqCst);
                Err(ErrorCategory::Timeout) as Result<(), ErrorCategory>
            })
            .await;
        assert!(out.value.is_none());
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        assert_eq!(out.attempts_used, 2);
        assert_eq!(out.last_category, ErrorCategory::Timeout);
    }

    #[tokio::test]
    async fn mock_provider_other_llm_allows_three_attempts() {
        let hits = AtomicU32::new(0);
        let out: crate::provider_request::AttemptOutcome<()> =
            execute_with_retries("fault-other", OperationKind::OtherLlm, || async {
                hits.fetch_add(1, Ordering::SeqCst);
                Err(ErrorCategory::RateLimit) as Result<(), ErrorCategory>
            })
            .await;
        assert!(out.value.is_none());
        assert_eq!(hits.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn mock_provider_recovers_after_transient_error() {
        let hits = AtomicU32::new(0);
        let out = execute_with_retries("fault-ok", OperationKind::Summarization, || async {
            let n = hits.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                Err(ErrorCategory::RateLimit)
            } else {
                Ok("ok")
            }
        })
        .await;
        assert_eq!(out.value.as_deref(), Some("ok"));
        assert_eq!(out.attempts_used, 2);
    }

    #[test]
    fn clock_set_foreground_then_job_expiry() {
        let mut clocks = ClockSet::new(Duration::from_secs(10), Duration::from_secs(4));
        clocks.started = Instant::now() - Duration::from_secs(5);
        assert!(clocks.foreground_expired());
        assert!(!clocks.job_expired());
        clocks.record_active(Duration::from_secs(1));
        clocks.started = Instant::now() - Duration::from_secs(11);
        assert!(clocks.job_expired());
        assert_eq!(clocks.remaining_lifetime(), Duration::ZERO);
    }

    #[test]
    fn clock_set_sequential_budget_is_sum_of_tool_estimates() {
        let tools = vec![
            Duration::from_secs(10),
            Duration::from_secs(20),
            Duration::from_secs(5),
        ];
        assert_eq!(
            ClockSet::sequential_tool_budget(&tools),
            Duration::from_secs(35)
        );
    }

    #[test]
    fn cache_invalidates_when_source_revision_changes() {
        let conn = Connection::open_in_memory().unwrap();
        migrate_tables(&conn).unwrap();
        let mut req = SummaryRequest {
            mode: SummarizationMode::FollowUpContext,
            sources: vec![SummarySource {
                id: "ans-1".into(),
                revision: "rev-1".into(),
                hash: "h1".into(),
                text: "hello".into(),
                meta: serde_json::json!({}),
            }],
            focus: "thread-a".into(),
            budget_chars: 400,
            required_fields: vec![],
            model: "m".into(),
            provider: "p".into(),
            prompt_version: SummarizationMode::FollowUpContext.prompt_version().into(),
        };
        let result = deterministic_follow_up("prior summary text for cache", 400);
        cache_put(&conn, &req, &result).unwrap();
        assert!(cache_get(&conn, &req).unwrap().is_some());
        req.sources[0].revision = "rev-2".into();
        assert!(
            cache_get(&conn, &req).unwrap().is_none(),
            "revision bump must miss cache"
        );
    }

    #[test]
    fn lease_fencing_rejects_stale_epoch_and_foreign_owner() {
        let conn = Connection::open_in_memory().unwrap();
        migrate_tables(&conn).unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        enqueue_job(
            &conn,
            &NewJob {
                id: "job-fence".into(),
                kind: "test".into(),
                owner_scope: "".into(),
                input_revision: "".into(),
                deadline_at: String::new(),
            },
            &now,
        )
        .unwrap();
        assert!(enqueue_task(
            &conn,
            &NewTask {
                id: "task-fence".into(),
                job_id: "job-fence".into(),
                operation: "graph_explanation".into(),
                dedupe_key: "fence-1".into(),
                priority: 10,
                input_ref: "mem".into(),
                input_hash: "h".into(),
                source_revision: "1".into(),
                role_snapshot: "summarization".into(),
                max_attempts: 2,
            },
            &now,
        )
        .unwrap());
        let id = claim_next(&conn, "owner-a", 30, &now).unwrap().unwrap();
        let epoch: i64 = conn
            .query_row(
                "SELECT lease_epoch FROM argos_tasks WHERE id=?1",
                [&id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(renew_lease(&conn, &id, "owner-a", epoch, 30, &now).unwrap());
        assert!(!renew_lease(&conn, &id, "owner-b", epoch, 30, &now).unwrap());
        assert!(!renew_lease(&conn, &id, "owner-a", epoch + 1, 30, &now).unwrap());
    }

    #[test]
    fn non_retryable_auth_errors_stop_immediately() {
        assert!(!can_retry(
            OperationKind::Summarization,
            1,
            ErrorCategory::AuthOrQuota
        ));
        assert!(can_retry(
            OperationKind::Summarization,
            1,
            ErrorCategory::RateLimit
        ));
    }

    #[test]
    fn task_state_round_trip_includes_retry_scheduled() {
        assert_eq!(
            TaskState::parse("retry_scheduled"),
            TaskState::RetryScheduled
        );
        assert_eq!(TaskState::RetryScheduled.as_str(), "retry_scheduled");
    }

    #[test]
    fn concurrent_insight_replace_serializes_on_same_article() {
        use crate::store::{AtlasInsightClaim, Store};
        use tempfile::tempdir;

        fn claim(entity: &str) -> AtlasInsightClaim {
            AtlasInsightClaim {
                fingerprint: String::new(),
                entity: entity.into(),
                namespace: "place".into(),
                predicate: "hosts".into(),
                object: "talks".into(),
                topic: "geopolitical".into(),
                claim: format!("{entity} hosts talks."),
                classification: "fact".into(),
                confidence: 0.5,
                article_id: "art-1".into(),
                source_url: "https://example.com/a".into(),
                published_at: "2026-10-04T12:00:00+00:00".into(),
                reliability: "B".into(),
                info_credibility: 2,
                admiralty: "B2".into(),
                rsp_status: String::new(),
            }
        }

        let dir = tempdir().unwrap();
        let path = dir.path().join("race.db");
        {
            let store = Store::open(&path).unwrap();
            store.atlas_insert_run("run-1", "{}", "{}").unwrap();
            // Minimal article row so FK/helpers stay happy if present.
            let _ = store.atlas_upsert_article(&crate::store::AtlasArticleRow {
                run_id: "run-1".into(),
                id: "art-1".into(),
                title: "Geneva hosts talks".into(),
                description: "Leaders meet.".into(),
                url: "https://example.com/a".into(),
                country: "CH".into(),
                source_name: "Ex".into(),
                source_domain: "example.com".into(),
                published_at: "2026-10-04T12:00:00+00:00".into(),
                provider: "news".into(),
                temperature: 0.4,
                category: "geopolitical".into(),
                seen_at: String::new(),
                author: String::new(),
                image_url: String::new(),
            });
            store
                .persist_atlas_insights("run-1", &[claim("geneva")], &[], "", "")
                .unwrap();
        }

        let path = Arc::new(path);
        let mut handles = Vec::new();
        for entity in ["alpha", "bravo", "charlie"] {
            let path = path.clone();
            let entity = entity.to_string();
            handles.push(std::thread::spawn(move || {
                let store = Store::open(&path).unwrap();
                store.commit_article_insight_replacement("run-1", "art-1", &[claim(&entity)], &[])
            }));
        }
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert!(
            results.iter().all(|r| r.is_ok()),
            "BEGIN IMMEDIATE must serialize replaces: {results:?}"
        );
        let store = Store::open(&path).unwrap();
        let rows = store.atlas_claims_for_article("run-1", "art-1").unwrap();
        // Last writer wins; set is consistent (single entity from one of the racers).
        assert_eq!(rows.len(), 1, "{rows:?}");
    }

    #[test]
    fn stream_interruption_preserves_partial_text() {
        use crate::osint::ToolResult;
        use crate::recon::budget;
        let results: Vec<(String, ToolResult)> = Vec::new();
        let partial = "The subject operates from Geneva.";
        let out = crate::recon::cut_short_answer(partial, &results, budget::STREAM_LOST);
        assert!(out.contains(partial), "{out}");
        assert!(
            out.contains("kept") || out.contains(budget::CUT_SHORT),
            "{out}"
        );
    }

    #[test]
    fn shared_rate_limit_cooldown_blocks_admission() {
        let mut gate = ProviderAdmission::default();
        gate.note_rate_limit("fault-local", Duration::from_secs(5));
        assert!(gate.cooling_down("fault-local"));
        assert!(!gate.try_acquire("fault-local", 2));

        note_shared_rate_limit("fault-global-cool", Duration::from_millis(250));
        assert!(
            AdmissionGuard::try_enter("fault-global-cool").is_none()
                || ProviderAdmission::global()
                    .lock()
                    .map(|g| g.cooling_down("fault-global-cool"))
                    .unwrap_or(true)
        );
    }
}
