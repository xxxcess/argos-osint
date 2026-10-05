//! Spec §19 fault-injection / reliability matrix (offline).
//!
//! Covers mock providers, clocks, retries, cache invalidation, and lease fencing
//! without network access. Insight-replace atomicity lives in
//! `intel_recon::replace_insights` tests; dual Lance activation in `brain_lance`.

#[cfg(test)]
mod tests {
    use crate::provider_request::execute_with_retries;
    use crate::recon::clocks::ClockSet;
    use crate::summarization::{
        cache_get, cache_put, deterministic_follow_up, SummaryRequest, SummarySource,
        SummarizationMode,
    };
    use crate::tasks::{
        can_retry, claim_next, enqueue_job, enqueue_task, migrate_tables, renew_lease,
        ErrorCategory, NewJob, NewTask, OperationKind, TaskState,
    };
    use rusqlite::Connection;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{Duration, Instant};

    #[tokio::test]
    async fn mock_provider_summarization_retries_then_gives_up() {
        let hits = AtomicU32::new(0);
        let out: crate::provider_request::AttemptOutcome<()> = execute_with_retries(
            "fault-summ",
            OperationKind::Summarization,
            || async {
                hits.fetch_add(1, Ordering::SeqCst);
                Err(ErrorCategory::Timeout) as Result<(), ErrorCategory>
            },
        )
        .await;
        assert!(out.value.is_none());
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        assert_eq!(out.attempts_used, 2);
        assert_eq!(out.last_category, ErrorCategory::Timeout);
    }

    #[tokio::test]
    async fn mock_provider_other_llm_allows_three_attempts() {
        let hits = AtomicU32::new(0);
        let out: crate::provider_request::AttemptOutcome<()> = execute_with_retries(
            "fault-other",
            OperationKind::OtherLlm,
            || async {
                hits.fetch_add(1, Ordering::SeqCst);
                Err(ErrorCategory::RateLimit) as Result<(), ErrorCategory>
            },
        )
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
}
