-- Job system schema (migration phase 1). Applied by storage::jobs::apply_schema.
-- Idempotent: every definition uses IF NOT EXISTS.

DEFINE TABLE IF NOT EXISTS job SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS kind ON job TYPE string DEFAULT 'task';
DEFINE FIELD IF NOT EXISTS operation ON job TYPE string;
DEFINE FIELD IF NOT EXISTS parent ON job TYPE option<record<job>>;
DEFINE FIELD IF NOT EXISTS ancestors ON job TYPE array<record<job>> DEFAULT [];
-- Descendant lookup (stop-batch) goes through job_ancestor below: on 3.3.4
-- no form of "$j IN ancestors" uses an index on this array.
-- Top ancestor (itself for a root): one guard and one cancel flag per tree.
DEFINE FIELD IF NOT EXISTS root ON job TYPE record<job>;
DEFINE FIELD IF NOT EXISTS state ON job TYPE string;
DEFINE FIELD IF NOT EXISTS cancel_requested ON job TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS stopped ON job TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS phase ON job TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS created_at ON job TYPE datetime READONLY;
DEFINE FIELD IF NOT EXISTS updated_at ON job TYPE datetime;
DEFINE FIELD IF NOT EXISTS finished_at ON job TYPE option<datetime>;
-- Dashboard metadata (jobs_view parity with argos_jobs); '' when unknown.
DEFINE FIELD IF NOT EXISTS app ON job TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS title ON job TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS provider ON job TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS model ON job TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS tool ON job TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS run_ref ON job TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS resource_ref ON job TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS result_ref ON job TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS error_category ON job TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS error_summary ON job TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS correlation_id ON job TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS stage_coverage_json ON job TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS worker_owner ON job TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS progress_done ON job TYPE option<int>;
DEFINE FIELD IF NOT EXISTS progress_total ON job TYPE option<int>;
DEFINE FIELD IF NOT EXISTS heartbeat_at ON job TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS active_since ON job TYPE option<datetime>;
-- Durable timing: NONE is "Unavailable", never zero-filled.
DEFINE FIELD IF NOT EXISTS active_ms ON job TYPE option<int>;
DEFINE FIELD IF NOT EXISTS queue_ms ON job TYPE option<int>;
DEFINE FIELD IF NOT EXISTS retry_wait_ms ON job TYPE option<int>;
DEFINE FIELD IF NOT EXISTS attempts_used ON job TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS attempt_cap ON job TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS cancellable ON job TYPE bool DEFAULT false;
DEFINE INDEX IF NOT EXISTS job_app ON job FIELDS app;
DEFINE INDEX IF NOT EXISTS job_root ON job FIELDS root, stopped;
DEFINE INDEX IF NOT EXISTS job_parent ON job FIELDS parent;
DEFINE INDEX IF NOT EXISTS job_state_created ON job FIELDS state, created_at;

-- One row per (ancestor, descendant), written with the job. The array
-- id keeps it unique; the ancestor index serves stop-batch by equality.
DEFINE TABLE IF NOT EXISTS job_ancestor SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS ancestor ON job_ancestor TYPE record<job>;
DEFINE FIELD IF NOT EXISTS job ON job_ancestor TYPE record<job>;
DEFINE FIELD IF NOT EXISTS root ON job_ancestor TYPE record<job>;
DEFINE INDEX IF NOT EXISTS job_ancestor_ancestor ON job_ancestor FIELDS ancestor;
DEFINE INDEX IF NOT EXISTS job_ancestor_root ON job_ancestor FIELDS root;
DEFINE INDEX IF NOT EXISTS job_ancestor_job ON job_ancestor FIELDS job;
-- Deleting a job by any path drops its links in the same transaction.
DEFINE EVENT IF NOT EXISTS job_ancestor_cleanup ON job WHEN $event = "DELETE" THEN {
    DELETE job_ancestor WHERE job = $before.id RETURN NONE;
    DELETE job_ancestor WHERE ancestor = $before.id RETURN NONE;
};

DEFINE TABLE IF NOT EXISTS task SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS job ON task TYPE record<job>;
DEFINE FIELD IF NOT EXISTS root ON task TYPE record<job>;
DEFINE FIELD IF NOT EXISTS pool ON task TYPE string;
DEFINE FIELD IF NOT EXISTS operation ON task TYPE string;
-- queued | running | completed | failed | cancelled. A scheduled retry is
-- `queued` with a later next_eligible_at, so claimable rows share one
-- equality prefix of task_claim_order.
DEFINE FIELD IF NOT EXISTS state ON task TYPE string;
DEFINE FIELD IF NOT EXISTS priority ON task TYPE int DEFAULT 100;
DEFINE FIELD IF NOT EXISTS deps ON task TYPE array<record<task>> DEFAULT [];
DEFINE FIELD IF NOT EXISTS input_hash ON task TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS output_hash ON task TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS lease_owner ON task TYPE string DEFAULT '';
-- The process whose worker holds the lease (scheduler::process_owner()).
-- lease_owner names the worker (w0..w7); registry recovery matches on this.
DEFINE FIELD IF NOT EXISTS process_owner ON task TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS lease_until ON task TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS lease_epoch ON task TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS attempts ON task TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS recoveries ON task TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS created_at ON task TYPE datetime READONLY;
DEFINE FIELD IF NOT EXISTS next_eligible_at ON task TYPE datetime;
DEFINE FIELD IF NOT EXISTS started_at ON task TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS heartbeat_at ON task TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS finished_at ON task TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS updated_at ON task TYPE datetime;
DEFINE FIELD IF NOT EXISTS max_attempts ON task TYPE int DEFAULT 3;
DEFINE FIELD IF NOT EXISTS active_ms ON task TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS queue_ms ON task TYPE option<int>;
DEFINE FIELD IF NOT EXISTS retry_wait_ms ON task TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS error_category ON task TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS error_message ON task TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS result_ref ON task TYPE string DEFAULT '';
-- Replaces the legacy store's partial index `argos_tasks_queued_order`.
DEFINE INDEX IF NOT EXISTS task_claim_order ON task FIELDS state, pool, priority, created_at;
DEFINE INDEX IF NOT EXISTS task_lease ON task FIELDS state, lease_until;
DEFINE INDEX IF NOT EXISTS task_job ON task FIELDS job, state;
DEFINE INDEX IF NOT EXISTS task_root ON task FIELDS root, state;

DEFINE TABLE IF NOT EXISTS attempt SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS task ON attempt TYPE record<task>;
DEFINE FIELD IF NOT EXISTS root ON attempt TYPE record<job>;
DEFINE FIELD IF NOT EXISTS number ON attempt TYPE int;
DEFINE FIELD IF NOT EXISTS epoch ON attempt TYPE int;
DEFINE FIELD IF NOT EXISTS owner ON attempt TYPE string;
DEFINE FIELD IF NOT EXISTS started_at ON attempt TYPE datetime;
DEFINE FIELD IF NOT EXISTS finished_at ON attempt TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS outcome ON attempt TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS duration_ms ON attempt TYPE option<int>;
DEFINE FIELD IF NOT EXISTS error_category ON attempt TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS error_message ON attempt TYPE string DEFAULT '';
DEFINE INDEX IF NOT EXISTS attempt_task ON attempt FIELDS task, number UNIQUE;
DEFINE INDEX IF NOT EXISTS attempt_root ON attempt FIELDS root;

DEFINE TABLE IF NOT EXISTS stage_output SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS task ON stage_output TYPE record<task>;
DEFINE FIELD IF NOT EXISTS root ON stage_output TYPE record<job>;
DEFINE FIELD IF NOT EXISTS stage ON stage_output TYPE string;
DEFINE FIELD IF NOT EXISTS output_hash ON stage_output TYPE string;
DEFINE FIELD IF NOT EXISTS output_json ON stage_output TYPE string;
DEFINE FIELD IF NOT EXISTS committed_at ON stage_output TYPE datetime;
DEFINE INDEX IF NOT EXISTS stage_output_root ON stage_output FIELDS root;

DEFINE TABLE IF NOT EXISTS brief_run SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS article_id ON brief_run TYPE string;
DEFINE FIELD IF NOT EXISTS generation ON brief_run TYPE int;
DEFINE FIELD IF NOT EXISTS job ON brief_run TYPE record<job>;
DEFINE FIELD IF NOT EXISTS input_revision ON brief_run TYPE string;
DEFINE FIELD IF NOT EXISTS state ON brief_run TYPE string;
DEFINE FIELD IF NOT EXISTS updated_at ON brief_run TYPE datetime;
DEFINE INDEX IF NOT EXISTS brief_run_generation ON brief_run FIELDS article_id, generation UNIQUE;

DEFINE TABLE IF NOT EXISTS neighborhood_brief SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS task ON neighborhood_brief TYPE record<task>;
DEFINE FIELD IF NOT EXISTS body ON neighborhood_brief TYPE string;
DEFINE FIELD IF NOT EXISTS route ON neighborhood_brief TYPE string;
DEFINE FIELD IF NOT EXISTS generated_at ON neighborhood_brief TYPE datetime;

DEFINE TABLE IF NOT EXISTS job_retirement SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS retired_at ON job_retirement TYPE datetime;
DEFINE FIELD IF NOT EXISTS reason ON job_retirement TYPE string;

-- Guard records: written first by every transaction that must serialize
-- against another one, so concurrent writers conflict at commit.
DEFINE TABLE IF NOT EXISTS pool_guard SCHEMALESS;
DEFINE TABLE IF NOT EXISTS job_guard SCHEMALESS;
DEFINE TABLE IF NOT EXISTS lease_sweep_guard SCHEMALESS;
DEFINE TABLE IF NOT EXISTS cleanup_guard SCHEMALESS;
DEFINE TABLE IF NOT EXISTS brief_chain_guard SCHEMALESS;
DEFINE TABLE IF NOT EXISTS scheduler_lease SCHEMALESS;
-- Stable operation receipts for claims and stage commits (unknown commit).
DEFINE TABLE IF NOT EXISTS tx_receipt SCHEMALESS;

-- Durable job events (argos_events subset for the Jobs dashboard).
DEFINE TABLE IF NOT EXISTS job_event SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS job_id ON job_event TYPE string;
DEFINE FIELD IF NOT EXISTS app ON job_event TYPE string;
DEFINE FIELD IF NOT EXISTS event_type ON job_event TYPE string;
DEFINE FIELD IF NOT EXISTS severity ON job_event TYPE string DEFAULT 'info';
DEFINE FIELD IF NOT EXISTS message ON job_event TYPE string;
DEFINE FIELD IF NOT EXISTS ts ON job_event TYPE datetime;
DEFINE INDEX IF NOT EXISTS job_event_job ON job_event FIELDS job_id;
