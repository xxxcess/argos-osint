CREATE TABLE jobs (
    id TEXT PRIMARY KEY,
    parent_id TEXT REFERENCES jobs(id),
    operation TEXT NOT NULL,
    metadata TEXT NOT NULL CHECK(json_valid(metadata)),
    state TEXT NOT NULL DEFAULT 'queued',
    cancel_requested INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    finished_at INTEGER,
    progress_done INTEGER,
    progress_total INTEGER,
    error TEXT NOT NULL DEFAULT ''
);
CREATE INDEX jobs_created ON jobs(created_at DESC, id);
CREATE TABLE tasks (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    operation TEXT NOT NULL,
    pool TEXT NOT NULL,
    input TEXT NOT NULL CHECK(json_valid(input)),
    output TEXT CHECK(output IS NULL OR json_valid(output)),
    state TEXT NOT NULL DEFAULT 'queued',
    priority INTEGER NOT NULL,
    next_eligible_at INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    owner TEXT,
    epoch INTEGER NOT NULL DEFAULT 0,
    lease_until INTEGER,
    attempts_used INTEGER NOT NULL DEFAULT 0,
    attempt_cap INTEGER NOT NULL CHECK(attempt_cap > 0),
    error TEXT NOT NULL DEFAULT ''
);
CREATE INDEX tasks_claim ON tasks(pool, state, next_eligible_at, priority, created_at);
CREATE INDEX tasks_job ON tasks(job_id);
CREATE INDEX tasks_expired ON tasks(state, lease_until);
CREATE TABLE dependencies (
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    dependency_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    PRIMARY KEY(task_id, dependency_id),
    CHECK(task_id != dependency_id)
);
CREATE TABLE attempts (
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    epoch INTEGER NOT NULL,
    owner TEXT NOT NULL,
    state TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    finished_at INTEGER,
    error TEXT NOT NULL DEFAULT '',
    PRIMARY KEY(task_id, epoch)
);
CREATE TABLE events (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id TEXT REFERENCES jobs(id) ON DELETE CASCADE,
    task_id TEXT REFERENCES tasks(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    body TEXT NOT NULL CHECK(json_valid(body)),
    created_at INTEGER NOT NULL
);
CREATE INDEX events_job ON events(job_id, seq);
CREATE INDEX events_created ON events(created_at);
CREATE TABLE receipts (
    operation_key TEXT PRIMARY KEY,
    job_id TEXT REFERENCES jobs(id) ON DELETE CASCADE,
    request TEXT NOT NULL,
    response TEXT NOT NULL CHECK(json_valid(response))
);
CREATE TABLE outbox (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    body TEXT NOT NULL CHECK(json_valid(body)),
    created_at INTEGER NOT NULL,
    acknowledged_at INTEGER
);
CREATE INDEX outbox_pending ON outbox(acknowledged_at, seq);

-- Standalone operational logs may refer to domain jobs not owned by SQLite yet.
CREATE TABLE logs (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    body TEXT NOT NULL CHECK(json_valid(body)),
    ts TEXT NOT NULL
);
CREATE INDEX logs_time ON logs(ts);
CREATE INDEX logs_job ON logs(json_extract(body, '$.job_id'));
