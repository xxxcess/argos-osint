//! Normalized LLM failures and bounded recovery actions.

use serde::{Deserialize, Serialize};

use crate::provider_diag::{Category, ProviderFailure};

/// Bounded recovery action for a normalized LLM failure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecoveryAction {
    /// Retry after backoff / Retry-After wait.
    RetryBackoff { delay_ms: u64 },
    /// Provider/account circuit breaker triggered; cooldown before probe.
    CircuitCooldown { duration_ms: u64 },
    /// Requires credentials or reconnection; do not retry blindly.
    RequireAuth,
    /// Hard quota or credits exhausted; checkpoint and link settings.
    CreditsExhausted,
    /// Regional/entitlement restriction; do not attempt evasion.
    AccessRestricted { reason: String },
    /// Model refused prompt content; terminal for this request.
    ContentRefused { reason: String },
    /// Context window overflow; compress or reduce context.
    ReduceContext,
    /// Truncated JSON / malformed schema; bounded repair attempt.
    RepairOutput { attempt: u32 },
    /// Switch to eligible configured fallback model.
    FallbackModel { target_model: String },
    /// Defer task or mark unresolved when retries/budget exhausted.
    TerminalUnresolved { reason: String },
}

/// Map a `ProviderFailure` to its authoritative recovery action.
pub fn classify_recovery(
    failure: &ProviderFailure,
    current_attempt: u32,
    max_attempts: u32,
    fallback_model: Option<&str>,
) -> RecoveryAction {
    // 1. Terminal / non-retryable failures
    if failure.category == Category::Auth {
        return RecoveryAction::RequireAuth;
    }

    if failure.category == Category::Permission {
        return RecoveryAction::AccessRestricted {
            reason: failure.summary(),
        };
    }

    if failure.category == Category::Refused {
        return RecoveryAction::ContentRefused {
            reason: failure.summary(),
        };
    }

    // 2. Check credits exhaustion (often reported as 429 quota or 402 billing)
    let msg_lower = failure.message.to_ascii_lowercase();
    let prov_msg_lower = failure
        .provider_message
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();
    if msg_lower.contains("quota")
        || msg_lower.contains("credit")
        || msg_lower.contains("insufficient")
        || prov_msg_lower.contains("quota")
        || prov_msg_lower.contains("credit")
    {
        return RecoveryAction::CreditsExhausted;
    }

    // 3. Exhausted parent attempt budget
    if current_attempt >= max_attempts {
        return RecoveryAction::TerminalUnresolved {
            reason: format!(
                "attempt limit reached ({}/{}) without usable completion",
                current_attempt, max_attempts
            ),
        };
    }

    // 4. Rate limiting / 429
    if failure.category == Category::RateLimited {
        let delay = failure.retry_after_ms.unwrap_or(2_000).min(30_000);
        return RecoveryAction::RetryBackoff { delay_ms: delay };
    }

    // 5. Server overload / 503 / 529
    if failure.category == Category::Server {
        return RecoveryAction::CircuitCooldown {
            duration_ms: failure.retry_after_ms.unwrap_or(5_000).min(60_000),
        };
    }

    // 6. Token limit / truncated output / invalid result
    if matches!(
        failure.category,
        Category::TokenLimit | Category::MalformedPayload | Category::InvalidResult
    ) {
        return RecoveryAction::RepairOutput {
            attempt: current_attempt + 1,
        };
    }

    // 7. Missing model / 404
    if failure.category == Category::InvalidModel {
        if let Some(target) = fallback_model {
            return RecoveryAction::FallbackModel {
                target_model: target.to_string(),
            };
        }
        return RecoveryAction::TerminalUnresolved {
            reason: "configured model not found and no fallback configured".into(),
        };
    }

    // 8. Transient network / timeout / premature stream EOF
    if failure.category.retryable() {
        return RecoveryAction::RetryBackoff { delay_ms: 1_000 };
    }

    RecoveryAction::TerminalUnresolved {
        reason: failure.summary(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider_diag::Stage;

    #[test]
    fn auth_failure_is_not_retryable() {
        let failure = ProviderFailure::new(Stage::Response, Category::Auth, "401 Unauthorized");
        assert_eq!(
            classify_recovery(&failure, 1, 3, None),
            RecoveryAction::RequireAuth
        );
    }

    #[test]
    fn rate_limit_honors_retry_after() {
        let mut failure = ProviderFailure::new(
            Stage::Response,
            Category::RateLimited,
            "429 Too Many Requests",
        );
        failure.retry_after_ms = Some(4_500);
        assert_eq!(
            classify_recovery(&failure, 1, 3, None),
            RecoveryAction::RetryBackoff { delay_ms: 4_500 }
        );
    }

    #[test]
    fn quota_exhaustion_identified_distinct_from_rate_limit() {
        let mut failure = ProviderFailure::new(
            Stage::Response,
            Category::RateLimited,
            "429 Too Many Requests",
        );
        failure.provider_message =
            Some("You have exceeded your current quota, please check your plan".into());
        assert_eq!(
            classify_recovery(&failure, 1, 3, None),
            RecoveryAction::CreditsExhausted
        );
    }

    #[test]
    fn content_refusal_is_terminal() {
        let failure =
            ProviderFailure::new(Stage::Response, Category::Refused, "Policy blocked query");
        assert!(matches!(
            classify_recovery(&failure, 1, 3, None),
            RecoveryAction::ContentRefused { .. }
        ));
    }
}
