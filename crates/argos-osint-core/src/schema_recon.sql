CREATE TABLE IF NOT EXISTS recon_threads (
  id TEXT PRIMARY KEY, title TEXT NOT NULL, created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL, draft TEXT NOT NULL DEFAULT '',
  scroll INTEGER NOT NULL DEFAULT 0, deleted INTEGER NOT NULL DEFAULT 0
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
  PRIMARY KEY(fingerprint, answer_id, call_id)
);
CREATE TABLE IF NOT EXISTS insight_user_edits (memory_id TEXT PRIMARY KEY REFERENCES memories(id) ON DELETE CASCADE);
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
