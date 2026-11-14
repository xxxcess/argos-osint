# SQLite Jobs cutover handoff

## Agreed outcome and current boundary

User selected SQLite as durable authoritative storage for **all operational job data**: scheduling, tasks, attempts, leases, retries, progress, logs and operational telemetry. SurrealDB retains product/domain data and the minimum domain intents, publication fences and applied-operation receipts required for atomic domain correctness. Delivery uses a Jobs-owned RocksDB embedded queue paired with SQLite operational state. Only the Jobs service opens SQLite, on a persistent local volume. Workers and other services use authenticated private HTTP. On 2026-11-12 the user confirmed a fresh deployment with no existing jobs or queue data to migrate; earlier drain-first rollout requirements are superseded.

The verified checkpoint includes SQLite store primitives, standalone operational `argos_event` logs, the Jobs-owned RocksDB queue and the Atlas HTTP delivery adapter. This is **not full job isolation**. The scheduler, domain-integrated task transitions, process recovery, `job_event` lifecycle logs, attempts and telemetry remain Surreal-backed. Never describe generic store methods as an active production scheduler merely because they exist. Verification outcomes belong in acceptance/progress records; this handoff makes no pass claim.

Network clients fail their log operation on a configured Jobs RPC failure; they do not fall back to Surreal. Legacy/unconfigured embedded clients retain their existing sink. Historical Surreal log rows are not automatically copied. Logs filtered by a job retain descendant expansion and the existing domain `job_event` merge, so historical lifecycle events remain available through that route.

## Current integration interfaces

- `fleetworks-jobs::Store` is the disk-backed SQLite owner. WAL, FULL synchronization and a patched bundled SQLite are required. Writes run on blocking workers with bounded in-flight access. Separate `logs` storage has no foreign keys to unmigrated Surreal jobs/tasks.
- `storage::jobs_client::{install_local, initialize_remote_from_environment, configured, execute}` installs the local Store in the Jobs role or an HTTP connection elsewhere. Configuration is `FLEETWORKS_JOBS_ORIGIN`, plus `FLEETWORKS_JOBS_TOKEN` or `_FILE`; the private route is `POST /internal/jobs/logs` with `LogCommand`. A bearer-protected `POST /internal/jobs/store` exposes the foundation typed commands, but production workers do not use it yet; the host supplies authoritative time and replay identity excludes timestamps.
- `storage::event_log` redirects standalone append/list/get/count/apps/prune/clear calls. `investigation_event` remains a domain trace and is untouched. Every existing producer passing through `record_event` uses the configured sink.
- The existing public Jobs/Logs response contracts and private authentication remain authoritative. Keep job-filtered `job_event` merge and sequence cursor behavior while the scheduler remains in Surreal.

## Remaining implementation sequence

### 1. Define the full service boundary before switching writers

Introduce an async operational backend API used by existing scheduler and worker callers: idempotent submission, claims, lease renewal, finish/yield, retry/cancel, job/task/attempt reads, progress, process registration/recovery, delivery outbox and retention. Use typed owned DTOs; do not expose raw SQL or Surreal handles. Preserve owner, epoch, workspace generation, input hash, dependency and retry-cap semantics. Mutation replay must return the original result after an ambiguous HTTP response.

The service owns authoritative time. Wire clients must not choose claim/renew/cancel/completion time or extend leases through an arbitrary `now_ms`. Stable operation keys and canonical payloads must resolve retries without treating server timestamps as a new request. The foundation store RPC supplies service time and excludes timestamps from replay identity; the full production scheduler still needs the complete validated domain contract.

Refactor together: core `tasks.rs`, `scheduler.rs`, `job_registry.rs`, storage `jobs`, `jobs_view`, `app_jobs`, `recon_jobs`, `processes`, `job_dispatch`, and host `atlas_worker`. Keep the public dashboard contract while changing persistence. Do not relocate process heartbeats alone: `processes::recover_orphans` currently reads freshness and requeues Surreal task epochs atomically; splitting those reads/writes would falsely recover healthy workers. Task-less job progress/cancellation also depends on these existing tables.

### 2. Replace domain/job transactions with durable handoffs

Inventory and explicitly port each transaction-scoped helper caller; an HTTP call cannot replace a helper inside a domain transaction. Important callers include:

- `atlas_derivative_worker::{enqueue_in, Fence::check_in, worker completion/yield/control}` and `storage::classifier_admission` admission and terminal hooks.
- `storage::classifier_calibration` capture/job submission and derivative authority checks.
- `storage::brief_stages::{commit, ...}` and `brief_stages::publication::{process,current}`.
- `storage::job_dispatch::submit` and its Surreal queued-task event/outbox.
- `osint::manual::admit`, recon attempts, and neighborhood publication.

Domain mutations that require follow-up jobs commit a small deterministic command intent/outbox **in the same Surreal transaction**. A relay submits it idempotently to Jobs and acknowledges only after SQLite durable acceptance. Domain references to operational jobs become stable IDs rather than dereferences of Surreal job/task records. Retain business quota, credit and paid-call admission ledgers with domain correctness; they are not disposable telemetry.

### 3. Preserve publication and cancellation ordering

Current code checks lease owner, epoch, expiry, cancellation and source generation inside the same Surreal transaction as domain output. Checking a remote SQLite lease and then writing Surreal creates a cancellation/expiry/takeover race. An epoch mirror alone also fails when a newer epoch has not yet reached Surreal.

Use Jobs-owned durable publication proposals, domain-owned generation fences and domain applied-operation receipts. Workers submit publication proposals; the Jobs coordinator serializes publication, cancellation and ownership changes for the same job/root. Do not acknowledge cancellation/takeover as effective until any preceding proposal is resolved or its domain authority is revoked. Domain effects and their receipt commit atomically. A lost reply replays the receipt, never the external effect. SQLite finalization follows durable domain acceptance and is replayable after a crash.

Before accepting new claims after restart, reconcile unresolved proposals. Already applied domain receipts finish the SQLite transition idempotently; unapplied proposals must pass persisted lease-expiry, cancellation, source-generation and ownership authority before publication. A proposal cannot extend an expired lease merely by being queued. Brief generation/source replacement and deletion fences remain domain checks. Preserve the existing rule that interrupted paid calls are not blindly reissued.

This protocol requires integration tests at the real boundary. Do not replace current fences with best-effort RPC checks or claim exactly-once effects from delivery acknowledgments.

### 4. Move telemetry and its readers together

Port `storage::telemetry` writes, prune/coverage/prefix queries and Profile rollup/read paths together. Existing writes update raw rows and hourly/daily rollups atomically with first-seen idempotence. Moving writes alone silently empties Profile analytics. Similarly preserve lifecycle log ordering/filtering and job detail attempt data when removing Surreal operational tables.

### 5. Initialize a fresh deployment

Initialize fresh persistent operational stores and a backend generation. Preflight validates writable SQLite storage, schema, authenticated service availability and the complete domain handoff/fencing protocol before enabling SQLite job submissions. The deployment has no pre-existing work to migrate or drain. Keep one scheduling authority for each job generation.

After initial setup, restart preserves the recorded generation, acknowledged work, pending intents and publication receipts. Reconcile unfinished effects before new claims; stale-generation deliveries cannot execute. An empty query result or unavailable service never authorizes reinitialization, deleting operational records or running both scheduling backends.

## Acceptance before declaring full isolation

- Disk reopen and process-kill recovery preserve acknowledged jobs/logs, receipts, attempts and pending outbox work; incompatible schema fails startup.
- Multiple workers racing a claim produce one owner/epoch. Expired owners cannot renew, finish, publish or resurrect authority. Dependency order, bounded retries and cancellation of descendants match current behavior.
- Lost responses to submit/claim/finish/cancel/dispatch reuse stable keys and return original committed results. Duplicate queue deliveries do not duplicate execution effects.
- Crash tests cover before/after domain intent commit, SQLite acceptance, domain publication receipt and SQLite finalization. Cancellation, lease expiry, takeover, generation replacement and source deletion race publication through controlled barriers.
- Process recovery preserves the existing rule that datastore/service unavailability is not proof of dead workers. No false orphan recovery after outages.
- Logs retain redaction/bounds, filters, descendant selection, cursors, counts, apps and retention. Profile raw/rollup values retain first-seen idempotence and coverage behavior.
- Fresh-start and restart tests prove the generation is initialized once, acknowledged work and durable intents survive restart, stale-generation deliveries cannot execute and only one scheduler has authority.
- Authorized isolated startup smoke uses synthetic workspace/credentials and leaves the user's stack untouched. A concurrent synthetic job/log load test measures core-domain write latency and verifies operational polling/leases/progress no longer write Surreal job tables. Shared host/disk contention still exists; do not promise physical isolation.

Only after these integration contracts pass should docs call SQLite the authoritative production job backend or remove Surreal operational schema/compatibility paths.

## Updated scope: embedded transport

See [Jobs persistence](../../docs/wiki/code/jobs-persistence.md) for canonical ownership and phases. The embedded queue uses the existing compatible Surreal RocksDB binding, sync WAL, indexed ready/inflight/dead state, owner/epoch visibility and idempotent mutation receipts. Atlas uses the Jobs HTTP queue while existing Surreal execution fences remain. Direct Docker setup provisions a fresh backend. These changes do not complete the full SQLite scheduler and telemetry port; existing runtime compatibility guards are unchanged by this documentation decision.
