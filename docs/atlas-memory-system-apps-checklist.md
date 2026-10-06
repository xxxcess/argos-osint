# Atlas memory completion and System apps — implementation checklist

Spec: `specs/argos-atlas-memory-system-apps-spec.md` (agent box copy), reviewed against `main` @ `44214c0`.
Branch: `feat/atlas-memory-system-apps` · PR: https://github.com/xxxcess/argos-osint/pull/37

Work proceeds in the spec's §9 order, one phase per push. Status is honest:
**Done** = implemented and covered by tests; **Partial** = some of the requirement landed;
**Not started** = nothing yet.

## §9 implementation order

| # | Area | Status | Notes |
| --- | --- | --- | --- |
| 1 | Core `tasks.rs`, `scheduler.rs`, `store.rs`: typed outcomes, worker routing, safe claims/leases, additive schema | **Done** | See phase 1 below |
| 2 | Transactional outbox, revision-aware indexing, coverage receipts | **Done** | See phase 2 below; Recon (`recon.rs`) still uses best-effort hooks → phase 4 |
| 3 | Atlas extraction checkpoint, phase 5, truthful state/counts, repair/resume | Not started | Checkpoint/receipt/tombstone tables exist (schema only) |
| 4 | Intel Recon / other memory mutation paths reuse publication + refresh | Not started | |
| 5 | TUI: Brain refresh, read errors, Jobs/Logs views, renames, Related/Summary layout | Not started | Default worker pools are now spawned by the TUI |
| 6 | Shared job registration for all async entry points | Not started | `enqueue_job_with` + `JobMeta` available |
| 7 | Typed provider diagnostics, unified graph explanation, bounded transport, revision cache | Not started | |
| 8 | CLI/README/docs/tests | Not started | |

### Phase 1 — what landed

- **Pool routing.** `tasks::pool_for_operation` routes every operation to exactly one pool
  (`summary`, `index`, `atlas`, `network`, `llm`); `argos_tasks.pool` is written at enqueue and
  backfilled for older rows. Workers claim only their pool. The summary worker's
  `skipped_non_summarization` completion is gone.
- **Atomic claims.** `claim_next_in` is a single `UPDATE … WHERE id=(SELECT …) RETURNING`, so
  only a row this call actually leased is returned. Legacy `claim_index_changes` uses the same
  pattern per row.
- **Owner/epoch guards.** `complete_claimed`, `fail_claimed`, `block_claimed`, `requeue_claimed`
  and `renew_lease` all require the current `(lease_owner, lease_epoch)`; a stale worker cannot
  publish.
- **Lease recovery.** `interrupt_expired_leases` closes the open attempt as `lease_expired`,
  requeues while attempts remain, and fails once the cap is reached (no permanently running
  tasks). It covers index work too, because each outbox row now references one leased task.
- **Typed errors.** `TaskError { category, message }`; new categories `index_failure`,
  `configuration_missing`, `lease_expired`. Worker errors are recorded as failures, never as a
  completion result string.
- **Index outbox on the task machinery.** `enqueue_index_work` inserts/coalesces an
  `argos_index_changes` row by work key (kind + id + revision + operation) and creates its leased
  task; it issues plain statements so it can run inside a publisher's transaction.
  `claim_index_work` / `finish_index_work` acknowledge per typed `IndexOutcome`
  (`ready`, `pending`, `disabled`, `retryable_failure`, `permanent_failure`). Disabled embeddings
  park the task (`paused`/`embeddings_disabled`, attempt refunded) and `resume_blocked` requeues it
  once vectors are enabled. Pre-existing pending rows are adopted into tasks.
- **Typed store indexing.** `Store::try_index_upsert`, `try_index_remove_missing`,
  `try_process_vector_rebuild` return `IndexOutcome`; a rebuild batch is `pending`, not completion.
  `scheduler::apply_index_change` returns the typed outcome (no more `"upserted:…"`).
- **Timing.** Tasks record `started_at`, `heartbeat_at`, `finished_at`, accumulated `active_ms`,
  first-claim `queue_ms` and `retry_wait_ms`; attempts record owner, `duration_ms`, outcome and
  error. `refresh_job_state` derives parent job state (`partial` when some children failed) and
  timing from durable tasks; unknown timing stays NULL ("Unavailable"), not zero.
- **Additive schema (v19).** Job metadata/timing columns, task pool/timing columns, attempt
  outcome columns, outbox lease columns, `argos_events`, `argos_atlas_checkpoints`,
  `argos_atlas_publications`, `argos_memory_tombstones`. Uses the existing `user_version`
  migration chain; the newer-schema refusal test is now version-relative.
- **Durable events.** New `events.rs`: `record_event` (redacts API keys, auth headers, sensitive
  query parameters, credentialed URLs, known key prefixes; bounds detail), `list_events` with a
  job-and-descendants filter, `get_event` (None ⇒ expired), `prune_events`/`clear_events` touch
  only `argos_events`.
- **Workers actually run.** Before this branch nothing spawned `WorkerPool`; the TUI now spawns
  the index and summary pools (`WorkerPool::spawn_default`) with one per-process owner id and
  scheduler election.

### Phase 2 — what landed

- **Transactional publisher.** New `store/publication.rs`: `Store::publish_atlas_insights` runs one
  `BEGIN IMMEDIATE` transaction that validates claims (invalid ones become `RejectedClaim` with a
  reason, never silently dropped), upserts canonical memories and claim/source links by
  fingerprint, restores a missing canonical memory row from the authoritative claim text
  (`repaired`), publishes/updates the cycle brief, queues index-outbox work for every required
  memory whose current revision is not recorded as indexed (reused claims included), persists the
  `PublicationReceipt` in `argos_atlas_publications`, and bumps a durable `memories_changed_seq`.
  Any failure rolls everything back, outbox rows included. `persist_atlas_insights` is now a thin
  wrapper that returns the receipt (Atlas phase 4 is unchanged until phase 3).
- **Receipts.** Run/revision (defaults to a payload hash, so retries are idempotent), input vs
  accepted counts, created/reused/repaired ids, updated ids, rejected claims, brief id, per-claim
  `(fingerprint, memory id)`, affected ids, queued outbox rows/tasks, already-indexed count.
  `reconciles()` checks created + reused + repaired = accepted and accepted + rejected = input
  (brief counted separately).
- **Revision-aware indexing.** Revision = SHA-256 of the memory text. `argos_memory_index_state`
  records revision, embedding fingerprint and serving generation per memory.
  `Store::try_index_memory` reads text+revision, embeds/writes outside any SQLite write lock,
  verifies the exact id in the serving Lance table (`BrainIndex::present_ids`, a filter scan, not
  top-k), then records the revision only if the memory still has it; a revision change mid-write
  returns `pending` so the task reruns on the newest text. A newer indexed revision satisfies older
  work. Rebuilds/reindex/incremental reconciliation record states; legacy vectors without a
  recorded revision are adopted instead of being re-embedded on the recall path.
- **Verification.** `verify_memory_coverage` (exact id + current revision; reports missing rows,
  pending reasons, disabled), `requeue_uncovered`, `verify_atlas_publication` (claim → memory,
  source links, brief, reconciliation, coverage).
- **Tombstones.** `delete_memory` records `argos_memory_tombstones` (fingerprint + the cycles it was
  linked to) and queues durable vector removal in the same transaction. Retries of those cycles and
  any repair-mode publication reject tombstoned fingerprints; a later cycle with new evidence may
  publish the claim again. Late upsert work for a deleted memory removes its vector.
- **Store mutation paths on the outbox.** `add_memory`, `update_memory`, `delete_memory`,
  `delete_article_insights`, `commit_article_insight_replacement` and `delete_cycle_memories` queue
  durable work inside their transaction, then (except cycle deletion, which runs inside a caller's
  transaction) attempt it immediately via `Store::index_now`, which claims exactly those leased
  tasks and acknowledges them through the same typed path.
- **Index worker uses the revision-aware path** (`Store::apply_index_work`). `get_memory` now
  fetches by id directly instead of scanning the list.
- **Fault injection.** `embed::testing::fail()` makes embeddings fail on the current thread.

## §10 acceptance checks

| # | Check | Status | Evidence / gap |
| --- | --- | --- | --- |
| 1 | Atlas run creates visible memories in-session | Not started | Phase 3/5 |
| 2 | Brain entry refreshes externally committed memories | Not started | Phase 5 |
| 3 | Publication failure cannot report completed; retry uses saved payload | Partial | Publication is atomic and rolls back memories/links/outbox (`failed_publication_rolls_back_memories_links_and_outbox`); Atlas run state + checkpoint retry is phase 3 |
| 4 | Embedding/Lance failure: memories visible, phase 5 incomplete, retry reaches verified | Partial | Core done: `embedding_failure_keeps_memories_visible_and_retry_reaches_verified`; Atlas phase-5 row/UI pending (phase 3/5) |
| 5 | Embeddings disabled: honest status; enabling resumes | Partial | Disabled ⇒ blocked + `resume_blocked` (`disabled_outcome_blocks_without_spending_attempts_and_resumes`, `index_drain_on_store_without_vectors_blocks_honestly`); UI status pending |
| 6 | Kill/restart at each boundary | Partial | Outbox claim / lease-expiry boundaries covered (`expired_index_lease_recovers…`, `two_processes_cannot_own…`, `index_outbox_coalesces_and_rolls_back…`); checkpoint/commit boundaries pending |
| 7 | Duplicate claims reuse canonical memories, keep links, repair vectors | **Done (core)** | `publication_receipt_reconciles_and_reports_rejections`, `retrying_the_same_payload_is_idempotent`, `reused_claims_with_damaged_rows_are_repaired_and_reindexed`, `indexing_is_verified_by_exact_id_and_revision` |
| 8 | Change/delete during indexing cannot overwrite newer/resurrect | **Done (core)** | `stale_work_cannot_overwrite_a_newer_revision`, `deletion_during_indexing_is_not_resurrected`; mid-write revision change returns `pending` (code path, not separately fault-injected) |
| 9 | Historical repair | Not started | Phase 3 |
| 10 | Summary worker cannot claim Atlas/index tasks; two processes cannot own one attempt | **Done** | `summary_pool_never_claims_index_or_atlas_work`, `summary_worker_leaves_index_and_atlas_tasks_alone`, `two_processes_cannot_own_the_same_attempt_and_stale_owner_cannot_publish` |
| 11 | Jobs timing/history for every async op | Partial | Durable timing + aggregate state (`job_timing_and_partial_aggregate_come_from_durable_tasks`); registration of all entry points and UI pending |
| 12 | Job↔log navigation after restart; redaction | Partial | `argos_events` + redaction + job-descendant filter tested; navigation UI pending |
| 13 | Home System category / nine routes | Not started | Phase 5 |
| 14 | Tools/Models retain behavior; System hardware/paths only | Not started | Phase 5 |
| 15 | Targeted tests / CI gates | Partial | Per-phase runs recorded below; real local Lance fixture (fake embedder) used for exact id/revision checks |
| 16–22 | Graph-summary diagnostics, budget, cache, retry | Not started | Phase 7 |
| 23–26 | Claim/Recon layout, Related navigation | Not started | Phase 5 |

## Test log

- Phase 1: `ARGOS_EMBED=0 cargo test -p argos-osint-core --lib` → 390 passed, 0 failed, 5 ignored.
  `cargo check -p argos-osint-bin --tests` clean apart from pre-existing dead-code warnings.
- Phase 2: `ARGOS_EMBED=0 cargo test -p argos-osint-core --lib` → 398 passed, 0 failed, 5 ignored.
  `ARGOS_EMBED=0 cargo test -p argos-osint-bin` → 59 passed.
