//! Transactional actor projection intent and one leased, paced delivery cursor.
//! Canonical actor/alias reads are bulk bounded; no CSV or search engine lives here.
//!
//! Delivery is at least once across the source/index boundary: the database
//! persists one exact pending body, and only a matching committed + visible
//! receipt advances its cursor. This is not a cross-service atomic transaction.
//! Initial adoption and explicit shadow rebuilds scan canonical DB keys.
//! One resource lease/rate fence covers live, shadow and recovery commands.
//! Generic producers without an actual parent argument remain unattributed.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::future::Future;

use chrono::DateTime;
use chrono::Duration;
use chrono::Utc;
use fleetworks_contracts::actor_index::ActorIndexBatch;
use fleetworks_contracts::actor_index::ActorIndexReceipt;
use fleetworks_contracts::actor_index::ActorIndexStatus;
use fleetworks_contracts::actor_index::ActorProjection;
use fleetworks_contracts::actor_index::MAX_BATCH_DOCUMENTS;
use serde::Deserialize;
use serde::Serialize;
use surrealdb::types::Datetime;
use surrealdb::types::RecordId;
use surrealdb::types::SurrealValue;

use super::jobs;
use super::repo::Repository;
use super::tx::run_tx;
use super::tx::RetryPolicy;
use super::tx::Tx;
use super::tx::TxError;

#[path = "actor_search_delivery.rs"]
mod delivery;
#[path = "actor_search_projection.rs"]
mod projection;
#[path = "actor_search_rebuild.rs"]
mod rebuild;
#[path = "actor_resource_policy.rs"]
mod resource_policy;
#[path = "actor_search_resources.rs"]
mod resources;

pub use rebuild::next_rebuild;
pub use rebuild::observe_rebuild;
pub use rebuild::rebuild_acknowledge;
pub use rebuild::rebuild_promoted;
pub use rebuild::rebuild_repaired;
pub use rebuild::rebuild_started;
pub use rebuild::request_rebuild;
pub use rebuild::RebuildAction;
pub use rebuild::RebuildProgress;
pub use resource_policy::resource_limits;
pub use resources::await_bulk_capacity;
pub(crate) use resources::await_bulk_capacity_claimed;
pub(super) use resources::check_bulk_capacity_in;
pub use resources::observe_resources;

pub const SCHEMA: &str = concat!(
    include_str!("schema/actor_search_outbox.surql"),
    "\n",
    include_str!("schema/actor_search_rebuild.surql")
);
const BACKFILL_PAGE: usize = 128;
const INITIAL_RETIREMENT_LIMIT: i64 = 100_000;
const INITIAL_RETIREMENT_MIN_CATALOG: i64 = 4_096;

/// Actual server-side execution identity, never supplied by a UI context field.
#[derive(Clone)]
pub struct SourceCause {
    pub parent_job: String,
    pub operation_id: String,
}

fn invalid(code: &str) -> TxError {
    TxError::Abort(code.to_owned())
}
fn valid(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}
fn hash(parts: &[&str]) -> String {
    crate::evidence::content_hash(&super::codec::encode(parts))
}
fn child_id(cause: &SourceCause) -> String {
    format!(
        "actor-index-{}",
        hash(&[&cause.parent_job, &cause.operation_id])
    )
}
fn instant(value: &str) -> Result<DateTime<Utc>, TxError> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| invalid("actor_index_time_invalid"))
}
fn text(value: &str, bytes: usize) -> Option<String> {
    let mut result = String::new();
    for character in value.trim().chars() {
        let character = if character.is_control() {
            ' '
        } else {
            character
        };
        if result.len() + character.len_utf8() > bytes {
            break;
        }
        result.push(character);
    }
    (!result.trim().is_empty()).then(|| result.trim().to_owned())
}

async fn root_guard_in(tx: &Tx, job: &str) -> Result<(), TxError> {
    let mut response = jobs::run(
        tx,
        tx.query("SELECT VALUE record::id(root) FROM ONLY $job")
            .bind(("job", jobs::rid("job", job))),
    )
    .await?;
    let root = jobs::take::<Option<String>>(&mut response, 0)?
        .ok_or_else(|| invalid("actor_index_job_missing"))?;
    jobs::bump(tx, "job_guard", &root).await
}

pub(super) async fn guard(tx: &Tx) -> Result<(), TxError> {
    jobs::bump(tx, "job_guard", "actor-search-coordinator").await?;
    jobs::run(
        tx,
        tx.query("UPSERT actor_search_control:catalog SET revision=(revision ?? 0)+1 RETURN NONE;"),
    )
    .await?;
    Ok(())
}

async fn ensure_child_in(
    tx: &Tx,
    cause: &SourceCause,
    now: DateTime<Utc>,
) -> Result<String, TxError> {
    if !valid(&cause.parent_job) || !valid(&cause.operation_id) {
        return Err(invalid("invalid_index_cause"));
    }
    let child = child_id(cause);
    root_guard_in(tx, &cause.parent_job).await?;
    let mut reply = jobs::run(
        tx,
        tx.query("RETURN record::exists($job)")
            .bind(("job", jobs::rid("job", &child))),
    )
    .await?;
    if jobs::take::<Option<bool>>(&mut reply, 0)? != Some(true) {
        jobs::create_job_in(
            tx,
            &jobs::NewJob {
                id: &child,
                kind: "maintenance",
                operation: "index_actor_catalog",
                parent: Some(&cause.parent_job),
                created_at: now,
            },
        )
        .await?;
        super::job_context::record_child_in(tx, &cause.parent_job, &child).await?;
        jobs::run(tx, tx.query("UPDATE $job SET state='queued',cancellable=false,resource_ref='backend actor catalog',progress_done=0,progress_total=0 RETURN NONE;")
            .bind(("job",jobs::rid("job",&child)))).await?;
        jobs::run(
            tx,
            tx.query("CREATE $row SET parent_job=$parent,operation_id=$operation RETURN NONE;")
                .bind(("row", jobs::rid("actor_search_cause", &child)))
                .bind(("parent", cause.parent_job.clone()))
                .bind(("operation", cause.operation_id.clone())),
        )
        .await?;
    }
    Ok(child)
}

#[derive(SurrealValue)]
struct HeadWrite {
    id: RecordId,
    actor_id: String,
    revision: i64,
    updated_at: Datetime,
}

#[derive(SurrealValue)]
struct EventWrite {
    id: RecordId,
    sequence: i64,
    actor_id: String,
    revision: i64,
    cause_job: Option<String>,
    created_at: Datetime,
}

/// Join the actual source transaction after actor/alias/lifecycle writes. A
/// rollback cannot leave index intent. Even no-op seed retries reconcile work.
pub fn enqueue_actors_in<'a>(
    tx: &'a Tx,
    actors: &'a [String],
    cause: Option<&'a SourceCause>,
    now: DateTime<Utc>,
) -> impl Future<Output = Result<(), TxError>> + Send + 'a {
    // Seed, lifecycle, and index backfill all nest this transaction operation.
    Box::pin(enqueue_actor_events_in(tx, actors, cause, now, true))
}

/// Append from a seed transaction that already holds the coordinator guard.
pub(super) fn enqueue_actors_under_guard_in<'a>(
    tx: &'a Tx,
    actors: &'a [String],
    cause: Option<&'a SourceCause>,
    now: DateTime<Utc>,
) -> impl Future<Output = Result<(), TxError>> + Send + 'a {
    Box::pin(enqueue_actor_events_in(tx, actors, cause, now, false))
}

async fn enqueue_actor_events_in(
    tx: &Tx,
    actors: &[String],
    cause: Option<&SourceCause>,
    now: DateTime<Utc>,
    acquire_guard: bool,
) -> Result<(), TxError> {
    if actors.is_empty() {
        return Ok(());
    }
    if actors.len() > MAX_BATCH_DOCUMENTS || actors.iter().any(|actor| !valid(actor)) {
        return Err(invalid("actor_index_source_bound"));
    }
    if acquire_guard {
        guard(tx).await?;
    }
    let actors: Vec<String> = actors
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let child = if let Some(cause) = cause {
        Some(ensure_child_in(tx, cause, now).await?)
    } else {
        None
    };
    // The coordinator guard protects this whole consecutive sequence range.
    // One control write suffices for the sorted, distinct actor IDs.
    let count = actors.len() as i64;
    let heads: Vec<_> = actors
        .iter()
        .map(|actor| HeadWrite {
            id: jobs::rid("actor_search_head", actor),
            actor_id: actor.clone(),
            revision: 1,
            updated_at: now.into(),
        })
        .collect();
    let mut response = jobs::run(
        tx,
        tx.query(
            "UPDATE actor_search_control:catalog SET next_sequence += $count RETURN NONE;
             SELECT VALUE next_sequence FROM ONLY actor_search_control:catalog;
             INSERT INTO actor_search_head $heads ON DUPLICATE KEY UPDATE
                 actor_id=$input.actor_id,revision=(revision ?? 0)+1,
                 updated_at=$input.updated_at RETURN AFTER;",
        )
        .bind(("count", count))
        .bind(("heads", heads)),
    )
    .await?;
    let end = jobs::take::<Option<i64>>(&mut response, 1)?
        .filter(|end| *end >= count)
        .ok_or_else(|| invalid("actor_index_sequence_invalid"))?;
    let revisions: BTreeMap<_, _> = jobs::take::<Vec<HeadRow>>(&mut response, 2)?
        .into_iter()
        .map(|head| (head.actor_id, head.revision))
        .collect();
    let events = (end - count + 1..=end)
        .zip(actors)
        .map(|(sequence, actor_id)| {
            let revision = *revisions
                .get(&actor_id)
                .ok_or_else(|| invalid("actor_index_revision_missing"))?;
            Ok(EventWrite {
                id: RecordId::new("actor_search_event", sequence),
                sequence,
                actor_id,
                revision,
                cause_job: child.clone(),
                created_at: now.into(),
            })
        })
        .collect::<Result<Vec<_>, TxError>>()?;
    jobs::run(
        tx,
        tx.query(
            "INSERT INTO actor_search_event $events RETURN NONE;
             IF $cause != NONE {
                 UPDATE type::record('actor_search_cause',$cause)
                     SET required_sequence=$end,total_events+=$count,closed=false RETURN NONE;
                 UPDATE type::record('job',$cause)
                     SET progress_total=(SELECT VALUE total_events FROM ONLY type::record('actor_search_cause',$cause)) RETURN NONE;
             };",
        )
        .bind(("events", events))
        .bind(("cause", child))
        .bind(("end", end))
        .bind(("count", count)),
    )
    .await?;
    Ok(())
}

/// Close the committed range independently of successful/partial source outcome.
pub async fn close_cause_in(
    tx: &Tx,
    cause: &SourceCause,
    partial: bool,
    now: DateTime<Utc>,
) -> Result<(), TxError> {
    guard(tx).await?;
    jobs::run(
        tx,
        tx.query("UPDATE $row SET closed=true,partial_source=$partial RETURN NONE;")
            .bind(("row", jobs::rid("actor_search_cause", &child_id(cause))))
            .bind(("partial", partial)),
    )
    .await?;
    let control = control_in(tx).await?;
    finish_causes_in(tx, control.acknowledged_sequence as u64, now).await?;
    Ok(())
}

#[derive(SurrealValue)]
struct ActorRow {
    actor_id: String,
    display_name: Option<String>,
    subtype: String,
    scope_key: String,
    catalog_id: String,
    metadata_json: Option<String>,
    lifecycle: String,
    memory_kind: String,
    redirected: bool,
}
#[derive(SurrealValue)]
struct AliasRow {
    actor_id: String,
    alias: Option<String>,
    provenance_json: Option<String>,
}
#[derive(SurrealValue)]
struct HeadRow {
    actor_id: String,
    revision: i64,
}

/// One canonical snapshot for ingestion and current gateway eligibility checks.
#[derive(Clone, Debug)]
pub struct CanonicalProjection {
    pub actor_id: String,
    pub revision: u64,
    pub projection: Option<ActorProjection>,
}

pub async fn canonical_many_in(
    tx: &Tx,
    ids: &[String],
) -> Result<Vec<CanonicalProjection>, TxError> {
    if ids.len() > MAX_BATCH_DOCUMENTS || ids.iter().any(|id| !valid(id)) {
        return Err(invalid("actor_projection_read_bound"));
    }
    let records: Vec<_> = ids.iter().map(|id| jobs::rid("brain_actor", id)).collect();
    let heads: Vec<_> = ids
        .iter()
        .map(|id| jobs::rid("actor_search_head", id))
        .collect();
    let mut response=jobs::run(tx,tx.query("SELECT record::id(id) AS actor_id,
        IF string::len(display_name)<=1024 {display_name} ELSE {NONE} AS display_name,
        subtype,scope_key,catalog_id ?? '' AS catalog_id,
        IF string::len(provenance_json)<=8192 {provenance_json} ELSE {NONE} AS metadata_json,
        type::record('memory',record::id(id)).lifecycle ?? '' AS lifecycle,
        type::record('memory',record::id(id)).memory_kind ?? '' AS memory_kind,
        record::exists(type::record('brain_actor_redirect',record::id(id))) AS redirected FROM $actors;
        RETURN $ids.map(|$actor| { SELECT actor_id, IF bytes::len(type::bytes(alias))<=256 {alias} ELSE {NONE} AS alias,
          IF string::len(provenance_json)<=8192 {provenance_json} ELSE {NONE} AS provenance_json
          FROM brain_actor_alias WITH INDEX actor_search_alias_owner WHERE actor_id=$actor AND alias_type='name' ORDER BY alias LIMIT 64 });
        SELECT actor_id,revision FROM $heads;")
        .bind(("actors",records)).bind(("heads",heads)).bind(("ids",ids.to_vec()))).await?;
    let actors: Vec<ActorRow> = jobs::take(&mut response, 0)?;
    let aliases: Vec<Vec<AliasRow>> = jobs::take(&mut response, 1)?;
    let heads: Vec<HeadRow> = jobs::take(&mut response, 2)?;
    let actors: BTreeMap<_, _> = actors
        .into_iter()
        .map(|row| (row.actor_id.clone(), row))
        .collect();
    let heads: BTreeMap<_, _> = heads
        .into_iter()
        .map(|row| (row.actor_id, row.revision))
        .collect();
    let mut grouped = BTreeMap::<String, Vec<AliasRow>>::new();
    for alias in aliases.into_iter().flatten() {
        grouped
            .entry(alias.actor_id.clone())
            .or_default()
            .push(alias);
    }
    let mut result = Vec::new();
    for id in ids {
        let revision = heads.get(id).copied().unwrap_or(0);
        let projection = if let Some(actor) = actors.get(id).filter(|actor| {
            actor.lifecycle == "active"
                && actor.memory_kind == "actor"
                && !actor.redirected
                && actor.scope_key == crate::actor_catalog::SCOPE
        }) {
            // Oversized/malformed optional metadata is omitted, not a poison
            // record that indefinitely prevents following canonical actors.
            let metadata: serde_json::Value = actor
                .metadata_json
                .as_deref()
                .and_then(|value| serde_json::from_str(value).ok())
                .unwrap_or_default();
            let display_name = projection::identity(
                actor
                    .display_name
                    .as_deref()
                    .ok_or_else(|| invalid("actor_name_bound"))?,
                1024,
            )?;
            let aliases =
                projection::aliases(&display_name, grouped.remove(id).unwrap_or_default())?;
            Some(ActorProjection {
                display_name,
                aliases,
                summary: metadata["description"]
                    .as_str()
                    .and_then(|value| text(value, 2048))
                    .filter(|value| projection::indexable_text(value)),
                actor_type: projection::optional_identity(&actor.subtype, 128)?,
                countries: projection::countries(&metadata)?,
                source_id: projection::optional_identity(&actor.catalog_id, 256)?,
                authority_scopes: vec!["backend".into()],
                eligible: true,
            })
        } else {
            None
        };
        result.push(CanonicalProjection {
            actor_id: id.clone(),
            revision: u64::try_from(revision)
                .map_err(|_| invalid("projection_revision_invalid"))?,
            projection,
        });
    }
    Ok(result)
}

pub async fn canonical_many(
    repo: &Repository,
    ids: &[String],
) -> Result<Vec<CanonicalProjection>, TxError> {
    let ids = ids.to_vec();
    Ok(run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let ids = ids.clone();
        async move { canonical_many_in(&tx, &ids).await }
    })
    .await?
    .value)
}

/// Read-only source/dispatcher evidence for the public gateway. Observation
/// never starts a seed, creates intent, repairs state or calls the index.
#[derive(Clone, Debug, SurrealValue)]
pub struct Observation {
    pub deployment_id: String,
    pub source_epoch: String,
    pub index_generation: String,
    pub next_sequence: i64,
    pub acknowledged_sequence: i64,
    pub blocked: bool,
    pub backfill_complete: bool,
    pub seed_complete: bool,
    pub rebuild_reason_code: String,
    pub pressure_paused: bool,
    pub pressure_reason: String,
    pub backlog_paused: bool,
}

pub async fn observe(repo: &Repository) -> Result<Option<Observation>, TxError> {
    let mut response=repo.db().query("SELECT (SELECT VALUE instance_id FROM ONLY storage_meta:identity) ?? '' AS deployment_id,
        source_epoch,index_generation,next_sequence,acknowledged_sequence,blocked,backfill_complete,pressure_paused,pressure_reason,backlog_paused,
        ((SELECT VALUE status FROM ONLY brain_actor_seed:catalog)='complete') ?? false AS seed_complete,
        ((SELECT VALUE reason_code FROM ONLY actor_search_rebuild:catalog) ?? '') AS rebuild_reason_code FROM ONLY actor_search_control:catalog").await?.check()?;
    Ok(response.take(0)?)
}

/// Check whether indexing can wait for a local initial seed. The source can
/// still commit while the backlog has headroom. Prepared work must reconcile.
pub async fn defer_initial_seed_index(
    repo: &Repository,
    backlog_resume_events: u64,
) -> Result<bool, TxError> {
    let mut response = repo
        .db()
        .query(
            "SELECT VALUE (backfill_complete AND pending_json = '' AND !backlog_paused
                AND next_sequence - acknowledged_sequence < $limit)
             FROM ONLY actor_search_control:catalog;
             SELECT VALUE IF job_id != '' {
                 type::record('job',job_id).state IN ['queued','running','waiting','reconciling']
             } ELSE { false } FROM ONLY setup_resource:actor_catalog;",
        )
        .bind((
            "limit",
            i64::try_from(backlog_resume_events).unwrap_or(i64::MAX),
        ))
        .await?
        .check()?;
    Ok(response.take::<Option<bool>>(0)? == Some(true)
        && response.take::<Option<bool>>(1)? == Some(true))
}

#[derive(Clone, SurrealValue)]
struct Control {
    next_sequence: i64,
    acknowledged_sequence: i64,
    deployment_id: String,
    source_epoch: String,
    index_generation: String,
    lease_owner: String,
    lease_epoch: i64,
    lease_until: String,
    next_start_at: String,
    retry_at: String,
    failures: i64,
    last_error: String,
    blocked: bool,
    pending_json: String,
    backfill_after: String,
    backfill_complete: bool,
    effective_limits_json: String,
    pressure_paused: bool,
    pressure_reason: String,
    pressure_observed_at: String,
    backlog_paused: bool,
    tail_since: String,
}
#[derive(Clone, SurrealValue)]
struct Event {
    sequence: i64,
    actor_id: String,
    cause_job: Option<String>,
}

const CONTROL_SELECT_SURQL: &str = "SELECT next_sequence,acknowledged_sequence,deployment_id,source_epoch,index_generation,lease_owner,lease_epoch,<string>lease_until AS lease_until,<string>next_start_at AS next_start_at,<string>retry_at AS retry_at,failures,last_error,blocked,pending_json,backfill_after,backfill_complete,effective_limits_json,pressure_paused,pressure_reason,pressure_observed_at,backlog_paused,tail_since FROM ONLY actor_search_control:catalog";
const LEASE_RENEW_WINDOW_SECS: i64 = 15;

fn validate_control(control: Control) -> Result<Control, TxError> {
    if control.acknowledged_sequence < 0
        || control.acknowledged_sequence > control.next_sequence
        || control.lease_epoch < 0
        || control.failures < 0
    {
        return Err(invalid("actor_index_control_invalid"));
    }
    Ok(control)
}

async fn control_read_opt(repo: &Repository) -> Result<Option<Control>, TxError> {
    let mut response = repo.db().query(CONTROL_SELECT_SURQL).await?.check()?;
    response
        .take::<Option<Control>>(0)?
        .map(validate_control)
        .transpose()
}

async fn control_read(repo: &Repository) -> Result<Control, TxError> {
    control_read_opt(repo)
        .await?
        .ok_or_else(|| invalid("actor_index_control_missing"))
}

async fn control_in(tx: &Tx) -> Result<Control, TxError> {
    let mut response = jobs::run(tx, tx.query(CONTROL_SELECT_SURQL)).await?;
    let control = jobs::take::<Option<Control>>(&mut response, 0)?
        .ok_or_else(|| invalid("actor_index_control_missing"))?;
    validate_control(control)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PreparedBatch {
    pub batch: ActorIndexBatch,
    pub cause_counts: BTreeMap<String, u64>,
}

#[derive(Clone, Debug)]
pub struct Lease {
    pub owner: String,
    pub epoch: u64,
}

#[derive(Clone, Debug)]
pub enum Preparation {
    Waiting,
    Batch {
        lease: Lease,
        prepared: Box<PreparedBatch>,
    },
}

/// Elect before private status polling, so replicas do not multiply requests.
pub async fn claim(
    repo: &Repository,
    owner: &str,
    now: DateTime<Utc>,
) -> Result<Option<Lease>, TxError> {
    if !valid(owner) {
        return Err(invalid("actor_index_owner_invalid"));
    }
    if let Some(current) = control_read_opt(repo).await? {
        if current.blocked
            || instant(&current.retry_at)? > now
            || (instant(&current.lease_until)? > now && current.lease_owner != owner)
        {
            return Ok(None);
        }
        // Avoid an empty write transaction every 1s poll while our 45s lease still has margin.
        if current.lease_owner == owner
            && instant(&current.lease_until)? > now + Duration::seconds(LEASE_RENEW_WINDOW_SECS)
        {
            return Ok(Some(Lease {
                owner: owner.to_owned(),
                epoch: current.lease_epoch as u64,
            }));
        }
    }
    let owner = owner.to_owned();
    Ok(run_tx(repo.db(),RetryPolicy::default(),|tx| {let owner=owner.clone();async move {
        guard(&tx).await?;let control=control_in(&tx).await?;
        if control.blocked || instant(&control.retry_at)?>now || (instant(&control.lease_until)?>now && control.lease_owner!=owner) {return Ok(None);}
        let epoch=if control.lease_owner==owner && instant(&control.lease_until)?>now {control.lease_epoch} else {control.lease_epoch.checked_add(1).ok_or_else(||invalid("actor_index_epoch_exhausted"))?};
        jobs::run(&tx,tx.query("UPDATE actor_search_control:catalog SET lease_owner=$owner,lease_epoch=$epoch,lease_until=<datetime>$until RETURN NONE")
            .bind(("owner",owner.clone())).bind(("epoch",epoch)).bind(("until",(now+Duration::seconds(45)).to_rfc3339()))).await?;
        Ok(Some(Lease {owner,epoch:epoch as u64}))
    }}).await?.value)
}

/// A bounded HTTP operation renews its fencing lease without a detached task.
pub async fn renew(repo: &Repository, lease: &Lease, now: DateTime<Utc>) -> Result<(), TxError> {
    let lease = lease.clone();
    run_tx(repo.db(),RetryPolicy::default(),|tx| {let lease=lease.clone();async move {
        guard(&tx).await?;let control=control_in(&tx).await?;
        if control.lease_owner!=lease.owner || control.lease_epoch!=lease.epoch as i64 || instant(&control.lease_until)?<=now {return Err(invalid("actor_index_lease_lost"));}
        jobs::run(&tx,tx.query("UPDATE actor_search_control:catalog SET lease_until=<datetime>$until RETURN NONE").bind(("until",(now+Duration::seconds(45)).to_rfc3339()))).await?;Ok(())
    }}).await?;
    Ok(())
}

/// Gracefully release this owner's coordinator lease on clean shutdown while
/// advancing the fencing epoch so stale in-flight ticks cannot commit later.
pub async fn release_lease(repo: &Repository, owner: &str) -> Result<bool, TxError> {
    if !valid(owner) {
        return Err(invalid("actor_index_owner_invalid"));
    }
    let owner = owner.to_owned();
    Ok(run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let owner = owner.clone();
        async move {
            guard(&tx).await?;
            let control = control_in(&tx).await?;
            if control.lease_owner != owner {
                return Ok(false);
            }
            let epoch = control
                .lease_epoch
                .checked_add(1)
                .ok_or_else(|| invalid("actor_index_epoch_exhausted"))?;
            jobs::run(
                &tx,
                tx.query(
                    "UPDATE actor_search_control:catalog
                     SET lease_owner='', lease_epoch=$epoch, lease_until=<datetime>'1970-01-01T00:00:00Z'
                     RETURN NONE",
                )
                .bind(("epoch", epoch)),
            )
            .await?;
            Ok(true)
        }
    })
    .await?
    .value)
}

async fn backfill_in(tx: &Tx, control: &Control, now: DateTime<Utc>) -> Result<(), TxError> {
    if control.backfill_complete
        || (control.next_sequence - control.acknowledged_sequence) as u64
            >= resources::limits(control)?.backlog_pause_events
    {
        return Ok(());
    }
    let mut response = jobs::run(
        tx,
        tx.query(
            "SELECT VALUE record::id(id) FROM brain_actor WHERE id>$after ORDER BY id LIMIT $limit",
        )
        .bind(("after", jobs::rid("brain_actor", &control.backfill_after)))
        .bind(("limit", BACKFILL_PAGE as i64)),
    )
    .await?;
    let actors: Vec<String> = jobs::take(&mut response, 0)?;
    if !actors.is_empty() {
        enqueue_actors_in(tx, &actors, None, now).await?;
    }
    jobs::run(tx,tx.query("UPDATE actor_search_control:catalog SET backfill_after=$after,backfill_complete=$complete RETURN NONE")
        .bind(("after",actors.last().cloned().unwrap_or_else(||control.backfill_after.clone()))).bind(("complete",actors.len()<BACKFILL_PAGE))).await?;
    Ok(())
}

/// Live outbox event IDs are their numeric sequence. Reserve and address a
/// bounded consecutive range without scanning the rest of the backlog.
fn event_records(after: i64, through: i64) -> Result<Vec<RecordId>, TxError> {
    let count = through
        .checked_sub(after)
        .filter(|count| (0..=MAX_BATCH_DOCUMENTS as i64).contains(count))
        .ok_or_else(|| invalid("actor_index_event_range"))?;
    Ok((1..=count)
        .map(|offset| RecordId::new("actor_search_event", after + offset))
        .collect())
}

/// Prepare one bounded live source page under the shared lease/rate fence.
pub async fn prepare(
    repo: &Repository,
    owner: &str,
    status: &ActorIndexStatus,
    now: DateTime<Utc>,
) -> Result<Preparation, TxError> {
    prepare_with_embedded_policy(repo, owner, status, now, false).await
}

/// Embedded catch-up uses the configured writer batch with bounded source reads.
pub async fn prepare_with_embedded_policy(
    repo: &Repository,
    owner: &str,
    status: &ActorIndexStatus,
    now: DateTime<Utc>,
    embedded: bool,
) -> Result<Preparation, TxError> {
    delivery::prepare(repo, owner, status, now, embedded).await
}

/// Acknowledge only the exact persisted batch's reader-visible receipt.
pub async fn acknowledge(
    repo: &Repository,
    lease: &Lease,
    receipt: &ActorIndexReceipt,
    now: DateTime<Utc>,
) -> Result<(), TxError> {
    acknowledge_with_retirement(repo, lease, receipt, now, false).await
}

/// Retire a verified batch logically with its durable cursor. During an initial
/// Embedded import a bounded number of retired rows can be deleted later.
pub async fn acknowledge_with_retirement(
    repo: &Repository,
    lease: &Lease,
    receipt: &ActorIndexReceipt,
    now: DateTime<Utc>,
    embedded_catch_up: bool,
) -> Result<(), TxError> {
    let lease = lease.clone();
    let receipt = receipt.clone();
    run_tx(repo.db(),RetryPolicy::default(),|tx| {let lease=lease.clone();let receipt=receipt.clone();async move {
        guard(&tx).await?;let control=control_in(&tx).await?;
        if control.lease_owner!=lease.owner || control.lease_epoch!=lease.epoch as i64 || instant(&control.lease_until)?<=now {return Err(invalid("actor_index_lease_lost"));}
        let prepared:PreparedBatch=serde_json::from_str(&control.pending_json).map_err(|_|invalid("actor_index_pending_invalid"))?;
        if prepared.batch.batch_id!=receipt.batch_id || prepared.batch.content_checksum().map_err(|_|invalid("actor_index_checksum"))?!=receipt.checksum
            || prepared.batch.index_generation!=receipt.index_generation || receipt.visible_checkpoint!=prepared.batch.through_checkpoint
            || receipt.committed_checkpoint!=receipt.visible_checkpoint || receipt.writer_epoch!=lease.epoch {return Err(invalid("actor_index_receipt_mismatch"));}
        let from = i64::try_from(prepared.batch.from_checkpoint).map_err(|_| invalid("actor_index_event_range"))?;
        let through = i64::try_from(receipt.visible_checkpoint).map_err(|_| invalid("actor_index_event_range"))?;
        let mut events = event_records(from, through)?;
        if embedded_catch_up
            && control.next_sequence > INITIAL_RETIREMENT_MIN_CATALOG
            && through <= INITIAL_RETIREMENT_LIMIT
        {
            // The persisted cursor excludes retired events from delivery, even
            // after restart. Keep synchronous deletion out of the import path.
            events.clear();
        }
        jobs::run(&tx,tx.query("UPDATE actor_search_control:catalog SET acknowledged_sequence=$through,pending_json='',failures=0,last_error='',retry_at=<datetime>$now RETURN NONE;
            IF array::len($events)>0 { DELETE $events RETURN NONE; };")
            .bind(("through",through)).bind(("events",events)).bind(("now",now.to_rfc3339()))).await?;
        for (cause,count) in prepared.cause_counts {
            root_guard_in(&tx,&cause).await?;
            jobs::run(&tx,tx.query("UPDATE $cause SET acknowledged_events+=$count RETURN NONE;
                UPDATE $job SET progress_done=(SELECT VALUE acknowledged_events FROM ONLY $cause),updated_at=<datetime>$now RETURN NONE;")
                .bind(("cause",jobs::rid("actor_search_cause",&cause))).bind(("job",jobs::rid("job",&cause))).bind(("count",count as i64)).bind(("now",now.to_rfc3339()))).await?;
        }
        finish_causes_in(&tx,receipt.visible_checkpoint,now).await?;
        Ok(())
    }}).await?;
    Ok(())
}

async fn finish_causes_in(tx: &Tx, checkpoint: u64, now: DateTime<Utc>) -> Result<(), TxError> {
    #[derive(SurrealValue)]
    struct Cause {
        job_id: String,
        partial_source: bool,
    }
    let mut response=jobs::run(tx,tx.query("SELECT record::id(id) AS job_id,partial_source FROM actor_search_cause WITH INDEX actor_search_cause_finish WHERE done=false AND closed=true AND required_sequence<=$through LIMIT 100")
        .bind(("through",checkpoint as i64))).await?;
    let causes: Vec<Cause> = jobs::take(&mut response, 0)?;
    for cause in causes {
        root_guard_in(tx, &cause.job_id).await?;
        jobs::run(tx,tx.query("UPDATE $cause SET done=true,acknowledged_events=total_events RETURN NONE; UPDATE $job SET progress_done=progress_total,state='completed',finished_at=<datetime>$now,updated_at=<datetime>$now,result_ref=$result WHERE state NOT IN ['cancelled','failed','completed'] RETURN NONE;")
            .bind(("cause",jobs::rid("actor_search_cause",&cause.job_id))).bind(("job",jobs::rid("job",&cause.job_id))).bind(("now",now.to_rfc3339())).bind(("result",if cause.partial_source {"reader-visible committed partial catalog"} else {"reader-visible actor index"}))).await?;
        jobs::log_transition_in(
            tx,
            &cause.job_id,
            &format!("{}:index-visible", cause.job_id),
            "actor.index.visible",
            "Committed actor projections are reader-visible",
            &now.to_rfc3339(),
        )
        .await?;
        jobs::rollup_in(tx, &cause.job_id).await?;
    }
    Ok(())
}

/// Persist the sole retry owner's delay; auth/schema/oversize failures pause
/// intent rather than discarding it or hot-looping.
pub async fn defer(
    repo: &Repository,
    lease: &Lease,
    retry_after: Option<std::time::Duration>,
    permanent: bool,
    now: DateTime<Utc>,
) -> Result<(), TxError> {
    let lease = lease.clone();
    run_tx(repo.db(),RetryPolicy::default(),|tx| {let lease=lease.clone();async move {
        guard(&tx).await?;let control=control_in(&tx).await?;
        if control.lease_owner!=lease.owner || control.lease_epoch!=lease.epoch as i64 {return Err(invalid("actor_index_lease_lost"));}
        let failures=control.failures.saturating_add(1);
        let limits=resources::limits(&control)?;
        let cooling=failures>=i64::from(limits.cooldown_after);
        let local=if cooling {i64::from(limits.cooldown_ms)} else {i64::from(limits.backoff_initial_ms).saturating_mul(1_i64<<((failures-1) as u32).min(30)).min(i64::from(limits.backoff_cap_ms))};
        let jitter=hash(&[&lease.owner,&failures.to_string()]).bytes().next().unwrap_or_default() as i64*3;
        let local=Duration::milliseconds(local)+Duration::milliseconds(jitter);
        let remote=retry_after.map(Duration::from_std).transpose().ok().flatten();
        let retry=now.checked_add_signed(remote.unwrap_or_default().max(local));
        // An unrepresentable Retry-After pauses for correction rather than
        // silently shortening a valid upstream delay or spinning on an error.
        let permanent=permanent || (retry_after.is_some() && remote.is_none()) || retry.is_none();
        let retry=retry.unwrap_or(now+Duration::hours(24));
        jobs::run(&tx,tx.query("UPDATE actor_search_control:catalog SET failures=$failures,retry_at=<datetime>$retry,blocked=$blocked,last_error=$reason RETURN NONE")
            .bind(("failures",failures)).bind(("retry",retry.to_rfc3339())).bind(("blocked",permanent)).bind(("reason",if permanent {"actor_index_correction_required"} else if cooling {"actor_index_cooldown"} else {"actor_index_retry"}))).await?;
        jobs::run(&tx,tx.query("UPDATE actor_search_rebuild:catalog SET reason_code=$reason WHERE phase!='completed' RETURN NONE")
            .bind(("reason",if permanent {"actor_index_correction_required"} else {""}))).await?;
        Ok(())
    }}).await?;
    Ok(())
}
