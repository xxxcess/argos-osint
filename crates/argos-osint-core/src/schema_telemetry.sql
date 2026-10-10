-- Telemetry and activity history for the Profile dashboard (schema 27).
--
-- Design rules enforced here and in `crate::telemetry`:
--   * one authoritative row per logical fact (never double counted);
--   * bounded dimensions, no prompts, article bodies, model outputs or keys;
--   * raw events are retained 90 days, hourly rollups 365 days;
--   * aggregates are derived, never authored by the app.
--
-- The provider orchestration companion owns admission policy and quota
-- accounting. Nothing in this file reimplements a scheduler or a retry policy;
-- `telemetry_provider_capacity` is a cache of an externally supplied snapshot.

CREATE TABLE IF NOT EXISTS telemetry_events (
  id TEXT PRIMARY KEY,
  event_type TEXT NOT NULL,
  occurred_at TEXT NOT NULL,
  day TEXT NOT NULL DEFAULT '',
  hour TEXT NOT NULL DEFAULT '',
  -- Bounded attribution dimensions.
  app TEXT NOT NULL DEFAULT '',
  trigger TEXT NOT NULL DEFAULT '',
  tool_id TEXT NOT NULL DEFAULT '',
  category TEXT NOT NULL DEFAULT '',
  provider TEXT NOT NULL DEFAULT '',
  role TEXT NOT NULL DEFAULT '',
  model TEXT NOT NULL DEFAULT '',
  mode TEXT NOT NULL DEFAULT '',
  engine TEXT NOT NULL DEFAULT '',
  outcome TEXT NOT NULL DEFAULT '',
  reason TEXT NOT NULL DEFAULT '',
  -- Bounded linkage to existing durable IDs.
  job_id TEXT NOT NULL DEFAULT '',
  run_id TEXT NOT NULL DEFAULT '',
  thread_id TEXT NOT NULL DEFAULT '',
  turn_id TEXT NOT NULL DEFAULT '',
  article_id TEXT NOT NULL DEFAULT '',
  origin TEXT NOT NULL DEFAULT '',
  call_id TEXT NOT NULL DEFAULT '',
  canonical_ref TEXT NOT NULL DEFAULT '',
  metric_ms INTEGER,
  count INTEGER NOT NULL DEFAULT 1,
  payload_json TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS telemetry_events_time ON telemetry_events(occurred_at);
CREATE INDEX IF NOT EXISTS telemetry_events_type_time ON telemetry_events(event_type, occurred_at);
CREATE INDEX IF NOT EXISTS telemetry_events_tool ON telemetry_events(tool_id, occurred_at);
CREATE INDEX IF NOT EXISTS telemetry_events_provider ON telemetry_events(provider, occurred_at);
CREATE INDEX IF NOT EXISTS telemetry_events_role ON telemetry_events(role, occurred_at);
CREATE INDEX IF NOT EXISTS telemetry_events_mode ON telemetry_events(mode, occurred_at);
CREATE INDEX IF NOT EXISTS telemetry_events_job ON telemetry_events(job_id, occurred_at);
CREATE INDEX IF NOT EXISTS telemetry_events_run ON telemetry_events(run_id, occurred_at);
CREATE INDEX IF NOT EXISTS telemetry_events_canonical ON telemetry_events(canonical_ref);
CREATE INDEX IF NOT EXISTS telemetry_events_article ON telemetry_events(article_id, occurred_at);
CREATE INDEX IF NOT EXISTS telemetry_events_origin ON telemetry_events(origin, occurred_at);
CREATE INDEX IF NOT EXISTS telemetry_events_category ON telemetry_events(category, occurred_at);
CREATE INDEX IF NOT EXISTS telemetry_events_outcome ON telemetry_events(event_type, outcome, occurred_at);

-- Additive hourly rollup, retained 365 days. Dimension subsets are collapsed
-- into `dim_key` so bins and counts stay mergeable instead of averaging
-- pre-aggregated averages.
CREATE TABLE IF NOT EXISTS telemetry_hourly (
  bucket TEXT NOT NULL,
  event_type TEXT NOT NULL,
  dim_key TEXT NOT NULL DEFAULT '',
  events INTEGER NOT NULL DEFAULT 0,
  total_ms INTEGER NOT NULL DEFAULT 0,
  max_ms INTEGER NOT NULL DEFAULT 0,
  bins_json TEXT NOT NULL DEFAULT '[]',
  PRIMARY KEY (bucket, event_type, dim_key)
);
CREATE INDEX IF NOT EXISTS telemetry_hourly_type ON telemetry_hourly(event_type, bucket);

-- Daily rollup, retained 365 days (same mergeable shape).
CREATE TABLE IF NOT EXISTS telemetry_daily (
  bucket TEXT NOT NULL,
  event_type TEXT NOT NULL,
  dim_key TEXT NOT NULL DEFAULT '',
  events INTEGER NOT NULL DEFAULT 0,
  total_ms INTEGER NOT NULL DEFAULT 0,
  max_ms INTEGER NOT NULL DEFAULT 0,
  bins_json TEXT NOT NULL DEFAULT '[]',
  PRIMARY KEY (bucket, event_type, dim_key)
);

-- Migration / retention markers: `observed_since`, `backfill_version`,
-- `raw_pruned_at`, `rollup_pruned_at`.
CREATE TABLE IF NOT EXISTS telemetry_meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

-- Cached provider capacity snapshots supplied by the orchestration companion.
-- Read-only for this crate; never a source of policy.
CREATE TABLE IF NOT EXISTS telemetry_provider_capacity (
  captured_at TEXT NOT NULL,
  provider TEXT NOT NULL,
  quota_group TEXT NOT NULL DEFAULT '',
  scope TEXT NOT NULL DEFAULT '',
  sends_60s INTEGER NOT NULL DEFAULT 0,
  rpm_limit INTEGER,
  pace_per_min REAL,
  active INTEGER NOT NULL DEFAULT 0,
  max_concurrency INTEGER,
  queued INTEGER NOT NULL DEFAULT 0,
  oldest_wait_ms INTEGER,
  cooldown_ms INTEGER,
  quota_source TEXT NOT NULL DEFAULT '',
  PRIMARY KEY (provider, quota_group, scope, captured_at)
);

-- Idempotent backfill ledger: which source rows have already been converted.
CREATE TABLE IF NOT EXISTS telemetry_backfill (
  source TEXT NOT NULL,
  source_id TEXT NOT NULL,
  converted_at TEXT NOT NULL,
  PRIMARY KEY (source, source_id)
);
