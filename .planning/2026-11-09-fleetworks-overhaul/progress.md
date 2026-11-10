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

## Rebuild checkpoints 4–8

- The packaged web build excludes test sources after the hosted Next type-check failure. The production build passed locally with the same source inputs.
- Venture starts and stops its own SurrealDB in host memory or host disk mode. A native SurrealDB fixture exercised both restarts with simulated Docker commands. Host disk retained the marker. Host memory reset it.
- Atlas submits stable job and run IDs, publishes durable RabbitMQ deliveries, and fences replay with database identity, revision, and worker lease. Local SurrealDB and RabbitMQ passed submission replay, queued cancellation, completion replay, worker restart, and retry with monotonic attempts.
- A private warm Compromise NLP service serves ordered, bounded annotation batches. The local TCP test covered request order and cache reuse. An authenticated Rust-BERT service verifies exact model pins and receipts. The offline worker fixture covered both tags and rejection.
- Venture snapshots the selected CSV for its one-shot Natural training worker. A disposable workspace trained and activated the current dataset and a modified copy. The launcher and the training report agreed on the SHA-256 for each run.
- Intel macOS passed the native Venture and CLI build at the Atlas dispatch checkpoint. Real checkpoint inference on Intel and packaged Atlas Compose startup have dedicated hosted gates. Treat both as pending until their exact runs finish.

The full service cutover remains open. In particular, Brain and Intel integration, provider quotas, retained interactive terminal parity, release artifacts, and Recon retirement need implementation or acceptance evidence.

## 2026-11-10 — Cutover continuation

- GitHub Actions at `2934b1f` passed native Linux and Intel macOS, real Rust-BERT inference, the ARM64 server, and native RabbitMQ dispatch. Packaged Compose failed when the SurrealDB image invoked BusyBox under an unsupported executable name.
- `fea751a` fixes the startup paths. A local fixture reproduced the old failure with the configured BusyBox image binary and exercised valid, empty, and oversized credentials through the corrected script. The [packaged rerun](https://github.com/xxxcess/argos-osint/actions/runs/42777729999) owns container startup evidence.
- A private Brain role serves retained actor, topology and Intel brief HTTP operations. The gateway uses a separate bearer token. Background brief work and live streams still run in the existing worker and gateway.
- The interactive Jobs client supports listing, details, logs, retry and cancellation. Venture's web mode attaches to Jobs or follows Compose logs. Exiting the terminal keeps the owned stack.
- Atlas collection reserves daily per-account quota in SurrealDB before dispatch and transfers the slot to the fallback account when the transfer succeeds. Standalone NewsAPI and provider-wide model quotas and cooldowns remain open.
- The shared scope planner has six passing offline cases, including 28 path cases. The hosted workflow owns the real quota reservation test. Nine targeted native tests and 19 service-host/API unit tests passed locally. Local Compose configuration passed for disk, external and Atlas profiles. Packaged and real datastore acceptance for this continuation use the hosted run.
- Updated the README, deployment, active wiki, diagrams, agent guidance and current plan. The historical records remain available as evidence.

## Retention

This already-tracked plan records the shared cutover and its remaining acceptance work. Keep raw logs, fixtures and intermediate notes out of Git.
