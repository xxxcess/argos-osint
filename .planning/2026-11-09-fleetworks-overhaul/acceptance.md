# Verification and documentation acceptance plan

Status: hosted candidate evidence remains `37b9250`. Local cutover commits through `4c3b074` are recorded in [progress](progress.md#2026-11-10--retained-provider-transports). Full cutover acceptance is open.

## Verification interface
Introduce a shared scope-aware entrypoint with plan, local, pr and full modes. plan prints selected checks without executing. Select changed files plus dependents; take the union of scopes. Shared contracts, manifests, lockfiles and build tooling widen coverage; unknown paths fail safe to broad coverage. Required aggregate status must report for every revision and distinguish irrelevant, skipped, failed, cancelled and missing checks.

| Scope | Local iteration | Required PR evidence |
| --- | --- | --- |
| Docs/planning | Links, affected policy checks | No Rust/browser build |
| DB/schema | Focused real persistence contracts | Bootstrap, transactions, queries, disk reopen, memory reset, affected API consumers |
| Jobs/broker | State-machine and worker tests | Real broker+DB outbox, duplicates, crash, cancel, dependencies, dead letters |
| API/service | Unit and HTTP contracts | Auth/errors and consumer compatibility; real persistence where affected |
| TUI rendering | TestBackend/interaction fixtures; inspect PNGs | Lightweight CLI build and affected viewport matrix; no inference/DB engine dependencies |
| Venture/lifecycle | Focused process checks | PTY, signals, ownership, workspace paths, Compose start/stop/failure |
| RN Web UI | Types/lint/component/Query tests | Production build/Compiler guard, affected production Playwright and screenshots |
| Web runtime/relay | Transport contracts | Production+development lifecycle/browser, CSP/origins, reconnect/HMR |
| NLP/classifier | Deterministic batch fixtures | Affected runtime tests; real pinned model when runtime/model behavior changes |
| Cross-service/Compose | Contract/config validation | Real affected integration and Venture→Atlas→Brain/Intel smoke |

## Runner and execution policy
Primary Rust service runner: ubuntu-24.04-arm. Web: Linux x86_64; no measured ARM web advantage. Native CLI compatibility: macOS ARM64 for affected terminal/launcher/platform behavior; Intel for Intel-specific changes. Container/native-dependency/architecture-sensitive edits trigger both Linux architectures.
Run selected checks on every PR update including synchronize; cancel stale PR runs. Keep one always-reporting aggregate required check. Ordinary UI changes need production browser coverage; full dev-browser coverage runs for runtime/build/HMR changes and nightly.
Nightly runs full retained services, both image architectures, supported CLI platforms, production/dev browser suites and recovery. Scoped mutation runs hosted when relevant code changed. Release checks the exact published binaries/images; reuse immutable build outputs.
Replace blanket local embedded gate with scope gates once approved guidance is updated. Permit isolated local server/broker tests for affected scopes. Maintain wrapper/cache ownership and ARGOS_WEB_TEST_DEV=1. Retain full unit coverage currently absent from old server legs when retiring embedded checks. Never use real user data/secrets/providers for fixtures.
Record queue/setup/cache/compile/test/image/upload timings, SHA, runner, feature graph and cache hits; retain Cargo timings and test counts. Targets, not promises: static feedback <2m, lightweight client/UI <5m, main service lane <15m. Compare comparable successful-run medians; failures remain visible.

## Functional acceptance
- [ ] Venture displays resolved workspace and launches all CLI/web modes.
- [ ] API-key/model-default import preserves originals and redacts secrets.
- [x] Packaged smoke passes for Compose disk, host memory, host disk and external disk. The restart checks verify host-memory reset and disk retention.
- [ ] Confirm full storage and recovery parity, including the full embedded suite and rejection of stale broker jobs after a memory reset.
- [ ] Atlas→NLP/classifier→Brain/Intel and retained brief generation pass offline fixtures.
- [ ] Jobs survive worker/broker restart; cancel/retry/dependency/idempotency behavior holds.
- [ ] Tools/Models/Profile/export behavior is preserved without Recon orchestration.
- [ ] Client exit preserves running services; explicit shutdown respects ownership.
- [x] Both Linux container architectures and all four supported CLI targets pass downloaded-candidate smoke.
- [x] Packaged Venture activates Natural models for two exact CSV hashes while Atlas selects Rust-BERT.
- [ ] Verify the published registry artifacts after release. No registry push has run. This machine has no Docker daemon.
- [ ] Scope-selection fixtures cover docs, UI, TUI, DB, API, lockfiles, shared contracts and unknown paths.
- [ ] No stale revision or unexecuted check is reported as passed.

## Documentation and agent guidance inventory
Inventory tracked and local maintained surfaces before implementation; this is an exhaustive category checklist, not permission to delete unknown files blindly.

| Surface | Required reconciliation |
| --- | --- |
| Root AGENTS.md and README.md | Fleetworks identity, service map, workspace, scope gates, entrypoint and build outputs |
| docs/index.md and wiki indexes | New navigation and retired routes |
| docs/wiki/code/{agent-workflow,contributing,conventions,ci,planning-artifacts}.md | One canonical scope/finish-line/verification policy; remove contradictory embedded-only requirements |
| Code/concept docs for storage, architecture, interfaces, API/web, classifier, providers, usage, TUI and React | Actual ownership/contracts, environment variables and supported runtime modes |
| .opencode/AGENTS.md, commands, roles and skills | Align planning/implementation/testing/verification/review instructions and references |
| .agents/skills links | Preserve/repair links to canonical .opencode skills; no divergent copies |
| .agy/howto.md, lite-team.md, heavy-team.md and referenced guidance | Update applicable host-role instructions without imposing those roles globally |
| .planning and docs/archive plus other roadmaps/specs | Classify every artifact; promote valid decisions, preserve active/unresolved work, supersede outdated direction, remove reviewed obsolete records |
| CI/scripts/release instructions | Move checksum provenance out of planning; preserve current SurrealQL and dependency guards |
| Diagram HTML/SVG pairs, tags, catalogs and cross-links | Synchronize architecture labels/arrows and verify links/visuals |

Create a disposition inventory during Phase 2 before deleting historical material. Update all maintained affected documents; do not rewrite historical results as new results. Reconcile older plans with this plan and preserve unrelated unresolved evidence. Product guidance changes alongside implemented behavior, while transitional agent guidance explicitly names which rules apply before/after cutover. Workflow changes remain Code Architect-owned.

## Final handoff
Freeze, run selected checks and migration-wide parity, inspect affected UI/diagram captures, record exact revision/commands/results/failures/skips. Preserve useful evidence and promote durable docs per retention policy. Review only changed scope after freeze.
