//! Serializable requests shared by the Jobs service and its private clients.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One idempotent operation. Timestamps are Unix milliseconds.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Request {
    pub operation_key: String,
    pub now_ms: i64,
    pub command: Command,
}

impl Request {
    /// Constructs an operation with a stable retry key.
    pub fn new(operation_key: impl Into<String>, now_ms: i64, command: Command) -> Self {
        Self {
            operation_key: operation_key.into(),
            now_ms,
            command,
        }
    }
}

/// New operational job; metadata holds dashboard-specific labels and references.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NewJob {
    pub id: String,
    pub parent_id: Option<String>,
    pub operation: String,
    #[serde(default)]
    pub metadata: Value,
}

/// One task in a submitted job. Dependencies must belong to the same job.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NewTask {
    pub id: String,
    pub operation: String,
    pub pool: String,
    #[serde(default)]
    pub input: Value,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub priority: i64,
    #[serde(default)]
    pub next_eligible_at: i64,
    #[serde(default = "default_attempt_cap")]
    pub attempt_cap: i64,
}

fn default_attempt_cap() -> i64 {
    3
}

impl Default for NewTask {
    fn default() -> Self {
        Self {
            id: String::new(),
            operation: String::new(),
            pool: String::new(),
            input: Value::Null,
            dependencies: Vec::new(),
            priority: 0,
            next_eligible_at: 0,
            attempt_cap: default_attempt_cap(),
        }
    }
}

/// Lease identity. Expiry is checked against persisted state, never this token.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lease {
    pub task_id: String,
    pub owner: String,
    pub epoch: i64,
}

/// One job's persisted operational state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Job {
    pub id: String,
    pub parent_id: Option<String>,
    pub operation: String,
    pub metadata: Value,
    pub state: String,
    pub cancel_requested: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub finished_at: Option<i64>,
    pub progress_done: Option<i64>,
    pub progress_total: Option<i64>,
    pub error: String,
}

/// One persisted task and its current lease.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Task {
    pub id: String,
    pub job_id: String,
    pub operation: String,
    pub pool: String,
    pub input: Value,
    pub output: Option<Value>,
    pub state: String,
    pub priority: i64,
    pub next_eligible_at: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub owner: Option<String>,
    pub epoch: i64,
    pub lease_until: Option<i64>,
    pub attempts_used: i64,
    pub attempt_cap: i64,
    pub error: String,
}

impl Task {
    /// Returns a lease token only for a currently claimed task.
    pub fn lease(&self) -> Option<Lease> {
        (self.state == "running").then(|| Lease {
            task_id: self.id.clone(),
            owner: self.owner.clone().unwrap_or_default(),
            epoch: self.epoch,
        })
    }
}

/// Persisted history of one lease attempt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Attempt {
    pub task_id: String,
    pub epoch: i64,
    pub owner: String,
    pub state: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub error: String,
}

/// Ordered operational event or log record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Event {
    pub seq: i64,
    pub job_id: Option<String>,
    pub task_id: Option<String>,
    pub kind: String,
    pub body: Value,
    pub created_at: i64,
}

/// Pending delivery/publication intent committed together with job state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct OutboxEntry {
    pub seq: i64,
    pub job_id: String,
    pub task_id: String,
    pub kind: String,
    pub body: Value,
    pub created_at: i64,
}

/// Private service operations. No raw SQL is accepted over the service boundary.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Command {
    Submit {
        job: NewJob,
        tasks: Vec<NewTask>,
    },
    Claim {
        pool: String,
        owner: String,
        lease_ms: i64,
        task_id: Option<String>,
    },
    Renew {
        lease: Lease,
        lease_ms: i64,
    },
    Complete {
        lease: Lease,
        output: Value,
    },
    Fail {
        lease: Lease,
        error: String,
        retry_at: Option<i64>,
    },
    Cancel {
        job_id: String,
    },
    Retry {
        job_id: String,
    },
    Reap,
    Progress {
        job_id: String,
        state: String,
        done: Option<i64>,
        total: Option<i64>,
        error: String,
    },
    AppendEvent {
        job_id: Option<String>,
        task_id: Option<String>,
        kind: String,
        body: Value,
    },
    Get {
        job_id: String,
    },
    List {
        state: Option<String>,
        limit: usize,
    },
    Tasks {
        job_id: String,
    },
    Attempts {
        job_id: String,
    },
    Events {
        job_id: Option<String>,
        after: i64,
        limit: usize,
    },
    Outbox {
        kind: Option<String>,
        limit: usize,
    },
    Acknowledge {
        seq: i64,
    },
    Prune {
        before: i64,
    },
}

impl Command {
    pub(crate) fn is_read(&self) -> bool {
        matches!(
            self,
            Self::Get { .. }
                | Self::List { .. }
                | Self::Tasks { .. }
                | Self::Attempts { .. }
                | Self::Events { .. }
                | Self::Outbox { .. }
        )
    }
}

/// Result of a private Jobs operation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Response {
    Job(Option<Job>),
    Jobs(Vec<Job>),
    Task(Option<Task>),
    Tasks(Vec<Task>),
    Attempts(Vec<Attempt>),
    Events(Vec<Event>),
    Outbox(Vec<OutboxEntry>),
    Changed(bool),
    Count(usize),
    Event(i64),
}
