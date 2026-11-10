# Progress and handoff

## 2026-11-09 — Planning deliverable
- Audited service boundaries and retained dependencies; inspected existing hosted CI results read-only.
- Read planning-with-files skill and repository planning/retention rules.
- Initialized PLAN_ID=2026-11-09-fleetworks-overhaul and updated local .planning/.active_plan (previously 2026-11-05-atlas-performance-completion).
- Created task_plan.md, design.md, acceptance.md, findings.md and this progress record.
- Recorded full documentation/agent-guidance reconciliation as implementation work, including all older planning artifacts and symlinked skills.
- No product source, existing guidance, workflows or historical plans changed. No builds, runtime tests or CI dispatches run.

## Preserved dirty-tree baseline
README.md; crates/argos-osint-core/src/hardware.rs; crates/argos-osint-core/src/paths.rs; docs/wiki/code/classifier.md; docs/wiki/code/contributing.md; docs/wiki/code/storage.md; docs/wiki/concepts/providers.md; docs/wiki/concepts/usage.md.
Initial diff: 8 files, 143 insertions, 24 deletions. Do not overwrite or attribute these changes to this task.

## Verification status
Planning-file existence, relative links, explicit review boundary and unchanged tracked baseline checked after writing. Functional checks remain NOT RUN. CI figures are historical evidence, not validation of proposed Fleetworks code.

## Resume
Read task_plan.md, findings.md and progress.md from this exact named directory. Use design and acceptance as the approved target. Start the rebuild from the clean `158774d` branch baseline. The older dirty-tree note describes another checkout.

The user authorized implementation and incremental publication on `feat/atlas-zero-shot`. The previous local integration revision is unavailable. Every pushed slice needs its own verification.

## Rebuild checkpoint 1

Venture initializes the workspace and its private API token, imports a safe subset of legacy configuration, checks API readiness, and launches the six sibling clients. The native dependency graph excludes the backend storage and inference engines.

Linux x86-64 Rust 1.95.0 checks passed for the four new packages. All 12 focused Rust tests and nine release policy tests passed. A native smoke test exercised the built binaries through workspace initialization, JSON Profile output, six sibling launches, and an authenticated readiness request to a local fixture. The server roles, Compose and hosted platform gates are pending.

## Rebuild checkpoint 2

The API host now has distinct bootstrap, authenticated API and background worker roles. The legacy combined role stays available. Network storage uses separate bootstrap and database-scoped credentials. The runtime verifies the exact schema version and supports verified TLS. Tools, Models, Profile and Logs use retained API reads.

The network-enabled build and 13 focused Rust tests passed. The native smoke passed against SurrealDB 3.3.4 in memory and disk modes. It verified privileged bootstrap, scoped runtime login, the API ingress policy, retained reads, worker lifetime, identity reset for memory, and identity retention for disk. Hosted Linux ARM64 and Intel macOS results will be recorded separately. Compose runtime verification remains pending.

## Rebuild checkpoint 3

Venture now owns startup, readiness, logs, and shutdown for a local Compose project. Compose offers a managed disk database and an external database selection, with a separate bootstrap, API, worker and authenticated production web relay. The developer overlay and Venture-managed host storage are pending.

Eight scoped Venture and workspace Rust tests passed, and the native clients rebuilt. Web typecheck, lint, all 331 unit tests, four focused WebSocket tests, and the Next production build guard passed. Compose configuration resolved in both profiles with the pinned local Compose parser. Actual image build and startup are assigned to the hosted packaged smoke.

## Retention
Working task_plan/findings/progress are ignored local planning memory. design.md and acceptance.md are curated review artifacts eligible for Git, not staged or committed here.
