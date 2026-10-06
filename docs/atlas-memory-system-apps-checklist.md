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
| 3 | Atlas extraction checkpoint, phase 5, truthful state/counts, repair/resume | **Done (core)** | See phase 3 below; TUI resume/repair buttons and Brain provenance are phase 5 |
| 4 | Intel Recon / other memory mutation paths reuse publication + refresh | Not started | |
| 5 | TUI: Brain refresh, read errors, Jobs/Logs views, renames, Related/Summary layout | Not started | Default worker pools are spawned by the TUI; phase 3 added the minimal Atlas hooks (memory reload on `MemoriesChanged`/`AtlasDone`, stored run state, phase-5 row) |
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

### Phase 3 — what landed

Pre-phase work: rebuilt and reran core tests after the `rustfmt` pass on `store/publication.rs`
(399 passed), and added the fault-injection test
`text_changed_mid_index_write_reruns_on_the_new_revision`. A test-only hook fires between the Lance
write and the compare-and-record step and changes the memory text there. The test asserts that the
stale revision is not recorded (`pending`), that the rerun indexes the new revision, and that the
served vector matches the new text (commit `ced3514`).

- **New module `atlas_memory.rs`.** Holds the phase 4/5 lifecycle and the repair job. `atlas.rs`
  only orchestrates it.
- **Extraction checkpoint.** After extraction succeeds, phase 4 saves the validated payload
  (claims, relations, brief, insight stats, partial flag, notes) to `argos_atlas_checkpoints`
  under its payload revision, then moves the cursor to `publish`. On resume, an existing
  checkpoint is used and extraction never reruns. This also covers a crash between the
  checkpoint commit and the cursor save.
- **Publication from the checkpoint.** `publish_checkpoint` goes through the transactional
  publisher using the checkpoint revision. `insights_done` is no longer written after a failed
  write: the cursor reaches phase 5 only once a receipt exists. Runs left at the legacy
  `insights_done` marker are routed to phase 5 for verification.
- **Phase 5 "Index and verify memories".** `index_and_verify` makes one inline attempt per
  required memory through the same leased outbox tasks (`requeue_uncovered` + `index_now`), then
  only polls verification for a bounded number of rounds. It does not use up the task's retry
  budget; backoff stays with the index pool. Its scope is the run's own required memories, so
  unrelated backlog cannot block it. Coverage is checked by exact id and revision.
- **Independent child states.** `RunStats.memories: MemoryPhase` is additive (old JSON still
  parses). It tracks extraction, publication and indexing separately (`pending/running/completed/
  partial/blocked/failed/skipped`), plus extracted vs accepted/created/reused/repaired/rejected,
  brief, required/indexed/pending and a detail string. `finish()` derives `atlas_runs.state` from
  these: `completed`, `partial`, `blocked` or `failed`. `Stop::Failed` is returned only for
  `failed`.
- **Truthful counts and lines.** The phase-5 row shows
  `Memories: N created · M reused · i/r indexed`. A failed save shows `Memories: not saved — …`,
  and extraction counts are never shown as saved counts. With embeddings disabled the row shows
  `… · Saved; semantic indexing disabled` and the run state is `blocked`. Zero accepted insights
  after a genuine extraction is a `completed` no-op. Missing synthesis config or a decisions
  model gives `blocked`, and an extraction error gives `failed`; neither is treated as "zero
  findings".
- **Resume.** `run_atlas` can resume paused runs, and also `failed/partial/blocked` runs stopped in
  phase 4/5. The new `RunInput.run_id` targets a specific run. Phase 5 resumes from the stored
  receipt (`atlas_publication_receipt`), and pause works inside phase 5. Publication is
  idempotent per revision, so a resume does not duplicate work.
- **Automatic upgrade.** `refresh_run_indexing` / `refresh_incomplete_runs` re-verify
  `partial`/`blocked` phase-5 runs. The index pool calls this after it drains work (or about every
  30 s), so runs reach their verified status once background retries succeed or embeddings are
  re-enabled. Only memory-derived state changes.
- **Repair Atlas memories job.** `repair_run`, `repair_page`, `run_repair_job` (resumable keyset
  cursor in `app_state`, registered in `argos_jobs` with progress), `spawn_repair` (user-triggered,
  `restart=true`) and `spawn_startup_reconciliation` (one-time after migration; resumes an
  interrupted pass; the TUI calls it at startup). Behavior per run:
  - With a checkpoint, the run is republished in repair mode. Missing rows and orphan links are
    restored from authoritative claim text. Tombstones are respected, including a deleted brief.
  - Without one, a receipt is reconstructed from the stored links (revision `legacy`, state
    `reconstructed`). Links whose claim text is lost are reported as unrecoverable, never
    fabricated.
  - Missing or stale vectors are requeued through the same outbox.
  - Status is recorded in the new `argos_atlas_repairs` table; historical run state and dates are
    not rewritten.
  - Notable outcomes become `argos_events` (`atlas.memory_repair`).
  - A run that lost its whole payload reports `Extraction data missing; re-extraction required`.
    `prepare_reextraction` rewinds that run to phase 4 using its retained articles.
- **Retention.** Checkpoints, receipts and repair rows are deleted with the run, both on cycle
  deletion and on 36 h pruning.
- **TUI hooks (minimal, rest in phase 5).** New `AtlasEvent::MemoriesChanged` / `MemoryProgress`.
  Brain memories reload on `MemoriesChanged` and on `AtlasDone`. `AtlasDone` takes the run state
  and note from the DB instead of hard-coding "completed". The live insights pane and
  history/cycle stats show the phase-5 row.
- **Test seams.** `atlas_memory::testing::mock_extraction` (mocked synthesis),
  `store::publication_fault::fail_publish` (fails right before COMMIT, after rows were written),
  `embed::testing::disable()` (behaves like `ARGOS_EMBED=0`).

## §10 acceptance checks

| # | Check | Status | Evidence / gap |
| --- | --- | --- | --- |
| 1 | Atlas run creates visible memories in-session | Partial | Core: `successful_run_publishes_indexes_verifies_and_completes` (disk store; memories in `list_memories`, `MemoriesChanged` emitted). TUI reloads on `MemoriesChanged`/`AtlasDone`; full Brain refresh semantics (selection/filter preservation) and screenshot are phase 5 |
| 2 | Brain entry refreshes externally committed memories | Not started | Phase 5 |
| 3 | Publication failure cannot report completed; retry uses saved payload | **Done (core)** | `publication_failure_is_failed_not_completed_and_retry_uses_the_checkpoint`: state `failed`, extracted 2 vs created 0, rollback, resume completes with extraction called once |
| 4 | Embedding/Lance failure: memories visible, phase 5 incomplete, retry reaches verified | **Done (core)** | `embedding_failure_keeps_memories_visible_reports_partial_and_retry_verifies` (run `partial`, `0/3 indexed`, resume reaches `3/3`), `background_indexing_then_refresh_upgrades_a_partial_run`; UI screenshot phase 5 |
| 5 | Embeddings disabled: honest status; enabling resumes | **Done (core)** | `disabled_embeddings_save_memories_and_say_so_honestly` ("Saved; semantic indexing disabled", never claims indexed, work stays queued), `enabling_embeddings_resumes_outstanding_work_and_upgrades_the_run` |
| 6 | Kill/restart at each boundary | Partial | Outbox claim/lease expiry (phase 1/2 tests), extraction checkpoint (`crash_after_checkpoint_resumes_without_reextracting`), SQLite commit (rollback + retry test), phase-5 pause/resume from receipt (`pause_in_phase5_parks_then_resumes_from_the_stored_receipt`), index write (mid-write test). Not simulated: a process kill between the Lance write and the task acknowledgement (covered logically by revision-aware recording + lease expiry, but no dedicated test) |
| 7 | Duplicate claims reuse canonical memories, keep links, repair vectors | **Done (core)** | `publication_receipt_reconciles_and_reports_rejections`, `retrying_the_same_payload_is_idempotent`, `reused_claims_with_damaged_rows_are_repaired_and_reindexed`, `indexing_is_verified_by_exact_id_and_revision` |
| 8 | Change/delete during indexing cannot overwrite newer/resurrect | **Done (core)** | `stale_work_cannot_overwrite_a_newer_revision`, `deletion_during_indexing_is_not_resurrected`, `text_changed_mid_index_write_reruns_on_the_new_revision` (fault-injected) |
| 9 | Historical repair | **Done (core)** | `repair_requeues_vectors_for_a_false_completed_cycle_without_rewriting_it`, `repair_restores_orphaned_links_from_the_checkpoint_and_respects_tombstones`, `repair_reports_missing_payload_and_unrecoverable_links_without_fabricating`, `repair_job_pages_resumably_and_records_job_progress`. User-triggered repair button / re-extraction action in the TUI: phase 5 |
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
- Phase 3: `ARGOS_EMBED=0 cargo test -p argos-osint-core --lib` → 415 passed, 0 failed, 5 ignored
  (399 after the publication.rs rustfmt rebuild, including the new mid-write test; +16 in
  `atlas_memory`). `ARGOS_EMBED=0 cargo test -p argos-osint-bin` → 59 passed.
  `cargo clippy --workspace --all-targets` shows no findings in files touched by this branch apart
  from two that were already there (`format_article_card`, `run_live` argument counts). Other
  pre-existing clippy findings elsewhere mean `-D warnings` CI fails on `main` too.
