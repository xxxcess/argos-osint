CREATE TABLE IF NOT EXISTS atlas_work_units (
    run_id TEXT NOT NULL,
    unit_id TEXT NOT NULL,
    stage INTEGER NOT NULL,
    input_ids TEXT NOT NULL,
    input_rev TEXT NOT NULL,
    contract_version TEXT NOT NULL,
    dependency_ids TEXT NOT NULL,
    is_required INTEGER NOT NULL,
    output_refs TEXT NOT NULL,
    effective_model TEXT NOT NULL,
    attempt_history TEXT NOT NULL,
    next_eligible_at TEXT,
    terminal_reason TEXT,
    output_json TEXT NOT NULL DEFAULT '',
    disposition TEXT NOT NULL DEFAULT '',
    receipt_json TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (run_id, unit_id)
);

CREATE TABLE IF NOT EXISTS memory_metadata (
    memory_id TEXT PRIMARY KEY REFERENCES memories(id) ON DELETE CASCADE,
    memory_kind TEXT NOT NULL,
    domain_scope TEXT NOT NULL DEFAULT '',
    subjects_json TEXT NOT NULL DEFAULT '[]',
    assessment_state TEXT NOT NULL DEFAULT '',
    source_coverage TEXT NOT NULL DEFAULT '',
    derivation_json TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE IF NOT EXISTS memory_brief_memberships (
    brief_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    member_id TEXT NOT NULL,
    member_type TEXT NOT NULL,
    PRIMARY KEY (brief_id, member_id)
);
