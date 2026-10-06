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
| 2 | Transactional outbox, revision-aware indexing, coverage receipts | **Done** | See phase 2 below; Recon moved onto the outbox in phase 4 |
| 3 | Atlas extraction checkpoint, phase 5, truthful state/counts, repair/resume | **Done (core)** | See phase 3 below; TUI resume/repair buttons and Brain provenance are phase 5 |
| 4 | Intel Recon / other memory mutation paths reuse publication + refresh | **Done** | See phase 4 below |
| 5 | TUI: Brain refresh, read errors, Jobs/Logs views, renames, Related/Summary layout | **Done** | 5a: Home order/renames, nine routes, System = hardware + paths, Jobs and Logs dashboards with job↔log navigation. 5b: Brain refresh keeping selection/Find, memory read-error and empty states, claim/Recon detail layout (graph above, Related left, Summary right; stacked when narrow) with by-id related navigation and Back history, Atlas Resume / Repair memories buttons, Brain claim-detail screenshots. See the phase 5a/5b sections below |
| 6 | Shared job registration for all async entry points | **Done** (gaps listed) | Core `job_registry.rs` + TUI `tracked.rs`; inventory and gaps in "Phase 6 — what landed" below |
| 7 | Typed provider diagnostics, unified graph explanation, bounded transport, revision cache | **Done** (gaps listed) | Core `provider_diag.rs`, `provider_attempt.rs`, `summarization/exec.rs`, `graph_explanation.rs`, `store/graph_summaries.rs`; TUI `summary_card.rs`; details in "Phase 7 — what landed" below |
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

### Phase 4 — what landed

- **No best-effort index hooks left.** `Store::index_upsert`, `index_remove_missing` and
  `after_memory_write` are gone, along with Intel Recon's non-atomic `retag_memory_source`. Every
  memory mutation now commits its index-outbox rows (and the `memories_changed_seq` bump) in the
  same SQLite transaction as the change. Any immediate indexing after commit goes through
  `Store::index_now`, i.e. the same leased tasks the index pool runs.
- **`add_memory` / `update_memory`** (including the `insight_user_edits` row) go through a new
  `write_then_index` helper. It opens `BEGIN IMMEDIATE` when autocommit, or joins the caller's
  transaction. If the outbox insert fails, the memory write rolls back.
- **Recon (`recon.rs`).**
  - `persist_claims` queues outbox work inside its transaction for every memory the claims resolve
    to. Reused claims are re-verified, so a lost vector is repaired on reuse. It bumps the change
    counter when it writes and never recreates a claim the user deleted from the same run (the
    tombstone check).
  - `delete_thread` queues durable vector removal for the memories it deletes and bumps the counter
    for both deletes and provenance re-tags in that transaction.
- **Intel Recon (`intel_recon/brain.rs`).** `upsert_recon_insights` publishes through
  `publish_atlas_insights` with new `PublishOptions`:
  - `retag_app: "intel-recon"`: provenance is applied to new and reused memories in the same
    transaction. `retag_source` keeps Atlas provenance and adds the reference
    `atlas+intel-recon:{run}`. Any other provenance is re-attributed.
  - `skip_receipt: true`: no `argos_atlas_publications` row is written, so an Atlas cycle's
    receipt is never shadowed.

  `intel_recon/replace_insights.rs` already used `commit_article_insight_replacement`, which has
  been durable since phase 2. Other memory writers (`delete_memory`, `delete_article_insights`,
  `delete_cycle_memories`, the Atlas publisher) were already on the outbox. Audit: every
  `INSERT/UPDATE/DELETE` on `memories` outside tests now runs in a transaction that queues the
  outbox.
- **Refresh notifications.** New typed `store::MemoriesChanged { seq }` and
  `MemoryChangeWatcher`. Both are derived from the durable commit counter, so they fire only for
  committed changes, include commits from other processes and coalesce bursts. The TUI main loop
  polls the watcher about once per second and reloads Brain memories. Full Brain refresh semantics
  (keeping selection and filters, read-error states) are phase 5.
- **Kill test.** `worker_killed_after_the_lance_write_is_recovered_by_revision`:
  1. Worker A claims the index task, writes the vector to Lance and "dies" without recording or
     acknowledging.
  2. The text changes again while A is dead.
  3. The lease expires (`interrupt_expired_leases`) and worker B drains.

  The test asserts that the recorded revision equals the newest text, coverage is complete, no
  active outbox rows remain, there is exactly one vector for the id, and search serves the newest
  text. A's late acknowledgement is rejected by the owner/epoch guard and does not change the
  recorded revision.
- **Left as-is.** `ensure_memory_vectors`, the recall-path reconciliation, still writes Lance
  directly. It records revision states and is a self-healing reconciler, not a mutation path.

### Phase 5a — what landed

Phase 5 was split. 5a covers navigation, renames, System, Jobs and Logs; 5b covers Brain
refresh/read errors, the claim/Recon detail layout with related navigation, and the Repair/Resume
buttons.

- **Home order and renames.** `ModuleId::ALL` is now Intel, Atlas, Brain, Recon, Jobs, Logs,
  Osint, Providers, System. Home groups them as Applications (Intel, Atlas, Brain, Recon) and
  System (Jobs, Logs, Tools, Models, System). `Osint` shows as "Tools" and `Providers` as
  "Models". Internal ids, settings keys and stored values stay the same. Number keys 1–9, the
  header tabs, palette, slash commands and help follow the same order. Old aliases (`osint`,
  `providers`) still work next to the new `tools`, `models`, `jobs`, `logs`.
- **Core read model `jobs_view.rs`.** `list_jobs` returns top-level jobs only: active non-service
  work first, then everything else, service workers last. It filters by status, app and search.
  Also `job_counts`, `job_detail` (children, a bounded task list, ≤20 attempts, retryable flag,
  expired-events note), `retry_failed_tasks` (only failed INDEX/SUMMARY operations; resets
  attempts, re-pends outbox rows, records `job.retry`), `event_counts` (with failures in the last
  hour) and `event_apps`. `format_duration(None)` is "Unavailable". `Store` wrappers for these
  plus `register_job`, `set_job_progress`, `record_event`, `list_events`, `prune_events` and
  `clear_events`.
- **Jobs dashboard (`tui/jobs.rs`).** Has its own selection (kept by job id across reloads),
  scroll, detail scroll, status/app/search filters and counts. Wide terminals (≥100 cols) show the
  table and detail side by side. Narrow terminals show one panel: Enter opens the detail, Esc
  closes it. The table shows status, job, app, phase/progress, active, elapsed, attempts and a
  log marker. Detail shows timestamps, timing (unknown values read "Unavailable"), resources,
  latest error, phases/children, tasks and attempt history. Actions: View logs; Retry failed only
  when retryable work exists; Open source only when the job resolves to an Atlas run or Recon
  thread.
- **Logs dashboard (`tui/logs.rs`).** Replaces the in-memory System event log. Reads durable
  `argos_events` (24 h retention, pruned on the first dashboard tick) with its own selection,
  folding, scroll, level/app/job/search filters and Follow. Selecting a row other than the newest
  pauses Follow. Counts cover errors, warnings, info and failures in the last hour. Clear events
  removes events only; jobs stay.
- **Job↔log navigation.** In Jobs, `l` / View logs opens Logs filtered to the job and its
  descendants, with Back / Esc returning to the same job. In Logs, `o` / Open job selects the
  event's job, clearing any Jobs filters that would hide it. Both dashboards reload every second
  while open (`tick_dashboards`). Otherwise only counts refresh, for the Logs error badge.
- **System is hardware + paths.** It has a host pane, a paths pane (config and database always;
  data, memory index, credentials and hardware cache only when present) and one Refresh hardware
  button. `push_log_detail` (tool calls, settings saves, Atlas faults) now writes durable events.
  The error badge moved from System to Logs on Home and in the header.
- **Tests.** New TUI tests: `home_order_renames_and_nine_routes_agree`,
  `jobs_dashboard_navigates_to_logs_and_back_and_logs_open_jobs`,
  `system_shows_only_hardware_and_paths_and_logs_own_clear`. Existing log tests were rewritten
  against durable events. The ignored `dump_phase5_screens` writes TestBackend cell dumps, and
  `scripts/render_tui_cells.py` renders them to PNG (fixture data, not a live
  run).

### Phase 5b — what landed

- **Core `related_memories.rs`.** `Store::related_memories(id, RelatedLimits)` returns
  `RelatedMemory {memory_id, title, provenance, reason, kind, score}`. Explicit links rank first
  (stored claim relations such as conflict/revision, same source article or cited tool result,
  same entity, same investigation; up to 12), then up to 5 `Similar` rows whose reason says "not
  evidence". Every row resolves to an existing memory; self and duplicates are dropped; a missing
  memory is an error. `Store::memory_count` backs the "N of M" Find title.
- **Brain list.** `reload_memories` keeps the selected memory by id, the Find text and scroll, and
  reloads on Brain entry. A read failure keeps the last loaded list, titles it "read failed ·
  showing last loaded list", logs one event and sets a status message. Empty states distinguish
  loading, no memories, and "No memories match … Find is still active".
- **Detail layout (`tui/brain_detail.rs`).** A nav row (‹ Back, mode, title, history depth), the
  claim/Recon path, then Related | Summary side by side when there is room, stacked otherwise.
  The path pane no longer contains Related text. Detail opens focused on the graph; Tab cycles
  Back → path → Related → Summary.
- **Related navigation.** Selection and opening are separate (↑↓ vs Enter/click). Opening fetches
  by id (works when Find hides the memory), pushes a history snapshot (memory, Related selection,
  focus, scroll; capped at 32) and Esc/Back pops it, skipping deleted entries, then returns to the
  list. Related results arrive as `WorkEvent::Related` with a request id; stale results are
  ignored.
- **Atlas buttons.** Runs page: Live, Resume (dimmed unless the selected cycle is resumable via
  the now-public `atlas::resumable`; `run_live` takes `LiveRun {Fresh, Latest, Run(id)}`), Repair
  memories (`atlas_actions::start_repair` → `atlas_memory::spawn_repair`, one at a time) and
  Delete.
- **Jobs fixes (from review).** Active time is live: closed attempts plus the open attempt's
  running span, or start→finish/now for in-process jobs without attempts; "Unavailable" only when
  there is no start time. The "logs" header no longer truncates — optional columns (logs, then
  phase) drop on narrow widths. The screenshot fixture now records real attempts (core
  `fixtures` feature, dev-only).
- **Robustness.** The graph-summary request no longer panics without a Tokio runtime; it reports
  "Graph summary unavailable" instead.
- **Screenshots** (fixture data, TestBackend → PNG): `brain-claim-detail.png` (140×40),
  `brain-claim-detail-narrow.png` (64×32), refreshed `jobs.png`.

### Phase 6 — what landed

- **Core `job_registry.rs`.** `JobHandle::begin(db, JobSpec)` writes the job row (kind `inproc`,
  state running, owner = this process) before the caller spawns work. A known id is reused (a
  resumed operation): attempts go up, earlier active time is kept, no second top-level row.
  `child()` registers phases/sub-steps under a parent with the parent's correlation id;
  `phase()` records phase/progress and doubles as a heartbeat; `finish(Finish)` records
  completed / partial / failed / paused / cancelled with the monotonic active time, a redacted
  error summary and a `job.<state>` event. Dropping an unfinished handle (panic, abort, early
  return) records failed/"interrupted". `job_for_run` finds the canonical job for a legacy run
  id (Atlas run, Recon run, Intel Recon job).
- **Process liveness.** Registering (or the TUI at start-up) starts one heartbeat thread per
  process and state root (`argos_processes`, every 5 s). `recover_orphans` (at TUI start and
  every minute) marks open jobs of processes whose heartbeat is older than 30 s as failed
  "Argos exited before this job finished", ending their active time at the last heartbeat. It
  covers registry jobs and owned task-less jobs (the repair pass). CLI runs heartbeat too, so a
  TUI never orphans a running CLI investigation.
- **Cancellation.** `request_cancel` only sets `cancel_requested` and flips the operation's own
  stop flag (same process immediately, other processes on their next heartbeat). The job reads
  "Cancelling…" in Jobs until the operation stops, then "cancelled". Jobs shows Cancel (key `c`)
  only for running jobs registered as cancellable.
- **Schema (additive).** `argos_jobs.active_since`, `cancel_requested`, `cancellable`; table
  `argos_processes`. `set_job_progress` folds an open `active_since` span into `active_ms` when a
  job stops. `JobRow::active_now` uses `active_since` for live time.
- **Inventory** (spec §5.1; every `tokio::spawn` / `spawn_blocking` / worker thread):

| Entry point | Where | Registration | Cancel |
| --- | --- | --- | --- |
| Atlas cycle (manual, auto, resume) | core `atlas::run_atlas` | One `atlas_cycle` job per run (reused on resume) + `atlas_phase_N` children; truthful end from the run state (completed / partial / failed / paused) | Pause in Atlas (resumable) |
| Atlas phase 5 index work | core outbox tasks | Task-backed (index service + attempts) | — |
| Repair Atlas memories / startup reconciliation | core `atlas_memory` | Task-less job owned by the process; panic guard; orphan recovery | — |
| Recon investigation (ask / resume) | core `recon::Service` | `recon_investigation`, stages as phase, run linked, resume reuses the job; completed tool calls are not re-run | Yes (run's stop flag) |
| Intel Recon assessment | core `intel_recon::start_report_worker` | `intel_recon` canonical job, legacy job id as run ref, restart reuses it | Yes |
| Intel article body (fetch, fallback, insight replacement) | TUI | `article_body` | Yes |
| Intel Recon mode recommendation | TUI | `recon_mode` | — |
| Tool runs (Tools app) | TUI | `tool_run` with tool id | Yes |
| Graph explanation (Brain) | core `graph_explanation::explain` (launched by the TUI) | `graph_explanation` job + one `graph_explanation_attempt` child per outbound request (phase 7) | — (not cancellable; superseded results are discarded) |
| Model catalog load, OpenRouter verify, subscription sign-in/check, access probe | TUI | `model_catalog`, `provider_verify`, `provider_login` / `provider_check`, `access_probe` | — |
| Embedding model download | core `embed::ensure_file` | `model_download` (when the default state root exists) | — |
| Index worker, summary flush worker | core `scheduler` | Service-health rows (`svc-local-index`, `svc-summary-flush`); their tasks carry the history | — |
| Summary/compaction flushes | core summary pool | Task-backed | — |
| Related-memory loading (Brain) | TUI `spawn_blocking` | Not a job: ordinary bounded read | — |
| Hardware refresh, Brain recall | TUI | Not a job: synchronous | — |

- **Gaps (honest).** Recon tool calls stay durable `osint_calls` rows shown through the
  investigation's phase, not separate child jobs, and the short title-generation follow-up is
  not its own job. The unused `retry_insights` TUI path is not wrapped. Registry jobs have no
  Jobs → Retry button (resume stays in the source app: Atlas Resume, Recon resume), which
  reuses the canonical job and skips completed work. In-process jobs start immediately, so their
  queue time is 0 and retry wait applies only to task-backed jobs. Process exit is tested by
  simulating a dead owner, not by killing a real process.
- **Also.** The graph legend wraps instead of truncating on narrow widths (5b review nit).

### Phase 7 — what landed

- **Typed provider diagnostics (core `provider_diag.rs`).** `ProviderFailure` records the stage
  (configuration, admission, connect, first response, response, stream, parse, validation,
  persistence), a stable category (auth, permission, invalid model, configuration, malformed
  request, unsupported transport, rate limited, server, timeout, network, stream interrupted,
  premature EOF, SSE error, malformed payload, token limit, refused, empty, invalid result,
  persistence, cancelled), retryability, HTTP status, provider error code and message, request
  id, Retry-After, the endpoint (credentials and query string stripped), provider/model/transport,
  elapsed and first-response time, and stream state (chunks, bytes, events, content began, done
  marker, finish reason, partial length; the partial text is never kept). The full cause chain is
  kept outermost-first, and every cause passes through `events::redact` plus URL sanitizing.
  Nested causes and endpoint URLs are covered. Auth/permission/model/configuration failures
  carry guidance ("Reconnect … in Providers", "Choose another Summarization model in Models").
- **Single-request, final-only transport (core `provider_attempt.rs`).** `attempt()` sends exactly
  one request (or one subscription call). It never retries or switches transport itself. Connect,
  first-response, idle and total deadlines are separate. A streamed answer is buffered and
  returned only with an explicit completion indicator (`[DONE]` or a finish reason). A body read
  error, close without an indicator, `event: error` / `{"error":…}` events, unreadable event JSON
  and `finish_reason=length` each become typed failures. Non-streaming bodies are checked the same
  way (malformed JSON, error payload with HTTP 200, empty, refused, truncated).
- **Budgeted executor (core `summarization::complete_summary_report`).** At most **2** outbound
  requests per execution, stream/non-stream fallbacks included. Graph explanations start with a
  final-only non-streaming request. A transport the endpoint rejects switches once, and a broken
  stream falls back to non-streaming within the same budget. Admission waits poll the shared
  provider slot and consume no attempt. Only retryable categories retry, with the lower of
  Retry-After and the shared backoff (capped). Auth/model/permission/malformed/token-limit
  failures stop after one request. Each attempt is reported (`AttemptLog`) with a fallback reason.
  `complete_summary` (the scheduler's live summary upgrade) now delegates to it. That fixes the
  old loop, where an admission miss consumed an attempt.
- **One durable graph explanation (core `graph_explanation.rs`).** The TUI no longer calls
  `provider::complete` for graph summaries (`write_graph_summary` is gone). `explain()` registers
  one `graph_explanation` job (resource `memory:<id>`, run `graph:<id>`, model; a retry's
  correlation id is the failed job). Each outbound request is a `graph_explanation_attempt` child
  job with a structured `graph_explanation.attempt` event. On failure, the
  `graph_explanation.failed` error event (full sanitized failure, attempts, admission wait,
  fallback reason) and the diagnostic record are written **before** the report returns, so the
  UI is notified only after the logs exist. Logging errors are returned and disclosed in the
  card and the log. Saved summaries, deleted/changed memories (superseded, nothing published)
  and persistence failures (typed, stage `persistence`) are distinguished. A failure never
  touches the memory or Atlas indexing state.
- **Revision-aware cache (`store/graph_summaries.rs`, additive).** `memory_graph_summaries` gains
  `cache_key`, `memory_revision`, `graph_revision`, `provider`, `model`, `prompt_version`. The key
  is the memory text revision + graph brief (evidence) revision + focus + provider + model +
  prompt version (with a system-prompt hash). A save is refused when the memory is gone or its
  text revision changed since the request started. A row from other inputs is shown only as an
  "Earlier result". New table `argos_graph_explanations` keeps the latest execution per memory
  (state, category, reason, guidance, attempts, failure event id, sanitized diagnostic JSON). It
  outlives event retention and drives the retry cooldown.
- **Brain inline failure card (TUI `summary_card.rs`).** Replaces "Leave and open this memory
  again to retry". It shows "⚠ AI summary failed · <reason>", guidance when a setting is wrong,
  and the actions View details (sanitized cause chain, stage/category, endpoint, HTTP/provider
  code, timing, stream state, every attempt), View logs (Logs filtered to the job with the failure
  event selected and expanded; once events expire it says so and points at the job summary and
  View details), View job, Retry summary (fresh 2-request budget, linked to the failed job,
  stays in Brain) and Open Models (Models → Defaults with Summarization selected; only for
  auth/model/configuration failures). Labels shorten only when they would need more than two
  rows. Below the card: the last valid summary labeled "Earlier result", otherwise the
  deterministic "Basic graph explanation — AI summary unavailable" built from the path. Card
  buttons are in the focus order and hit-testable.
- **Duplicates, cooldown, late results.** Reopening a memory reuses a valid cache. It shows
  "already being written" while an execution runs (in-process pending + running job check), and
  inside the 2-minute cooldown it shows the saved failure from the record without a new job or
  request. Explicit Retry bypasses the cooldown but never a running execution. Completions carry
  a request id, and older ids are ignored.
- **Other completion callers.** `provider::complete` / `complete_once` now return a typed
  `ProviderFailure` for non-success HTTP. Its Display keeps the status and the provider message,
  so existing 429 detection still works, and the error downcasts for callers that want the
  category. Their retry budgets are unchanged (1 request on a 429, tested).
- **Screenshots** (mock provider returning 401 `invalid_api_key`, TestBackend → PNG):
  `brain-summary-failure.png` (card, Retry focused) and `brain-summary-failure-details.png`
  (details open), 140×40.
- **Gaps (honest).** Graph explanations are not cancellable from Jobs: there is no stop flag, and
  superseded results are discarded instead. The "earlier result" is the single stored row, not a
  history of summaries. The deterministic fallback is shown but not cached as a summary. The
  subscription (Codex CLI) transport has no stream state, and its errors are classified from
  text. Jobs shows the terminal error summary after event expiry but does not itself label the
  missing detail as expired; the Brain link does. Stream-only providers are not auto-detected:
  the non-stream → stream switch happens only when the endpoint rejects non-streaming.

## §10 acceptance checks

| # | Check | Status | Evidence / gap |
| --- | --- | --- | --- |
| 1 | Atlas run creates visible memories in-session | **Done** | Core: `successful_run_publishes_indexes_verifies_and_completes` (disk store; memories in `list_memories`, `MemoriesChanged` emitted). TUI reloads on `MemoriesChanged`/`AtlasDone` and on Brain entry, keeping the selected id, Find text and scroll (`brain_refresh_keeps_selection_and_find_and_shows_read_errors`). Screenshot: `brain-claim-detail.png` (fixture data) |
| 2 | Brain entry refreshes externally committed memories | **Done** | Every mutation path bumps the durable counter in its transaction. `MemoryChangeWatcher` notifies only after commit, across processes, coalesced (`change_watcher_sees_only_committed_changes_and_coalesces`). The TUI reloads on it (1 s poll) and on entering Brain. Selection by id and Find survive reloads; a failed read keeps the last list with a "read failed" title, an error note, one log event and a status message; an open detail whose memory was deleted says so (`brain_refresh_keeps_selection_and_find_and_shows_read_errors`) |
| 3 | Publication failure cannot report completed; retry uses saved payload | **Done (core)** | `publication_failure_is_failed_not_completed_and_retry_uses_the_checkpoint`: state `failed`, extracted 2 vs created 0, rollback, resume completes with extraction called once |
| 4 | Embedding/Lance failure: memories visible, phase 5 incomplete, retry reaches verified | **Done (core)** | `embedding_failure_keeps_memories_visible_reports_partial_and_retry_verifies` (run `partial`, `0/3 indexed`, resume reaches `3/3`), `background_indexing_then_refresh_upgrades_a_partial_run`; UI screenshot phase 5 |
| 5 | Embeddings disabled: honest status; enabling resumes | **Done (core)** | `disabled_embeddings_save_memories_and_say_so_honestly` ("Saved; semantic indexing disabled", never claims indexed, work stays queued), `enabling_embeddings_resumes_outstanding_work_and_upgrades_the_run` |
| 6 | Kill/restart at each boundary | Partial | Outbox claim/lease expiry (phase 1/2 tests), extraction checkpoint (`crash_after_checkpoint_resumes_without_reextracting`), SQLite commit (rollback + retry test), phase-5 pause/resume from receipt (`pause_in_phase5_parks_then_resumes_from_the_stored_receipt`), index write (mid-write test), and a process kill between the Lance write and the task acknowledgement (`worker_killed_after_the_lance_write_is_recovered_by_revision`: lease expiry, revision-aware re-record, no duplicate vector, stale ack rejected). Remaining: an end-to-end kill test of a real process |
| 7 | Duplicate claims reuse canonical memories, keep links, repair vectors | **Done (core)** | `publication_receipt_reconciles_and_reports_rejections`, `retrying_the_same_payload_is_idempotent`, `reused_claims_with_damaged_rows_are_repaired_and_reindexed`, `indexing_is_verified_by_exact_id_and_revision` |
| 8 | Change/delete during indexing cannot overwrite newer/resurrect | **Done (core)** | `stale_work_cannot_overwrite_a_newer_revision`, `deletion_during_indexing_is_not_resurrected`, `text_changed_mid_index_write_reruns_on_the_new_revision` (fault-injected). Phase 4: Recon/Intel Recon/manual edits use the same revision-aware outbox (`recon_claims_and_deletions_use_the_durable_index_outbox`, `intel_recon_publishes_durably_with_provenance_and_no_atlas_receipt`, `memory_writes_and_their_outbox_rows_commit_together`) |
| 9 | Historical repair | **Done** | `repair_requeues_vectors_for_a_false_completed_cycle_without_rewriting_it`, `repair_restores_orphaned_links_from_the_checkpoint_and_respects_tombstones`, `repair_reports_missing_payload_and_unrecoverable_links_without_fabricating`, `repair_job_pages_resumably_and_records_job_progress`. TUI (5b): Atlas runs page has a "Repair memories" button that starts one repair job (progress in Jobs; a second press while running says so) and a Resume button offered only for resumable cycles (`atlas_history_resume_is_offered_only_for_resumable_cycles_and_repair_starts_once`). A separate re-extraction action is not implemented: resume reuses the saved checkpoint |
| 10 | Summary worker cannot claim Atlas/index tasks; two processes cannot own one attempt | **Done** | `summary_pool_never_claims_index_or_atlas_work`, `summary_worker_leaves_index_and_atlas_tasks_alone`, `two_processes_cannot_own_the_same_attempt_and_stale_owner_cannot_publish` |
| 11 | Jobs timing/history for every async op | **Done** (gaps listed in phase 6) | Task-backed work (index, summary, publication) keeps tasks/attempts (5a). Every other inventoried async entry point registers a registry job before it launches, with monotonic active time (`active_since` + durable `active_ms`, resumes add up), phase/progress, correlation id = job id on all its events, and a truthful end state: explicit finish, `Drop`/panic → failed "interrupted", exited process → failed "Argos exited before this job finished" via the per-process heartbeat (`registration_is_durable_before_work_and_finish_records_monotonic_timing`, `dropped_or_panicking_work_reaches_a_failed_state`, `jobs_of_exited_processes_become_interrupted_and_live_ones_stay`, `registering_makes_this_process_live_for_other_processes`). Retry does not repeat successful child work: an Atlas resume reuses the cycle job (attempts 2) and only re-runs the unfinished phase child (`embedding_failure_keeps_memories_visible_reports_partial_and_retry_verifies`, `publication_failure_is_failed_not_completed_and_retry_uses_the_checkpoint`); Recon resume and Intel Recon restarts reuse their canonical job (`investigation_jobs_finish_truthfully_and_link_their_run`, `intel_recon_links_its_legacy_job_to_one_canonical_registry_job`). Cooperative Cancel in Jobs (`cancel_is_cooperative_and_only_for_cancellable_running_jobs`, `cross_process_cancel_reaches_the_owner_through_its_heartbeat`, TUI `registered_operations_show_in_jobs_and_cancel_is_cooperative`) |
| 12 | Job↔log navigation after restart; redaction | **Done (UI)** | Logs read durable `argos_events` (24 h retention), so links survive restart. Jobs → `l`/View logs opens Logs filtered to the job and its descendants with Back; Logs → `o`/Open job selects the job (clears hiding Jobs filters). `jobs_dashboard_navigates_to_logs_and_back_and_logs_open_jobs`. Redaction from phase 1 |
| 13 | Home System category / nine routes | **Done** | Home: Applications = Intel, Atlas, Brain, Recon; System = Jobs, Logs, Tools, Models, System; digits 1–9, palette, slash (old `osint`/`providers` plus `tools`/`models`/`jobs`/`logs`), header tabs and help agree (`home_order_renames_and_nine_routes_agree`). Renames are display-only: `ModuleId::Osint`/`Providers` and config keys unchanged |
| 14 | Tools/Models retain behavior; System hardware/paths only | **Done** | System shows host hardware + paths (config, database always; data, memory index, credentials, hardware cache when they exist) and a single Refresh hardware action; event log and Clear moved to Logs (`system_shows_only_hardware_and_paths_and_logs_own_clear`). Tools/Models screens are unchanged apart from titles |
| 15 | Targeted tests / CI gates | Partial | Per-phase runs recorded below; real local Lance fixture (fake embedder) used for exact id/revision checks |
| 16 | Stream failure before/after content: sanitized causes + transport metadata in Brain details and Jobs/Logs; partial never saved | **Done** | `premature_eof_before_and_after_content_is_typed` (content began / partial length, "discarded"), `partial_streamed_text_is_never_saved` (no summary row after two broken streams; attempt 2 is non-stream), `failure_is_durable_correlated_bounded_and_harmless` (job + 2 attempt children + failure event details + diagnostic record), TUI `brain_summary_failure_card_explains_links_and_retries_in_place` (details show stage/HTTP; Logs selects the failure event). Screenshot `brain-summary-failure-details.png` |
| 17 | Premature EOF, SSE error, malformed payload, token limit, empty, persistence failure → correct stage and recovery | **Done** | `sse_error_malformed_and_token_limit_are_typed`, `partial_streamed_text_is_never_saved` (SSE error → retry → token limit, not retried further), `malformed_and_empty_results_fail_without_publishing`, `persistence_failure_is_typed_and_logged` (stage `persistence`, cause chain kept), `empty_then_valid_retries_once` |
| 18 | ≤2 outbound requests incl. stream/non-stream fallbacks; admission contention consumes no attempt | **Done** | `never_more_than_two_requests_including_fallbacks` (persistent 503, stream→non-stream, unsupported→stream; server hit counts), `admission_contention_consumes_no_attempt` (saturated slot, 1 request, wait recorded), `failure_is_durable_correlated_bounded_and_harmless` |
| 19 | Auth/model guidance; transient retry within budget; exhausted retries keep valid cache or labeled deterministic text with visible failure history | **Done** | `configuration_errors_send_one_request_and_carry_guidance`, `auth_failure_sends_one_request_gives_guidance_and_redacts`, TUI card shows guidance + Open Models; `saved_summary_is_reused_until_its_inputs_change_then_shown_as_earlier` (503 retried once, earlier result kept and labeled); basic explanation heading asserted in the TUI test; jobs/events/diagnostic record keep the history |
| 20 | Retry without leaving Brain; away/back creates no duplicate jobs; cancelled/deleted/superseded requests cannot publish | **Done** (cancel N/A) | TUI test: reopen inside cooldown → 1 job / 1 request; Retry → 2nd job correlated to the 1st, module stays Brain. `reopening_respects_cooldown_and_running_jobs`, `late_completion_for_a_changed_or_deleted_memory_is_not_published`, TUI `late_summary_completions_are_ignored`. Graph explanations are not cancellable (gap) |
| 21 | Evidence/text/focus/model/prompt change invalidates cache; failure never removes a memory or fails Atlas indexing | **Done** | `success_is_keyed_and_any_input_change_invalidates_it` (all five inputs), `failure_is_durable_correlated_bounded_and_harmless` (memory kept, no failed tasks), TUI earlier-result test |
| 22 | Diagnostic links survive restart; after expiry Jobs keeps the error summary and expired detail is identified; redaction incl. nested causes and URLs | **Done** (Jobs label partial) | Diagnostic record + job row are durable (TUI reopen reads the record); after clearing events, View logs reports expired detail and the record keeps the cause chain (TUI test). `nested_causes_and_endpoints_are_kept_and_redacted`, `endpoint_strips_query_and_userinfo`, `http_errors_carry_status_code_and_redacted_message`, `auth_failure_sends_one_request_gives_guidance_and_redacts` (API key echoed by provider and URL userinfo absent from events and record), TUI screen has no key. Jobs itself does not label expired detail |
| 23–26 | Claim/Recon layout, Related navigation | **Done** (async edge partial) | Detail opens with the graph on top, Related left and Summary right on one row (≥68 cols below the graph), stacked Related-above-Summary when narrower with the focused pane taller; no Related text in the graph pane. Related is a selectable list from core `related_memories` (claim relations, shared source article/tool result, same entity, same investigation, then "Similar · not evidence"); unique, self excluded, each row resolves to an existing memory. ↑↓ selects, Enter/click opens by id even when Find hides the target, Esc/‹ Back restores the previous memory with its Related selection and focus, deleted targets keep the current view with a status message (`claim_detail_has_graph_above_related_left_summary_right_and_navigates_by_id`, `narrow_claim_detail_stacks_related_above_summary_and_keeps_both_reachable`, core `related_rows_are_unique_existing_memories_ranked_explicit_before_similar`). Related loads on a blocking task with request-id/memory-id rejection of stale results (tested by injecting late results); tests run the load inline, so a real-runtime rapid-navigation race is not exercised end to end. Summary failure card and in-place Retry landed in phase 7 (§10 16–22) |

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
- Phase 4: `ARGOS_EMBED=0 cargo test -p argos-osint-core --lib` → 420 passed, 0 failed, 5 ignored
  (+5: outbox atomicity/rollback for `add_memory`/`update_memory`, change-watcher, kill between the
  Lance write and the acknowledgement, Recon outbox/tombstone/deletion, Intel Recon provenance/no
  receipt). `ARGOS_EMBED=0 cargo test -p argos-osint-bin` → 59 passed. `cargo clippy --workspace
  --all-targets`: no new findings; all remaining findings were already on `main`.
- Phase 5a: `ARGOS_EMBED=0 cargo test -p argos-osint-core --lib` → 422 passed, 0 failed, 5 ignored
  (+2 `jobs_view`). `ARGOS_EMBED=0 cargo test -p argos-osint-bin` → 62 passed, 1 ignored (+3 new
  TUI tests; the ignored one is the screenshot dump; old System-log tests rewritten for durable
  Logs). `cargo clippy --workspace --all-targets`: no new findings.
- Phase 5b: `ARGOS_EMBED=0 cargo test -p argos-osint-core --lib` → 424 passed, 0 failed, 5 ignored
  (+1 `jobs_view` live active time, +1 `related_memories`). `ARGOS_EMBED=0 cargo test -p
  argos-osint-bin` → 67 passed, 2 ignored (+5: Jobs column dropping, claim detail wide/narrow,
  Brain refresh/read errors, Atlas resume/repair; the ignored ones are the two screenshot dumps).
  `cargo clippy --workspace --all-targets`: no new findings.
- Phase 6: `ARGOS_EMBED=0 cargo test -p argos-osint-core --lib` → 433 passed, 0 failed, 5 ignored
  (+9: 7 `job_registry`, 1 Recon and 1 Intel Recon canonical-job test; the Atlas cycle and
  repair job assertions extend existing tests). `ARGOS_EMBED=0 cargo test -p
  argos-osint-bin` → 68 passed, 2 ignored (+1 `registered_operations_show_in_jobs_and_cancel_is_cooperative`;
  the narrow claim-detail test now also checks the legend). `cargo clippy --workspace
  --all-targets`: no new findings.
- Phase 7: `ARGOS_EMBED=0 cargo test -p argos-osint-core --lib` → 453 passed, 0 failed, 5 ignored
  (+20: 3 `provider_diag`, 5 `provider_attempt` incl. typed errors for existing `complete`
  callers, 4 `summarization::exec`, 8 `graph_explanation` fault tests against a scripted local
  provider). `ARGOS_EMBED=0 cargo test -p argos-osint-bin` → 71 passed, 3 ignored
  (+3: failure card/links/retry/cooldown/expiry, earlier-result invalidation, late completions;
  +1 ignored `dump_phase7_screens`). `cargo clippy -p argos-osint-core -p argos-osint-bin
  --all-targets`: no findings in phase 7 files; remaining findings were already on `main`.
