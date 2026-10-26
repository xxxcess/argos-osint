# CI workflows

[Code route](index.md) · [Wiki](../../index.md)

Four workflows live in `.github/workflows/` (owned by Code Architect; CI changes are scoped separately from product edits).

![CI matrix](../../img/ci-matrix.svg)

## `ci.yml` — build and test matrix

Runs on pushes to `main` and when a PR is opened, reopened or marked ready for review (drafts skipped). PR pushes do not build. Adding the `ci:full` label to a PR runs the full matrix in that PR's checks; Code Architect adds it at the end of a feature, fix or review-fix round (remove and re-add it to run again). A manual dispatch (`gh workflow run ci.yml --ref <branch>`) runs it on a branch with no PR, but those runs don't show in PR checks. All 8 legs run every time. `fail-fast: false`, 90-minute timeout, Rust 1.95.0, no debuginfo.

| Platform | Runner | Legs |
|---|---|---|
| Linux x86_64 | `ubuntu-24.04` | embedded, server |
| Linux aarch64 | `ubuntu-24.04-arm` | embedded, server |
| macOS arm64 | `macos-14` | embedded, server |
| macOS Intel | `macos-15-intel` | embedded, server |

Every leg calls the same entry point you run locally, `scripts/local_matrix.sh`:

- **Embedded** (default features `embedded,nlp`): installs libclang on Linux, then `local_matrix.sh --storage embedded` = `cargo fmt --check` → clippy `-D warnings` → `cargo test --workspace --locked` → `test_surreal.py --mode embedded` (all `storage_*` tests on in-process RocksDB).
- **Server** (`--no-default-features --features server,nlp`): checks the pinned SurrealDB hashes in `scripts/test_surreal.py` match `.planning/2026-10-22-surrealdb/design.md`, fetches the SHA-256-verified SurrealDB 3.3.4 binary (cached), then `local_matrix.sh --storage server --surreal-bin <bin>` = clippy on core+bin → `test_surreal.py --mode server`. Unit tests are not rerun here.

Storage tests are selected by `ARGOS_SURREAL_MODE`; with no mode and no `ARGOS_SURREAL_BIN` they skip, so the plain `cargo test` in the embedded leg doesn't duplicate them. `test_surreal.py` sets `ARGOS_SURREAL_REQUIRED=1`, so a missing server fails instead of skipping.

Cache: `Swatinem/rust-cache` keyed per OS × storage, covering `target/agents` (where `scripts/agent_cargo.py` builds).

**Local rule:** agents run only `scripts/local_matrix.sh --storage embedded`. Server mode is CI-only.

## `commit-policy.yml`

On PRs (including title/body edits) and pushes to `main`: `node scripts/release_check.mjs event` enforces Conventional Commits on the PR title/body or new `main` commits, `release_check.mjs versions` checks workspace/lockfile/manifest versions agree, and the `.opencode/tests` release/tool/planning tests run.

## `release-please.yml`

On pushes to `main` (or manual dispatch): Release Please opens/updates the release PR and tags from Conventional Commits (`release-please-config.json`, `.release-please-manifest.json`). Uses a GitHub App token or `RELEASE_PLEASE_TOKEN` so release PRs trigger CI; see [versioning.md](versioning.md).

## `intel-web.yml` — local web client checks

The web workflow runs on matching web/config/build-check changes on `main` and pull requests (including PR pushes), plus manual dispatch. Node comes from `.node-version`; pnpm is pinned to 12.15.0. The first-party checkout/setup/artifact actions are pinned to their verified v7 commit SHAs. Existing Rust, commit-policy and release workflows are unchanged.

| Check | Enforced behavior |
| --- | --- |
| Frozen install and peer check | Reproducible lockfile and compatible dependency peers |
| `pnpm typecheck` / `pnpm lint` | Type checking and recommended Hooks/compiler diagnostics with zero warnings |
| `pnpm build` | Static export using the enabled strict React Compiler pipeline; no API needed |
| `pnpm check:web-build` | Built Next configuration retains strict compiler options and RN Web/shared transpilation; exported **app page** chunks contain compiler memoization (framework runtime alone is insufficient) |
| Offline Playwright | The same Chromium fixtures run against the static export and the Next development server; both exercise RN Web UI, compilation, hydration, connection/actions and responsive layout. Static-host CSP/header assertions apply to export only. |
| Retained evidence | Export/dev test results and screenshots use separate directories under `test-results/`; HTML reports use `playwright-report/export` and `playwright-report/dev`. Uploaded with `always()` and seven-day retention. |

The evidence upload ignores absent files when an earlier step fails; it does not turn a missing build/test into a pass. Captures are review evidence, not automatic visual approval. No public deployment, live provider keys, secrets or native builds are involved. Path filters mean a docs-only PR does not run this job; configure required checks with that behavior in mind rather than assuming a workflow run for every PR.

The user authorized the original web workflow addition and this follow-up CI update under Code Architect ownership. Workflow/action validation, the real build guard (including rejection cases), peer/type/lint/build/browser commands and artifact generation are checked locally. Hosted Actions execution requires publication and is not claimed as local evidence.
