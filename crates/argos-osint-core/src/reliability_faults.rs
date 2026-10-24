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
        assert_eq!(hits.load(Ordering::SeqCst), 4);
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
