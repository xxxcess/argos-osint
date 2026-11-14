//! Single-owner RocksDB delivery queue. SQLite remains the job source of truth.
//!
//! Each accepted mutation and its replay receipt share one synchronous WAL
//! batch. Queue envelopes are limited to 16 KiB. The Jobs service alone opens
//! the directory; all other processes use the authenticated queue endpoint.

use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, bail, ensure, Context, Result};
use rocksdb::{
    BlockBasedOptions, Cache, DBCompressionType, Direction, IteratorMode, Options, WriteBatch,
    WriteOptions, DB,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

pub use crate::queue_types::{Delivery, QueueCommand, QueueRequest, QueueResponse, QueueToken};

/// Maximum serialized delivery envelope; domain bodies belong in SurrealDB.
pub const MAX_PAYLOAD_BYTES: usize = 16 * 1024;
const MAX_NAME_BYTES: usize = 256;
const MAX_DURATION_MS: i64 = 24 * 60 * 60 * 1000;
const SCHEMA_KEY: &[u8] = b"version";

struct Inner {
    db: Mutex<DB>,
    permit: Arc<Semaphore>,
}

/// Cloneable handle to the Jobs process's embedded delivery queue.
#[derive(Clone)]
pub struct Queue(Arc<Inner>);

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Message {
    queue: String,
    id: String,
    payload: Vec<u8>,
    published_at: i64,
    state: State,
    owner: String,
    epoch: u64,
    visible_at: i64,
    delivery_count: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum State {
    Ready,
    Inflight,
    Acknowledged,
    Dead,
}

#[derive(Serialize, Deserialize)]
struct Receipt {
    command: String,
    response: QueueResponse,
}

impl Queue {
    /// Opens a dedicated disk directory with a 4 MiB block cache, two 4 MiB
    /// memtables, at most 32 open files and two background jobs. These are
    /// engine budgets, not a hard upper bound on all native allocations.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let db = tokio::task::spawn_blocking(move || -> Result<DB> {
            ensure!(
                !path.as_os_str().is_empty(),
                "queue requires a disk directory"
            );
            std::fs::create_dir_all(&path).context("create delivery queue directory")?;
            let mut options = Options::default();
            options.create_if_missing(true);
            options.set_write_buffer_size(4 * 1024 * 1024);
            options.set_max_write_buffer_number(2);
            options.set_max_total_wal_size(16 * 1024 * 1024);
            options.set_max_background_jobs(2);
            options.set_max_open_files(32);
            options.set_compression_type(DBCompressionType::None);
            let mut blocks = BlockBasedOptions::default();
            blocks.set_block_cache(&Cache::new_lru_cache(4 * 1024 * 1024));
            blocks.set_cache_index_and_filter_blocks(true);
            options.set_block_based_table_factory(&blocks);
            let db = DB::open(&options, path).context("open delivery queue RocksDB")?;
            if let Some(version) = db.get(SCHEMA_KEY)? {
                ensure!(version == b"1", "unsupported delivery queue schema");
            } else {
                let mut batch = WriteBatch::default();
                batch.put(SCHEMA_KEY, b"1");
                commit(&db, batch)?;
            }
            Ok(db)
        })
        .await
        .context("open delivery queue worker")??;
        Ok(Self(Arc::new(Inner {
            db: Mutex::new(db),
            permit: Arc::new(Semaphore::new(1)),
        })))
    }

    /// Executes an operation using the service's authoritative clock. Retries
    /// reuse the original key and command; their service timestamps may differ.
    pub async fn execute(&self, request: QueueRequest) -> Result<QueueResponse> {
        let permit = self.0.permit.clone().acquire_owned().await?;
        let inner = self.0.clone();
        tokio::task::spawn_blocking(move || -> Result<QueueResponse> {
            let _permit = permit;
            let db = inner
                .db
                .lock()
                .map_err(|_| anyhow!("delivery queue mutex poisoned"))?;
            if let QueueCommand::DeadLetters { queue, limit } = &request.command {
                validate_name(queue)?;
                return deadletters(&db, queue, *limit);
            }
            ensure!(
                !request.operation_key.is_empty() && request.operation_key.len() <= 512,
                "queue mutation requires an operation key of at most 512 bytes"
            );
            let serialized = serde_json::to_string(&request.command)?;
            let receipt_key = receipt_key(&request.operation_key);
            if let Some(bytes) = db.get(&receipt_key)? {
                let receipt: Receipt =
                    serde_json::from_slice(&bytes).context("decode queue receipt")?;
                ensure!(
                    receipt.command == serialized,
                    "queue operation key reused with a different command"
                );
                return Ok(receipt.response);
            }
            let mut batch = WriteBatch::default();
            let response = execute(&db, &mut batch, &request.command, request.now_ms)?;
            if matches!(request.command, QueueCommand::Claim { .. })
                && matches!(response, QueueResponse::Delivery(None))
            {
                return Ok(response);
            }
            let receipt = Receipt {
                command: serialized,
                response: response.clone(),
            };
            batch.put(receipt_key, serde_json::to_vec(&receipt)?);
            commit(&db, batch)?;
            Ok(response)
        })
        .await
        .context("delivery queue database worker")?
    }
}

fn commit(db: &DB, batch: WriteBatch) -> Result<()> {
    let mut options = WriteOptions::default();
    options.set_sync(true);
    options.disable_wal(false);
    db.write_opt(batch, &options)
        .context("commit delivery queue WAL")
}

fn validate_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty() && name.len() <= MAX_NAME_BYTES,
        "queue, id and owner must contain 1..256 bytes"
    );
    Ok(())
}

fn deadline(now: i64, duration: i64, allow_zero: bool) -> Result<i64> {
    ensure!(
        duration >= i64::from(!allow_zero) && duration <= MAX_DURATION_MS,
        "invalid queue visibility/delay duration"
    );
    now.checked_add(duration)
        .ok_or_else(|| anyhow!("queue deadline overflow"))
}

fn prefix(kind: u8, queue: &str) -> Vec<u8> {
    let mut key = vec![kind];
    key.extend_from_slice(&(queue.len() as u16).to_be_bytes());
    key.extend_from_slice(queue.as_bytes());
    key
}

fn message_key(queue: &str, id: &str) -> Vec<u8> {
    let mut key = prefix(b'M', queue);
    key.extend_from_slice(id.as_bytes());
    key
}

fn receipt_key(operation: &str) -> Vec<u8> {
    let mut key = vec![b'C'];
    key.extend_from_slice(operation.as_bytes());
    key
}

fn index_kind(state: State) -> Option<u8> {
    match state {
        State::Ready => Some(b'R'),
        State::Inflight => Some(b'I'),
        State::Dead => Some(b'D'),
        State::Acknowledged => None,
    }
}

fn index_key(message: &Message, kind: u8) -> Vec<u8> {
    let mut key = prefix(kind, &message.queue);
    key.extend_from_slice(&((message.visible_at as u64) ^ (1_u64 << 63)).to_be_bytes());
    key.extend_from_slice(message.id.as_bytes());
    key
}

fn get_message(db: &DB, queue: &str, id: &str) -> Result<Option<Message>> {
    db.get(message_key(queue, id))?
        .map(|bytes| serde_json::from_slice(&bytes).context("decode queue message"))
        .transpose()
}

fn save(batch: &mut WriteBatch, previous: Option<&Message>, message: &Message) -> Result<()> {
    if let Some(previous) = previous {
        if let Some(kind) = index_kind(previous.state) {
            batch.delete(index_key(previous, kind));
        }
    }
    let key = message_key(&message.queue, &message.id);
    if let Some(kind) = index_kind(message.state) {
        batch.put(index_key(message, kind), &key);
    }
    batch.put(key, serde_json::to_vec(message)?);
    Ok(())
}

fn first(db: &DB, queue: &str, kind: u8) -> Result<Option<Message>> {
    let prefix = prefix(kind, queue);
    if let Some(entry) = db
        .iterator(IteratorMode::From(&prefix, Direction::Forward))
        .next()
    {
        let (key, value) = entry?;
        if !key.starts_with(&prefix) {
            return Ok(None);
        }
        let bytes = db
            .get(value)?
            .ok_or_else(|| anyhow!("queue index references missing envelope"))?;
        return Ok(Some(serde_json::from_slice(&bytes)?));
    }
    Ok(None)
}

fn delivery(message: &Message) -> Delivery {
    Delivery {
        token: QueueToken {
            queue: message.queue.clone(),
            id: message.id.clone(),
            owner: message.owner.clone(),
            epoch: message.epoch,
        },
        payload: message.payload.clone(),
        published_at: message.published_at,
        visible_until: message.visible_at,
        delivery_count: message.delivery_count,
    }
}

fn leased(db: &DB, token: &QueueToken, now: i64) -> Result<Option<Message>> {
    validate_name(&token.queue)?;
    validate_name(&token.id)?;
    validate_name(&token.owner)?;
    Ok(get_message(db, &token.queue, &token.id)?.filter(|message| {
        message.state == State::Inflight
            && message.owner == token.owner
            && message.epoch == token.epoch
            && message.visible_at > now
    }))
}

fn execute(
    db: &DB,
    batch: &mut WriteBatch,
    command: &QueueCommand,
    now: i64,
) -> Result<QueueResponse> {
    let response = match command {
        QueueCommand::Publish { queue, id, payload } => {
            validate_name(queue)?;
            validate_name(id)?;
            ensure!(
                payload.len() <= MAX_PAYLOAD_BYTES,
                "queue payload exceeds 16 KiB"
            );
            if let Some(previous) = get_message(db, queue, id)? {
                ensure!(
                    &previous.payload == payload,
                    "queue message id already has a different payload"
                );
                QueueResponse::Published(false)
            } else {
                let message = Message {
                    queue: queue.clone(),
                    id: id.clone(),
                    payload: payload.clone(),
                    published_at: now,
                    state: State::Ready,
                    owner: String::new(),
                    epoch: 0,
                    visible_at: now,
                    delivery_count: 0,
                };
                save(batch, None, &message)?;
                QueueResponse::Published(true)
            }
        }
        QueueCommand::Claim {
            queue,
            owner,
            visibility_ms,
        } => {
            validate_name(queue)?;
            validate_name(owner)?;
            let until = deadline(now, *visibility_ms, false)?;
            let ready = first(db, queue, b'R')?.filter(|message| message.visible_at <= now);
            let expired = first(db, queue, b'I')?.filter(|message| message.visible_at <= now);
            let candidate = match (ready, expired) {
                (Some(ready), Some(expired)) => Some(if ready.visible_at <= expired.visible_at {
                    ready
                } else {
                    expired
                }),
                (Some(message), None) | (None, Some(message)) => Some(message),
                (None, None) => None,
            };
            if let Some(previous) = candidate {
                let mut message = previous.clone();
                message.state = State::Inflight;
                message.owner = owner.clone();
                message.epoch = message
                    .epoch
                    .checked_add(1)
                    .ok_or_else(|| anyhow!("queue epoch overflow"))?;
                message.delivery_count = message
                    .delivery_count
                    .checked_add(1)
                    .ok_or_else(|| anyhow!("delivery count overflow"))?;
                message.visible_at = until;
                save(batch, Some(&previous), &message)?;
                QueueResponse::Delivery(Some(delivery(&message)))
            } else {
                QueueResponse::Delivery(None)
            }
        }
        QueueCommand::Renew {
            token,
            visibility_ms,
        } => {
            let until = deadline(now, *visibility_ms, false)?;
            if let Some(previous) = leased(db, token, now)? {
                let mut message = previous.clone();
                message.visible_at = until.max(previous.visible_at);
                save(batch, Some(&previous), &message)?;
                QueueResponse::Changed(true)
            } else {
                QueueResponse::Changed(false)
            }
        }
        QueueCommand::Ack { token }
        | QueueCommand::Requeue { token, .. }
        | QueueCommand::DeadLetter { token } => {
            let requeue_at = if let QueueCommand::Requeue { delay_ms, .. } = command {
                Some(deadline(now, *delay_ms, true)?)
            } else {
                None
            };
            if let Some(previous) = leased(db, token, now)? {
                let mut message = previous.clone();
                message.state = match command {
                    QueueCommand::Ack { .. } => State::Acknowledged,
                    QueueCommand::Requeue { .. } => State::Ready,
                    _ => State::Dead,
                };
                message.visible_at = requeue_at.unwrap_or(now);
                message.owner.clear();
                save(batch, Some(&previous), &message)?;
                QueueResponse::Changed(true)
            } else {
                QueueResponse::Changed(false)
            }
        }
        QueueCommand::DeadLetters { .. } => bail!("dead-letter inspection is read-only"),
    };
    Ok(response)
}

fn deadletters(db: &DB, queue: &str, limit: usize) -> Result<QueueResponse> {
    let prefix = prefix(b'D', queue);
    let mut messages = Vec::new();
    for entry in db
        .iterator(IteratorMode::From(&prefix, Direction::Forward))
        .take(limit.min(100))
    {
        let (key, value) = entry?;
        if !key.starts_with(&prefix) {
            break;
        }
        let bytes = db
            .get(value)?
            .ok_or_else(|| anyhow!("dead-letter index references missing envelope"))?;
        let message: Message = serde_json::from_slice(&bytes)?;
        messages.push(delivery(&message));
    }
    Ok(QueueResponse::DeadLetters(messages))
}
