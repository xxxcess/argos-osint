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

Use the [Fleetworks scopes](ci.md#fleetworksyml--scoped-branch-checks) for network-client, deployment and service changes. The targeted Venture and client tests do not start a datastore. Run hosted Fleetworks checks for network-service or packaged Compose evidence and the legacy embedded gate for affected legacy runtime paths. Keep local and hosted results separate.

### Verification ownership and host resources

One integration/build owner uses a stable feature graph and resource-based concurrency; see [execution policy](agent-workflow.md#coordinate-work-that-can-actually-run-in-parallel). Freeze edits before builds and reuse unaffected evidence. Workspace bin tests activate core fixtures; switching to core-only commands may rebuild dependencies.

### Build cache hygiene

Check `df -h .` and scoped `du -sh` before builds, between large phases and at handoff.

- Retain the selected native/embedded `target/agents` profile, useful binary and reusable dependencies/fingerprints/incremental cache. Protect the user's `target/debug` by default.
- Remove only proven obsolete repo-local agent artifacts. Check exact ownership, path and active processes/locks first; age or an unused lock alone is insufficient proof.
- Never clean active builds, remove locks to bypass contention, run blanket `cargo clean`, or erase global caches, source, user data or unresolved evidence.
- Budget growth plus a reserve from a comparable build. If space is insufficient, report the shortfall instead of starting a likely failed build or silently reducing required checks.
- Cleanup proof must be proportional: do not hash whole caches. At handoff report removed paths/free space and confirm the retained binary exists; hash it only for required immutable provenance.

### Targeted local Rust builds and tests (fast iteration)

Detect the native host with `uname -ms` and `rustc -vV | grep '^host:'`. Reuse the selected feature graph, default `embedded,nlp,classifier`, and `target/agents`; set `CARGO_BUILD_JOBS` from memory pressure. Scope by package/target/filter without changing graphs unnecessarily.

```sh
python3 scripts/agent_cargo.py check --locked -p argos-osint-core
python3 scripts/agent_cargo.py clippy --workspace --all-targets --locked -- -D warnings
ARGOS_SURREAL_MODE=embedded ARGOS_SURREAL_REQUIRED=1 \
  python3 scripts/agent_cargo.py test --locked --workspace \
  --test storage_jobs <filter> -- --nocapture
```

Use `--lib`, `--bin <bin>` or `--test <name>` for the relevant target. Preserve activating targets when matching workspace features; zero-match targets are not passes. `python3 scripts/test_surreal.py --mode embedded --no-fail-fast` runs required embedded contracts. CLI/API/PTY checks follow the affected contract in [CI](ci.md).

### Fail early, then run the gate

Follow the [scope/freeze policy](agent-workflow.md#scope-and-finish-line). Exercise the decisive realistic fixture before the broad gate; see [native fixture prerequisites](../../../.opencode/skills/argos-testing/SKILL.md#31-native-fixture-prerequisites-and-diagnostic-output). Fix demonstrated in-scope failures and rerun affected checks; defer unrelated findings.

### Full local embedded gate & GitHub CI monitoring

After targeted stability, run:

```sh
env -u ARGOS_SURREAL_MODE -u ARGOS_SURREAL_BIN -u ARGOS_SURREAL_REQUIRED \
  scripts/local_matrix.sh --storage embedded
```

The gate runs format, Clippy, workspace tests, CLI smoke and required embedded storage. Parent mode variables stay unset so mode-selected storage runs once; targeted storage tests set mode/required explicitly. Some native-only fixtures execute in both phases: inspect their mode contracts before estimating work.

Never run the local all-platform, server storage or Rust mutation matrix/baselines. Once changes are published, inspect hosted checks when available (`gh pr checks`, `gh run list`, `gh run view --log-failed`); no hosted pass is implied by local results. `.github` remains Code Architect-owned. MiniLM requires explicit opt-in and is not part of ordinary checks.

## Storage facts

- Fleetworks uses a workspace-specific network datastore. See [Venture storage selections](fleetworks.md#launch-the-compose-stack).
- Legacy default features `embedded,nlp,classifier`: embedded SurrealDB (RocksDB, in-process) under `<ARGOS_HOME>/storage/surreal`; `ARGOS_HOME` defaults to `~/.argos`.
- Server mode is opt-in: `--no-default-features --features server,nlp,classifier`, selected by `argos storage init --mode server` or `ARGOS_STORAGE_MODE=server|embedded` (env → `storage/config.json` → embedded).
- Storage is SurrealDB only. CI fails if `rusqlite` or `lancedb` appears in `cargo tree`.
- Storage tests pick their mode from `ARGOS_SURREAL_MODE` and skip without it (and without `ARGOS_SURREAL_BIN`); run them via `test_surreal.py`.


## Classifier dataset resolution order

The seed corpus resolution order for the classifier is:

1. Explicit file path (passed via `--file` flag) — **authoritative**: an invalid or missing explicit selection is an error, never a silent fallback to the next candidate
2. `ARGOS_CLASSIFIER_CSV` environment variable — **authoritative**: an invalid or missing env selection is an error, never a silent fallback to the next candidate
3. Saved Profile `[data_paths].article_csv` — relative to `ARGOS_HOME`; a missing saved selection is an error.
4. Repo `dataset/` (**singular** directory) — the resolver looks for `dataset/argos_atlas_strategic_domains_v2.csv` **inside it**.
5. Repo `datasets/` (**plural** directory, the customer's directory) — the default seed CSV is `datasets/argos_atlas_strategic_domains_v2.csv`; 1,020 distinct synthetic scenarios with Train/Calibration/Development/Test partitions. The original unversioned CSV remains the frozen v1 fixture and is selectable explicitly. The seed corpus is **synthetic development material** and is never ingested into live Atlas/Brain evidence. Do not mix with `datasets/argos_actors.csv` or the old seven-topic corpus
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
- **Scope and verification:** follow the [finish-line policy](agent-workflow.md#scope-and-finish-line); no unrequested cleanup, late broad audits or unchanged full-suite retries.
- **Commits:** Conventional Commits (`feat` minor, `fix`/`perf` patch; breaking = `!` + `BREAKING CHANGE:` footer). Check with `node scripts/release_check.mjs message --file <path>` and `… versions`. Release Please owns version bumps; never hand-bump or invent `Release-As`. Stage exact paths; preserve unrelated changes.
- **`.github`** changes go through Code Architect.
- **Planning:** multi-phase work uses `.planning/<YYYY-MM-DD-slug>/` (`task_plan.md`, `findings.md`, `progress.md`); see [planning-artifacts](planning-artifacts.md).
- **Scratch** lives only in `.agent-scratch/`, never in `crates/`, `docs/`, `scripts/`; never stage it.
- **Docs:** architecture/contract changes update docs and diagrams in the same change ([conventions](conventions.md#architecture-and-concepts)). Tests stay offline.

## Evidence, estimates and workflow improvements

The [execution policy](agent-workflow.md) owns scope, delegation, evidence reuse and honest estimates. Keep commands here and link shared rules instead of duplicating them. Documentation-only work uses static checks, not Rust builds.
