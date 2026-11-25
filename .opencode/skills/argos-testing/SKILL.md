# Agent Testing Guidelines — Argos OSINT

Follow root `AGENTS.md`, [contributing](../../../docs/wiki/code/contributing.md), [conventions](../../../docs/wiki/code/conventions.md), and [scope and finish-line policy](../../../docs/wiki/code/agent-workflow.md#scope-and-finish-line). OpenCode boundaries apply only when selected.

## Authorization and scope

- **CI first (Nightly GSD):** Run affected checks, tests, builds, and captures in [GitHub Actions](../../../.agents/skills/gsd/SKILL.md#github-ci-first) and use exact-head artifacts for native review; run locally only for required local hardware/state under user authorization.
- **Confirm scope once:** Unless already authorized, offer (a) scoped static/feature checks + native review (reporting remaining Docker/embedded/platform gates as pending), or (b) all applicable gates through resolution; confirm local vs. CI. Authorization persists across affected retries. On CI selection, push the task branch and track exact-head checks; if a matching local-platform native artifact passes during interactive review, review it while broader gates continue (compile locally only if needed for that review).
- **Scope discipline:** Tie every test to acceptance or a concrete regression. Prove the critical contract early, freeze final inputs, and after freeze fix only demonstrated acceptance blockers with affected reruns. Reuse existing harnesses/dependencies/flags—inspect manifests instead of copying stale commands.

## Test contracts

- Assert observable behavior at the smallest useful level (unit, public integration/persistence, critical E2E); avoid implementation-mirroring assertions or incidental SQL/log/count checks.
- Cover boundaries, invalid/empty/Unicode inputs, partial failures, retries, ordering, cancellation, leases, and recovery; add a failing-before-fix regression. Coverage is diagnostic, not a numeric target.
- Never weaken/delete/ignore assertions, loosen tolerances, suppress warnings, or auto-update snapshots to get green; keep blockers and intentional skips explicit.
- Keep fixtures small, deterministic, isolated, and offline: own temp dirs, DB namespaces, loopback ports, controlled clocks/IDs/mocks (no sleeps, real Argos home, user/real people data, live providers, or external services). Test redaction and untrusted content as data. Keep golden updates deliberate and cleanup errors visible.

### 3.1 Native fixture prerequisites and diagnostic output

Use real local admission/publication, seeded catalog identities, current metadata, validated retained bodies, and complete native NLP receipts where required. Tags, forged completion markers, or hand-written checksums never grant current authority (forged rows are valid only to prove exclusion).

Exercise opt-in fixtures before citing them as evidence and preserve meaningful assertions during fixture fixes. Keep diagnostics bounded and free of secrets/article prose while retaining original publisher/transaction errors. Fix stack-allocation boundaries and verify on the normal public stack—never enlarge test stacks.

## Rust, storage, and backend

- Unit tests sit beside logic; cross-module tests live in `crates/<crate>/tests/` (reuse `tests/support` to avoid empty test crates). Load `argos-rust-style` for Rust edits/reviews.
- Test state machines and persistence: idempotence, owner/epoch fencing, timeout/backoff, cancellation, rollback, restart/reopen, and stale-generation exclusion via barriers/channels/controlled time (no sleeps).
- Jobs SQLite/RocksDB tests use private temp dirs and Jobs HTTP fixtures (crash/reopen, idempotent outbox, visibility expiry, stale delivery epochs; [jobs persistence](../../../docs/wiki/code/jobs-persistence.md)).
- SurrealDB contracts use current schemas, `storage::repo::Repository`, and the isolated harness (disk reopen tests must persist to disk). Mock providers/models at injectable boundaries without mocking away the tested integration. Use table-driven cases; add property testing only when existing deps and input space justify it.
- Linux backend/search changes require real isolated domain/outbox/index/gateway/dossier and restart/lease/pressure checks. Native PNGs, optional Natural training, or doc-only Rust-BERT compilation do not prove Linux model inference; both Linux and physical Mac architectures remain explicit platform evidence.
- **No local Rust mutation runs** (`cargo mutants`): use hosted `mutation.yml` and report local mutation as deliberately skipped.

## React, browser, and Rune UI

- **React/Playwright:** Reuse existing package manager/scripts/CI; keep type/lint/build/behavior evidence distinct with React Compiler/Hooks enabled. Test loading/success/empty/error/retry, validation, a11y roles/names, focus/keyboard, responsive viewports, cancellation, pagination, and stale-response races; reset caches/timers/storage between tests. Use repo Playwright with `ARGOS_WEB_TEST_DEV=1` (Next output/locks in `.agent-scratch/next-web-tests`; never touch the user's frontend).
- **Rune native:** Follow [fleetworks-rune-verify](../fleetworks-rune-verify/SKILL.md) and [native verification](../../../docs/wiki/code/native-verification.md). Test production `State`/handlers via Rune Runtime, then capture real-view PNG/tree/scripts via `fleetworks_verify.py native-ui --agent` on the shared debug graph with synthetic effects. Keep native UI, transport, packaging, inspected captures, and matching-hardware GUI evidence distinct; view-only changes need neither terminal captures nor backend engines.
- **PR visual review:** Dispatch [UI visual review](../../../docs/wiki/code/native-verification.md#dispatchable-ui-visual-review) at frozen PR head for affected scenes/surfaces, inspect captures, attach selected images inline, and retain artifacts + durable refs (generated-only or unavailable dispatch stays pending).

## Execution restrictions

Work in the combined integration tree with one build owner (worktrees are edit/format-only) per [build graph policy](../../../docs/wiki/code/agent-workflow.md#keep-one-reusable-build-graph) and [cache hygiene](../../../docs/wiki/code/contributing.md#build-cache-hygiene). All agent Rust commands use `python3 scripts/agent_cargo.py … --locked` (`cargo fmt` excepted). Targeted storage tests need `ARGOS_SURREAL_MODE=embedded ARGOS_SURREAL_REQUIRED=1`.

Required full native embedded gate (parent storage vars unset):

```sh
env -u ARGOS_SURREAL_MODE -u ARGOS_SURREAL_BIN -u ARGOS_SURREAL_REQUIRED \
  bash scripts/local_matrix.sh --storage embedded
```

Keep `ARGOS_EMBED` unset unless explicitly authorized. Local runs are host-native embedded only (server/all-platform matrices stay in CI); a plain workspace skip never substitutes for the dedicated embedded phase. Docs/config-only work uses static/policy checks, not a Rust rebuild.

## Evidence and independent verification

Record command, exit code, mode/env, executed/pass/fail/skip counts, tested scope, and artifact paths (hash only when perf/release provenance requires it). Zero-match filters, ignored tests, early-return harnesses, and missing services are not passes.

- **PASS:** all intended checks executed and passed.
- **FAIL:** intended build/assertion failed.
- **BLOCKED:** required tool/dependency/service prevented execution.
- **SKIPPED:** intentionally skipped under a documented condition (never PASS).
- **PENDING:** required evidence not yet attempted or deferred to CI/user.

The OpenCode `tester` is a read-only verifier (never edits code/tests to pass): return minimal reproducer, `file:line` findings, actual evidence, and unmet acceptance to the caller; do not start automatic rereviews after agreed checks pass.
