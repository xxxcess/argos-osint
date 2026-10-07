-- Unified investigation harness schema (version 20)
-- Tasks, dependencies, evidence passages, claim assessments, durable events, and stream parts.

CREATE TABLE IF NOT EXISTS investigation_tasks (
  id TEXT PRIMARY KEY,
  investigation_id TEXT NOT NULL,
  run_id TEXT NOT NULL DEFAULT '',
  directive_id TEXT NOT NULL DEFAULT '',
  task_id TEXT NOT NULL,
  revision INTEGER NOT NULL DEFAULT 1,
  surface TEXT NOT NULL,
  objective TEXT NOT NULL,
  report_mode TEXT NOT NULL,
  strategy TEXT NOT NULL,
  bindings_json TEXT NOT NULL DEFAULT '[]',
  required_evidence_json TEXT NOT NULL DEFAULT '[]',
  freshness_cutoff TEXT NOT NULL DEFAULT '',
  allowed_capabilities_json TEXT NOT NULL DEFAULT '[]',
  policy_json TEXT NOT NULL DEFAULT '{}',
  budget_ceiling_json TEXT NOT NULL DEFAULT '{}',
  output_schema_json TEXT NOT NULL DEFAULT '{}',
  completion_criteria TEXT NOT NULL DEFAULT '',
  attempts INTEGER NOT NULL DEFAULT 0,
  max_attempts INTEGER NOT NULL DEFAULT 3,
  status TEXT NOT NULL,
  output_ref TEXT NOT NULL DEFAULT '',
  unmet_needs_json TEXT NOT NULL DEFAULT '[]',
  terminal_reason TEXT NOT NULL DEFAULT '',
  superseded_revision INTEGER,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_investigation_tasks_inv ON investigation_tasks(investigation_id, status);
CREATE INDEX IF NOT EXISTS idx_investigation_tasks_run ON investigation_tasks(run_id, status);

CREATE TABLE IF NOT EXISTS investigation_task_dependencies (
  task_id TEXT NOT NULL REFERENCES investigation_tasks(id) ON DELETE CASCADE,
  depends_on_task_id TEXT NOT NULL,
  PRIMARY KEY (task_id, depends_on_task_id)
);

CREATE TABLE IF NOT EXISTS investigation_evidence_passages (
  id TEXT PRIMARY KEY,
  investigation_id TEXT NOT NULL,
  task_id TEXT NOT NULL DEFAULT '',
  call_id TEXT NOT NULL DEFAULT '',
  source_url TEXT NOT NULL DEFAULT '',
  source_domain TEXT NOT NULL DEFAULT '',
  passage_text TEXT NOT NULL,
  observed_at TEXT NOT NULL DEFAULT '',
  published_at TEXT NOT NULL DEFAULT '',
  stance TEXT NOT NULL DEFAULT 'mention',
  relevance_score REAL NOT NULL DEFAULT 1.0,
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_investigation_evidence_inv ON investigation_evidence_passages(investigation_id, created_at);

CREATE TABLE IF NOT EXISTS investigation_claim_assessments (
  id TEXT PRIMARY KEY,
  investigation_id TEXT NOT NULL,
  claim_id TEXT NOT NULL,
  evidence_id TEXT NOT NULL REFERENCES investigation_evidence_passages(id) ON DELETE CASCADE,
  stance TEXT NOT NULL,
  rationale TEXT NOT NULL,
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_investigation_claim_assessments_claim ON investigation_claim_assessments(claim_id);

CREATE TABLE IF NOT EXISTS investigation_events (
  id TEXT PRIMARY KEY,
  investigation_id TEXT NOT NULL,
  sequence INTEGER NOT NULL,
  occurrence_time TEXT NOT NULL,
  recording_time TEXT NOT NULL,
  surface TEXT NOT NULL,
  run_id TEXT NOT NULL DEFAULT '',
  directive_id TEXT NOT NULL DEFAULT '',
  task_id TEXT NOT NULL DEFAULT '',
  revision INTEGER NOT NULL DEFAULT 1,
  parent_event_id TEXT,
  role TEXT NOT NULL DEFAULT '',
  model TEXT NOT NULL DEFAULT '',
  provider TEXT NOT NULL DEFAULT '',
  attempt_id TEXT NOT NULL DEFAULT '',
  event_type TEXT NOT NULL,
  status TEXT NOT NULL,
  summary TEXT NOT NULL,
  payload_json TEXT NOT NULL DEFAULT '{}',
  evidence_refs_json TEXT NOT NULL DEFAULT '[]',
  superseded_event_id TEXT
);
CREATE INDEX IF NOT EXISTS idx_investigation_events_seq ON investigation_events(investigation_id, sequence);

CREATE TABLE IF NOT EXISTS investigation_stream_parts (
  id TEXT PRIMARY KEY,
  event_id TEXT NOT NULL REFERENCES investigation_events(id) ON DELETE CASCADE,
  stream_kind TEXT NOT NULL,
  chunk_offset INTEGER NOT NULL,
  content TEXT NOT NULL,
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_investigation_stream_parts_event ON investigation_stream_parts(event_id, chunk_offset);
