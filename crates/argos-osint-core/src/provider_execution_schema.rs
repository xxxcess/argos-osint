//! Migration 41 (spec §9): per-attempt usage/timing counters, policy
//! diagnostics and task epochs. Purely additive and idempotent: every column
//! is checked before it is added, inside `ensure_schema`'s transaction. New
//! counters stay NULL (unknown) for pre-41 rows — never zero.

const ATTEMPT_COLUMNS: [(&str, &str); 8] = [
    ("reasoning_tokens", "reasoning_tokens INTEGER"),
    ("visible_tokens", "visible_tokens INTEGER"),
    ("reported_cost_usd", "reported_cost_usd REAL"),
    ("response_headers_ms", "response_headers_ms INTEGER"),
    ("first_reasoning_ms", "first_reasoning_ms INTEGER"),
    ("first_output_ms", "first_output_ms INTEGER"),
    ("stream_completion_ms", "stream_completion_ms INTEGER"),
    (
        "policy_diagnostics_json",
        "policy_diagnostics_json TEXT NOT NULL DEFAULT '[]'",
    ),
];
