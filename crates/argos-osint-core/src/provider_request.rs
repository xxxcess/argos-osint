//! Typed provider request policy and error recovery.
//!
//! Individual adapters normalize transport failures into [`ErrorCategory`]. Retry
//! decisions belong to the shared task scheduler (`tasks`), not nested loops here.

use std::time::Duration;

use crate::tasks::{self, AdmissionGuard, ErrorCategory, OperationKind};

/// Named timeout profiles for LLM operations (seconds).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeoutProfile {
    pub connection: u64,
    pub first_response: u64,
    pub stream_inactivity: u64,
    pub attempt_execution: u64,
}

impl TimeoutProfile {
    pub const CLASSIFIER: Self = Self {
        connection: 15,
        first_response: 30,
        stream_inactivity: 30,
        attempt_execution: 45,
    };
    pub const SUMMARIZATION: Self = Self {
        connection: 15,
        first_response: 45,
        stream_inactivity: 45,
        attempt_execution: 90,
    };
    pub const EXTRACTION: Self = Self {
        connection: 15,
        first_response: 60,
        stream_inactivity: 60,
        attempt_execution: 120,
    };
    pub const PLANNING: Self = Self {
        connection: 15,
        first_response: 45,
        stream_inactivity: 45,
        attempt_execution: 120,
    };
    pub const SYNTHESIS: Self = Self {
        connection: 20,
        first_response: 90,
        stream_inactivity: 60,
        attempt_execution: 300,
    };

    pub fn attempt_timeout(self) -> Duration {
        Duration::from_secs(self.attempt_execution)
    }
}

/// Map HTTP status + body heuristics onto a typed category.
pub fn categorize_http(status: u16, body: &str) -> ErrorCategory {
    let lower = body.to_ascii_lowercase();
    match status {
        401 | 403 => ErrorCategory::AuthOrQuota,
        402 | 429 => {
            if lower.contains("quota") || lower.contains("billing") || lower.contains("credit") {
                ErrorCategory::AuthOrQuota
            } else {
                ErrorCategory::RateLimit
            }
        }
        408 | 504 => ErrorCategory::Timeout,
        400 if lower.contains("context") || lower.contains("token") || lower.contains("too long") => {
            ErrorCategory::ContextLimit
        }
        400 if lower.contains("unsupported") || lower.contains("not supported") => {
            ErrorCategory::UnsupportedOption
        }
        408..=499 if status != 429 => ErrorCategory::InvalidResult,
        500..=599 => ErrorCategory::TemporaryNetwork,
        _ if status == 0 => ErrorCategory::TemporaryNetwork,
        _ => ErrorCategory::Unknown,
    }
}

/// Parse Retry-After header value (seconds or HTTP-date seconds fallback).
pub fn retry_after_delay(raw: Option<&str>) -> Option<Duration> {
    let raw = raw?.trim();
    if let Ok(secs) = raw.parse::<u64>() {
        return Some(Duration::from_secs(secs.min(300)));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categorizes_rate_limit_and_auth() {
        assert_eq!(categorize_http(429, "slow down"), ErrorCategory::RateLimit);
        assert_eq!(categorize_http(401, "bad key"), ErrorCategory::AuthOrQuota);
        assert_eq!(
            categorize_http(400, "context length exceeded"),
            ErrorCategory::ContextLimit
        );
        assert_eq!(categorize_http(503, "upstream"), ErrorCategory::TemporaryNetwork);
    }

    #[test]
    fn retry_after_seconds() {
        assert_eq!(retry_after_delay(Some("12")), Some(Duration::from_secs(12)));
        assert!(retry_after_delay(Some("nope")).is_none());
    }
}


/// Outcome of a bounded provider attempt loop owned by the shared policy.
#[derive(Clone, Debug)]
pub struct AttemptOutcome<T> {
    pub value: Option<T>,
    pub attempts_used: u32,
    pub last_category: ErrorCategory,
}

/// Run `op` under admission + attempt caps. The closure performs one provider call.
pub async fn execute_with_retries<T, F, Fut>(
    account: &str,
    kind: OperationKind,
    mut op: F,
) -> AttemptOutcome<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, ErrorCategory>>,
{
    let mut attempts = 0u32;
    let mut last = ErrorCategory::Unknown;
    while attempts < kind.attempt_cap() {
        let guard = loop {
            if let Some(g) = AdmissionGuard::try_enter(account) {
                break g;
            }
            tokio::time::sleep(tasks::backoff_delay(attempts.max(1))).await;
        };
        attempts += 1;
        match op().await {
            Ok(value) => {
                drop(guard);
                return AttemptOutcome {
                    value: Some(value),
                    attempts_used: attempts,
                    last_category: ErrorCategory::Unknown,
                };
            }
            Err(category) => {
                last = category;
                drop(guard);
                if !tasks::can_retry(kind, attempts, category) {
                    break;
                }
                tokio::time::sleep(tasks::backoff_delay(attempts)).await;
            }
        }
    }
    AttemptOutcome {
        value: None,
        attempts_used: attempts,
        last_category: last,
    }
}

#[cfg(test)]
mod retry_tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test]
    async fn summarization_stops_at_two_attempts() {
        let hits = AtomicU32::new(0);
        let out = execute_with_retries("acct-test", OperationKind::Summarization, || async {
            hits.fetch_add(1, Ordering::SeqCst);
            Err(ErrorCategory::RateLimit) as Result<(), ErrorCategory>
        })
        .await;
        assert_eq!(out.attempts_used, 2);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        assert!(out.value.is_none());
    }

    #[tokio::test]
    async fn other_llm_allows_three() {
        let hits = AtomicU32::new(0);
        let out = execute_with_retries("acct-test-2", OperationKind::OtherLlm, || async {
            hits.fetch_add(1, Ordering::SeqCst);
            Err(ErrorCategory::TemporaryNetwork) as Result<(), ErrorCategory>
        })
        .await;
        assert_eq!(out.attempts_used, 3);
        assert_eq!(hits.load(Ordering::SeqCst), 3);
    }
}
