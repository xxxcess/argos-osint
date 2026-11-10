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
- `fea751a` fixes the startup paths. A local fixture reproduced the old failure with the configured BusyBox image binary and exercised valid, empty, and oversized credentials through the corrected script. The [ARM64 packaged rerun](https://github.com/xxxcess/argos-osint/actions/runs/42777729999) passed Venture startup, authenticated readiness, web relay, Atlas delivery and owned shutdown at `fea751a`.
- A private Brain role serves retained actor, topology and Intel brief HTTP operations. The gateway uses a separate bearer token. Background brief work and live streams still run in the existing worker and gateway.
- The interactive Jobs client supports listing, details, logs, retry and cancellation. Venture's web mode attaches to Jobs or follows Compose logs. Exiting the terminal keeps the owned stack.
- Atlas collection reserves daily per-account quota in SurrealDB before dispatch and transfers the slot to the fallback account when the transfer succeeds. Standalone NewsAPI and provider-wide model quotas and cooldowns remain open.
- The shared scope planner has six passing offline cases, including 28 path cases. The hosted workflow owns the real quota reservation test. Nine targeted native tests and 19 service-host/API unit tests passed locally. Local Compose configuration passed for disk, external and Atlas profiles. Packaged and real datastore acceptance for this continuation use the hosted run.
- Updated the README, deployment, active wiki, diagrams, agent guidance and current plan. The historical records remain available as evidence.

- The required aggregate passed on `43e2725`. It included native Linux and Intel macOS, real Intel Rust-BERT, ARM64 server and private Brain network checks, the quota concurrency contract, native RabbitMQ dispatch and packaged ARM64 Compose. [Hosted run](https://github.com/xxxcess/argos-osint/actions/runs/42778990451)
- Moved the SurrealDB release version and four archive pins to the maintained harness manifest. All four offline provenance and resolver tests passed. This change still needs hosted coverage.
- Reconciled OpenCode role and verification guidance with its existing policy. All 22 local policy tests passed with no skips. The prior five failures no longer reproduce.
- Brief admission now commits the job, runnable tasks, captured inputs and retained run fence together. All five brief storage tests passed against the pinned SurrealDB 3.3.4 disk server. The new fixture forced a rollback and verified retry, captured inputs, reload idempotency and stale lease rejection. Dedicated worker and replay of retained projections remain open.
- Storage-backed NewsAPI tools now reserve the same daily quota as Atlas. Eleven NewsAPI/CourtListener adapter tests passed, including the new real-storage quota and loopback provider fixture. The two live-provider tests stayed ignored. The local embedded build stopped in RocksDB's bindgen setup because Clang could not find `stdbool.h`. The passing storage and adapter results used server mode.
- The Compose scope now schedules the same packaged Venture smoke on native Linux AMD64 alongside ARM64. Offline workflow and aggregate fixtures passed. The first hosted AMD64 result is pending.

## Branding, diagrams and brief completion

- The [required check at `4038049`](https://github.com/xxxcess/argos-osint/actions/runs/42781577487) passed, including native Linux and Intel macOS clients, real Intel Rust-BERT, ARM64 service and RabbitMQ checks, and Venture-owned packaged Compose on ARM64 and AMD64.
- Brief stage output, task state, metadata and the final retained run fence now commit together. Six storage tests passed against SurrealDB 3.3.4. The new test injected a fence failure, verified rollback and retried the same lease. Display and canonical publication still follow in separate commits.
- The active README, wiki, CLI/dataset guidance and diagrams use Fleetworks and Venture. The old Intel roadmap, investigation harness and superseded figures moved to the archive. Implementation identifiers and verification for compatibility code stay discoverable. All 19 changed active diagram pairs matched and rendered in Inkscape. The repository link and anchor scan reported zero broken references.

- Retained Atlas, Brain and Intel callers now import the shared model executor directly. The old Recon import forwards to the same implementation. Local server-mode checks passed 29 model routing and format tests, the real storage runtime fixture, and 19 API and host tests. Process isolation and shared model admission remain open.

## Service cutover and candidate verification

- Brief completion now persists publication work. A separate brief worker publishes saved output without rerunning models. A receipt fault fixture proved transaction rollback, replay from a fresh connection and no duplicate index work. The brief and curated-link suites passed. The curated-link performance benchmark stayed ignored.
- The network workspace now shares a provider RPM and cooldown clock. The cross-connection storage fixture and seven pacing unit tests passed. Account concurrency and interactive priority are still process-local.
- The private Tools process uses its own token. Manual calls keep durable request IDs, and an interrupted paid call retains its estimated charge. Tools, saved role defaults, owner-only Profile export, Brain, the brief worker, NLP and the classifier passed the disposable network smoke on pinned SurrealDB. All 21 API/host tests and 15 native client/Venture tests passed.
- Venture offers six interactive clients, a Compose development profile and startup with preloaded candidate images. The minimum Compose parser accepted the base, Atlas, development and external configurations. Docker startup and image identity verification are hosted.
- Scoped verification, scheduled full runs, candidate archives, and independent artifact verification are wired. All 21 offline fixtures, 22 policy checks, workflow lint and documentation checks passed. Hosted candidate and full outcomes remain separate evidence.

## Shared account admission and private operations

- The network store now shares per-account tickets with interactive priority, FIFO ordering and lease expiry. Calls without a configured alias use the default provider account. The pinned SurrealDB account contract verified capacity, priority, renewal and stale ticket rejection across connections.
- Control and Jobs now have separate private roles, credentials and route allowlists. The host entrypoint moved to `services/api-host`. Default workspace builds select Venture and the terminal clients.
- Venture can seed the Brain actor catalog from a pinned workspace CSV. Brain supports exact-name and indexed alias discovery. The disposable smoke rejected a wrong hash, replayed the same seed, searched both routes through the gateway and exercised outages for Brain, Tools, Control and Jobs.
- Brief publication now shares a retention fence with Atlas removal. The pinned server suite covered publication replay, removal, schema upgrade and actor catalog behavior. All 19 storage tests, 24 API and host tests, and 16 native client and Venture tests passed. The same binary passed the network smoke in memory and disk modes, including database identity on restart.
- The [scoped run at `9af6e50`](https://github.com/xxxcess/argos-osint/actions/runs/42798161278) passed native clients on Linux and both macOS architectures, real Intel Rust-BERT, and the ARM64 server lane. The web unit job exposed inherited browser isolation settings. The correction is pushed separately at `616a82f`.
- The [candidate run at `9af6e50`](https://github.com/xxxcess/argos-osint/actions/runs/42798161375) independently verified downloaded CLI packages on all four architectures. Container artifact verification and the additional packaged host-storage restart checks require hosted results.

## Model attempt boundary and retained history

- The private execution service now performs eligible Focus Brief generation attempts. It resolves provider credentials on the server, returns bounded typed progress and final results, and aborts its upstream attempt if the caller disconnects. The brief worker retains its account lease, retry chain and publication fence. Direct transports and subscription routes remain separate.
- Retained cancellation, application jobs, tool state, credit holds and historical reads now use shared modules. The compatibility commands retain their existing imports and archived record formats. Full retirement remains gated on compatibility and retained-service verification.
- All four private execution transport tests and five shared cancellation tests passed locally. The scoped compatibility storage and host integration suite runs separately.
- The original [candidate run at `9af6e50`](https://github.com/xxxcess/argos-osint/actions/runs/42798161375) passed image archive startup on both Linux architectures. The AMD64 candidate smoke now also exercises host-memory reset, host-disk retention and external-disk ownership across Venture restarts. The exact extended smoke has its own hosted result.
- The [web fix at `6b4d03f`](https://github.com/xxxcess/argos-osint/actions/runs/42800879060) passed hosted live relay, runtime restart and browser hot reload. The broader browser suite is recorded separately.
- The Natural training worker activated the checked-in CSV at SHA-256 `625f10150ba413f2fcae49ef425f9829004daaf9e69e49eecf66f7598de80d93` and a changed copy at the same path. Atlas remained on Rust-BERT.

## Retention

This already-tracked plan records the shared cutover and its remaining acceptance work. Keep raw logs, fixtures and intermediate notes out of Git.
