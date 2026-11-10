# Fleetworks service extraction and repository overhaul

## Goal
Replace the Terminal OSINT research desk with Fleetworks: a Venture-managed collection of isolated network services, terminal clients and React Native Web, with scope-based fast verification.

## Current Phase
Implementation in progress on `feat/atlas-zero-shot`. Keep shipping verified slices from the approved plan and record hosted outcomes by revision.

## Scope
Docker images, Compose, Venture and the CLI suite, workspace/storage configuration, retained Atlas/Brain/Intel/tool/model functionality, RabbitMQ dispatch, repository reorganization, obsolete Recon removal, documentation/planning reconciliation and agent guidance updates.
Preserve unrelated changes and existing API-key configurations/model defaults. Start fresh application data. No remote application-stack management, Kubernetes, new Recon pipeline, automatic deletion of user state, or performance claims without measurement.

## Review documents
- [Design and service composition](design.md)
- [Verification and documentation acceptance](acceptance.md)
- [Audit and CI evidence](findings.md)
- [Progress and handoff](progress.md)

## Finish line
All retained capabilities run through Venture with network-only clients; selected storage modes and restart/recovery contracts pass; images and CLI outputs are verified; Recon is unreachable; docs and agent guidance describe the actual delivered architecture. Freeze before final checks, reuse unaffected evidence, and repair only demonstrated in-scope blockers afterward.

## Phases

### Phase 1 — Audit and reviewable plan
Status: complete
- [x] Inspect source boundaries, workspace conventions, existing CI and hosted job timings.
- [x] Record user decisions, architecture, verification matrix and documentation obligations.
- [x] Create named planning-with-files records and select this plan.
Acceptance: user can review scope and implementation order without relying on chat history.

### Phase 2 — Baseline, guidance transition and contracts
Status: in implementation
- [x] Record current tree and retained-function acceptance map; preserve dirty files.
- [ ] Inventory all planning/docs/agent surfaces; assign retain, update, supersede or remove dispositions, preserving unresolved work.
- [ ] Update canonical execution/testing guidance to distinguish legacy checks from approved Fleetworks checks; do not claim unbuilt features exist.
- [x] Establish lightweight versioned HTTP contracts, clients and workspace configuration. Service ownership continues in Phase 4.
- [x] Move the legacy SurrealDB CI archive pins to a maintained release manifest with an offline validation fixture.
Acceptance: no cleanup destroys a runtime/CI dependency; guidance and scope selection have a single source of truth.

### Phase 3 — Workspace, storage and shared primitives
Status: in implementation
- [x] Implement workspace resolution, configuration/secrets loading and explicit API-key/model-default import.
- [x] Implement managed host memory/disk SurrealDB and external disk-server configuration, authentication and TLS.
- [ ] Extract shared graph, model execution, jobs and cancellation primitives from Recon.
- [ ] Split contracts/client/UI crates from RocksDB, ONNX and QuickJS dependencies.
Acceptance: all three storage selections work through a network endpoint; original .argos state remains untouched.

### Phase 4 — Services and durable execution
Status: in implementation
- [x] Separate API serving, schema bootstrap and background workers. Dedicated domain entrypoints and their actions continue below.
- [ ] Complete service boundaries. Atlas, NLP and classifier roles are wired. The private Brain role serves retained actor, topology and Intel brief HTTP operations. Separate brief execution and live progress, model execution and tool execution remain in scope.
- [x] Admit Focus Brief jobs, runnable tasks, captured inputs and the retained run fence in one transaction before splitting the worker.
- [x] Introduce the Atlas RabbitMQ outbox and durable dependency, cancellation and completion fences. Expansion to the remaining service pools is pending.
- [ ] Centralize provider quota and cooldown admission. Atlas daily collection and storage-backed NewsAPI tools use atomic shared reservations. Shared model/account RPM and cooldowns remain in scope.
Acceptance: retained pipeline fixture completes across processes; duplicate delivery cannot duplicate committed effects.

### Phase 5 — Venture, clients and Compose
Status: in implementation
- [ ] Convert terminal screens to API clients and expose the agreed CLI suite. The six clients expose command entrypoints, and Venture can attach the web stack terminal to the network Jobs view or service logs. Interactive parity for the other clients remains open.
- [x] Add Venture service reconciliation, configuration, visible workspace, health and logs for the managed disk and external Compose profiles.
- [ ] Add Compose baseline, optional disk SurrealDB profile, development overrides and multi-architecture image outputs.
- [ ] Configure API/web bind addresses, Host/origin policy and service reconnection.
Acceptance: Venture launches every client and web mode; client exit leaves services running; explicit stop respects service ownership.

### Phase 6 — Verification automation and repository reorganization
Status: in implementation
- [ ] Implement scope planner plus local/pr/full execution using shared scope definitions. Local planning and hosted scope selection use the same script. Unified execution modes are still open.
- [ ] Add dependency-aware selection, always-reporting aggregate check and phase timing artifacts.
- [ ] Introduce Linux ARM64 primary service lane, scoped web/TUI lanes and platform compatibility coverage.
- [ ] Reorganize into apps, services, shared crates/packages and deployment assets; rename product outputs.
- [ ] Remove Recon only after retained dependencies are extracted; retire embedded application storage after parity.
Acceptance: scope fixtures prove narrow changes stay narrow and shared changes widen correctly.

### Phase 7 — Documentation and planning reconciliation
Status: in implementation
- [ ] Execute the documentation/guidance inventory in acceptance.md; update all applicable maintained surfaces.
- [ ] Reconcile every existing plan: retain active work, promote lasting decisions, mark superseded direction, remove reviewed obsolete artifacts and repair links.
- [ ] Retire old SQL migration/cleanup instructions; retain live SurrealQL and useful dependency guards.
- [ ] Update paired diagrams, tags, indexes, onboarding, release instructions and agent skills/links.
Acceptance: maintained instructions consistently describe Fleetworks and scoped verification; historical evidence is not rewritten as current behavior.

### Phase 8 — Verification and handoff (separate from implementation)
Status: pending
- [ ] Freeze and run agreed affected checks plus full migration parity/recovery suite.
- [ ] Verify native CLI and container release targets, storage modes and actual packaged artifacts.
- [ ] Inspect UI/TUI captures and changed diagram pairs; check documentation links.
- [ ] Record timing comparison, failures, skips, evidence revision and remaining limitations.
Acceptance: no unrun test is presented as passed; independent findings are deferred.

## Ownership
Root agent owns integration, plan summaries and shared build cache. Delegate disjoint service/client/documentation work when useful. Workflow changes remain Code Architect-owned. Workers report evidence and never independently overwrite shared summaries.

## Blockers and decisions
- The previous unpublished integration commit is unavailable. Treat results from that tree as historical and verify the rebuilt source independently.
- Existing instructions still govern until their approved transition is implemented.
- Local memory mode is a host SurrealDB server, not embedded storage in each app.
- External SurrealDB must be disk-backed; persistence cannot be proven from endpoint URL alone.
- Remote storage only in v1; application services are locally orchestrated.
- Preserve Intel brief generation and shared functionality while removing Recon research.

## Errors Encountered
TinyFish CLI was unavailable during earlier research; used official RabbitMQ/Lapin pages through web fallback. GitHub connector omitted duration fields; read-only gh API supplied timestamps. No new execution failures during plan creation.

## Next Step
Verify both packaged Compose architectures on the selected hosted revision. Make retained brief publication replayable before splitting the brief worker. Continue extracting model and tool execution before retiring Recon. Advance terminal parity and release verification while keeping maintained guidance aligned.
