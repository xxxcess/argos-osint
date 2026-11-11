//! Private, typed delivery queue protocol.

use serde::{Deserialize, Serialize};

/// One queue operation. The service supplies `now_ms`, never a remote client.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct QueueRequest {
    pub operation_key: String,
    pub now_ms: i64,
    pub command: QueueCommand,
}

impl QueueRequest {
    /// Builds an operation with a stable retry key.
    pub fn new(operation_key: impl Into<String>, now_ms: i64, command: QueueCommand) -> Self {
        Self {
            operation_key: operation_key.into(),
            now_ms,
            command,
        }
    }
}

/// Fences delivery settlement against visibility expiry and reassignment.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueToken {
    pub queue: String,
    pub id: String,
    pub owner: String,
    pub epoch: u64,
}

/// One leased envelope; payload contains identifiers, never large domain data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Delivery {
    pub token: QueueToken,
    pub payload: Vec<u8>,
    pub published_at: i64,
    pub visible_until: i64,
    pub delivery_count: u64,
}

/// Queue mutations and bounded dead-letter inspection.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum QueueCommand {
    Publish {
        queue: String,
        id: String,
        payload: Vec<u8>,
    },
    Claim {
        queue: String,
        owner: String,
        visibility_ms: i64,
    },
    Renew {
        token: QueueToken,
        visibility_ms: i64,
    },
    Ack {
        token: QueueToken,
    },
    Requeue {
        token: QueueToken,
        delay_ms: i64,
    },
    DeadLetter {
        token: QueueToken,
    },
    DeadLetters {
        queue: String,
        limit: usize,
    },
}

/// Queue operation result. `Changed(false)` indicates an expired/stale token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
#[non_exhaustive]
pub enum QueueResponse {
    Published(bool),
    Delivery(Option<Delivery>),
    Changed(bool),
    DeadLetters(Vec<Delivery>),
}
