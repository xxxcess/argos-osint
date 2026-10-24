# SurrealDB migration checklist (acceptance)

See [plan.md](design.md). Status is honest: unchecked means not done or not
verified.

## Still needs SQLite (2026-10-24, combined tree at 4760a9e)

Authoritative list; older "Gaps" bullets below are historical where they
conflict. Revision fence for cycle evidence and derivation membership
(67ee9ee, 2049aa7) is closed. Model attempts/drafts, `task_epoch`, the Recon
job registry, stage/activity events, `insight.superseded` and recall
telemetry are wired (see "Combined tree" at the end).

- [ ] Brief chain engine: `brief_stages::drain_parallel` / `claim_next` /
  `run_claimed`, `brief_pipeline::publish` and `brief_canonical::publish_stage_on`
  still read and write SQLite. `storage::brief` is registered and has every
  table they need, but no production caller writes them, so
  `storage::brief::record_run` and `replace_leads` have no callers yet.
  `spawn_intel_brief_drainer` is not gated on `surreal_selected()`.
- [x] Logs pane, its severity counts, the TUI session log and Jobs → Logs
  counts read and write `argos_event` under a storage config
  (`tui::surreal_brain::SurrealLogs`, worker threads); `argos_event` has prune
  and clear; `telemetry_meta`, the telemetry prune and coverage are on
  SurrealDB (`storage::telemetry::SCHEMA`, not yet in bootstrap). There is no
  telemetry backfill to port (clean reset). `tests/storage_logs.rs`.
- [x] Report surface of `run_pipeline` reads its result bundle from
  `intel_report_result` by source hash on SurrealDB
  (`ExecStore::report_details`, `storage::intel::report_details_by_hash`).
  The Intel report worker stays retired (`report_execution_enabled()` is
  false), so no production run reaches it yet.
- [ ] Article refresh input: `insight_persist::article_input` reads SQLite
  evidence tables.
- [ ] `pipeline_exec::load_memory_evidence` has no legacy (pre-derivation)
  fallback; its recall has no recall cache or kind filter. Turn recall is the
  BM25 stopgap until vector recall lands.
- [ ] `enqueue_refresh` does not requeue paused tasks; `validate_retry` and
  `retire_atlas_insight_work` stay SQLite.
- [ ] Topology: retained Intel seeds are refused, the neighbourhood summary
  is skipped, snapshot reads are not one transaction, no TUI test drives the
  SurrealDB workers.
- [ ] Atlas, Atlas insight/memory writers and the investigation harness
  (`investigation::engine`) stay SQLite.

## P0 — foundation (spec step 1)

- [x] Branch from current `origin/main` (`71797ee`); diff since spec baseline `7569da1` is `AGENTS.md` only
- [x] Rust 1.95.0 pinned; workspace `rust-version = "1.95"`; AGENTS.md/README toolchain lines
- [x] 1.95 lints fixed (clippy 1.95 adds collapsible_match, unnecessary_sort_by, explicit_counter_loop)
- [x] `surrealdb = "=3.3.4"`, default features off, `protocol-ws` only, behind non-default `surreal` feature
- [x] Non-default `embedded` feature = `surreal` + `surrealdb/kv-rocksdb`
- [x] `scripts/check_storage_deps.py`: default build links no SurrealDB; `surreal` links no embedded engine
- [x] `cargo tree -p argos-osint-bin --features surreal`: no rocksdb, surrealkv, tikv, mem/indxdb, surrealdb-kvs, engine-local or surrealdb-core
- [x] Default build tree has no surrealdb crate
- [x] `scripts/test_surreal.py`: pinned download + SHA-256, loopback RocksDB temp dir, sync every, versioning off, appendix env profile, teardown
- [x] `tests/storage_contracts.rs`: auth, rollback, structured retryable conflict, DISKANN F32/384/cosine filtered search, restart persistence, structured errors
- [x] Suite skips cleanly without a server; fails with `ARGOS_SURREAL_REQUIRED=1`
- [x] Contract suite passes locally, server mode (real 3.3.4 disk server) and embedded mode, Linux x86_64
- [x] Embedded mode: auth not applicable; raw conflict is kind Internal, normalized to the structured conflict
- [ ] Local matrix on final head (see PR body for results)
- [ ] CI: toolchain 1.95 action, dependency check, `surreal` and `embedded` clippy, contract suite in server and embedded mode on all four jobs. CI green on 4 jobs; contract-suite CI pending the workflow push.
- [ ] CI green on all four jobs with the contract suite (depends on the item above)
- [x] Server lifecycle design written (design.md); not implemented
- [x] Existing `argos.db`, sidecars and `memory_lancedb/` untouched
- [ ] Release binary size and clean build time, before vs after, default and storage builds, per OS (see PR body; macOS/aarch64 need CI)

## P1 — storage traits + job system (spec steps 2–4)

- [x] Mode decided: `Surreal<Any>`, server default, embedded optional (user + Code Architect, 2026-10-22)
- [ ] Embedded single-owner lock `storage/embedded.lock` with the "Argos home in use by PID N" error
- [x] `storage::error::normalize` maps the embedded conflict (kind Internal) to `QueryError::TransactionConflict`; contract test asserts the structured form in both modes
- [ ] Fresh versioned schema files, checksummed ledger, database identity, reset epoch
- [ ] Typed key codec with round-trip/collision tests
- [ ] Async repository boundary; no raw connections
- [ ] Transaction/retry helpers; per-statement result checks
- [ ] Scheduler election, atomic claims, leases, owner/epoch fences, cancellation, heartbeats
- [ ] `Drop` database work replaced by bounded completion queue + lease recovery

## P2 — memories, graph, vectors (spec steps 5 memory part, 6)

- [x] Memory publication with graph projection, tombstones, change sequence, embedding outbox in one transaction (group-commit publisher; `memory::put_typed_in` projects the graph in the same transaction)
- [x] Seven edge meanings, parallel Link/Related IDs, retirement behaviour: Link, Related, Concerns, Premise projected from memories (edge keys deduped; owner-index delete pinned by a plan test). Source, MentionedIn, Contains come from atlas/insight records and land in P3 (see below)
- [x] DISKANN recall with revision/generation fencing and lexical fallback; bounded top-k heap. Generation and model filter inside the KNN search (plan-pinned; 25 stale near neighbours do not hide a fresh one); revision, lifecycle and kind are checked on returned rows. Fewer than 5 surviving vector hits are topped up with lexical hits, paged via the `memory_lifecycle_kind` index, at most 4,096 rows per pass
- [x] Recall caps (5 memories, 1,500 tokens), opt-out, default kind exclusions (actor, link, related unless asked for)

## P3 — Recon, Intel, Atlas + savepoints (spec step 5 rest)
- Insight publication (`storage/insight.rs`): `commit_article_insight_replacement` (bc3184e), `publish_atlas_insights` and `publish_snapshot` ported. Publishes bump `memories_changed` only when they changed something (SQLite's rule); `write_then_index` only when it touched memory ids.
- [x] `legacy_receipt` (`storage/provenance.rs`): reads, index work and the `reconstructed` receipt row in one publisher transaction. Links are ordered by fingerprint (SurrealDB has no rowid order).
- [ ] `brain_backfill::bind_claim_subject`: the `bind` engine is already ported (`storage::actor_bind::bind_in`, `bind_claim_subject_in`, phase 4b), and the actor alias/external-ref, memory endpoint and actor-ref tables are on SurrealDB (`lifecycle`, `recon` schemas; typed `memory_metadata` lives on `memory`). Missing were `recon_entities`, `brain_backfill_receipts` and `brain_actor_dedup_receipts`.
- [x] Item 2 part 1: `storage::entities` defines `recon_entity` (unique `(kind, namespace, canonical)`, insert-or-ignore, label fallback to canonical), `brain_backfill_receipt` (upsert by `(source, unit_id)`) and `brain_actor_dedup_receipt` (explicit split by sorted pair key). `tests/storage_entities.rs`, both modes.
- [x] Item 2 part 2: derivation publication inserts-or-ignores the subject's `recon_entity`, and `bind_claim_subject_in` reads the subject label from it (label, canonical, entity id) and writes the `insight_claims` backfill receipt (same revision hash and coverage labels as SQLite) in the publishing transaction (`storage_derivations::claims_bind_actors_and_kind_refresh_carries_the_other_kind`, both modes).
- [x] Item 2 `bind` engine: already on SurrealDB since phase 4b (`storage::actor_bind::bind_in`, `add_memory_actor_ref_in`, `sync_claim_actor_payload_in`, `bind_claim_subject_in`), exercised by `storage_derivations`; no separate port commit was needed.
- [x] Item 2 part 3: validation inside the transaction (see `live_evidence` above).
- [ ] Item 2 gaps: `brain_actors::split` and the dedup job are not on SurrealDB (only their receipt table is); the legacy backfill sweep (`brain_backfill::run` over `recon_entities`/`recon_thread_entities`) has no SurrealDB driver.
- [ ] `refresh_cached_publication` (P3 item 3, checked 2026-10-23, deferred): it is not a single-function move. Its only caller is `ensure_intel_brief_pipeline`, which runs only on SQLite (the brief chain on SurrealDB, `jobs::ensure_chain`, has no caller and stores no metadata snapshot / `input_ref`). Porting it needs, on SurrealDB: `intel_brief_publications` and `intel_inference_article_leads` (no tables yet), `brief_pipeline::publish` / `publish_completed_stage`, `brief_canonical::publish_stage_on` with its `app_state` stage receipts and `required_index_receipts`, `prepare_for_stage` (inputs now available: `pipeline_inputs::article_input`, `insight::desired_generation`, `provenance::stage_outputs`), plus the brief-chain metadata snapshot. Estimated as several commits over the ~1.5k cap in total; it should land with the brief pipeline moving to the repository rather than before it. No code changed for this item.
- [x] `publish_snapshot` validation inside the transaction: `storage::live_evidence::validate_in` (and `step(snapshot)`, the `validate` step for `publish_snapshot` / `run_refresh_slice`) checks the article owner, article metadata, body version, report authority and retirement, retained artifacts and memory premise revisions through the publishing transaction (`tests/storage_live_evidence.rs`, both modes).
- [ ] Live-evidence gaps: Recon owners (`recon_runs`) and tool evidence (`call:`, `osint_calls`) are refused as not on SurrealDB yet; the `report:` branch has no contract test; no production caller passes `step` yet (`run_refresh_slice` has no production caller on SurrealDB; the Recon dispatch skips the refresh, see Core dispatch below).
- [x] `storage::insight::delete_memory` caller: `browse::delete_memory` (TUI `surreal_brain::delete`, plain thread through `runtime::blocking`) wraps it, drops the vector and runs the SurrealDB job sweep (`jobs::cleanup_batch`). The memory's NLP source refs are removed in the delete transaction (`remove_nlp_owner_in`), and only the artifacts they named that no other ref, derivation evidence or snapshot input references are collected (`storage_insights::deleting_a_memory_drops_its_nlp_refs_and_only_unshared_artifacts`, both modes).
- [ ] `delete_memory` gaps: there is no SurrealDB `nlp_features` table, so there is nothing to collect there; the artifact reference check is a string match on the evidence/inputs JSON (conservative: it can only keep extra artifacts) and scans those tables (perf item, held). No SurrealDB NLP cache reader keyed by memory exists yet, so "missing memory is a miss" has no reader to change.

- [x] `schedule_article_refresh` (`storage/refresh.rs`) with `enqueue_refresh_in`, `retire_replaced_in` and `supersession`. Intent, retirement check, prior-task comparison, generation advance, job/task creation, the owner's `refresh_job_id` and supersession all commit in one transaction. The refresh task's `input_ref`, `role_snapshot` and `source_revision`, plus the job's `owner_scope`, live in an `insight_refresh_task` sidecar, because `task` and `job` have no such fields.
- [x] `schedule_article_refresh`: paused SurrealDB refresh tasks are requeued by `enqueue_refresh_in` (d83bc2f). Still SQLite: the `insight.superseded` event (`events` table).
- [x] Step 5: `storage::bootstrap::ensure_schema` applies all 13 module schemas plus `storage_meta` in dependency order, in one transaction, and records `storage_meta:schema.version` (`SCHEMA_VERSION = 1`). It is idempotent, and it refuses (and rolls back) a database whose version is newer, like the SQLite `user_version` check.
- [x] `ensure_schema` caller: `home::open_home` (so `Repository::open_home` and `storage::runtime`) reaches it on both paths, through `open::open` (embedded and server) and also at server spawn. Covered by `storage_runtime::open_home_runs_ensure_schema` in both modes (every module table defined; reopen is a no-op).
- [ ] `ensure_schema` gaps: SQLite `Store::open` is unaffected (SQLite mode never opens SurrealDB). There is no migration ladder between Surreal schema versions (the store starts fresh, P4). The embeddings DISKANN dimension is fixed at 384 in `embeddings.surql` rather than taken from the configured model.

- [ ] Recon/OSINT messages, tools, artifacts, ordered counters, credit hold/reconcile/release
  - [x] Credit hold/reconcile/release (`storage::credits`, `tests/storage_credits.rs`, both modes): trial first, monthly reset, Whoxy prepaid pool, concurrent holds never overdraw, `release_held_credits` recovery. Gap: Recon callers still use the SQLite `Store`; messages, tools, artifacts and ordered counters are unported.
  - [x] Live-evidence `report:` branch contract test (latest result authority, retirement) in both modes.
- [ ] Intel stage publication with failure fallback (no savepoints)
- [ ] Atlas occurrences, metadata revisions, actor index receipts
- [ ] `brain_backfill`, `atlas_actor_index`, `job_cleanup`, `brief_canonical` savepoint replacements
- [x] Graph edges from atlas/insight records: Source, MentionedIn, Contains (moved from P2). `provenance::memory_record_edges` / `actor_mention_edges` read them at query time from canonical records, as `brain_topology_query` does, rather than storing `brain_graph_edge` rows. Gaps: the `current_metadata_sql` article-revision filter is not applied; thread sources carry no title (`RecordEdge` has no label field); since the topology gaps pass they skip deleted and unknown Recon threads, as the SQLite join on `recon_threads.deleted = 0` does (`tests/storage_topology_gaps.rs`).
- [ ] The 14 write sites moved from P2 (`delete_memory`, atlas deletes/prune, insight publication, metadata, `ensure_schema`; see design.md)

## P4 — TUI/CLI + storage commands (spec steps 7–9)

- [x] Wipe vs import decided: wipe, no import (user, 2026-10-22)
- [x] `storage::open::open` is the only production path to a SurrealDB client: it connects (loopback server with root sign-in, or embedded RocksDB), selects ns/db and runs `ensure_schema` before returning. `Target`'s Debug redacts the password. Gap: `Store::open` and the CLI/TUI do not call it yet, and credentials/port still come from the caller (no `server.json`/`credentials.json` discovery).
- [x] Server lifecycle library (`storage::server`): discovery under `server.lock` (pid, TCP, root sign-in, 3.3.4 version, `storage_meta:identity`), start on demand (free or configured port, bounded port retries, credentials via env, detached process group, 30 s readiness, schema bootstrap, identity kept across restarts), leases, and last-client stop.
- [ ] Server lifecycle gaps: the `argos storage install|start|stop|status` CLI is not wired. The resolved binary is not hash-checked at start; only `install` checks it. Health is sign-in plus version, not the HTTP `/health` endpoint. Pid liveness uses `ps -p`, which is Unix-only.
- [x] Pinned 3.3.4 SHA-256 per OS confirmed by two independent HTTPS fetches + GitHub asset digest (box)
- [x] `argos storage install [--from <archive>]` (`storage::install`): HTTPS-only download from the GitHub release, archive SHA-256 checked against the compiled-in pin, unpacked with system `tar` (Unix), `surreal version` must report 3.3.4, then installed atomically as `bin/surreal-3.3.4`. A real download on the box (linux-amd64) installed and reported 3.3.4.
- [ ] Same hashes re-confirmed from a CI runner network
- [x] `server.lock` and `embedded.lock` as OS advisory locks (std `File::try_lock`, `flock` on Unix), pid informational only (`storage::home::try_lock`). `open_embedded` takes `embedded.lock` before opening and fails fast with the documented message. Config and credentials files are written atomically with mode 0600, and the root secret comes from `/dev/urandom`, which is Unix-only.
- [x] `clients/<pid>` leases written atomically (temp + fsync + rename)
- [x] Tests: stale `server.json` dropped and restarted; live server adopted; wrong-identity port refused; another live lease keeps the server running; last client stops the server; a second embedded owner fails (same process, separate lock fd)
- [x] `argos storage init --mode server|embedded`, `status` (mode, address, server, schema, epoch, readiness, pending reset), `start`, `stop [--force]` and `reset --yes` in both modes (`storage::commands`, CLI behind the `surreal` feature). Reset requires exclusive ownership and keeps config, credentials, settings, models and datasets. It wipes `argos.db` and its sidecars plus `memory_lancedb/`, then removes the `argos` database (database-scoped in embedded mode too, because the SDK cannot close a datastore; design.md 7b), bootstraps and publishes a new `storage_meta:epoch`. Embedded opens wait out the SDK's asynchronous RocksDB LOCK release (20 s bound, this process's own handle only).
- [ ] Storage command gaps: reset does not stop running workers itself; it refuses while other leases are live. `init` refuses a mode switch until reset, but reset does not migrate between modes, and the old mode's data directory is left in place. `status` in embedded mode opens the store, which applies the idempotent schema.
- [x] `storage::home::open_home` is the consumer entry point: it opens the store in the configured mode (server lease or embedded lock) and always goes through `storage::open` and `ensure_schema`. An uninitialized home is an error and there is no SQLite fallback.
- [x] Option B (Code Architect, 2026-10-23): `storage::repo::Repository`, an async, cloneable handle whose constructors (`open_home`, `connect`, `attach`) all run `ensure_schema`. `storage::runtime` is the single entry point: `init` records the binary's one Tokio runtime handle and the home, `repository()` opens the home once per process, and `blocking()` serves plain threads but refuses inside an async context, so there is no nested `block_on`. Parity: `tests/support::root` now hands out clients through `Repository` in both modes, so every existing storage contract suite runs against the repository path. Only the bootstrap test uses `raw_root`.
- [x] `storage::memories` (repository area): `add_memory`, `list_memories` and `recall` on the repository, in the same JSON shapes as the SQLite `Store`. Index work is queued in the same publication. Gap: `recall` has no query embedding, so the lexical path ranks (see the sign-off blocker below).
- [x] `storage::insight_views` (repository area): `search_insights`, `insight_for_memory` and `insight_metadata` on the repository, in the same JSON shapes as SQLite (the active snapshot wins, and edited memories read as `user_edited`). Gap: there is no `recon_entities` table in SurrealDB, so `entity` is the claim's `entity_id` and the entity filter matches that id only, not a label.
- [x] `storage::browse` (repository area): `get_memory`, `delete_memory` and `brain_browse`/`actor_groups`. Claims are active `claim` memories, pinned first then newest. Actor groups are live (active, non-redirected) actors grouped by normalized name, with every redirected member included. `delete_memory` wraps the P3 `insight::delete_memory` publication and, after commit, drops the vector and runs the job cleanup sweep. The `storage_browse` tests run in both modes and include the lexical recall fallback with embedding disabled or failing. Gaps: there is no untyped legacy claim fallback (every SurrealDB memory has a kind), and the NLP cache removal stays in SQLite.
- [x] Recon threads, messages, message-memory links, `brain_memory_endpoint` and `brain_memory_actor_ref` have SurrealDB tables; the SQLite FK cascades are delete events (thread → messages → links; memory → endpoints and actor refs), and `answer_memories` reads them (`tests/storage_recon.rs`, both modes, asserts no orphans after a thread delete).
- [x] `brain_browse_memories`, `claim_associations`, `actor_group_associations` and `article_origins_for_memory` run on the repository (`storage::browse_assoc`, `tests/storage_browse_assoc.rs`, both modes): typed claim actors through redirects, actor refs, direct mentions (stale revisions skipped), active Links, insight sources and live derivations. Not ported: the cycle-scoped variants (`actor_groups_scoped`, `actor_group_associations_scoped`) and `actor_group_graph`.
- [x] Phase 4b part 1: insight derivation writes and article association retirement (`storage::derivations`, `tests/storage_derivations.rs`, both modes). `derivation_step` is the `publish` step of `insight::publish_snapshot_on`; it writes canonical memories, `insight_claim`, `insight_source`, derivations and evidence (premise and memory-ref evidence included), readiness and links, and queues index work plus the `memories_changed` bump in the same transaction. Decision: the caller supplies the live Recon opt-in (`DerivationContext::save_insights`, already rechecked) and the Recon run's thread, because `recon_runs` is not on SurrealDB. Gaps: no `recon_entities` rows; `model_versions_json` stays `{}` (no stage checkpoints on SurrealDB); the publisher prior comes from any run's metadata for the article (SQLite uses the newest rowid); the job retention sweep runs after commit, not in the transaction. Claim actor binding, typed claim/inference payloads and the per-kind carry-forward are part 2.
- [x] Phase 4b part 2: claim-subject and actor binding plus the per-kind carry-forward (`storage::actor_bind`, `storage::derivations::write_article_derivations_for_kind_in`, `tests/storage_derivations.rs`, both modes). `bind_in` ports `brain_actors::bind` (scoped external ref, then the sole scoped alias match, then the deterministic provisional actor) over the new `brain_actor_alias` and `brain_actor_external_ref` tables. Claims get typed payloads with their subject/mentioned actor refs, inferences get premise memories and premise actors, and a single-kind article refresh carries the other kind's derivations and evidence forward (recorded in `readiness_json.carried_kinds`). Gaps: trusted identifiers are refused in `bind_in` (publication never passes any); no `brain_backfill_receipts`; the subject label comes from the extraction because `recon_entities` is not on SurrealDB.
- [ ] **Phase 4 sign-off blocker (Code Architect):** recall on the repository must use a query embedding (vector recall over `memory_embeddings`), not lexical-only.
  - Library side (this step): `storage::vectors` adds the serving generation (`storage_meta:vectors`), `reindex`, post-commit embedding of new memories, and `query_vector`. `memories::recall` embeds the query when the serving generation's model matches and falls back to lexical on embedding failure; `storage_vectors` covers this in both modes with a fake embedder. Still open before sign-off: the TUI wiring.
  - CLI (this step): smoke-tested with the real all-MiniLM model in embedded mode. `argos recall "When does the ship leave port?"` ranks "The vessel departs Rotterdam at dawn" first (vector, 0.46), and with `ARGOS_EMBED=0` the same query returns nothing.
  - Gaps: `index_change` rows that inline indexing satisfies stay pending, because the outbox worker isn't ported. `argos memories reindex` reports `generation` instead of `lance_dir`.
- [x] CLI (surreal builds): `main` calls `storage::runtime::init` once inside the Tokio runtime. When `storage/config.json` exists, `remember`, `recall` (with `insight_metadata`), `memories`, `memories reindex` and `insights` run only on `runtime::repository()`, and no `argos.db` is opened or created. Without a config they stay on SQLite. Smoke-tested in embedded mode. Gap: no bin-level automated test yet; the core contract tests cover the operations.
- [ ] CLI and TUI call sites still use the SQLite `Store`; they move area by area onto `storage::runtime` (CLI, then TUI). The SQLite path stays compiling and green under default clippy until phase 5.
- [ ] Jobs on the repository (step order: Jobs, then Recon remainder, then Profile and the rest)
  - [x] Part 1: `storage::jobs_view` — `list_jobs` (eligible = 72-hour window, not retired; active first, services last; status/operation/text/since filters), `get_job`, `job_counts`, `job_apps`, `job_detail` (children, bounded tasks with open/failed first, 20 newest attempts with durations, retryable count) and `retry_failed_tasks` (index/summary tasks only, under `job_guard`, refused on cancel request; reopens a failed/partial job). Same `jobs_view` types as SQLite. `tests/storage_jobs_view.rs`, both modes. Gaps (closed by the metadata commit below where marked).
  - [x] Job metadata (Code Architect): `job` gains app, title, provider, model, tool, worker owner, run/resource/result refs, error category/summary, correlation id, stage coverage, progress, heartbeat/active-since, durable active/queue/retry-wait ms (NONE = Unavailable), attempts used/cap and a cancellable flag; `task` gains max attempts, durable timing, error text and result ref; `attempt` gains duration and error text. Written with `jobs_view::set_job_meta`. The app filter and `job_apps` work. Retry clears the requeued tasks' error text, resets their failed `index_change` rows to pending and records a `job.retry` row in the new `job_event` table, all in its transaction; `JobRow.events` counts `job_event` rows for the job and its children. Gap: executors don't populate the metadata yet (callers set it when they move to the repository); `job_event` is a Jobs-only subset of `argos_events` (the Logs view stays on SQLite).
  - [x] Write side 1: every repository job gets its app (from the operation, `jobs::job_app`) and title (`jobs::job_title`) at creation; closing an attempt (complete, requeue/fail, stop) stores its `duration_ms`; completing a task stores its `active_ms`; `finalize_job` and stop (`cancel_one_in`) roll up attempts used/cap, active time, queue time (NONE when no task started) and the failed-task summary/category in the same transaction (`jobs::rollup_in`). `storage_jobs_view::jobs_record_dashboard_metadata_on_create_and_finish`, both modes. Gaps: provider/model/tool, run/resource/result refs, progress and heartbeat stay for the executors to set with `set_job_meta` as they move; requeue does not yet store a task error message.
  - [x] Write side 2: under a storage config the TUI runs the job retention sweep (`jobs_view::sweep_expired` over `jobs::cleanup_batch`, 64 batches max) at startup and hourly on a plain std thread; the SQLite `clear_old_jobs` is skipped. Contract test `retention_sweep_deletes_expired_trees_only` in both modes. Gaps: sweep failures are silent until logs move; the CLI `open_store` still clears SQLite jobs only (no CLI job commands exist).
  - [ ] Retry of Intel brief chains (`brief_pipeline::retry_stage`) and insight extraction (`insight_execution::retry_extraction`): land with the brief pipeline and insight-execution moves. Brief stage labels, stage warnings and wire-request counts in Job detail move with them.
  - [x] Part 2: under a storage config the TUI Jobs pane reads and acts only on the repository: list, counts, apps, detail and focus run through `jobs::JobsSource` on `surreal_brain::SurrealJobs` from plain worker threads (`runtime::blocking`), and Retry, Stop (`jobs_view::cancel_job` → `jobs::stop_job`) and Clear history (`jobs_view::clear_jobs`) run off the UI thread; without a config the pane stays on SQLite. `storage_jobs_view::jobs_pane_stop_and_clear_history_on_the_repository`, both modes. The CLI has no job commands; `open_store` still runs the SQLite `clear_old_jobs` for the non-Brain commands that open the SQLite store. Gaps: no bin-level automated test of the surreal Jobs pane; Logs (and the Logs → Jobs event counts) stay on SQLite `argos_events`; the retention sweep (`jobs::cleanup_batch`) has no periodic caller in the TUI yet.
- [ ] Recon remainder (Code Architect, after Jobs, before Profile): move the Recon callers off the SQLite `Store` (credits now in `storage::credits`), and port Recon messages, tools, artifacts and ordered counters.
  - [x] Part 1: `storage::recon_threads` — `new_thread`, `list_threads` (title or linked-entity canonical, case-insensitive, newest first), `rename_thread`, `set_recall_insights`, `save_draft`, `select_thread`/`last_thread` (`app_state`), `list_messages`, `add_message` (sequence from the ordered counter `counter:[recon_message:<thread>]` in the message's transaction, first raised to the thread's highest stored sequence), `link_entity`/`thread_entities` (new `recon_thread_entity`, cascades with the thread) and tool preferences (`osint_preference`, unset = `osint::default_enabled`). `tests/storage_recon.rs` (`recon_threads_and_messages_match_the_sqlite_store`, `concurrent_appends_get_distinct_sequences`, `tool_preferences_default_and_persist`), both modes. Gaps: `link_entity` does not yet record the Brain actor mention (`record_thread_actor`); thread delete stays the hard delete of `recon::delete_thread`.
  - [ ] Part 2: runs, tool calls (`osint_calls`), message evidence and the tool cache (`osint_cache`), then the run executor's writes (`new_run`, `add_answer`, `queue_call`/`finish_call`, `cancel_run`/`recover_runs`) and the full `delete_thread`.
  - [x] Part 3: TUI and CLI Recon callers on the repository under a storage config (never mixed with the SQLite store). TUI: `tui::surreal_recon` (Jobs worker pattern: plain worker threads through `storage::runtime::blocking`, results as `WorkEvent`s, nothing on the UI thread) serves the thread list and run states, the open thread's messages/calls/runs/answer memories, Tools history, new threads (Home launch, Tools "start Recon"), drafts, selection, rename, delete, recall and tool toggles and result attach; boot reads the last thread, thread list and tool settings once before the event loop. CLI: `recon list/new/show/rename/delete/ask-new/retry/resume`, the ask output and `osint history/attach/enable/disable` open `ReconStore`. `ReconStore` gained `all_calls_for_thread`, `manual_calls`, `latest_run_state`, `save_draft`, `select_thread`, `last_thread`, `delete_thread` (`tests/storage_recon_dispatch.rs`, both backends). Gaps:
    - Deleting a thread with its insights (TUI `:delete`/Delete, CLI `--with-insights`) is SQLite-only: the TUI deletes the thread and keeps its insights on SurrealDB; the CLI flag fails with "not on SurrealDB yet".
    - ~~Investigation events and tool-result call logs in the Recon pane come from the SQLite event log and are empty on SurrealDB~~ Done in the telemetry track: the pane reads `investigation_event` and records call logs in `argos_event` (see Telemetry below).
    - Under SurrealDB the TUI resolves thread drafts and tab restore from the loaded thread list (filtered by the Recon search), not a per-thread read.
    - No TUI end-to-end test on SurrealDB; the dispatch contract test covers the store calls.
  - [x] Core dispatch: `recon::backend::ReconStore` routes every Recon store call to the SQLite `Store` or to `storage::*`, chosen once per process by `storage::runtime::configured()` (never both). `recon::Service`, `orchestrate` and `tool_runner` open `ReconStore` and await it. `tests/storage_recon_dispatch.rs` drives the same thread/run/call/answer/credits scenario on both backends and compares the results. Still unported under SurrealDB (skipped, or SQLite-only):
    - Telemetry (`StageClock`, directive and run notes, tool-runner notes) moved in the telemetry track (see Telemetry below). Model attempts and drafts and the model capability cache (`model_exec` gets no db), the job registry (Recon runs are untracked), the summarization/NLP cache (`nlp::cache::register`, `compact_page_evidence` caching), and the NewsAPI quota (`charge_quota`).
    - Recall: `recon_extract::recall_for_turn` only. `admit_memory`, `brain_is_thin` and the Brain resource summary are not ported, so recall telemetry is absent and the summary is the default.
    - `previous_synthesis` is the deterministic follow-up with no cache.
    - Claim assessment: `insights::run_pipeline` runs on SurrealDB through `investigation::exec_store::ExecStore` and `storage::pipeline_exec` (6ae61be); `refresh_driver::recover_and_run` drives refreshes at startup and a completed refresh does not fall through to mining (d83bc2f).
    - Retry: `retry_insights` on SurrealDB only re-mines the answer; there is no shared-refresh retry.
    - No end-to-end `Service` (ask/resume) test on SurrealDB; the contract test covers the store calls.
    - `ReconStore::open` per run/resume: under SurrealDB it returns the process repository cached by `storage::runtime::repository` (a `OnceCell`, opened once per process and cloned), so the embedded file is not reopened; under SQLite it opens a `Store` per call as before.
  - Job executor refs (Code Architect): the provider, model, tool and run/resource/result refs on `job` are deferred until the executors themselves move to the repository; each executor sets them with `jobs_view::set_job_meta` in its move.
- [ ] CLI/TUI workers, Profile (20 views), telemetry, summaries, NLP artifacts
  - Includes deferred P3 item 3: `refresh_cached_publication` moves with the summaries / Intel brief pipeline move (dependency list in the P3 entry above).
- [x] `argos storage status`, `init`, `reset --yes` with resumable phases (`storage/reset.json`: wipe_legacy, wipe_store, bootstrap; tested by resuming from bootstrap)
- [ ] Fresh population checks; two-session operation; restart reads
- [ ] RSS/swap/disk/p95 measurements on the target machine

## P5 — `feat!` removal (spec step 10)

- [ ] Remove rusqlite, LanceDB, SQL schemas/migrations, protoc shim
- [ ] Docs, diagrams, CI and packaging for the separate server; backup/restore notes

## Gates (every phase)

- [ ] `cargo fmt --all --check`
- [ ] clippy `-D warnings` + tests via `scripts/agent_cargo.py`: default; `--locked --no-default-features`; `--no-default-features --features nlp`; `--no-default-features --features surreal`; `--no-default-features --features embedded`
- [ ] `python3 scripts/test_surreal.py` (server + embedded)
- [ ] `graphify update .` as the last commit
- [ ] Atlas deletes (step 3) on Surreal leave to the caller: NLP cache/`nlp_source_refs`, `brain_backfill_receipts` cleanup, `jobs::cleanup_batch`, and `retire_deleted_article_leads` (reads `app_state`).
- [ ] `atlas_save_metadata` on Surreal does not run `persist_article_rating` (confidence inputs still SQLite).
- [ ] Step 4 remainder: `legacy_receipt` and the Source/MentionedIn/Contains edges are done. Still open: `bind_claim_subject` and `refresh_cached_publication`; see P3 above for exactly what each still reads from SQLite.

### Phase 4c — pipeline inputs behind the repository

`storage::pipeline_inputs` adds `article_body`, `nlp_artifact`, `nlp_source_ref` and `app_state` (bootstrap now 16 schemas) and ports `article_input` (retained clean/baseline body, acquisition coverage, metadata passage and atlas metadata actors) and the Recon input (thread `recall_insights` gates both saving and recall; generation advanced from 0). Covered in both modes by `tests/storage_pipeline_inputs.rs`.

Gaps: no tool evidence on article inputs (`osint_calls`/`intel_evidence` not on SurrealDB yet); Recon evidence is built by the caller (`evidence_from_tools` still reads the SQLite NLP cache) and only its texts are retained here; source-ref recency uses `registered_at` instead of SQLite rowid; nothing writes these tables from the pipeline yet (acquisition writers move with phase 5).

### Phase 4d — refresh slice and recovery

`storage::refresh_exec` ports `run_refresh_slice` (exact-task claim, admission, 30 s heartbeat with supersession drop, slice continuation, supersession, cancel-to-paused, failure, fenced publication with derivations) and the admission half of `recover_refreshes` (article intents rescheduled from `pipeline_inputs::article_input`, expired leases reaped, queued ids returned with captured settings). Task messages and `blocked_reason` live on the `insight_refresh_task` sidecar. Covered in both modes by `tests/storage_refresh_exec.rs`.

Gaps: the pipeline run, live-evidence validation and the derivation context (live Recon opt-in) are caller-supplied; `insight.recovery_error` events are returned, not recorded; `retire_atlas_insight_work` is not ported; no error category or `max_attempts` on Surreal tasks (failures are terminal); the slice-yield path is not covered by a contract test (`SliceYield` is crate-private); nothing in the CLI calls these yet.

### P4 — TUI Brain on the repository

Under a storage config the TUI Brain list, preview and memory delete run only on SurrealDB (`tui::surreal_brain`), through `storage::runtime::blocking` from the Brain's existing plain worker threads (never async code or the draw loop). An unreadable config counts as configured, so it errors on the SurrealDB path instead of silently falling back. Gaps: pin/edit, Find (`brain_browse_memories` with a query), the synchronous non-background path and the rest of the TUI still use the SQLite `Store`; no TUI test drives the SurrealDB branch (verified by both-feature clippy only).

### P4 — scoped Actor-group views and graph

`storage::browse_assoc` adds `actor_groups_scoped`, `actor_group_associations_scoped` and `actor_group_graph` (the graph reuses `Store::actor_group_graph_from`, so both stores build it identically). `actor_groups_scoped` reads the association inputs once for all groups instead of once per group. Covered in both modes by `tests/storage_browse_assoc.rs`. Gap: the TUI's Actor-group views still call the SQLite `Store`.

### P4 — TUI refuses the Actor-group topology on SurrealDB

Under a storage config `App::open_topology` refuses `TopologySeed::ActorGroup` with "Actor-group topology isn't available on SurrealDB yet." before anything is queried, so the TUI never reads SQLite for it (covered by `surreal_refuses_only_the_actor_group_topology`). Other topology seeds are unchanged.

## Phase 5 — first item (blocks phase 5 sign-off)

- [x] Port the `brain_topology_query` actor-group seed to the repository (contract tests in both modes) and wire the TUI topology worker to it through `storage::runtime::blocking`, removing the SurrealDB refusal above.
  - [x] Repository port: `storage::topology::topology_snapshot` for `TopologySeed::ActorGroup` (member resolution through redirects; the members' typed Links/Related, actor refs, typed payload actors and native mentions, re-pointed at the group; SQLite admission, findings and deterministic brief shared through `brain_topology::query::admit`). Covered in both modes by `tests/storage_topology.rs`.
  - Gaps: other seeds are refused with an error; depth > 1 does not expand past the group's own neighbours (reported as the bounded `DEPTH_GAP`); `ThisCycle` scope skips the current-metadata fence; memory nodes carry no revision (SurrealDB memories have none, so typed-payload receipts read `memory_metadata:<id>@`); reads are not one snapshot transaction.
  - [x] TUI wiring: under a storage config `request_topology` reads `TopologySeed::ActorGroup` only through `tui::surreal_brain::topology` (`storage::runtime::blocking` on a plain worker thread, also for in-memory fixtures); the refusal from 4caaf10 is removed (`surreal_reads_only_the_actor_group_topology_from_the_repository`).
  - Gaps: the generated neighbourhood summary is skipped for this seed on SurrealDB (its cache and tasks are SQLite-only; the deterministic brief shows); other topology seeds and the rest of the TUI still use the SQLite `Store`; no TUI test drives the SurrealDB worker (both-feature clippy only).

### Phase 5 — remaining topology seeds and TUI wiring (a6ac67b, 08f4fca, 05880e0)

- [x] `storage::topology::topology_snapshot` answers the Claim, Actor, Inference, Article, Topic, Source and Link/Related seeds as well as the Actor group, walking breadth-first to depth 1–3; admission still goes through the shared `brain_topology::query::admit`. Covered in both modes by `tests/storage_topology.rs` (every seed, premises both ways, stale revisions, cycle scope, depth 2 and 3), which supersedes the `DEPTH_GAP` above.
- [x] TUI wiring (05880e0): under a storage config the topology worker reads every seed through `tui::surreal_brain::topology`, so topology never mixes stores.
- Retained Intel seeds read `storage::brief` publications and bind canonical memories through `intel_brief_stage_receipt` / `intel_brief_index_state` (3dae43c).

### Phase 5 — Profile read model (542b7c0, 9490f2f)

- [x] Profile reads its rows through a `ProfileSource`; `storage::profile_view` loads the source tables from SurrealDB into `ProfileTables` and builds the snapshot with the shared aggregation, adding the telemetry tables schema. SQLite parity covered in both modes by `tests/storage_profile_view.rs`.
- [x] TUI Profile on the repository under a storage config: the 20-second refresh reads the snapshot and filter options through `tui::surreal_profile::dashboard` (`profile_view::dashboard`, one table load) on a plain worker thread, and the attention actions' run lookup goes through `recon_runs::get_run` on a worker (`WorkEvent::ProfileRunOwner`, only the newest lookup while System is showing). Stale snapshots (generation or filters changed) and stale lookups are dropped; the SQLite path is unchanged and never mixed in. Covered by `storage_profile_view` (dashboard parity, both modes) and the TUI unit tests `a_stale_profile_run_lookup_is_dropped`, `a_profile_run_lookup_after_leaving_system_is_dropped`, `a_current_profile_run_lookup_reports_a_missing_or_failed_run`, `a_stale_profile_snapshot_generation_is_dropped`.
- Gaps: ~~nothing writes the telemetry tables on SurrealDB yet~~ (telemetry writers moved, see Telemetry below); recon run states/coverage, intel report jobs/tasks and recon model operations/attempts stay unavailable on SurrealDB until their writers move; no end-to-end TUI test of the Profile screen on SurrealDB.

### Phase 5 — Recon runs, calls, evidence and cache (3fe8789)

- [x] `storage::recon_runs` adds `recon_run`, `osint_call`, `recon_message_evidence` and `osint_cache`, mirroring the SQLite `Store` (run state machine, call queue/finish/attach, in-flight duplicate detection, cache freshness). Covered in both modes by `tests/storage_recon_runs.rs`.
- Gaps: nothing calls `storage::recon_runs` yet; the executor writes, the full `delete_thread` cascade and the caller switch follow.

Bootstrap registers 21 schemas after the topology, Profile and Recon tracks merge.

### Phase 5 — Recon executor writes

- [x] `storage::recon_exec` adds `extraction_job`, `recon_model_operation` and `investigation_strategy` (bootstrap now 22 schemas) and ports `add_answer` (one transaction: active-run check, message, evidence, surviving memories in order, extraction job, synthesis operation finalized, run stage complete), `has_final_answer_for_run`, `recover_runs` (runs and calls interrupted, running extraction requeued, held credits released), the model operation upsert/draft/finish the executor streams into, and `record_strategy`/`latest_strategy_kind`. Covered in both modes by `tests/storage_recon_exec.rs`.
- Gaps: nothing calls these yet; `recon_model_attempts`, `task_epoch` and the job executor refs are not ported; `recover_runs` releases credits after its updates, not in the same transaction (as SQLite).

### Phase 5 — full Recon `delete_thread`

- [x] `storage::recon::delete_thread` now runs the SQLite cascade (insights kept) in one transaction: actor mentions; insight sources detached and marked `deleted_origin`; in-flight calls cancelled; the runs' extraction jobs and model operations, strategies, messages (evidence and memory links by event), runs, entity links, message counter and `last_thread`; calls detached, then detached Recon calls no insight source or derivation evidence cites deleted. Manual calls and `osint_cache` stay (the cache is keyed by tool inputs, as in SQLite). Already-deleted threads return false. Covered in both modes by `tests/storage_recon_delete.rs`.
- [x] `delete_thread_with_insights`: hard delete of the doomed memories, recon owner generation bump, `origin_deleted` links, NLP cache owner removal (18f6b74).

### Phase 5 — Recon answer extraction and recall stopgap

- [x] `storage::recon_extract::persist_claims` ports `recon::persist_claims` as one publisher transaction: answer/thread liveness check, opt-out completes the job with no claims, up to five validated claims (memory, `insight_claim`, subject binding, `conflict_or_revision` relations, `insight_source` rows for completed cited evidence), tombstoned claims from the same run skipped, extraction job completed, memory-index outbox work queued for every new or reused memory, `memories_changed` bumped only when a memory was written. Covered in both modes by `tests/storage_recon_extract.rs`.
- [ ] `storage::recon_extract::recall_for_turn` is a **temporary stopgap** for `orchestrate::recall_for_turn`: SurrealDB full-text search (BM25, `memory_text_search` index) instead of LanceDB vector recall. It must be replaced when vector recall lands; the fixed-corpus contract test locks in today's ranking so the swap can be checked. It runs one `@1@` query per distinct question term (OR semantics) and ranks by terms matched, then summed BM25 score, because BM25 idf is negative for a term in more than half the memories and a positive-score cut dropped real matches. `persist_claims` reports the subject actor memories it binds in `actors` (index work is queued for them) and keeps `touched` to claim memories, as on SQLite.
- [x] Turn recall admits candidates before the five-memory cap and adds the brain resource summary (eb9814a). Recall telemetry: see "Still needs SQLite".

### Phase 5 — Recon insight owner state (Recon switch, step 2)

- [x] `storage::insight_owner` ports the Recon side of `insight_persist` as repository methods: `desired_generation`, `advance_generation` and `enqueue_refresh` (each one transaction, over `insight::desired_generation` / `advance_generation` and `refresh::enqueue_refresh_in`), `active_output` (owner head, snapshot, live derivations and assessed links read in one transaction; a non-`active` lifecycle is ineligible with an `insight lifecycle: …` gap), `reconcile_deleted_memories_in` / `reconcile_deleted_memories` (the SQLite batch: dangling derivations `deleted` and detached, their items' links `deleted_endpoint`, owners on those snapshots `previous` with `memory endpoint removed`, dangling evidence memory ids cleared, evidence kept), and `recon_context_in` / `recon_context`, the live Recon opt-in (`save_insights` AND a run on an undeleted thread with `recall_insights`, plus the run's thread) that `run_refresh_slice` takes as its `DerivationContext`.
- [x] `live_evidence::validate_in` now validates Recon owners (`recon_run` exists, not cancelled, on an undeleted `recon_thread`) and tool evidence (`call:<id>[#page…]`: a `completed` `osint_call` whose `result_json` hashes to the reference revision), so `run_refresh_slice` and `recover_refreshes` (unchanged; they already admit any owner's queued tasks) can run a Recon refresh with `live_evidence::step`.
- Covered in both modes by `tests/storage_insight_owner.rs`; `tests/storage_live_evidence.rs` now expects `Recon owner no longer active` for a missing run. No new tables (bootstrap count unchanged).
- Gaps: no caller uses these yet (step 3). `enqueue_refresh` does not requeue a paused task (`blocked_reason='cancelled; evidence dirty'`): SurrealDB tasks have no `paused` state. `reconcile_deleted_memories` is not yet wired into the SurrealDB delete paths that need it (`insight::delete_memory` does its own targeted cleanup; the Recon `delete_thread` `insights = true` path is still unported). `validate_dispatch` / `validate_retry` (insight-execution retry) and `retire_atlas_insight_work` stay SQLite. Recon `PipelineInput` building and recall belong to step 1.
- [x] Merged with step 1: bootstrap registers 23 schemas (22 + `recon_extract`; step 2 adds none), asserted by `tests/storage_bootstrap.rs`.

### Phase 5 — topology gaps (TUI memory check, thread provenance)

- [x] The TUI memory check that runs when Enter opens a Brain memory's topology (`Store::get_memory` plus `brain_kinds::memory_kind`) no longer reads SQLite under a storage config: `storage::topology::memory_seed_kind` returns the memory's kind (`None` when it no longer exists; a missing or unknown kind reads as a Claim, like SQLite's untyped rows), and the TUI calls it through `tui::surreal_brain::memory_kind` (`storage::runtime::blocking` on a plain worker thread, the Jobs worker pattern). `WorkEvent::MemoryGraphChecked` opens the mapper only while that memory is still selected. The SQLite and SurrealDB paths share `memory_graph_seed` for the seed and status text. The Brain screen's topology path (list, preview, delete, memory check, snapshot) now reads only one store.
- [x] `provenance::memory_record_edges` skips thread sources whose `recon_thread` is deleted or missing.
- Covered in both modes by `tests/storage_topology_gaps.rs` (added to `scripts/test_surreal.py`); TUI unit tests cover the seed mapping and that a late check is dropped. No new tables (bootstrap count unchanged at 23).
- Remaining topology gaps: the retained Intel display seeds (`RetainedClaim`, `RetainedInference`, `RetainedActor`) are still refused on SurrealDB, since Intel brief publications are SQLite; the generated neighbourhood summary is skipped on SurrealDB (its cache and tasks are SQLite-only; the deterministic brief shows); the `current_metadata_sql` article-revision fence is not applied; memory nodes carry no revision; snapshot reads are not one transaction; thread provenance edges carry no title; no TUI test drives the SurrealDB workers end to end (both-feature clippy only).

## Telemetry on SurrealDB (telemetry track)

- [x] `storage::telemetry::record` is `telemetry::insert` on the repository: one transaction writes `telemetry_event` (keyed by event id, including `thread_id`/`turn_id`) and, only the first time an id is seen, folds it into `telemetry_hourly`/`telemetry_daily` (same additive events/total/max and `bins_json` as SQLite), so replays never double count. `stage_attempt` gives the next `recon_stage` attempt ordinal. Uses `profile_view`'s tables (no new schema).
- [x] Routing: under a storage config `telemetry::record(db_path, …)` never opens SQLite; it starts the repository write on the process runtime (`storage::runtime::spawn`) and the binary waits for those writes before exiting (`storage::runtime::drain`). This covers `StageClock` (attempt read from the repository), `note_directives`, `note_run`, the tool runner's invocation/failure/cancel notes, the OSINT wire-request/engine-query notes (no longer skipped when the SQLite file is absent) and evidence-item events. Without a config every writer stays on SQLite.
- [x] `storage::event_log` adds `argos_event` (the `argos_events` sink: redacted, bounded, `seq` assigned in the write transaction) and `investigation_event` (insert or status/summary/payload update by id, `next_event_sequence`, list by sequence). Bootstrap now registers 24 schemas (`event_log`), asserted by `tests/storage_bootstrap.rs`.
- [x] `ReconStore` gained `list_investigation_events`, `insert_investigation_event`, `next_event_sequence`, `record_event` and `list_events`. Under a storage config the TUI Recon snapshot (`tui::surreal_recon::snapshot`, worker thread) reads the thread's investigation trace and records finished tool results not yet logged as `argos_event` rows, like the SQLite reader worker.
- [x] Contract tests (both modes, `tests/storage_telemetry.rs`): `telemetry_events_and_rollups_match_sqlite` (events, hourly/daily rollups, replay, stage attempts), `investigation_events_match_sqlite`, `event_log_records_redacted_ordered_events`.
- Gaps:
  - ~~The Logs pane, its severity counts, the TUI session log and the Jobs → Logs event counts still read/write SQLite `argos_events`~~ Done (logs track): under a storage config Logs reads `argos_event` on worker threads, `push_log` records there (ordered, `runtime::spawn_ordered`), startup and first-tick prune and Clear log run on the repository, `list_events` walks child jobs (`event_log::job_tree`), and `JobRow.events` counts `argos_event` plus `job_event` rows (job and direct children). `job.retry` is still written only to `job_event`, so it is counted but not listed in Logs.
  - Telemetry writes are fire-and-forget; a TUI killed mid-write can lose the last events. ~~`telemetry_meta`, `telemetry_backfill` and the 90/365-day prune are not on SurrealDB~~ Done (logs track): `storage::telemetry::{get_meta, set_meta, ensure_observed_since, coverage, prune}`; `telemetry::prune` routes there under a config and the TUI startup prune runs it. Register `("telemetry", super::telemetry::SCHEMA)` after `profile_view` at merge (+1 bootstrap count).
  - Writers that are still SQLite-only paths keep writing SQLite only when no config is set: Atlas, Atlas insights/memory, model attempts (`model_exec`, jobs track) and recall telemetry (`admit_memory` not ported).
  - The investigation harness (`investigation::engine`) has no production caller and still writes through the SQLite `Store`.
  - No end-to-end TUI test of the Recon pane trace or Profile event views on SurrealDB.


### Phase 5 — Recon job writers (repository layer)

- [x] `storage::recon_jobs` ports the remaining Recon/Intel job writers as repository methods with their own schema (`recon_jobs::SCHEMA`: `task_epoch` on `recon_model_operation`, `recon_model_attempt` keyed by `(operation_id, generation, route_index, attempt)` and removed with its operation, `recon_coverage`, `intel_report_job` and `intel_report_task` removed with their job): model operation `task_epoch` (kept when a later ensure omits it) and attempt start/finish/list (a replayed start resets the same row); coverage record save/load by scope and generation; Intel report job insert/get/list/revision/update (retired jobs untouched) and task insert/list/claim (dependencies complete, lease guarded so one claim wins)/complete/fail/expired-lease interrupt; Recon job registry rows (begin or resume with attempts + 1, phase returning the cancel flag, finish, `job_for_run`). Covered in both modes by `tests/storage_recon_jobs.rs`, which applies the full bootstrap plus `recon_jobs::SCHEMA` in setup.
- [x] Profile on SurrealDB now loads recon run states, report jobs/tasks, coverage and model operations/attempts from these tables. If the schema is not registered, they show as unavailable.
- Gaps: `recon_jobs::SCHEMA` is not in `storage::bootstrap` yet. Register `("recon_jobs", super::recon_jobs::SCHEMA)` after `recon_exec` at merge, which adds one to the bootstrap count. No production caller is switched yet. On SurrealDB, the executor's attempt ledger, task epochs and the Recon job registry row are still skipped, as before. The Intel report worker and `brain_neighborhood_brief_exec` (the only `task_epoch` writer) remain SQLite subsystems. Coverage has no production caller on either store. There is no capability-cache table to port.

## Combined tree (telemetry + job writers + remaining gaps)

- [x] Merged telemetry (304aec4), job writers (7b58568) and the remaining
  gaps branch (233b224). Bootstrap registers 27 schemas (`recon_jobs` after
  `recon_exec`, `brief` after `intel`, `pipeline_exec`), asserted by
  `tests/storage_bootstrap.rs`; per-test `apply_schema` calls dropped.
- [x] Model ledger on the repository (e835e5b): `ensure_operation` with
  `task_epoch`, drafts, status, attempt start/finish, in order through
  `storage::runtime::spawn_ordered` (drained on exit).
- [x] Recon job registry on the repository (4ec0549): begin/resume, phase with
  cancel, finish.
- [x] Events on SurrealDB (c4a5def): `ExecStore` stage and activity events
  write `argos_event`, turn recall records recall telemetry, and article
  refresh admission logs `insight.superseded` per retired task after commit.
- [x] `run_pipeline` reaches storage only through `ExecStore`; on SurrealDB it
  opens no SQLite file (telemetry path `None`). `ExecStore` keeps SQLite behind
  a mutex so spawned pipelines stay `Send` (0bd6912).
- [x] Server fixes: no `math::max`/`math::min` (floats on a server), no empty
  record ids in pipeline lookups.
- Gate at 4760a9e: fmt, clippy (core, workspace, bin), bin tests and
  `test_surreal.py` in server and embedded modes, all green.
