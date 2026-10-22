# SurrealDB migration checklist (acceptance)

See [plan.md](design.md). Status is honest: unchecked means not done or not
verified.

## P0 — foundation (spec step 1, this PR)

- [x] Branch from current `origin/main` (`71797ee`); diff since spec baseline `7569da1` is `AGENTS.md` only
- [x] Rust 1.95.0 pinned; workspace `rust-version = "1.95"`; AGENTS.md/README toolchain lines
- [x] 1.95 lints fixed (clippy 1.95 adds collapsible_match, unnecessary_sort_by, explicit_counter_loop)
- [x] `surrealdb = "=3.3.4"`, default features off, `protocol-ws` only, behind non-default `surreal` feature
- [x] `cargo tree -p argos-osint-bin --features surreal`: no rocksdb, surrealkv, tikv, mem/indxdb, surrealdb-kvs, engine-local or surrealdb-core
- [x] Default build tree has no surrealdb crate
- [x] `scripts/test_surreal.py`: pinned download + SHA-256, loopback RocksDB temp dir, sync every, versioning off, appendix env profile, teardown
- [x] `tests/storage_contracts.rs`: auth, rollback, structured retryable conflict, DISKANN F32/384/cosine filtered search, restart persistence, structured errors
- [x] Suite skips cleanly without a server; fails with `ARGOS_SURREAL_REQUIRED=1`
- [x] Contract suite passes locally against the real 3.3.4 disk server (Linux x86_64)
- [ ] Local matrix on final head (see PR body for results)
- [ ] CI: toolchain 1.95 action + `surreal` clippy/test + contract suite on all four jobs. **Blocked:** the box's GitHub token lacks the `workflow` scope, so `.github/workflows/ci.yml` cannot be pushed from here. Patch prepared.
- [ ] CI green on all four jobs with the contract suite (depends on the item above)
- [x] Existing `argos.db`, sidecars and `memory_lancedb/` untouched
- [ ] Release binary size and clean build time, before vs after (see PR body)

## P1 — storage traits + job system (spec steps 2–4)

- [ ] Decide server vs embedded (pending user)
- [ ] Fresh versioned schema files, checksummed ledger, database identity, reset epoch
- [ ] Typed key codec with round-trip/collision tests
- [ ] Async repository boundary; no raw connections
- [ ] Transaction/retry helpers; per-statement result checks
- [ ] Scheduler election, atomic claims, leases, owner/epoch fences, cancellation, heartbeats
- [ ] `Drop` database work replaced by bounded completion queue + lease recovery

## P2 — memories, graph, vectors (spec steps 5 memory part, 6)

- [ ] Memory publication with graph projection, tombstones, change sequence, embedding outbox in one transaction
- [ ] Seven edge meanings, parallel Link/Related IDs, retirement behaviour
- [ ] DISKANN recall with revision/generation fencing and lexical fallback; bounded top-k heap
- [ ] Recall caps (5 memories, 1,500 tokens), opt-out, default kind exclusions

## P3 — Recon, Intel, Atlas + savepoints (spec step 5 rest)

- [ ] Recon/OSINT messages, tools, artifacts, ordered counters, credit hold/reconcile/release
- [ ] Intel stage publication with failure fallback (no savepoints)
- [ ] Atlas occurrences, metadata revisions, actor index receipts
- [ ] `brain_backfill`, `atlas_actor_index`, `job_cleanup`, `brief_canonical` savepoint replacements

## P4 — TUI/CLI + storage commands (spec steps 7–9)

- [ ] Decide wipe vs import (pending user; spec assumes wipe)
- [ ] CLI/TUI workers, Profile (20 views), Jobs, telemetry, summaries, NLP artifacts
- [ ] `argos storage status`, `init`, `reset --yes` with resumable phases
- [ ] Fresh population checks; two-session operation; restart reads
- [ ] RSS/swap/disk/p95 measurements on the target machine

## P5 — `feat!` removal (spec step 10)

- [ ] Remove rusqlite, LanceDB, SQL schemas/migrations, protoc shim
- [ ] Docs, diagrams, CI and packaging for the separate server; backup/restore notes

## Gates (every phase)

- [ ] `cargo fmt --all --check`
- [ ] clippy `-D warnings` + tests via `scripts/agent_cargo.py`: default; `--locked --no-default-features`; `--no-default-features --features nlp`; `--no-default-features --features surreal`
- [ ] `python3 scripts/test_surreal.py` against the real server
- [ ] `graphify update .` as the last commit
