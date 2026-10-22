# SurrealDB storage migration: phase plan (design)

Checklist and gate status: [acceptance.md](acceptance.md).

Spec: implementation spec, storage audit and inventory handed over on
2026-10-22 (audited baseline `7569da1`). This branch is based on `71797ee`;
the only change since the baseline is `AGENTS.md`, which does not affect the
edit map.

Goal: replace SQLite + LanceDB with one local, disk-backed SurrealDB 3.3.4
(RocksDB, sync every commit, versioning off, DISKANN F32/384/cosine).

## Decisions pending the user

- **Wipe vs import.** The spec assumes a complete wipe and fresh population
  (no row import). Not confirmed for this PR; nothing here deletes or opens
  any existing `argos.db`, its sidecars or `memory_lancedb/`.
- **Shared loopback server vs embedded engine.** The spec selects a separate
  shared server per Argos home. Phase 1 builds only the remote client (no
  embedded kv engine), so the choice stays open until P1.

## Phases (Code Architect's P0–P5, mapped to the spec's 10 steps)

| PR phase | Spec steps | Scope |
|---|---|---|
| **P0 foundation (this PR)** | 1 | Rust 1.95, remote-only SDK behind `surreal`, pinned server lifecycle script, disk contract suite, `cargo tree` proof. No Store or caller conversion. |
| P1 storage traits + job system | 2, 3, 4 | Fresh versioned SurrealQL schema + ledger, async repository boundary, transactions/retry helpers, guard records; scheduler election, claims, leases, cancellation, fences, heartbeats. |
| P2 memories / graph / vectors | 5 (memory part), 6 | Memory publication, native graph projection, embedding outbox, DISKANN recall with lexical fallback and stale-hit rejection. |
| P3 Recon / Intel / Atlas + savepoints | 5 (rest) | Recon, OSINT, Intel, Atlas, investigation persistence; the four savepoint replacements; credit accounting. |
| P4 TUI / CLI + storage commands | 7, 8, 9 | Consumers and fixtures, `argos storage status|init|reset --yes`, repopulation checks, memory measurements. |
| P5 `feat!` removal | 10 | Delete rusqlite, LanceDB, SQL schemas/migrations, protoc tooling; docs, diagrams, packaging. |

## Spec notes found while doing P0 (inputs for P1+)

1. The SDK crates (surrealdb 3.3.4 and its workspace crates) declare **no
   `rust-version`**. The 1.95 minimum is real: `surrealdb-common` uses
   `std::hint::cold_path`, which fails on 1.94.0 with E0658.
2. The 3.3.4 GitHub release has **no published checksum file**; the spec asks
   for "verified release checksums". `scripts/test_surreal.py` pins SHA-256
   values computed from the release assets on 2026-10-22 (trust on first use).
3. SurrealDB 3 returns `NotFound(Table)` for reads of an **undefined table**.
   "Empty logical tables" kept for compatibility must be explicitly DEFINEd at
   bootstrap, or reads error instead of returning nothing.
4. Unique-index violations and field coercion failures come back as kind
   **`Internal` with no details**, not `AlreadyExists`/`Validation`. Repository
   error mapping cannot match typed variants for them; the contract test pins
   this so a server upgrade that adds details is noticed.
5. Write conflicts are structured: kind `Query`, details
   `QueryError::TransactionConflict` ("can be retried").
6. The spec's illustrative DISKANN definition is accepted as written; the
   server fills in `DEGREE 64 L_BUILD 100 ALPHA 1.2`. A `WHERE` filter is pushed
   into the `KnnScan` predicate and still returns k hits.
7. No TLS feature is needed for a 127.0.0.1 server, so none is enabled; the
   spec's "transport/TLS features" wording should be read as transport only
   unless a non-loopback endpoint is ever allowed.
8. The server log confirms the RocksDB sync mode and cache sizes, but does
   not print the `versioned=false` state; it is set by URL and
   `SURREAL_DATASTORE_VERSIONED` but not observed.
9. Server credentials are passed through `SURREAL_USER`/`SURREAL_PASS` env, not
   argv. Env is still readable by the same OS user via `/proc`; P4's protected
   bootstrap should decide whether that is acceptable.
