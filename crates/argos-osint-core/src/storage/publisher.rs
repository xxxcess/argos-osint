//! Group-commit publisher for memory writes.
//!
//! Every memory write allocates outbox seqs from `index_change` and bumps
//! `memories_changed`. Both are single records and the outbox seq must stay
//! gap-free and commit in order, so concurrent writers would serialize on
//! conflicts and backoff. Instead each process runs one [`Publisher`]:
//! writers submit their write and await a oneshot, and the publisher drains
//! up to [`BATCH_CAP`] queued writes into one [`run_tx`] transaction that
//! applies them in order, enqueues their outbox changes, and bumps
//! `memories_changed` once. Cross-process conflicts still go through the
//! typed retry in `run_tx`.

use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use chrono::Utc;
use futures_util::future::BoxFuture;
use futures_util::FutureExt;
use surrealdb::engine::any::Any;
use surrealdb::Surreal;
use tokio::sync::{mpsc, oneshot, Mutex, Notify};
use tokio::task::JoinHandle;

use super::counters;
use super::outbox::{self, Enqueued, NewChange};
use super::tx::{run_tx, Committed, Tx, TxError};

/// Counter bumped once per committed batch.
pub const CHANGED_COUNTER: &str = "memories_changed";
/// Most writes per transaction.
pub const BATCH_CAP: usize = 32;
/// Queued writes before `publish` waits for room (backpressure).
pub const QUEUE_CAP: usize = BATCH_CAP * 8;

/// Outbox work a write queues, owned so it can outlive the write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub record_kind: String,
    pub record_id: String,
    pub revision: String,
    pub operation: String,
    pub generation: String,
    pub content_hash: String,
    pub op_key: String,
}

/// One memory write: database statements run inside the batch transaction,
/// returning the outbox changes to enqueue. It may run more than once
/// (retries, a split), so it must be pure database work.
pub type Write = Arc<dyn Fn(Tx) -> BoxFuture<'static, Result<Vec<Change>, TxError>> + Send + Sync>;

/// What a committed write got.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Published {
    /// Outbox rows for this write's changes, in order.
    pub enqueued: Vec<Enqueued>,
    /// `memories_changed` after the batch that carried it.
    pub changed_seq: i64,
    /// Writes in that batch (1 after a split).
    pub batch_size: usize,
}

type Reply = oneshot::Sender<Result<Committed<Published>, TxError>>;

/// Handle to a process's publisher task. Cloning shares the task.
#[derive(Clone)]
pub struct Publisher {
    queue: mpsc::Sender<(Write, Reply)>,
    stop: Arc<Notify>,
    task: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl Publisher {
    /// Start the publisher on `db`. Run one per process; a second one works
    /// but its batches conflict with the first's (resolved by `run_tx`).
    pub fn spawn(db: Surreal<Any>) -> Publisher {
        let (queue, rx) = mpsc::channel::<(Write, Reply)>(QUEUE_CAP);
        let stop = Arc::new(Notify::new());
        let task = tokio::spawn(run(db, rx, stop.clone()));
        Publisher {
            queue,
            stop,
            task: Arc::new(Mutex::new(Some(task))),
        }
    }

    /// Submit `write` and wait for its batch to commit. Waits for queue room
    /// when [`QUEUE_CAP`] writes are already pending.
    pub async fn publish(&self, write: Write) -> Result<Committed<Published>, TxError> {
        let (tx, rx) = oneshot::channel();
        self.queue
            .send((write, tx))
            .await
            .map_err(|_| TxError::PublisherUnavailable { reason: "stopped" })?;
        rx.await.map_err(|_| TxError::PublisherUnavailable {
            reason: "dropped the write",
        })?
    }

    /// Stop accepting writes, publish everything already queued, and wait
    /// for the task to finish. Later `publish` calls fail with
    /// [`TxError::PublisherUnavailable`]. Safe to call more than once.
    pub async fn shutdown(&self) {
        self.stop.notify_one();
        let task = self.task.lock().await.take();
        if let Some(task) = task {
            if let Err(e) = task.await {
                tracing::warn!(error = %e, "memory publisher task ended abnormally");
            }
        }
    }
}

async fn run(db: Surreal<Any>, mut rx: mpsc::Receiver<(Write, Reply)>, stop: Arc<Notify>) {
    let mut closing = false;
    loop {
        let first = tokio::select! {
            biased;
            _ = stop.notified(), if !closing => {
                // Buffered writes are still delivered, then `recv` ends.
                rx.close();
                closing = true;
                continue;
            }
            next = rx.recv() => next,
        };
        let Some(first) = first else { break };
        let mut batch = vec![first];
        while batch.len() < BATCH_CAP {
            match rx.try_recv() {
                Ok(next) => batch.push(next),
                Err(_) => break,
            }
        }
        let (writes, replies): (Vec<_>, Vec<_>) = batch.into_iter().unzip();
        // Per-write panics are caught in `commit`; this backstop keeps the
        // loop alive if anything else in the batch path panics.
        let results = match AssertUnwindSafe(publish_batch(&db, &writes))
            .catch_unwind()
            .await
        {
            Ok(results) => results,
            Err(panic) => {
                let why = panic_message(&*panic);
                tracing::warn!(batch_size = writes.len(), panic = %why, "memory publish batch panicked");
                writes
                    .iter()
                    .map(|_| Err(TxError::Abort(format!("publish batch panicked: {why}"))))
                    .collect()
            }
        };
        for (reply, result) in replies.into_iter().zip(results) {
            // A dropped waiter still had its write committed.
            let _ = reply.send(result);
        }
    }
}

/// Commit `writes` as one transaction; one result per write, in order.
/// A conflict retries the whole batch (`run_tx`). A write-specific failure
/// (an abort, a panicking write, a validation-class database error) splits
/// the batch so each write commits or fails on its own. Anything else
/// (exhausted retries, timeouts, a lost connection, a commit with an
/// unknown outcome) fails the whole batch: retrying writes one by one
/// would only repeat it.
pub async fn publish_batch(
    db: &Surreal<Any>,
    writes: &[Write],
) -> Vec<Result<Committed<Published>, TxError>> {
    match commit(db, writes).await {
        Ok(done) => done.into_iter().map(Ok).collect(),
        Err(e) if writes.len() > 1 && splittable(&e) => {
            tracing::warn!(batch_size = writes.len(), error = %e, "memory publish batch failed; splitting");
            let mut out = Vec::with_capacity(writes.len());
            for w in writes {
                let r = commit(db, std::slice::from_ref(w))
                    .await
                    .map(|mut v| v.remove(0));
                if let Err(e) = &r {
                    tracing::warn!(batch_size = 1, error = %e, "memory write failed");
                }
                out.push(r);
            }
            out
        }
        Err(e) => {
            tracing::warn!(batch_size = writes.len(), error = %e, "memory publish batch failed");
            writes.iter().map(|_| Err(e.clone())).collect()
        }
    }
}

/// True for failures that belong to one write rather than the batch.
fn splittable(e: &TxError) -> bool {
    match e {
        TxError::Abort(_) => true,
        TxError::Db(e) => {
            e.is_validation()
                || e.is_already_exists()
                || e.is_not_found()
                || e.is_not_allowed()
                || e.is_serialization()
                // Statement errors (ASSERT, type checks) carry no details;
                // timeouts, cancellation and conflicts do.
                || (e.is_query() && e.query_details().is_none())
        }
        TxError::Contention { .. } | TxError::PublisherUnavailable { .. } => false,
    }
}

fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-string panic".into())
}

async fn commit(db: &Surreal<Any>, writes: &[Write]) -> Result<Vec<Committed<Published>>, TxError> {
    let committed = run_tx(db, counters::HOT_RECORD_POLICY, |tx| {
        let writes = writes.to_vec();
        async move {
            let now = Utc::now();
            let mut all = Vec::new();
            let mut counts = Vec::with_capacity(writes.len());
            for w in &writes {
                let changes = match std::panic::catch_unwind(AssertUnwindSafe(|| w(tx.clone()))) {
                    Ok(fut) => AssertUnwindSafe(fut).catch_unwind().await,
                    Err(panic) => Err(panic),
                }
                .map_err(|panic| {
                    TxError::Abort(format!("write panicked: {}", panic_message(&*panic)))
                })??;
                counts.push(changes.len());
                all.extend(changes);
            }
            // All outbox entries and the one counter bump in one round trip.
            let new: Vec<(NewChange<'_>, &str)> = all
                .iter()
                .map(|c| {
                    (
                        NewChange {
                            record_kind: &c.record_kind,
                            record_id: &c.record_id,
                            revision: &c.revision,
                            operation: &c.operation,
                            generation: &c.generation,
                            content_hash: &c.content_hash,
                            task_id: "",
                            job_id: "",
                        },
                        c.op_key.as_str(),
                    )
                })
                .collect();
            let (enqueued, bumped) =
                outbox::enqueue_many_in(&tx, &new, Some(CHANGED_COUNTER), now).await?;
            let changed_seq = bumped
                .ok_or_else(|| TxError::Abort("memories_changed returned no value".into()))?;
            let mut rest = enqueued.into_iter();
            let per_write: Vec<Vec<Enqueued>> = counts
                .iter()
                .map(|&n| rest.by_ref().take(n).collect())
                .collect();
            Ok((per_write, changed_seq))
        }
    })
    .await?;
    let (per_write, changed_seq) = committed.value;
    let batch_size = per_write.len();
    Ok(per_write
        .into_iter()
        .map(|enqueued| Committed {
            value: Published {
                enqueued,
                changed_seq,
                batch_size,
            },
            retries: committed.retries,
        })
        .collect())
}
