CREATE TABLE IF NOT EXISTS investigation_strategies (
  id TEXT PRIMARY KEY,
  thread_id TEXT NOT NULL,
  run_id TEXT NOT NULL,
  kind TEXT NOT NULL,
  rationale TEXT NOT NULL,
  previous_kind TEXT,
  change_reason TEXT,
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS investigation_strategies_thread
  ON investigation_strategies(thread_id, created_at);
CREATE TABLE IF NOT EXISTS investigation_hypotheses (
  id TEXT PRIMARY KEY,
  thread_id TEXT NOT NULL,
  run_id TEXT NOT NULL,
  question TEXT NOT NULL,
  alternatives_json TEXT NOT NULL,
  status TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS investigation_hypotheses_thread
  ON investigation_hypotheses(thread_id, updated_at);
CREATE TABLE IF NOT EXISTS investigation_gaps (
  id TEXT PRIMARY KEY,
  thread_id TEXT NOT NULL,
  run_id TEXT NOT NULL,
  question TEXT NOT NULL,
  kind TEXT NOT NULL,
  status TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS investigation_actions (
  id TEXT PRIMARY KEY,
  thread_id TEXT NOT NULL,
  run_id TEXT NOT NULL,
  gap_id TEXT NOT NULL,
  tool_id TEXT NOT NULL,
  arguments_json TEXT NOT NULL,
  purpose TEXT NOT NULL,
  evidence_json TEXT NOT NULL,
  expected TEXT NOT NULL,
  credit_cost INTEGER NOT NULL,
  cache_available INTEGER NOT NULL,
  rank_reason TEXT NOT NULL,
  status TEXT NOT NULL,
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS investigation_actions_thread
  ON investigation_actions(thread_id, created_at);
CREATE TABLE IF NOT EXISTS investigation_entities (
  id TEXT PRIMARY KEY,
  thread_id TEXT NOT NULL,
  canonical_name TEXT NOT NULL,
  entity_type TEXT NOT NULL,
  identifiers_json TEXT NOT NULL,
  evidence_json TEXT NOT NULL,
  relationships_json TEXT NOT NULL,
  unresolved_json TEXT NOT NULL,
  certainty TEXT NOT NULL,
  why TEXT NOT NULL,
  selected INTEGER NOT NULL DEFAULT 1,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS investigation_entities_thread
  ON investigation_entities(thread_id, updated_at);
CREATE TABLE IF NOT EXISTS investigation_discovery (
  thread_id TEXT PRIMARY KEY,
  status TEXT NOT NULL,
  note TEXT NOT NULL,
  subject TEXT NOT NULL,
  objective TEXT NOT NULL,
  constraints_text TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS provider_quota (
  provider TEXT PRIMARY KEY,
  allowance INTEGER NOT NULL,
  trial_remaining INTEGER NOT NULL,
  trial_seed INTEGER NOT NULL,
  reserved INTEGER NOT NULL,
  spent INTEGER NOT NULL,
  reset_policy TEXT NOT NULL,
  period_start TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS credit_reservations (
  id TEXT PRIMARY KEY,
  provider TEXT NOT NULL,
  trial_credits INTEGER NOT NULL,
  allowance_credits INTEGER NOT NULL,
  state TEXT NOT NULL,
  created_at TEXT NOT NULL
);
