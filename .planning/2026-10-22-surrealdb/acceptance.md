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

- [ ] Recon/OSINT messages, tools, artifacts, ordered counters, credit hold/reconcile/release
- [ ] Intel stage publication with failure fallback (no savepoints)
- [ ] Atlas occurrences, metadata revisions, actor index receipts
- [ ] `brain_backfill`, `atlas_actor_index`, `job_cleanup`, `brief_canonical` savepoint replacements
- [x] Graph edges from atlas/insight records: Source, MentionedIn, Contains (moved from P2). `provenance::memory_record_edges` / `actor_mention_edges` read them at query time from canonical records, as `brain_topology_query` does, rather than storing `brain_graph_edge` rows. Gaps: the `current_metadata_sql` article-revision filter is not applied; thread sources carry no title and no `recon_threads.deleted` filter, because Recon threads are still SQLite.
- [ ] The 14 write sites moved from P2 (`delete_memory`, atlas deletes/prune, insight publication, metadata, `ensure_schema`; see design.md)

## P4 — TUI/CLI + storage commands (spec steps 7–9)

- [x] Wipe vs import decided: wipe, no import (user, 2026-10-22)
- [ ] Server lifecycle per design.md: `argos storage install|start|stop|status`
- [x] Pinned 3.3.4 SHA-256 per OS confirmed by two independent HTTPS fetches + GitHub asset digest (box)
- [ ] Same hashes re-confirmed from a CI runner network
- [ ] `server.lock` and `embedded.lock` as OS advisory locks (`flock`), pid informational only
- [ ] `clients/<pid>` leases written atomically (temp + fsync + rename)
- [ ] Tests: stale `server.json` adopt; wrong-identity port refused; last client stops server; second embedded process fails
- [ ] `argos storage init --mode server|embedded`; `status` shows mode; `reset --yes` in both modes
- [ ] CLI/TUI workers, Profile (20 views), Jobs, telemetry, summaries, NLP artifacts
- [ ] `argos storage status`, `init`, `reset --yes` with resumable phases
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
