# Agent Testing Guidelines — Argos OSINT

Applies to test design, changes, review and execution. Follow root `AGENTS.md`, [contributor workflow](../../../docs/wiki/code/contributing.md), relevant [conventions](../../../docs/wiki/code/conventions.md), and the canonical [scope and finish-line policy](../../../docs/wiki/code/agent-workflow.md#scope-and-finish-line). OpenCode role boundaries apply only when that profile is selected.

## Authorization and scope

- Confirm verification tests and scope before execution unless this session already explicitly authorizes them. Authorization persists for affected retries; do not ask repeatedly. If the user will verify, leave runtime tests to them and report not run.
- Tie each test to requested acceptance or a concrete regression. Prove the critical contract early, freeze final-check inputs, and run agreed required gates. After freeze, fix only demonstrated acceptance blockers and rerun affected evidence; do not start unrelated audits or repeat unchanged broad checks. Honor wrap-up immediately.
- Reuse existing harnesses/dependencies and the current toolchain. Do not add frameworks, feature flags or infrastructure without an in-scope need. Inspect manifests and actual flags rather than copying stale commands.

## Test contracts

- Assert observable behavior through the smallest useful level: pure unit logic, public integration/persistence contracts, or critical end-to-end journeys. Avoid redundant layers, implementation-mirroring assertions and incidental SQL/log/count checks.
- Cover relevant boundaries, invalid/empty/Unicode input, partial failures, retries, ordering, cancellation, leases and recovery. Add a regression that would fail if the demonstrated bug returned. Coverage is diagnostic, not a numerical target.
- Never weaken/delete/ignore assertions, loosen tolerances, suppress warnings or auto-update snapshots just to obtain green results. Diagnose the original failure; environmental blockers and intentional skips remain explicit.
- Keep fixtures small, deterministic, isolated and offline. Each test owns its temporary directories, DB namespace/database, ports, state and mocks. Bind services to loopback; control clocks/IDs/providers instead of relying on sleeps, current dates, locale, DNS or external availability.
- Never use the real Argos home, user data, credentials, production/shared databases, live OSINT providers, paid APIs, real people/breach data or external infrastructure. Use synthetic targets and fake secrets; test redaction and untrusted content as data.
- Keep golden updates deliberate and versioned. Prefer focused assertions over huge snapshots. Cleanup errors must remain visible without hiding the original failure.

### 3.1 Native fixture prerequisites and diagnostic output

Use real local admission/publication, seeded catalog identities, current metadata, validated retained bodies and complete native NLP receipts where required. Tags, forged completion markers or hand-written checksums cannot grant current authority; adversarial forged rows are valid only to prove exclusion.

Exercise opt-in fixtures before using them as evidence. Preserve meaningful assertions during fixture corrections. Diagnostics stay bounded and omit secrets/article/body prose; retain original publisher/transaction failures. Fix stack allocation boundaries and verify the public path on its normal stack rather than enlarging test stacks.

## Rust and storage

- Unit tests live beside meaningful logic; public/cross-module integration tests live under `crates/<crate>/tests/`. Reuse `tests/support` rather than creating an accidentally discovered empty test crate. CLI process tests prove CLI contracts; executable examples follow public APIs.
- Test state machines and persistence: idempotence, owner/epoch fencing, timeout/backoff, cancellation, rollback/partial commits, restart/reopen and stale-generation exclusion. Use explicit barriers/channels or supported controlled time, never sleeps to hide races.
- Jobs SQLite/RocksDB tests use temporary local directories and the Jobs HTTP fixtures; never share their files between process owners. Test crash/reopen, idempotent outbox handoff, visibility expiry and stale delivery epochs. See [planned persistence scopes](../../../docs/wiki/code/jobs-persistence.md).
- For domain contracts, use current SurrealDB schemas, `storage::repo::Repository` and the existing isolated harness. Assert schema/bootstrap/version guards and records through public contracts; raw query tests are appropriate when query semantics are the contract. Disk reopen tests must use persisted storage.
- Keep provider/model calls injectable and mocked at the boundary. Cover quotas, budgets, fallback and malformed/error responses when those contracts change. Do not mock away the integration being proved.
- Use table-driven finite cases; add property-based tooling only when existing dependencies and the affected input space justify it. Include useful assertion diagnostics and reproduction data.
- Load `argos-rust-style` for Rust edits/review. No local Rust mutation tests or baselines (`cargo mutants` included); use hosted `mutation.yml` and report local mutation execution as deliberately skipped.

## React, browser and terminal

- Discover the existing package manager, lockfile, scripts and CI; do not scaffold or migrate a test stack. Keep type/lint/build/behavior evidence distinct and preserve React Compiler/Hooks checks.
- Test user-visible loading/success/empty/error/retry, validation, accessible roles/names, focus/keyboard and relevant responsive states. Test hooks through consumers and asynchronous API/cache behavior through realistic boundary fixtures. Reset caches, timers, handlers and storage between tests.
- Cover cancellation, pagination and stale-response races where relevant. Use accessible locators and Playwright auto-waiting; isolate accounts/backend data and preserve the real app path while stubbing external providers.
- Browser verification uses the repository Playwright runner. Set `ARGOS_WEB_TEST_DEV=1`; Next output/locks belong under `.agent-scratch/next-web-tests`. Never stop the user's frontend or remove its lock. Retain bounded traces/screenshots/errors for failed affected cases.
- Load applicable React/TUI skills. Use deterministic terminal state/cell fixtures, terminal restoration checks and actual PNG inspection. Generated images alone are not visual evidence; snapshots alone do not prove interactions. Missing visual evidence remains pending.

## Execution restrictions

Use the designated combined integration tree and one build owner. Worktrees are edit/format-only. Follow the [stable graph/resource policy](../../../docs/wiki/code/agent-workflow.md#keep-one-reusable-build-graph) and [cache hygiene](../../../docs/wiki/code/contributing.md#build-cache-hygiene); never bypass locks or clean active/user/global caches.

All agent Rust build/check/clippy/test commands use `python3 scripts/agent_cargo.py … --locked`; `cargo fmt` is the direct exception. Preserve the chosen feature graph, including activating fixture targets. Target storage tests explicitly require `ARGOS_SURREAL_MODE=embedded ARGOS_SURREAL_REQUIRED=1`; inspect each harness for skips.

For the required full native embedded gate, leave parent storage-mode variables unset:

```sh
env -u ARGOS_SURREAL_MODE -u ARGOS_SURREAL_BIN -u ARGOS_SURREAL_REQUIRED \
  bash scripts/local_matrix.sh --storage embedded
```

Keep `ARGOS_EMBED` unset unless its opt-in tests are explicitly authorized. Local agents run host-native embedded storage only; server/all-platform CI matrices remain hosted. Monitor available hosted results for other modes. A plain workspace storage skip does not replace the dedicated required embedded phase.

Documentation/config-only changes use their relevant authorized static/policy checks, not a Rust rebuild. Frontend checks use existing package scripts and locked dependencies. Required gates are not waived by retry caps, but unchanged failures do not justify repeating broad runs; return the demonstrated blocker and affected next check.

## Evidence and independent verification

Record command, exit, relevant mode/environment, executed/pass/fail/skip counts, tested input scope and affected artifact paths. Capture hashes only when performance/release provenance needs immutable identity; do not repeatedly hash unrelated files. Inspect intended execution: zero-match filters, ignored tests, early-return harnesses and missing services are not passes.

- **PASS:** all intended checks executed and passed.
- **FAIL:** intended assertions/build checks failed.
- **BLOCKED:** a required dependency/tool/service prevented execution.
- **SKIPPED:** intentionally not executed under a documented condition; never PASS.
- **PENDING:** required evidence not yet attempted or deferred to CI/user.

The OpenCode tester is an independent verifier, not a fixer: do not edit source/tests to make checks pass. Report the minimal reproducer, concrete `file:line` findings, actual evidence and unmet acceptance to the caller; the caller owns phase acceptance/plans. Distinguish local/hosted and visual/interaction/performance evidence. Do not begin an automatic final rereview or new acceptance search after agreed checks pass.
