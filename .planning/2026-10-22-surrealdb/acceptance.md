# SurrealDB migration checklist (acceptance)

See [plan.md](design.md). Status is honest: unchecked means not done or not
verified.

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
- [ ] `brain_backfill::bind_claim_subject`: not ported. It needs the `brain_actors::bind` engine (1,092 lines), which reads and writes `brain_actor_aliases`, `brain_actor_external_refs`, `brain_actor_dedup_receipts`, `brain_memory_actor_refs`, `brain_memory_endpoints` and `memory_metadata`. None of those have a SurrealDB table yet. `claim_subject` also reads `recon_entities` (Recon, still SQLite) and writes `brain_backfill_receipts`. Port it together with the actor tables and the Recon tables. Until then, `ArticleReplacement::created` and the receipts' fingerprints are returned for the caller to bind on SQLite.
- [ ] `refresh_cached_publication`: only its SurrealDB inputs are ported (`provenance::stage_outputs` through `stage_output_root`, and `republished_stages`). The captured metadata reads `argos_tasks.input_ref`, and SurrealDB brief-chain tasks have no `input_ref`. `prepare_for_stage` reads retained article evidence (`article_input`, SQLite), and `publish_completed_stage` writes `intel_brief_publications`, also SQLite. The function moves once those do.
- [ ] `publish_snapshot`'s validate callback still reads live evidence from SQLite outside the Surreal transaction. Unavoidable until the source tables move; then validate inside the tx.
- [ ] `storage::insight::delete_memory` has no callers yet. The caller wrapper must also clear the NLP cache (`nlp::cache::remove_owner`) and run the job sweep (`job_cleanup::cleanup_batch`), both still SQLite, and the NLP cache must treat a missing memory as a miss.

- [x] `schedule_article_refresh` (`storage/refresh.rs`) with `enqueue_refresh_in`, `retire_replaced_in` and `supersession`. Intent, retirement check, prior-task comparison, generation advance, job/task creation, the owner's `refresh_job_id` and supersession all commit in one transaction. The refresh task's `input_ref`, `role_snapshot` and `source_revision`, plus the job's `owner_scope`, live in an `insight_refresh_task` sidecar, because `task` and `job` have no such fields.
- [ ] `schedule_article_refresh` gaps: the caller still builds the `PipelineInput` from SQLite evidence (`article_input`), and does so before the intent check. The `insight.superseded` event is not recorded (`events` is still SQLite). The SQLite paused-task requeue (`blocked_reason='cancelled; evidence dirty'`) has no counterpart, because SurrealDB tasks have no `paused` state.
- [x] Step 5: `storage::bootstrap::ensure_schema` applies all 13 module schemas plus `storage_meta` in dependency order, in one transaction, and records `storage_meta:schema.version` (`SCHEMA_VERSION = 1`). It is idempotent, and it refuses (and rolls back) a database whose version is newer, like the SQLite `user_version` check.
- [ ] `ensure_schema` gaps: it has no caller yet, because `Store::open` is still SQLite. There is no migration ladder between Surreal schema versions (the store starts fresh, P4). The embeddings DISKANN dimension is fixed at 384 in `embeddings.surql` rather than taken from the configured model.

- [ ] Recon/OSINT messages, tools, artifacts, ordered counters, credit hold/reconcile/release
- [ ] Intel stage publication with failure fallback (no savepoints)
- [ ] Atlas occurrences, metadata revisions, actor index receipts
- [ ] `brain_backfill`, `atlas_actor_index`, `job_cleanup`, `brief_canonical` savepoint replacements
- [x] Graph edges from atlas/insight records: Source, MentionedIn, Contains (moved from P2). `provenance::memory_record_edges` / `actor_mention_edges` read them at query time from canonical records, as `brain_topology_query` does, rather than storing `brain_graph_edge` rows. Gaps: the `current_metadata_sql` article-revision filter is not applied; thread sources carry no title and no `recon_threads.deleted` filter, because Recon threads are still SQLite.
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
- [ ] **Phase 4 sign-off blocker (Code Architect):** recall on the repository must use a query embedding (vector recall over `memory_embeddings`), not lexical-only.
  - Library side (this step): `storage::vectors` adds the serving generation (`storage_meta:vectors`), `reindex`, post-commit embedding of new memories, and `query_vector`. `memories::recall` embeds the query when the serving generation's model matches and falls back to lexical on embedding failure; `storage_vectors` covers this in both modes with a fake embedder. Still open before sign-off: the TUI wiring.
  - CLI (this step): smoke-tested with the real all-MiniLM model in embedded mode. `argos recall "When does the ship leave port?"` ranks "The vessel departs Rotterdam at dawn" first (vector, 0.46), and with `ARGOS_EMBED=0` the same query returns nothing.
  - Gaps: `index_change` rows that inline indexing satisfies stay pending, because the outbox worker isn't ported. `argos memories reindex` reports `generation` instead of `lance_dir`.
- [x] CLI (surreal builds): `main` calls `storage::runtime::init` once inside the Tokio runtime. When `storage/config.json` exists, `remember`, `recall` (with `insight_metadata`), `memories`, `memories reindex` and `insights` run only on `runtime::repository()`, and no `argos.db` is opened or created. Without a config they stay on SQLite. Smoke-tested in embedded mode. Gap: no bin-level automated test yet; the core contract tests cover the operations.
- [ ] CLI and TUI call sites still use the SQLite `Store`; they move area by area onto `storage::runtime` (CLI, then TUI). The SQLite path stays compiling and green under default clippy until phase 5.
- [ ] CLI/TUI workers, Profile (20 views), Jobs, telemetry, summaries, NLP artifacts
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
