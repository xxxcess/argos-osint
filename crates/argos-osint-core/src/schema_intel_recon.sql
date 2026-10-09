-- Intel article-body cache and Recon report jobs (schema version 16).

CREATE TABLE IF NOT EXISTS article_bodies (
  id TEXT PRIMARY KEY,
  article_id TEXT NOT NULL,
  run_id TEXT NOT NULL DEFAULT '',
  original_url TEXT NOT NULL,
  resolved_url TEXT NOT NULL DEFAULT '',
  source_domain TEXT NOT NULL DEFAULT '',
  source_name TEXT NOT NULL DEFAULT '',
  body_markdown TEXT NOT NULL DEFAULT '',
  content_hash TEXT NOT NULL DEFAULT '',
  quality TEXT NOT NULL DEFAULT 'unavailable',
  quality_rationale TEXT NOT NULL DEFAULT '',
  body_version INTEGER NOT NULL DEFAULT 1,
  fetch_tool TEXT NOT NULL DEFAULT '',
  fetched_at TEXT NOT NULL DEFAULT '',
  retry_after TEXT NOT NULL DEFAULT '',
  state TEXT NOT NULL DEFAULT 'idle',
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE(article_id, original_url)
);
CREATE INDEX IF NOT EXISTS article_bodies_article ON article_bodies(article_id, updated_at DESC);

CREATE TABLE IF NOT EXISTS article_retrieval_attempts (
  id TEXT PRIMARY KEY,
  body_id TEXT NOT NULL REFERENCES article_bodies(id) ON DELETE CASCADE,
  tool_id TEXT NOT NULL,
  input_identity TEXT NOT NULL DEFAULT '',
  state TEXT NOT NULL,
  started_at TEXT NOT NULL,
  finished_at TEXT NOT NULL DEFAULT '',
  reason TEXT NOT NULL DEFAULT '',
  remote_job_id TEXT NOT NULL DEFAULT '',
  body_version INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX IF NOT EXISTS article_retrieval_attempts_body
  ON article_retrieval_attempts(body_id, started_at);

CREATE TABLE IF NOT EXISTS intel_investigations (
  id TEXT PRIMARY KEY,
  article_id TEXT NOT NULL,
  run_id TEXT NOT NULL DEFAULT '',
  article_url TEXT NOT NULL,
  thread_id TEXT,
  scope_json TEXT NOT NULL DEFAULT '{}',
  shared_assessment_json TEXT NOT NULL DEFAULT '{}',
  shared_assessment_version INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE(article_id)
);
CREATE INDEX IF NOT EXISTS intel_investigations_article
  ON intel_investigations(article_id);

CREATE TABLE IF NOT EXISTS intel_report_jobs (
  id TEXT PRIMARY KEY,
  investigation_id TEXT NOT NULL REFERENCES intel_investigations(id) ON DELETE CASCADE,
  article_id TEXT NOT NULL,
  mode TEXT NOT NULL,
  revision INTEGER NOT NULL DEFAULT 1,
  state TEXT NOT NULL,
  stage TEXT NOT NULL DEFAULT '',
  settings_json TEXT NOT NULL DEFAULT '{}',
  budget_json TEXT NOT NULL DEFAULT '{}',
  parent_job_id TEXT,
  sections_done INTEGER NOT NULL DEFAULT 0,
  sections_total INTEGER NOT NULL DEFAULT 0,
  elements_done INTEGER NOT NULL DEFAULT 0,
  elements_total INTEGER NOT NULL DEFAULT 0,
  tool_calls_done INTEGER NOT NULL DEFAULT 0,
  tool_calls_allowance INTEGER NOT NULL DEFAULT 0,
  current_tool TEXT NOT NULL DEFAULT '',
  warning TEXT NOT NULL DEFAULT '',
  error TEXT NOT NULL DEFAULT '',
  generation INTEGER NOT NULL DEFAULT 1,
  started_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  finished_at TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS intel_report_jobs_article
  ON intel_report_jobs(article_id, updated_at DESC);
CREATE INDEX IF NOT EXISTS intel_report_jobs_investigation
  ON intel_report_jobs(investigation_id, mode, revision);

CREATE TABLE IF NOT EXISTS intel_report_tasks (
  id TEXT PRIMARY KEY,
  job_id TEXT NOT NULL REFERENCES intel_report_jobs(id) ON DELETE CASCADE,
  task_type TEXT NOT NULL,
  section_key TEXT NOT NULL DEFAULT '',
  depends_on_json TEXT NOT NULL DEFAULT '[]',
  input_hash TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL,
  attempts INTEGER NOT NULL DEFAULT 0,
  max_attempts INTEGER NOT NULL DEFAULT 3,
  output_ref TEXT NOT NULL DEFAULT '',
  lease_owner TEXT NOT NULL DEFAULT '',
  lease_until TEXT NOT NULL DEFAULT '',
  lease_epoch INTEGER NOT NULL DEFAULT 0,
  error TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS intel_report_tasks_job
  ON intel_report_tasks(job_id, status);
CREATE INDEX IF NOT EXISTS intel_report_tasks_lease
  ON intel_report_tasks(status, lease_until);

CREATE TABLE IF NOT EXISTS intel_report_attempts (
  id TEXT PRIMARY KEY,
  job_id TEXT NOT NULL REFERENCES intel_report_jobs(id) ON DELETE CASCADE,
  generation INTEGER NOT NULL,
  task_id TEXT NOT NULL REFERENCES intel_report_tasks(id) ON DELETE CASCADE,
  tool_id TEXT NOT NULL,
  state TEXT NOT NULL,
  started_at TEXT NOT NULL,
  finished_at TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS intel_report_attempts_job ON intel_report_attempts(job_id, generation);

CREATE TABLE IF NOT EXISTS intel_report_sections (
  id TEXT PRIMARY KEY,
  job_id TEXT NOT NULL REFERENCES intel_report_jobs(id) ON DELETE CASCADE,
  section_key TEXT NOT NULL,
  title TEXT NOT NULL,
  ordinal INTEGER NOT NULL,
  markdown TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL,
  revision INTEGER NOT NULL DEFAULT 1,
  evidence_ids_json TEXT NOT NULL DEFAULT '[]',
  judgment_json TEXT NOT NULL DEFAULT '{}',
  assessment_version INTEGER NOT NULL DEFAULT 0,
  waiting_on TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE(job_id, section_key)
);
CREATE INDEX IF NOT EXISTS intel_report_sections_job
  ON intel_report_sections(job_id, ordinal);

CREATE TABLE IF NOT EXISTS intel_element_ledger (
  id TEXT PRIMARY KEY,
  investigation_id TEXT NOT NULL REFERENCES intel_investigations(id) ON DELETE CASCADE,
  element_key TEXT NOT NULL,
  fingerprint TEXT NOT NULL DEFAULT '',
  element_type TEXT NOT NULL,
  original_text TEXT NOT NULL,
  provenance TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'pending',
  stance TEXT NOT NULL DEFAULT '',
  assessment TEXT NOT NULL DEFAULT '',
  uncertainty TEXT NOT NULL DEFAULT '',
  evidence_ids_json TEXT NOT NULL DEFAULT '[]',
  directive_ids_json TEXT NOT NULL DEFAULT '[]',
  section_keys_json TEXT NOT NULL DEFAULT '[]',
  excluded_reason TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE(investigation_id, element_key)
);
CREATE INDEX IF NOT EXISTS intel_element_ledger_investigation
  ON intel_element_ledger(investigation_id, status);

CREATE TABLE IF NOT EXISTS intel_evidence (
  id TEXT PRIMARY KEY,
  investigation_id TEXT NOT NULL REFERENCES intel_investigations(id) ON DELETE CASCADE,
  source_url TEXT NOT NULL DEFAULT '',
  source_domain TEXT NOT NULL DEFAULT '',
  excerpt TEXT NOT NULL,
  location TEXT NOT NULL DEFAULT '',
  claim_ids_json TEXT NOT NULL DEFAULT '[]',
  source_date TEXT NOT NULL DEFAULT '',
  event_date TEXT NOT NULL DEFAULT '',
  retrieved_at TEXT NOT NULL,
  stance TEXT NOT NULL DEFAULT 'mention',
  origin TEXT NOT NULL DEFAULT '',
  independence TEXT NOT NULL DEFAULT '',
  limitations TEXT NOT NULL DEFAULT '',
  tool_id TEXT NOT NULL DEFAULT '',
  call_id TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS intel_evidence_investigation
  ON intel_evidence(investigation_id, created_at);

CREATE TABLE IF NOT EXISTS intel_assessments (
  id TEXT PRIMARY KEY,
  investigation_id TEXT NOT NULL REFERENCES intel_investigations(id) ON DELETE CASCADE,
  element_id TEXT NOT NULL REFERENCES intel_element_ledger(id) ON DELETE CASCADE,
  origin TEXT NOT NULL,
  stance TEXT NOT NULL DEFAULT '',
  rationale TEXT NOT NULL DEFAULT '',
  confidence REAL NOT NULL DEFAULT 0,
  sources_json TEXT NOT NULL DEFAULT '[]',
  original_value_json TEXT NOT NULL DEFAULT '{}',
  revised_value_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS intel_assessments_element
  ON intel_assessments(element_id, origin);

CREATE TABLE IF NOT EXISTS intel_link_explanations (
  article_id TEXT NOT NULL,
  left_id TEXT NOT NULL,
  right_id TEXT NOT NULL,
  explanation TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  PRIMARY KEY (article_id, left_id, right_id)
);
CREATE INDEX IF NOT EXISTS intel_link_explanations_article
  ON intel_link_explanations(article_id);

