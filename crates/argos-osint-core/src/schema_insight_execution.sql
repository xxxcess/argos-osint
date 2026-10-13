-- Additive execution metadata; immutable inputs and completed checkpoints stay intact.
CREATE TABLE IF NOT EXISTS insight_refresh_execution (
    task_id TEXT PRIMARY KEY REFERENCES argos_tasks(id) ON DELETE CASCADE,
    retry_epoch INTEGER NOT NULL DEFAULT 0,
    stage TEXT NOT NULL DEFAULT 'queued',
    windows_total INTEGER NOT NULL DEFAULT 0,
    withheld_count INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL DEFAULT ''
);
