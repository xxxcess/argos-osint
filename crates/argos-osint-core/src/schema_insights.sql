-- Schema 29: provenance records share the existing canonical Brain database.
CREATE TABLE IF NOT EXISTS insight_owner_state (
    owner_kind TEXT NOT NULL, owner_id TEXT NOT NULL,
    desired_generation INTEGER NOT NULL DEFAULT 1,
    active_snapshot_id TEXT NOT NULL DEFAULT '', refresh_job_id TEXT NOT NULL DEFAULT '',
    lifecycle TEXT NOT NULL DEFAULT 'pending', last_error TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL, PRIMARY KEY(owner_kind,owner_id)
);
CREATE TABLE IF NOT EXISTS insight_snapshots (
    id TEXT PRIMARY KEY, owner_kind TEXT NOT NULL, owner_id TEXT NOT NULL,
    generation INTEGER NOT NULL, input_key TEXT NOT NULL,
    inputs_json TEXT NOT NULL, entities_json TEXT NOT NULL, coverage_json TEXT NOT NULL,
    readiness_json TEXT NOT NULL, contract_version TEXT NOT NULL, policy_version TEXT NOT NULL,
    model_versions_json TEXT NOT NULL DEFAULT '{}', created_at TEXT NOT NULL,
    UNIQUE(owner_kind,owner_id,generation,input_key)
);
CREATE INDEX IF NOT EXISTS insight_snapshot_owner ON insight_snapshots(owner_kind,owner_id,generation);
CREATE TABLE IF NOT EXISTS insight_derivations (
    id TEXT PRIMARY KEY, snapshot_id TEXT NOT NULL REFERENCES insight_snapshots(id) ON DELETE CASCADE,
    memory_id TEXT, insight_kind TEXT NOT NULL CHECK(insight_kind IN ('claim','inference')),
    fingerprint TEXT NOT NULL, item_json TEXT NOT NULL, assessment_json TEXT NOT NULL,
    status TEXT NOT NULL, revision TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS insight_derivation_memory ON insight_derivations(memory_id);
CREATE INDEX IF NOT EXISTS insight_derivation_snapshot ON insight_derivations(snapshot_id);
CREATE TABLE IF NOT EXISTS insight_derivation_evidence (
    derivation_id TEXT NOT NULL REFERENCES insight_derivations(id) ON DELETE CASCADE,
    source_id TEXT NOT NULL, source_revision TEXT NOT NULL, memory_id TEXT,
    premise_id TEXT NOT NULL DEFAULT '', evidence_json TEXT NOT NULL,
    PRIMARY KEY(derivation_id,source_id,source_revision,premise_id,evidence_json)
);
CREATE INDEX IF NOT EXISTS insight_evidence_source ON insight_derivation_evidence(source_id,source_revision);
CREATE INDEX IF NOT EXISTS insight_evidence_memory ON insight_derivation_evidence(memory_id);
CREATE TABLE IF NOT EXISTS insight_derivation_links (
    id TEXT PRIMARY KEY, snapshot_id TEXT NOT NULL REFERENCES insight_snapshots(id) ON DELETE CASCADE,
    left_kind TEXT NOT NULL, left_id TEXT NOT NULL, right_kind TEXT NOT NULL, right_id TEXT NOT NULL,
    relation TEXT NOT NULL, rationale TEXT NOT NULL, assessment_json TEXT NOT NULL,
    evidence_json TEXT NOT NULL, status TEXT NOT NULL, revision TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS insight_links_snapshot ON insight_derivation_links(snapshot_id);
CREATE INDEX IF NOT EXISTS insight_links_left ON insight_derivation_links(left_kind,left_id);
CREATE INDEX IF NOT EXISTS insight_links_right ON insight_derivation_links(right_kind,right_id);
CREATE TABLE IF NOT EXISTS insight_stage_checkpoints (
    operation_key TEXT PRIMARY KEY, owner_kind TEXT NOT NULL, owner_id TEXT NOT NULL,
    generation INTEGER NOT NULL, stage TEXT NOT NULL, window_id TEXT NOT NULL,
    state TEXT NOT NULL, output_json TEXT NOT NULL DEFAULT '',
    route_json TEXT NOT NULL DEFAULT '{}', error TEXT NOT NULL DEFAULT '', updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS insight_checkpoint_owner ON insight_stage_checkpoints(owner_kind,owner_id,generation);
