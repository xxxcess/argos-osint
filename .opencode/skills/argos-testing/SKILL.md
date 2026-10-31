# Agent Testing Guidelines — Argos OSINT

> Applies to every agent that writes, changes, reviews, or executes tests in this repository. These rules extend (they do not replace) the root `AGENTS.md`, `docs/wiki/code/contributing.md`, and `.opencode/AGENTS.md`.

## 1. Mission and non-negotiable principles

Tests are executable product contracts. Write them to catch realistic regressions and prove externally observable behavior, not to make a coverage number look good.

1. Read the current implementation, callers, architecture notes, and contributor workflow before deciding what to test. Follow the source-of-truth code and live feature flags; do not copy assumptions from stale comments or migration-era docs.
2. Tests must be deterministic, isolated, repeatable, and safe to run locally and in CI.
3. Prefer behavior and contract assertions over implementation-detail assertions. Refactors that preserve behavior should not require rewriting tests.
4. Every test should have a clear reason to exist: a requirement, acceptance criterion, bug regression, invariant, or public contract.
5. Never weaken, delete, ignore, skip, loosen, or rewrite a failing test merely to obtain a green run. Diagnose the failure and fix the product or document a genuine environmental blocker.
6. Do not add dependencies, test frameworks, feature flags, or infrastructure until you have inspected the existing workspace and justified the addition. Reuse existing patterns and fixtures.
7. Never claim a check passed unless it was run and its exit status/result is known. Report skipped tests as skipped, not passed.
8. Protect user data. Never run destructive tests against the user's real `~/.argos`, configured production data, shared development database, real API accounts, or externally owned targets.

## 2. Start here: repository-specific constraints

Before running tests, read:

- `AGENTS.md`
- `docs/wiki/code/contributing.md`
- `docs/wiki/code/conventions.md`
- `docs/wiki/code/storage.md` for storage work
- `docs/wiki/code/architecture.md` and the relevant concept/code route for cross-cutting work
- `.opencode/AGENTS.md` when operating as an OpenCode agent

Current verified repository facts (re-check files if the branch has changed):

- Rust toolchain is pinned to **1.95.0** in `rust-toolchain.toml`.
- Cargo workspace members are `crates/argos-osint-core` and `crates/argos-osint-bin`.
- Core and binary default features are `embedded,nlp`. Embedded storage uses SurrealDB/RocksDB; server mode is opt-in through `server,nlp`.
- SurrealDB is the active storage system. SQLite/LanceDB are removed; CI explicitly rejects `rusqlite` and `lancedb` in the dependency graph.
- All agent Cargo commands must use `python3 scripts/agent_cargo.py … --locked`. Do not run Cargo directly for builds/tests/clippy. `cargo fmt` is the documented exception and runs directly.
- The agent local gate is `bash scripts/local_matrix.sh --storage embedded`. This includes format check, clippy with warnings denied, workspace tests, storage CLI smoke test, and embedded storage contracts.
- Agents run the **embedded** matrix locally. The server storage matrix is CI-only.
- Storage contract tests use `crates/argos-osint-core/tests/support` and `scripts/test_surreal.py`. Their environment controls matter; an unconfigured test may skip.
- Serialize Cargo commands that share `target/agents`. Do not delete lock files, kill unrelated Cargo jobs, or launch competing builds to speed up verification.
- Worktrees are edit-only. Run formatting there; run tests in the designated combined integration tree.

The repository currently contains a Rust workspace and OpenCode agent/test infrastructure. Do not assume a React app, package manager, Vitest, Testing Library, MSW, or Playwright is present. First inspect the current tree, lockfiles, package manifests, scripts, and CI. Apply the React guidance below only to an actual React surface and its existing toolchain. Do not scaffold a frontend test stack as part of unrelated Rust work.

## 3. Test design workflow (all languages)

For every behavior change:

1. **Map the change.** Identify touched modules, entry points, public interfaces, callers, persistence, events/jobs, CLI/TUI routes, UI surfaces, feature gates, and failure paths. Use tags and docs routes where available.
2. **State the contract.** Write down inputs, outputs, side effects, invariants, failure modes, and relevant security/privacy properties. Translate acceptance criteria into observable assertions.
3. **Choose the smallest useful test level.** Unit test pure logic; integration test module boundaries and persistence; end-to-end test only critical user journeys. Avoid testing the same behavior at every layer without a reason.
4. **Design cases before implementation.** Include normal behavior, boundaries, invalid input, empty/missing data, partial failure, retries/timeouts, concurrency or ordering where relevant, and regression examples.
5. **Use controlled fixtures.** Make every test own its data, temporary directories, ports, clock, environment and mocks. Avoid shared mutable process-wide state.
6. **Assert meaningful outcomes.** Assert returned values, persisted records, externally visible UI, emitted events, and side effects. Verify absence of dangerous or unintended effects too.
7. **Run targeted checks first.** Reproduce the focused behavior, then run the repository-required gate once the combined tree is ready.
8. **Review the tests themselves.** Ensure they fail for the intended reason, are not over-mocked, have useful assertion messages, and do not depend on execution order.
9. **Report evidence.** Include exact commands, exit codes, commit/revision or diff identity, environment/mode, pass/fail/skip counts, and any limitations.

Use Arrange–Act–Assert (or Given–When–Then) where it clarifies the behavior. Test names should describe a condition and expected result, e.g. `expired_lease_is_not_reclaimed_by_a_different_owner`, not `test_works`.

## 4. Rust testing standards

### 4.1 Placement and scope

- Keep focused unit tests close to the module under `#[cfg(test)]`; they may test private helpers only when the helper itself represents meaningful logic.
- Put public API and cross-module contracts in `crates/<crate>/tests/`. Each integration test should exercise the crate as a consumer would.
- Use `tests/support/mod.rs` or an existing shared harness for reusable integration setup; do not create a top-level `tests/common.rs` that Cargo will discover as a separate empty test crate.
- Keep doc examples executable where practical. Update examples when public APIs change.
- For the binary crate, test reusable behavior through core/public modules or execute the CLI as a process where the CLI boundary itself is the contract. Do not rely on inaccessible `main.rs` internals.
- Name files by contract or subsystem (the repository already uses names such as `storage_bootstrap.rs`, `storage_recall_turn.rs`, and `storage_lifecycle.rs`).

### 4.2 What to test

**Pure logic and data transformations**
- Serialization/deserialization round trips, schema compatibility, defaults, normalization, parsing, sorting, scoring, filtering, token/size limits and deterministic output.
- Boundary values: empty, singleton, maximum, Unicode, malformed, duplicate, reordered, missing optional values, and overflow/underflow where applicable.
- Prefer table-driven cases for finite input matrices.
- Use property-based testing only when it adds value for broad input spaces or invariants (e.g. URL normalization, parser robustness, graph edge normalization, ranking invariants). Check whether `proptest` or another generator is already present before adding it. Keep seeds/reproduction information and shrinking output actionable.

**Async services, jobs, and pipelines**
- Test success, error, cancellation, deadline expiration, retry limits, backoff, idempotency, lease ownership, stale worker behavior, and partial completion.
- Use explicit barriers/channels or controlled fakes instead of arbitrary sleeps. Use paused/injected time only where supported by the project's existing Tokio test-util feature and test patterns.
- Assert durable state transitions and recovery behavior, not just that a task returned `Ok(())`.
- For concurrent operations, test conflicting updates, duplicate delivery, worker restart, lost connection, and that one failed item does not corrupt successful work.
- Keep provider/model APIs behind injectable interfaces. Mock external providers at the boundary and test retry/rate-limit/circuit behavior without consuming credits.

**SurrealDB and persistence**
- Follow `docs/wiki/code/storage.md`, the actual schema constants/`.surql` declarations, the current `storage::repo::Repository` path, and the shared harness. Do not invent old SQLite/LanceDB assumptions.
- Use an isolated temporary datastore/database per test. Do not share namespaces, record IDs or state between tests unless the test explicitly verifies isolation.
- Cover schema bootstrap/idempotency/version guards; repository behavior; query and transaction semantics; uniqueness and relationship constraints; indexes/recall behavior; lifecycle/state transitions; durable job leasing/recovery; and error/rollback semantics.
- Prefer assertions on records and behavior through the repository contract. Test raw SurrealQL directly when the query/schema contract itself is under test.
- Verify atomicity and partial-failure boundaries. For batched ingestion, assert which individual records/receipts persist after one item fails.
- Test disk persistence/reopen behavior when relevant to the change. Embedded in-memory fakes do not prove disk durability.
- Storage suites must use the existing harness and mode contract. A harness returning `None` or printing `SKIPPED` is **not a passing storage test**. Confirm required suites actually executed and inspect the test summary.
- Agents run embedded mode locally only. Server-mode contracts are verified by CI as documented. Never point the test harness to the user's real Argos home.

**CLI and TUI**
- Test command parsing, validation, exit codes, output, state transitions, navigation, focus, resize behavior, keyboard/mouse interactions, loading/empty/error states, and terminal restoration at the correct layer.
- Prefer pure state/update functions and deterministic snapshots/fixtures over brittle terminal transcript matching.
- For visual TUI work, follow `argos-tui-verify`, `docs/wiki/code/tui-verification.md`, and existing screenshot fixture tooling. Open and inspect produced PNGs; a generated screenshot alone is not visual evidence.
- Keep visual, behavioral, and quantitative/performance evidence distinct. A snapshot does not prove interaction correctness.

### 4.3 Rust quality rules

- Avoid tests that assert private implementation structure, exact SQL text, incidental log wording, or internal call counts unless that is the contract.
- Avoid broad mocks that replace the very logic the test is supposed to exercise.
- Avoid `unwrap()` in test setup where a short contextual `expect("...")` materially improves diagnosis; production code should propagate errors rather than panic.
- Do not use ignored tests as a substitute for unavailable infrastructure. Explain why an opt-in test is ignored and provide the supported command to run it.
- Do not add sleeps to "make races go away." Synchronize on observable readiness or explicit coordination.
- Do not catch and discard errors in cleanup. Cleanup should be reliable and should not hide the primary assertion failure.

## 5. React and TypeScript testing standards (conditional on React code existing)

### 5.1 Discover the real stack first

Before writing frontend tests, inspect the app's package manifest, lockfile, workspace config, build tool, existing test files, CI workflow, and installed dependencies. Follow the repository's package manager and scripts. Do not assume Vite/Vitest simply because the code uses React.

Preferred modern patterns **when compatible with the actual app**:

- Component and hook behavior: React Testing Library with DOM-centric assertions and user-event.
- Unit/integration runner: existing Vitest if the app already uses Vite/Vitest; otherwise retain the configured runner (e.g. Jest) rather than migrating without scope.
- API boundary: MSW or the project's existing request mock layer; intercept at the network boundary instead of mocking every internal hook.
- Critical browser workflows: Playwright using accessible, user-visible selectors and isolated browser contexts.
- Type checking and linting remain separate checks; passing tests do not replace them.

React's current guidance discourages `react-test-renderer` and the old `react-dom/test-utils` APIs. Prefer React Testing Library for supported React DOM projects. Use async `act` from `react` only when library helpers do not already wrap it.

### 5.2 Component and hook behavior

- Render through the app's real providers where practical: router, query/cache client, theme, localization, and application context. Provide a small documented `renderWithProviders` helper if repeated setup exists.
- Query by role, accessible name, label, or visible text. Prefer `getByRole`/accessible queries; use test IDs only when no meaningful user-facing selector exists.
- Drive controls as a user would: click, type, select, keyboard navigation, submit, retry, dismiss, and wait for visible result.
- Assert loading, success, empty, validation, permission-denied, network-error, stale-data, and retry states where applicable.
- For hooks, verify public behavior through a consuming component when possible; avoid depending on hook internals or React scheduling details.
- Test accessibility semantics that affect operation: labels, roles, names, focus order, keyboard operation, disabled states, and live announcements for important updates.

### 5.3 Server state, APIs, and asynchronous UI

- Use realistic request/response fixtures that match current API contracts and schemas.
- Prefer MSW/network-level interception or an established application test adapter, preserving the real query/cache and serialization path.
- Cover success, empty responses, malformed payloads, 4xx/5xx, offline/network failure, timeout, cancellation, duplicate submissions, stale responses, retries and pagination.
- Test race conditions such as switching investigations while an old request is pending; assert that stale results do not overwrite the currently selected investigation.
- Reset query clients, handlers, timers, storage and mocks between tests. Avoid sharing a mutable singleton cache between cases.
- Do not call live OSINT providers, paid model APIs, real external search targets, or production services from automated tests.

### 5.4 Playwright and end-to-end tests

- Keep E2E tests focused on high-value end-to-end journeys: launch/open the app, start or resume an investigation, inspect results/evidence, filter/search, navigate Atlas/Brain, and verify job progress/recovery when those screens exist.
- Use accessible locators and assertions against user-observable behavior. Avoid CSS-class and component implementation selectors.
- Isolate each test's account/session/storage and backend data. Use unique test IDs and deterministic fixtures.
- Avoid fixed sleeps. Rely on Playwright auto-waiting and explicit web-first assertions.
- Stub external providers at the API boundary while keeping the app/backend path under test.
- Capture traces, screenshots and console/network errors on failure; do not use screenshot snapshots as the sole assertion.
- Keep browser tests out of the fast unit suite if runtime or environment makes them expensive; run them in the appropriate CI tier.

## 6. Argos OSINT domain-specific test matrix

Select relevant cases for the changed subsystem; not every change needs every row.

| Area | Minimum useful contract coverage |
| --- | --- |
| SurrealDB bootstrap/schema | Fresh DB, repeat bootstrap, version-too-new refusal, schema declarations, isolated DB per case |
| Storage/repository | CRUD/read filters, missing/duplicate records, query errors, transaction rollback, reconnect/reopen |
| Brain memories and graph | Allowed record kinds, actor/claim/inference/link relationships, deduplication, orphan handling, provenance, graph traversal boundaries |
| Recall and embeddings | Active/current revision/model filters, missing vectors, lexical fallback, ranking, top-K/token caps, stale generation exclusion |
| Atlas ingestion | Duplicate article, malformed metadata, partial article fetch, deterministic NLP extraction, retry/resume, phase receipts, related-source links, actor indexing |
| Claims/inferences | Evidence/source provenance, confidence bounds, idempotent re-extraction, replacement/supersession, unsupported claim rejection |
| Recon/investigations | Input validation, authorization/scope where enabled, deterministic planning gates, tool selection, timeout/cancellation, provider failure, resume/replay |
| Provider/model orchestration | Budget/credit accounting, global RPM limits, fallback order, bounded retries, malformed/empty model output, timeout and provider outage |
| Jobs/workers | State machine, lease expiry, owner checks, heartbeat, retry budget, cancellation, stale worker, crash/restart, cleanup TTL |
| CLI/TUI | Command contract, navigation, keyboard behavior, resize/layout constraints, loading/empty/error states, terminal cleanup |
| React surfaces (if present) | Accessible user flows, server-state lifecycle, error/retry, stale response handling, responsive/keyboard behavior |
| Security/privacy | Input validation, path/URL boundaries, secret redaction, no credential leakage in logs/errors, safe handling of untrusted article/provider content |

For OSINT specifically, use synthetic, clearly non-real identities/domains/IPs and fixtures. Do not query real people, real breach data, or real infrastructure during tests. Treat scraped content and provider responses as untrusted input. Test HTML/script-looking payloads as data and verify they cannot become executable UI or prompt/control instructions.

## 7. Test data, mocks, and fixtures

- Prefer small fixtures that express the contract; keep large fixture corpora out of hot unit tests.
- Give each test its own temp directory, unique DB namespace/database, process, and network port. Bind local-only services to loopback.
- Do not depend on wall-clock time, random IDs, external DNS, the public internet, provider availability, machine locale, or current date. Inject clock/ID/provider dependencies where necessary.
- Freeze time or use a controllable clock for leases, TTL, expiry and scheduling. Test boundaries immediately before, at, and after expiry.
- Seed only what the scenario needs. Assert cleanup and isolation.
- Never store real credentials in fixture files. Use deliberately fake values and assert secrets are redacted.
- Avoid snapshots of huge API payloads or complete pages. Prefer focused assertions and named fixtures.
- Keep golden fixtures intentionally versioned and explain how they are updated. Never auto-update snapshots merely to silence a failure.

## 8. Coverage and mutation thinking

- Coverage is a diagnostic, not a target to game. Focus on branch and failure-path coverage for critical stateful logic, not a repository-wide arbitrary percentage.
- Prioritize high-risk paths: persistence, retries, idempotency, state transitions, provider quota accounting, cancellation, fallback, parsing of untrusted data and user-visible failures.
- When a bug is fixed, add a regression test that failed before the fix and passes after it.
- Ask: "Would this test fail if the bug returned?" and "Could the test pass while the user-visible behavior is still broken?"
- Use mutation testing selectively for high-risk pure logic or policy modules when the tooling is already available or the setup cost is justified. Do not introduce a mutation-testing dependency for a trivial change.
- Do not inflate test count with multiple tests that assert the same outcome without adding distinct boundary or failure coverage.

## 9. Running checks and interpreting results

### Rust local gate

From the designated combined integration tree:

```bash
bash scripts/local_matrix.sh --storage embedded
```

This is the authoritative local agent gate. It runs format checking, Clippy with `-D warnings`, all workspace tests, a CLI smoke test, and embedded SurrealDB contracts. The script invokes the Cargo wrapper according to repository policy.

For a targeted diagnosis, use the required wrapper, preserve `--locked`, and keep the command scoped to the affected crate/test. Examples (adjust package/test names after inspecting the workspace):

```bash
python3 scripts/agent_cargo.py test -p argos-osint-core --test storage_bootstrap --locked
python3 scripts/agent_cargo.py test -p argos-osint-core <test_filter> --locked
python3 scripts/agent_cargo.py clippy -p argos-osint-core --all-targets --locked -- -D warnings
```

Do not copy/paste a targeted command blindly if the target is feature-gated or requires the SurrealDB harness. Use `scripts/test_surreal.py --help` and repository docs to select the supported mode. Do not run the server matrix locally as an agent.

The documented repeat budget for the full gate is 1–2 runs, maximum 3. On failure, diagnose with a focused check, fix the cause, then run one final full gate. Do not loop retries without new evidence.

### Frontend checks

Only after discovering the current frontend tooling, run the package's existing scripts for focused tests, type checking, lint, build and E2E. Use the lockfile's package manager and locked/frozen install mode when appropriate. Do not invent command names: read `package.json` and workspace scripts.

### Result classification

- **PASS:** command executed and all intended tests ran and passed.
- **FAIL:** one or more intended assertions/build checks failed.
- **BLOCKED:** execution could not proceed because a required tool/service/dependency was unavailable.
- **SKIPPED:** test intentionally did not run due to a documented condition. This is not PASS.
- **PENDING:** required verification has not yet been attempted or is deferred to CI.

For every run, capture command, working directory, tested revision/diff, exit code, relevant environment flags, test counts, and skipped tests. Separate local embedded results from CI server-mode results.

## 10. Tester agent operating procedure

The tester agent is an independent verifier, not a fixer.

1. Read the active plan/acceptance criteria, contributor workflow, and changed-file list.
2. Wait for editors to finish. Record the exact combined-tree revision and dirty diff identity before testing.
3. Verify that tests cover each acceptance criterion and important failure mode. Identify missing tests even when CI is green.
4. Do not edit source or tests to make verification pass. Report a minimal reproducer and evidence to the architect.
5. Run targeted tests, then the applicable required gate in the integration root. Follow the serialization and retry limits.
6. Inspect logs and artifacts; for TUI/UI work, inspect screenshots and browser evidence rather than assuming artifacts prove correctness.
7. Check for skipped tests, filtered-out suites, feature-gated tests that did not compile, tests that return early, and environment variables that silently bypass coverage.
8. Return exactly one overall status: **PASS**, **FAIL**, **BLOCKED**, or **PENDING**. Include:
   - PLAN_ID and phase (if applicable)
   - tested revision/diff identity
   - commands and exit codes
   - suites/tests passed, failed, skipped, or not run
   - environment and storage mode
   - artifact paths and inspected evidence
   - findings with file:line references and severity
   - unmet acceptance criteria and next required verification

A green command is not sufficient if required tests silently skipped or the wrong feature/mode was compiled.

## 11. What a good test looks like

Rust:

```rust
#[test]
fn normalizes_equivalent_hostnames_without_changing_the_target() {
    let cases = [
        ("Example.COM", "example.com"),
        ("example.com.", "example.com"),
    ];

    for (input, expected) in cases {
        assert_eq!(normalize_host(input), expected, "input: {input}");
    }
}
```

React (only when React + Testing Library are actually configured):

```tsx
it('shows a retry action when loading an investigation fails', async () => {
  const user = userEvent.setup();
  render(<InvestigationPanel investigationId="fixture-investigation" />);

  expect(await screen.findByRole('alert')).toHaveTextContent(/could not load/i);

  await user.click(screen.getByRole('button', { name: /retry/i }));

  expect(await screen.findByRole('heading', { name: /investigation/i })).toBeVisible();
});
```

These snippets illustrate intent, not confirmed repository APIs. Adapt imports, fixtures, render helpers and commands to the implementation that exists.

## 12. Reference material (reviewed 2026-10-29)

Use upstream documentation as the primary authority. Check the version relevant to the pinned toolchain before adopting new APIs.

### Rust
- The Rust Book — writing automated tests: https://doc.rust-lang.org/book/ch11-00-testing.html
- The Rust Book — test organization (unit vs integration): https://doc.rust-lang.org/book/ch11-03-test-organization.html
- Cargo Book — tests and doctests: https://doc.rust-lang.org/cargo/guide/tests.html
- Proptest — property-based testing and shrinking: https://proptest-rs.github.io/proptest/ (optional; inspect existing dependencies first)
- Tokio testing guide (only where Tokio is already used): https://tokio.rs/tokio/topics/testing

### React / browser
- React — `act`: https://react.dev/reference/react/act
- React — `react-dom/test-utils` deprecation and Testing Library recommendation: https://react.dev/warnings/react-dom-test-utils
- React Testing Library — introduction and guiding principle: https://testing-library.com/docs/react-testing-library/intro/
- Vitest — guide: https://vitest.dev/guide/
- Vitest — mocking: https://vitest.dev/guide/mocking
- Vitest — browser mode: https://vitest.dev/guide/browser/
- Playwright — best practices: https://playwright.dev/docs/best-practices
- Playwright — writing tests and auto-waiting assertions: https://playwright.dev/docs/writing-tests
- MSW — documentation: https://mswjs.io/docs/

### Argos-specific local authority
- Contributor workflow: `docs/wiki/code/contributing.md`
- Storage map: `docs/wiki/code/storage.md`
- CI matrix: `docs/wiki/code/ci.md`
- TUI verification: `docs/wiki/code/tui-verification.md`
- OpenCode workflow and handoffs: `docs/wiki/code/opencode-v2.md`

When upstream guidance conflicts with repository-specific safety, reproducibility, CI, or resource constraints, follow the repository policy. Do not upgrade toolchain/dependencies just to use a newly documented feature.

## Rust mutation execution restriction

Until further notice, agents must not run Rust mutation tests or mutation baselines locally, including `cargo mutants` and cargo-mutants runner invocations. Use the GitHub Actions `mutation.yml` workflow for execution and inspect its hosted results. Local mutation execution is deliberately skipped, not passed; ordinary targeted checks and the embedded gate remain required.
