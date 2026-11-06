# Contributor workflow

[Code route](index.md) · [Shared agent execution policy](agent-workflow.md) · [Wiki](../../index.md)

Detailed agent rules, loaded before editing or running checks.

## Build and test

Before running verification tests, ask the user to confirm the proposed tests and scope unless those tests are already explicitly authorized in the current session. When the user will verify, run only the requested minimal static checks and report tests and gates as not run. The gate commands below describe authorized verification; they do not override this confirmation requirement.

Browser checks use the repository Playwright runner. Its development server sets `ARGOS_WEB_TEST_DEV=1` to isolate Next output and locks under `.agent-scratch/next-web-tests`; custom test configurations must inherit this setting. Different ports alone do not isolate Next’s development lock. Preserve the user’s `.next` cache and frontend process.

The same existing setting isolates an authorized production build and its compiled-output check. Use the matching pair so both inspect `.agent-scratch/next-web-tests`:

```sh
ARGOS_WEB_TEST_DEV=1 pnpm build
ARGOS_WEB_TEST_DEV=1 pnpm check:web-build
```

Without that setting, both retain the normal launcher `.next` behavior. The isolated build/check pair does not establish a production browser pass or the full frontend gate. After that pair, `ARGOS_WEB_TEST_DEV=1 pnpm test:e2e` runs the repository’s default production browser profile against the same isolated output. Development uses `ARGOS_WEB_PROFILE=dev pnpm test:e2e` and always isolates output. Never stop the user’s launcher or remove its lock to make verification run.

Rust 1.95.0 pinned. Agents always build through `python3 scripts/agent_cargo.py … --locked` (artifacts in `target/agents`; `target/debug` is the user's), with `ARGOS_EMBED` unset (the wrapper removes inherited values). `cargo fmt` runs directly. Local Rust builds and tests target the **host machine's native platform** (`uname -ms`, `rustc -vV | grep '^host:'`) and **embedded storage mode** (`--storage embedded`, default features `embedded,nlp,classifier`).

### Verification ownership and host resources

Follow the [shared ownership, handoff and resource policy](agent-workflow.md#coordinate-work-that-can-actually-run-in-parallel). Finish assigned production edits before build handoff; do not make last-minute edits to files under compilation. Report a necessary later change to the build owner so its affected evidence is identified.

The binary's test dependency enables core `fixtures`, so alternating core-only and workspace tests can rebuild/link a different core. Preserve activating targets when matching the workspace graph; use package-only commands when their alternate graph genuinely saves work. Inspect RAM with `sysctl -n hw.memsize`/`vm_stat` on macOS or `free -h` on Linux, then record `CARGO_BUILD_JOBS` and test concurrency under the [build-graph policy](agent-workflow.md#keep-one-reusable-build-graph).

### Build cache hygiene

Agents must manage disk space proactively before builds, between large verification phases and at handoff. Check `df -h .` and inspect the relevant target/profile sizes with scoped `du -sh` calls. Do not wait for an out-of-space failure. Avoid repeatedly scanning the entire target tree while cleanup or compilation is running.

- Name the build cache to retain before deleting anything: profile, target triple, feature mode and the latest useful Argos executable. Preserve its dependency artifacts, fingerprints, build-script outputs and incremental cache together, so the next matching build can reuse them. Keeping only the executable does not preserve a reusable build.
- `target/debug` is the user's cache and is protected by default. Agents reuse the single `target/agents` integration cache; never create per-worker/per-task Cargo targets. Keep the current useful agent cache across tasks instead of cleaning it at every handoff. Explicit user cleanup instructions may select a different retained cache.
- Routinely remove known superseded agent caches: obsolete profiles/target triples, abandoned check trees such as inactive `target/flycheck*`, completed temporary build trees and identified stale test/linker artifacts. Verify ownership and that they are unrelated to the retained build or any pending verification. Age alone does not establish obsolescence. Do not indiscriminately prune files from the retained `deps`, `build`, `.fingerprint` or `incremental` directories.
- Check active processes and lock holders (`ps`, `lsof -nP <target>/<profile>/.cargo-lock`) before deleting a candidate. Serialize cleanup with builds and other cleanup owners. Never delete an in-use directory, remove a lock to bypass contention, or kill an unrelated process. A missing lock holder alone is not proof that a cache is safe to remove.
- Budget free space from the next phase: recent artifact growth, the largest observed link/test temporary footprint, binaries/evidence intentionally retained, and a stated safety margin. Use recent comparable measurements; when none exist, start with a conservative estimate and measure a focused build first. Recheck before the next large phase. Do not impose an arbitrary universal free-space floor or repeatedly stop useful work to restore one.
- Cleanup of clearly obsolete repo-local agent build artifacts is routine authorized maintenance; do it without repeated permission requests. Stay inside verified artifact roots, avoid symlink traversal and delete exact candidates. Never run blanket `cargo clean`, wipe all of `target`, delete the kept user cache, or clean global Cargo/pnpm/model caches or other repositories as a space shortcut. Preserve source, dependencies needed by running services, data, plans and unresolved evidence.
- If safe cleanup cannot provide enough space, report the measured shortfall and retained cache rather than starting a likely failing build or deleting protected material. Do not trade the required verification scope for disk space silently.
- At handoff, remove superseded artifacts created during the task, retain the selected reusable cache and binary, and report free space, removed paths and remaining constraints. Verify the retained binary still exists. Hash it when immutable benchmark/release provenance or an explicit preservation check requires byte identity; routine exact-path cache cleanup does not require hashing whole files or the repository. Do not rebuild merely to verify cleanup.

### Targeted local Rust builds and tests (fast iteration)

Choose the stable package/feature graph first, then scope execution with target (`--lib`, `--bin`, `--test`) flags. Package (`-p`) flags are useful when their smaller graph saves work; avoid switching graphs solely because a command looks narrower. Run focused targets before the full embedded matrix:

```sh
# 1. Detect the native host; choose jobs from RAM/pressure as described above
uname -ms
rustc -vV | grep '^host:'

# Workspace-compatible focused example: retain the activating bin/API targets.
# The filter limits execution; zero-match companion targets are not test passes.
ARGOS_SURREAL_MODE=embedded ARGOS_SURREAL_REQUIRED=1 \
  python3 scripts/agent_cargo.py test --locked --workspace \
  --test storage_jobs --test stack_cli --test api_contract <filter> -- --nocapture

# Package-only examples below intentionally use an alternate Cargo graph.
# Choose them when that smaller graph saves work; do not alternate by habit.
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

### Fail early, then run the gate

Apply the shared [early-smoke](agent-workflow.md#establish-the-critical-path) and [diagnosis](agent-workflow.md#diagnose-before-repeating) policy before the final gate. Native Atlas fixtures must use real admission, seeded catalogs and current metadata/body/NLP receipts where required; follow the [testing guidelines](../../../.opencode/skills/argos-testing/SKILL.md#31-native-fixture-prerequisites-and-diagnostic-output). Opt-in fixtures must actually execute before their results are used as acceptance evidence.

### Full local embedded gate & GitHub CI monitoring

```sh
# Leave mode selection to the gate's dedicated storage phase.
env -u ARGOS_SURREAL_MODE -u ARGOS_SURREAL_BIN -u ARGOS_SURREAL_REQUIRED \
  scripts/local_matrix.sh --storage embedded
# fmt → clippy -D warnings → workspace tests/CLI smoke → required embedded contracts
```

With the parent storage mode unset, the plain workspace phase honestly skips mode-selected storage tests and the dedicated `test_surreal.py` phase runs them once with required embedded mode. Targeted storage commands still set `ARGOS_SURREAL_MODE=embedded ARGOS_SURREAL_REQUIRED=1`. Do not export that mode around the whole gate and accidentally run storage twice.

Never attempt to run the full multi-platform / server-mode CI matrix (`ci.yml` across `ubuntu-24.04`, `ubuntu-24.04-arm`, `macos-14`, `macos-15-intel` and `--storage server`) locally. Agents must **get the local host-platform embedded build and tests to a stable, passing state first**, then rely on hosted GitHub Actions for non-host platforms and `server` storage mode ([docs/wiki/code/ci.md](ci.md)). When git/GitHub CLI access is available, monitor hosted PR/branch checks (`gh pr checks`, `gh run list --branch "$(git branch --show-current)"`, `gh run view <run-id> --log-failed`) to diagnose and fix issues on other platforms. Serialize Cargo jobs sharing `target/agents`; never delete locks or kill unrelated Cargo processes (diagnose with `lsof -nP target/agents/debug/.cargo-lock`). MiniLM tests are `#[ignore]` and need `ARGOS_EMBED=1`.

## Storage facts

- Default features `embedded,nlp,classifier`: embedded SurrealDB (RocksDB, in-process) under `<ARGOS_HOME>/storage/surreal`; `ARGOS_HOME` defaults to `~/.argos`.
- Server mode is opt-in: `--no-default-features --features server,nlp,classifier`, selected by `argos storage init --mode server` or `ARGOS_STORAGE_MODE=server|embedded` (env → `storage/config.json` → embedded).
- Storage is SurrealDB only. CI fails if `rusqlite` or `lancedb` appears in `cargo tree`.
- Storage tests pick their mode from `ARGOS_SURREAL_MODE` and skip without it (and without `ARGOS_SURREAL_BIN`); run them via `test_surreal.py`.


## Classifier dataset resolution order

The seed corpus resolution order for the classifier is:

1. Explicit file path (passed via `--file` flag) — **authoritative**: an invalid or missing explicit selection is an error, never a silent fallback to the next candidate
2. `ARGOS_CLASSIFIER_CSV` environment variable — **authoritative**: an invalid or missing env selection is an error, never a silent fallback to the next candidate
3. Saved Profile `[data_paths].article_csv` — relative to `ARGOS_HOME`; a missing saved selection is an error.
4. Repo `dataset/` (**singular** directory) — the resolver looks for `dataset/argos_atlas_strategic_domains.csv` **inside it**.
5. Repo `datasets/` (**plural** directory, the customer's directory) — the seed CSV is `datasets/argos_atlas_strategic_domains.csv`; 1,700 original synthetic rows (220 scenario families). The seed corpus is **synthetic development material** and is never ingested into live Atlas/Brain evidence. Do not mix with `datasets/argos_actors.csv` or the old seven-topic corpus
6. `<ARGOS_HOME>/dataset/` (runtime default)

**Rust dataset resolver (unit U4) rules:**
- Explicit, environment and saved Profile selections are **authoritative** — an invalid or missing explicit/env/saved selection is an **error**, never a silent fallback to the next candidate
- There is **no** `CARGO_MANIFEST_DIR` developer-path fallback

## Classifier bundle build workflow

**Status:** bundle build/parity scripts, Venture setup, local admission and the background monitoring consumer are implemented. Verification results are recorded separately; these commands do not establish a quality or performance result. Synthetic-only installations retain the explicitly activated baseline until independently reviewed promotion evidence exists.

- **Rebuild:** run `node scripts/build_classifier_bundle.mjs` to generate the committed bundle at `crates/argos-osint-core/assets/classifier/natural-classifier.js` from the verified vendored upstream files (`natural@8.2.6`, `apparatus@0.0.10`) via the deterministic adaptation patch. The script performs **no network access** at build time; the pinned packages must already be present locally.
- **Verify:** run `node scripts/build_classifier_bundle.mjs --check` to compute the SHA-256 of the current bundle and compare it against the committed `source-manifest.json` entry; this verifies byte-for-byte fidelity and fails closed on upstream drift. Output includes `crates/argos-osint-core/assets/classifier/source-manifest.json` with upstream versions/commits/tarball+per-file hashes/adaptation list/bundle hash.
- **Parity test:** run `node scripts/test_classifier_bundle.mjs` to load the same upstream modules in Node with unmodified semantics and compare against the committed bundle (same labels, same feature order, same log scores) against its pinned oracle and declared tolerance; record the actual measured delta for each regenerated fixture set. The test also asserts the bundle has no unresolved `require`, no `fs`/`events`/`os`/`webworker-threads` usage, and no dependency on a Node-only global. Run the check on the current accumulated tree before recording success.
- The bundle is **committed** in the repository; whenever a pinned dependency version changes, the bundle must be regenerated and `--check`-verified. The root `package.json` devDependencies pin `natural: "8.2.6"`, `apparatus: "0.0.10"`, `esbuild: "0.21.5"`.
- **No Node.js for classifier runtime:** classifier build/test commands use Node only as tooling. The shipped classifier bundle runs inside the Rust binary's embedded QuickJS worker with 128 MiB memory / 1 MiB stack ceilings and inference batch size 64.

## Workflow rules

- **Worktrees are edit-only:** in a feature worktree run only `cargo fmt`. One combined integration tree runs the gate (`local_matrix.sh --storage embedded`).
- **Incremental verification:** plan one final full gate after focused closure. On failure, diagnose and rerun the affected target, then repeat the full gate only when the repaired inputs justify it. Record the reason; do not loop unchanged runs or stop incomplete work because a numerical repeat count was reached.
- **Commits:** Conventional Commits (`feat` minor, `fix`/`perf` patch; breaking = `!` + `BREAKING CHANGE:` footer). Check with `node scripts/release_check.mjs message --file <path>` and `… versions`. Release Please owns version bumps; never hand-bump or invent `Release-As`. Stage exact paths; preserve unrelated changes.
- **`.github`** changes go through Code Architect.
- **Planning:** multi-phase work uses `.planning/<YYYY-MM-DD-slug>/` (`task_plan.md`, `findings.md`, `progress.md`); see [planning-artifacts](planning-artifacts.md).
- **Scratch** lives only in `.agent-scratch/`, never in `crates/`, `docs/`, `scripts/`; never stage it.
- **Docs:** architecture/contract changes update docs and diagrams in the same change ([conventions](conventions.md#architecture-and-concepts)). Tests stay offline.

## Evidence, estimates and workflow improvements

The [shared execution policy](agent-workflow.md#separate-evidence-by-what-it-proves) owns proportionate provenance and evidence reuse; its [communication and improvement guidance](agent-workflow.md#communicate-and-improve-the-workflow) owns honest estimates and reversible improvements within the authorized scope. Keep technical command recipes here and update the shared policy rather than creating another competing workflow.
