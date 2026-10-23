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

use std::sync::Arc;

use chrono::Utc;
use futures_util::future::BoxFuture;
use surrealdb::engine::any::Any;
use surrealdb::Surreal;
use tokio::sync::{mpsc, oneshot};

use super::counters;
use super::outbox::{self, Enqueued, NewChange};
use super::tx::{run_tx, Committed, Tx, TxError};

/// Counter bumped once per committed batch.
pub const CHANGED_COUNTER: &str = "memories_changed";
/// Most writes per transaction.
pub const BATCH_CAP: usize = 32;

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
    queue: mpsc::UnboundedSender<(Write, Reply)>,
}

impl Publisher {
    /// Start the publisher on `db`. Run one per process; a second one works
    /// but its batches conflict with the first's (resolved by `run_tx`).
    pub fn spawn(db: Surreal<Any>) -> Publisher {
        let (queue, mut rx) = mpsc::unbounded_channel::<(Write, Reply)>();
        tokio::spawn(async move {
            while let Some(first) = rx.recv().await {
                let mut batch = vec![first];
                while batch.len() < BATCH_CAP {
                    match rx.try_recv() {
                        Ok(next) => batch.push(next),
                        Err(_) => break,
                    }
                }
                let (writes, replies): (Vec<_>, Vec<_>) = batch.into_iter().unzip();
                let results = publish_batch(&db, &writes).await;
                for (reply, result) in replies.into_iter().zip(results) {
                    // A dropped waiter still had its write committed.
                    let _ = reply.send(result);
                }
            }
        });
        Publisher { queue }
    }

    /// Submit `write` and wait for its batch to commit.
    pub async fn publish(&self, write: Write) -> Result<Committed<Published>, TxError> {
        let (tx, rx) = oneshot::channel();
        self.queue
            .send((write, tx))
            .map_err(|_| TxError::Abort("publisher stopped".into()))?;
        rx.await
            .map_err(|_| TxError::Abort("publisher dropped the write".into()))?
    }
}

/// Commit `writes` as one transaction; one result per write, in order.
/// A conflict retries the whole batch (`run_tx`). Any other failure splits
/// the batch so each write commits or fails on its own.
pub async fn publish_batch(
    db: &Surreal<Any>,
    writes: &[Write],
) -> Vec<Result<Committed<Published>, TxError>> {
    match commit(db, writes).await {
        Ok(done) => done.into_iter().map(Ok).collect(),
        Err(TxError::Contention { attempts }) => writes
            .iter()
            .map(|_| Err(TxError::Contention { attempts }))
            .collect(),
        Err(e) if writes.len() == 1 => vec![Err(e)],
        Err(_) => {
            let mut out = Vec::with_capacity(writes.len());
            for w in writes {
                out.push(
                    commit(db, std::slice::from_ref(w))
                        .await
                        .map(|mut v| v.remove(0)),
                );
            }
            out
        }
    }
}

async fn commit(db: &Surreal<Any>, writes: &[Write]) -> Result<Vec<Committed<Published>>, TxError> {
    let committed = run_tx(db, counters::HOT_RECORD_POLICY, |tx| {
        let writes = writes.to_vec();
        async move {
            let now = Utc::now();
            let mut all = Vec::new();
            let mut counts = Vec::with_capacity(writes.len());
            for w in &writes {
                let changes = w(tx.clone()).await?;
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
