-- Immutable completed report results survive operational job retention.
CREATE TABLE IF NOT EXISTS intel_report_results (
    job_id TEXT NOT NULL, generation INTEGER NOT NULL, article_id TEXT NOT NULL,
    completion_sequence INTEGER NOT NULL, source_hash TEXT NOT NULL,
    details_json TEXT NOT NULL, completed_at TEXT NOT NULL,
    PRIMARY KEY(job_id,generation), UNIQUE(article_id,completion_sequence)
);
CREATE TRIGGER IF NOT EXISTS intel_report_results_immutable BEFORE UPDATE ON intel_report_results
BEGIN SELECT RAISE(ABORT,'completed report results are immutable'); END;
CREATE TABLE IF NOT EXISTS article_refresh_intents (
    article_id TEXT PRIMARY KEY, requested_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS intel_report_support_receipts (
    job_id TEXT NOT NULL, generation INTEGER NOT NULL, element_id TEXT NOT NULL,
    assertion TEXT NOT NULL, receipts_json TEXT NOT NULL,
    PRIMARY KEY(job_id,generation,element_id)
);
CREATE TABLE IF NOT EXISTS article_refresh_retirements (
    article_id TEXT NOT NULL, input_key TEXT NOT NULL, retired_at TEXT NOT NULL,
    PRIMARY KEY(article_id,input_key)
);
CREATE TABLE IF NOT EXISTS argos_job_retirements (
    job_id TEXT PRIMARY KEY, retired_at TEXT NOT NULL, reason TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS argos_job_owners (
    job_id TEXT NOT NULL, owner_kind TEXT NOT NULL, owner_id TEXT NOT NULL,
    required INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY(job_id,owner_kind,owner_id)
);
