CREATE TABLE IF NOT EXISTS recon_threads (
  id TEXT PRIMARY KEY, title TEXT NOT NULL, created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL, draft TEXT NOT NULL DEFAULT '',
  scroll INTEGER NOT NULL DEFAULT 0, deleted INTEGER NOT NULL DEFAULT 0,
  recall_insights INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS recon_threads_updated ON recon_threads(updated_at DESC);
CREATE TABLE IF NOT EXISTS recon_messages (
  id TEXT PRIMARY KEY, thread_id TEXT NOT NULL REFERENCES recon_threads(id) ON DELETE CASCADE,
  sequence INTEGER NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL,
  run_id TEXT, created_at TEXT NOT NULL, UNIQUE(thread_id, sequence)
);
CREATE TABLE IF NOT EXISTS recon_runs (
  id TEXT PRIMARY KEY, thread_id TEXT NOT NULL REFERENCES recon_threads(id) ON DELETE CASCADE,
  turn_id TEXT NOT NULL, state TEXT NOT NULL, stage TEXT NOT NULL,
  recon_model TEXT NOT NULL, synthesis_model TEXT NOT NULL,
  tool_picker_model TEXT NOT NULL DEFAULT '',
  max_rounds INTEGER NOT NULL DEFAULT 6, max_calls INTEGER NOT NULL DEFAULT 12,
  turn_seconds INTEGER NOT NULL DEFAULT 300,
  plan_json TEXT, error TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS osint_calls (
  id TEXT PRIMARY KEY, tool_id TEXT NOT NULL, run_id TEXT,
  thread_id TEXT, turn_id TEXT, origin TEXT NOT NULL, inputs_json TEXT NOT NULL,
  status TEXT NOT NULL, attempts INTEGER NOT NULL DEFAULT 0,
  result_json TEXT, started_at TEXT NOT NULL, completed_at TEXT
);
CREATE INDEX IF NOT EXISTS osint_calls_thread ON osint_calls(thread_id, started_at);
CREATE TABLE IF NOT EXISTS recon_message_evidence (
  message_id TEXT NOT NULL REFERENCES recon_messages(id) ON DELETE CASCADE,
  call_id TEXT NOT NULL REFERENCES osint_calls(id),
  PRIMARY KEY(message_id,call_id)
);
CREATE TABLE IF NOT EXISTS osint_cache (
  key TEXT PRIMARY KEY, result_json TEXT NOT NULL, expires_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS osint_preferences (
  tool_id TEXT PRIMARY KEY, enabled INTEGER NOT NULL DEFAULT 1
);
CREATE TABLE IF NOT EXISTS recon_entities (
  id TEXT PRIMARY KEY, kind TEXT NOT NULL, namespace TEXT NOT NULL DEFAULT '',
  canonical TEXT NOT NULL, label TEXT NOT NULL,
  UNIQUE(kind, namespace, canonical)
);
CREATE TABLE IF NOT EXISTS recon_thread_entities (
  thread_id TEXT NOT NULL REFERENCES recon_threads(id) ON DELETE CASCADE,
  entity_id TEXT NOT NULL REFERENCES recon_entities(id),
  source_call_id TEXT, PRIMARY KEY(thread_id, entity_id, source_call_id)
);
CREATE TABLE IF NOT EXISTS insight_claims (
  fingerprint TEXT PRIMARY KEY, memory_id TEXT NOT NULL REFERENCES memories(id),
  entity_id TEXT NOT NULL, predicate TEXT NOT NULL, object_value TEXT NOT NULL,
  topic TEXT NOT NULL, classification TEXT NOT NULL, confidence REAL NOT NULL,
  created_at TEXT NOT NULL, updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS insight_sources (
  fingerprint TEXT NOT NULL REFERENCES insight_claims(fingerprint) ON DELETE CASCADE,
  thread_id TEXT, run_id TEXT, answer_id TEXT NOT NULL,
  call_id TEXT NOT NULL, source_url TEXT,
  deleted_origin INTEGER NOT NULL DEFAULT 0,
  published_at TEXT NOT NULL DEFAULT '',
  PRIMARY KEY(fingerprint, answer_id, call_id)
);
CREATE TABLE IF NOT EXISTS insight_user_edits (memory_id TEXT PRIMARY KEY REFERENCES memories(id) ON DELETE CASCADE);
CREATE TABLE IF NOT EXISTS memory_graph_summaries (
  memory_id TEXT PRIMARY KEY,
  summary TEXT NOT NULL,
  created_at TEXT NOT NULL,
  focus TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS insight_relations (
  left_fingerprint TEXT NOT NULL, right_fingerprint TEXT NOT NULL,
  relation TEXT NOT NULL, PRIMARY KEY(left_fingerprint,right_fingerprint,relation)
);
CREATE TABLE IF NOT EXISTS extraction_jobs (
  answer_id TEXT PRIMARY KEY, run_id TEXT NOT NULL, state TEXT NOT NULL,
  error TEXT, updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS app_state (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS recon_message_memories (
  message_id TEXT NOT NULL REFERENCES recon_messages(id) ON DELETE CASCADE,
  memory_id TEXT NOT NULL,
  ordinal INTEGER NOT NULL,
  PRIMARY KEY(message_id, memory_id)
);
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
CREATE TABLE IF NOT EXISTS recon_model_operations (
  id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL DEFAULT '',
  task_id TEXT NOT NULL DEFAULT '',
  role TEXT NOT NULL,
  generation INTEGER NOT NULL DEFAULT 1,
  status TEXT NOT NULL,
  draft TEXT NOT NULL DEFAULT '',
  final_message_id TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS recon_operations_final_msg
  ON recon_model_operations(run_id, final_message_id)
  WHERE final_message_id IS NOT NULL AND run_id != '';
CREATE INDEX IF NOT EXISTS recon_operations_run ON recon_model_operations(run_id);
CREATE INDEX IF NOT EXISTS recon_operations_task ON recon_model_operations(task_id);

CREATE TABLE IF NOT EXISTS recon_model_attempts (
  id TEXT PRIMARY KEY,
  operation_id TEXT NOT NULL REFERENCES recon_model_operations(id) ON DELETE CASCADE,
  generation INTEGER NOT NULL DEFAULT 1,
  route_index INTEGER NOT NULL,
  attempt INTEGER NOT NULL,
  provider TEXT NOT NULL,
  account TEXT NOT NULL,
  model TEXT NOT NULL,
  transport TEXT NOT NULL,
  dispatched INTEGER NOT NULL,
  outcome TEXT NOT NULL,
  failure_category TEXT NOT NULL DEFAULT '',
  http_status INTEGER,
  request_id TEXT NOT NULL DEFAULT '',
  finish_reason TEXT NOT NULL DEFAULT '',
  char_count INTEGER NOT NULL DEFAULT 0,
  wait_ms INTEGER NOT NULL DEFAULT 0,
  error_message TEXT NOT NULL DEFAULT '',
  started_at TEXT NOT NULL,
  finished_at TEXT NOT NULL DEFAULT '',
  UNIQUE(operation_id, generation, route_index, attempt)
);
CREATE INDEX IF NOT EXISTS recon_attempts_op ON recon_model_attempts(operation_id, generation);

CREATE TABLE IF NOT EXISTS recon_coverage (
  id TEXT PRIMARY KEY,
  scope TEXT NOT NULL,
  generation INTEGER NOT NULL DEFAULT 1,
  directive_index INTEGER NOT NULL DEFAULT 0,
  category TEXT NOT NULL,
  payload_json TEXT NOT NULL DEFAULT '{}',
  updated_at TEXT NOT NULL,
  UNIQUE(scope, generation, directive_index, category)
);
CREATE INDEX IF NOT EXISTS recon_coverage_scope ON recon_coverage(scope, generation);

