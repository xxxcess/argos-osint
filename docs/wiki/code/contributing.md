# Contributor workflow

[Code route](index.md) · [Wiki](../../index.md)

Detailed agent rules, loaded before editing or running checks.

## Build and test

Before running verification tests, ask the user to confirm the proposed tests and scope unless those tests are already explicitly authorized in the current session. When the user will verify, run only the requested minimal static checks and report tests and gates as not run. The gate commands below describe authorized verification; they do not override this confirmation requirement.

Browser checks use the repository Playwright runner. Its development server sets `ARGOS_WEB_TEST_DEV=1` to isolate Next output and locks under `.agent-scratch/next-web-tests`; custom test configurations must inherit this setting. Different ports alone do not isolate Next’s development lock. Preserve the user’s `.next` cache and frontend process.

Rust 1.95.0 pinned. Agents always build through `python3 scripts/agent_cargo.py … --locked` (artifacts in `target/agents`; `target/debug` is the user's), with `ARGOS_EMBED` unset (the wrapper removes inherited values). `cargo fmt` runs directly. Local Rust builds and tests target the **host machine's native platform** (`uname -ms`, `rustc -vV | grep '^host:'`) and **embedded storage mode** (`--storage embedded`, default features `embedded,nlp`).

### Build cache hygiene

Agents must manage disk space proactively before builds, between large verification phases and at handoff. Check `df -h .` and inspect the relevant target/profile sizes with scoped `du -sh` calls. Do not wait for an out-of-space failure. Avoid repeatedly scanning the entire target tree while cleanup or compilation is running.

- Name the build cache to retain before deleting anything: profile, target triple, feature mode and the latest useful Argos executable. Preserve its dependency artifacts, fingerprints, build-script outputs and incremental cache together, so the next matching build can reuse them. Keeping only the executable does not preserve a reusable build.
- `target/debug` is the user's cache and is protected by default. Agents reuse the single `target/agents` integration cache; never create per-worker/per-task Cargo targets. Keep the current useful agent cache across tasks instead of cleaning it at every handoff. Explicit user cleanup instructions may select a different retained cache.
- Routinely remove known superseded agent caches: obsolete profiles/target triples, abandoned check trees such as inactive `target/flycheck*`, completed temporary build trees and identified stale test/linker artifacts. Verify ownership and that they are unrelated to the retained build or any pending verification. Age alone does not establish obsolescence. Do not indiscriminately prune files from the retained `deps`, `build`, `.fingerprint` or `incremental` directories.
- Check active processes and lock holders (`ps`, `lsof -nP <target>/<profile>/.cargo-lock`) before deleting a candidate. Serialize cleanup with builds and other cleanup owners. Never delete an in-use directory, remove a lock to bypass contention, or kill an unrelated process. A missing lock holder alone is not proof that a cache is safe to remove.
- Start with a free-space budget: at least **100 GiB for targeted embedded Rust work**, **200 GiB for a full embedded gate**, or **20 GiB for web-only builds**, plus any larger headroom indicated by recent measurements. These are conservative starting floors, not guarantees; heavy link/test phases may need more. Recheck between phases and stop launching more jobs when the remaining budget is inadequate.
- Cleanup of clearly obsolete repo-local agent build artifacts is routine authorized maintenance; do it without repeated permission requests. Stay inside verified artifact roots, avoid symlink traversal and delete exact candidates. Never run blanket `cargo clean`, wipe all of `target`, delete the kept user cache, or clean global Cargo/pnpm/model caches or other repositories as a space shortcut. Preserve source, dependencies needed by running services, data, plans and unresolved evidence.
- If safe cleanup cannot provide enough space, report the measured shortfall and retained cache rather than starting a likely failing build or deleting protected material. Do not trade the required verification scope for disk space silently.
- At handoff, remove superseded artifacts created during the task, retain the selected reusable cache and binary, and report free space, removed paths and remaining constraints. Verify the retained binary still exists; when cleaning around a user-selected binary, compare its before/after checksum. Do not rebuild merely to verify cleanup.

### Targeted local Rust builds and tests (fast iteration)

Use package (`-p`) and target (`--lib`, `--bin`, `--test`) flags to scope compilation and test execution to the crates and binaries touched before running the full embedded matrix:

```sh
# 1. Detect host platform and CPU count (build for the local host target only)
uname -ms && rustc -vV | grep '^host:' && (sysctl -n hw.ncpu 2>/dev/null || nproc)

# 2. Scoped compile check and Clippy by crate (argos-osint-core | argos-osint-bin | argos-osint-api | argos-api-host)
python3 scripts/agent_cargo.py check --locked -p argos-osint-core
python3 scripts/agent_cargo.py clippy --locked -p argos-osint-core --all-targets -- -D warnings

# 3. Scoped unit tests (--lib / --bin) with optional test name filter
python3 scripts/agent_cargo.py test --locked -p argos-osint-core --lib <filter>
python3 scripts/agent_cargo.py test --locked -p argos-osint-bin --bin argos <filter>
python3 scripts/agent_cargo.py test --locked -p argos-api-host <filter>

# 4. Scoped integration test binary (--test <name>)
python3 scripts/agent_cargo.py test --locked -p argos-osint-api --test api_contract
python3 scripts/agent_cargo.py test --locked -p argos-osint-bin --test stack_cli

# 5. Scoped embedded storage contract test (single storage_* binary instead of all 60+ binaries)
ARGOS_SURREAL_MODE=embedded ARGOS_SURREAL_REQUIRED=1 \
  python3 scripts/agent_cargo.py test --locked -p argos-osint-core --test storage_jobs <filter> -- --nocapture

# 6. Full embedded storage contract suite (when broad storage/schema contracts change)
python3 scripts/test_surreal.py --mode embedded --no-fail-fast

# 7. Composer-launch PTY regression (POSIX only; only `argos` must be built)
python3 scripts/test_web_stack.py --composer-smoke
```

### Full local embedded gate & GitHub CI monitoring

```sh
scripts/local_matrix.sh --storage embedded   # fmt → clippy -D warnings → cargo test --workspace → test_surreal.py --mode embedded
```

Never attempt to run the full multi-platform / server-mode CI matrix (`ci.yml` across `ubuntu-24.04`, `ubuntu-24.04-arm`, `macos-14`, `macos-15-intel` and `--storage server`) locally. Agents must **get the local host-platform embedded build and tests to a stable, passing state first**, then rely on hosted GitHub Actions for non-host platforms and `server` storage mode ([docs/wiki/code/ci.md](ci.md)). When git/GitHub CLI access is available, monitor hosted PR/branch checks (`gh pr checks`, `gh run list --branch "$(git branch --show-current)"`, `gh run view <run-id> --log-failed`) to diagnose and fix issues on other platforms. Serialize Cargo jobs sharing `target/agents`; never delete locks or kill unrelated Cargo processes (diagnose with `lsof -nP target/debug/.cargo-lock`). MiniLM tests are `#[ignore]` and need `ARGOS_EMBED=1`.

## Storage facts

- Default features `embedded,nlp`: embedded SurrealDB (RocksDB, in-process) under `<ARGOS_HOME>/storage/surreal`; `ARGOS_HOME` defaults to `~/.argos`.
- Server mode is opt-in: `--no-default-features --features server,nlp`, selected by `argos storage init --mode server` or `ARGOS_STORAGE_MODE=server|embedded` (env → `storage/config.json` → embedded).
- Storage is SurrealDB only. CI fails if `rusqlite` or `lancedb` appears in `cargo tree`.
- Storage tests pick their mode from `ARGOS_SURREAL_MODE` and skip without it (and without `ARGOS_SURREAL_BIN`); run them via `test_surreal.py`.

## Workflow rules

- **Worktrees are edit-only:** in a feature worktree run only `cargo fmt`. One combined integration tree runs the gate (`local_matrix.sh --storage embedded`).
- **Repeat cap:** run the full gate 1–2 times, max 3. On failure fix with `cargo check --workspace --all-targets` and rerun only the failing `--test <name>`, then one full run.
- **Commits:** Conventional Commits (`feat` minor, `fix`/`perf` patch; breaking = `!` + `BREAKING CHANGE:` footer). Check with `node scripts/release_check.mjs message --file <path>` and `… versions`. Release Please owns version bumps; never hand-bump or invent `Release-As`. Stage exact paths; preserve unrelated changes.
- **`.github`** changes go through Code Architect.
- **Planning:** multi-phase work uses `.planning/<YYYY-MM-DD-slug>/` (`task_plan.md`, `findings.md`, `progress.md`); see [planning-artifacts](planning-artifacts.md).
- **Scratch** lives only in `.agent-scratch/`, never in `crates/`, `docs/`, `scripts/`; never stage it.
- **Docs:** architecture/contract changes update docs and diagrams in the same change ([conventions](conventions.md#architecture-and-concepts)). Tests stay offline.
