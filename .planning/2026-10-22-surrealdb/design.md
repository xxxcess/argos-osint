# SurrealDB storage migration: phase plan (design)

Checklist and gate status: [acceptance.md](acceptance.md).

Spec: implementation spec, storage audit and inventory handed over on
2026-10-22 (audited baseline `7569da1`). This branch is based on `71797ee`;
the only change since the baseline is `AGENTS.md`, which does not affect the
edit map.

Goal: replace SQLite + LanceDB with one local, disk-backed SurrealDB 3.3.4
(RocksDB, sync every commit, versioning off, DISKANN F32/384/cosine).

## Decisions (user, 2026-10-22)

- **Wipe, no import.** Start from an empty database and repopulate through
  Argos. Nothing in P0 deletes or opens any existing `argos.db`, its sidecars
  or `memory_lancedb/`; the wipe happens only through the explicit P4 reset,
  after the user confirms it.
- **Shared per-home loopback server**, not an embedded engine. The Argos
  binary links only the remote client (no kv engine); `cargo tree` proves it.
- **One draft PR** for the whole migration, built phase by phase with
  Conventional Commits. Each phase must pass CI on its own. The `feat!`
  commit with `BREAKING CHANGE` lands with P5.

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

## Server lifecycle (design only; implemented in P4, not in store code yet)

Per Argos home (`$ARGOS_HOME`, default `~/.argos`):

```
storage/surreal/            RocksDB data directory (server-owned)
storage/server.json         {pid, port, version, instance_id, started_at}  0600
storage/server.lock         advisory lock held while starting/stopping
storage/credentials.json    root + argos user secrets                     0600
storage/clients/<pid>       one lease file per live Argos client
bin/surreal-3.3.4           installed, checksum-verified server binary
```

**Install, never silently.** `argos storage install` downloads
`surreal-v3.3.4.<os-arch>.tgz` from the official GitHub release, checks the
pinned SHA-256 compiled into Argos, and writes `bin/surreal-3.3.4`
atomically (temp file, fsync, rename). A documented manual step is
equivalent: place a 3.3.4 binary there and run `argos storage install
--from <path>`, which verifies the same hash. Ordinary startup never
downloads; a missing or wrong binary is a clear error naming the command.
Pinned assets (official v3.3.4 release; no vendor checksum file exists, so
the hashes were computed from the assets on 2026-10-22):

| OS / arch | Asset | Archive size | SHA-256 |
|---|---|---:|---|
| Linux x86_64 | linux-amd64.tgz | 105,385,846 B | `6593071833ad9c85cf533cf70b1643eb08966c868f8624001e30839ea29a2689` |
| Linux aarch64 | linux-arm64.tgz | 98,086,044 B | `978262fd9d1f3adfc5791031663855baf0a3cf8a89883428109734ba5a065811` |
| macOS arm64 | darwin-arm64.tgz | 91,843,710 B | `d0b0afc47948a46ceac9db178d2a10a7eb21e1c4f219a74b3573c850122ed9fb` |
| macOS Intel | darwin-amd64.tgz | 102,240,575 B | `0723b555340da2cee1e0cff6f95552438536eab8d984b91f227b3ce2d52a5006` |

**Discovery.** A client takes `server.lock`, reads `server.json`, and treats
the server as live only if the pid exists, the port accepts on 127.0.0.1,
the health request succeeds, the reported version is 3.3.4 and a signed-in
read of the `argos` database's identity record returns the expected
`instance_id`. Anything else is stale: remove `server.json`, start fresh.
A port answering with the wrong identity is never adopted (another
instance or an unrelated process).

**Start on demand.** Any Argos process that needs storage starts the server
if none is live, under `server.lock`, so two sessions cannot both start one.
`argos storage start` does the same explicitly. Launch: absolute
`rocksdb://<home>/storage/surreal?versioned=false&sync=every`, `--bind
127.0.0.1:<port>`, the audit tuning env, credentials through
`SURREAL_USER`/`SURREAL_PASS` in the child environment (never argv, never
logged), stdout/stderr to `storage/server.log`, detached from the
terminal's process group so it outlives the starting TUI.

**Port selection.** Configurable `storage.port`; default: try the last port in
`server.json`, else bind 127.0.0.1:0 to get a free port, then pass it to
the server and record it. A bind failure retries with a new port a bounded
number of times. Never bind a non-loopback address.

**Health.** Ready means: TCP accept, `/health` OK, version check, root
sign-in, identity read, schema ledger at the expected version. Timeouts
are bounded (30 s start). Unavailable is a distinct error from "dead
owner"; it never triggers a reset or a SQLite fallback.

**Shutdown.** Each client writes `clients/<pid>` and refreshes it with the
existing 5 s process heartbeat. On exit a client removes its lease; the
last client (no live leases after a short grace) sends SIGTERM, waits for
exit, and removes `server.json`. Crashed clients are pruned by pid check
and a 30 s stale boundary, as `job_registry.rs` does today. `argos storage
stop` stops it explicitly, refusing while other live clients hold leases
unless `--force`. A TUI exit never kills a server another session uses.

**Credentials.** `argos storage init` generates random root and
least-privilege `argos` database-user secrets into `credentials.json`
(0600, owner check on open, excluded from config export). Clients sign
in as the `argos` user; root is only for bootstrap/reset. Secrets are
never printed, logged or put in argv; `argos storage status` shows only
whether they exist.

**Windows** is out of scope (Argos supports the four targets above).
