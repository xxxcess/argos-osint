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
- [x] Inventory planning, archives and agent surfaces and record retention decisions. See [dispositions](decisions.md).
- [x] Update canonical execution and testing guidance for scoped Fleetworks checks and compatibility coverage.
- [x] Establish lightweight versioned HTTP contracts, clients and workspace configuration. Service ownership continues in Phase 4.
- [x] Move the legacy SurrealDB CI archive pins to a maintained release manifest with an offline validation fixture.
Acceptance: no cleanup destroys a runtime/CI dependency; guidance and scope selection have a single source of truth.

### Phase 3 — Workspace, storage and shared primitives
Status: in implementation
- [x] Implement workspace resolution, configuration/secrets loading and explicit API-key/model-default import.
- [x] Implement managed host memory/disk SurrealDB and external disk-server configuration, authentication and TLS.
- [x] Move shared model execution out of Recon and update retained consumers to use the shared module.
- [x] Extract shared Brain graph types from Recon and move retained storage selection to the common runtime.
- [ ] Remove remaining retained job, tool-binding and historical record dependencies from Recon.
- [x] Split the default Venture and terminal client build graph from RocksDB, ONNX and QuickJS.
Acceptance: all three storage selections work through a network endpoint; original .argos state remains untouched.

### Phase 4 — Services and durable execution
Status: in implementation
- [x] Separate API serving, schema bootstrap and background workers. Dedicated domain entrypoints and their actions continue below.
- [x] Wire private Atlas, NLP, classifier, Brain and Tools roles, with brief execution in its own worker.
- [x] Separate private Control configuration and Jobs operations from the gateway.
- [ ] Move retained provider transport into the dedicated model execution service.
- [x] Admit Focus Brief jobs, runnable tasks, captured inputs and the retained run fence in one transaction before splitting the worker.
- [x] Commit each brief stage output, metadata, task state and final retained run fence together.
- [x] Persist and replay brief display, Brain and index publication from completed stages.
- [x] Introduce the Atlas RabbitMQ outbox and durable dependency, cancellation and completion fences. Expansion to the remaining service pools is pending.
- [x] Centralize daily news quota reservations, model provider pacing and configured model account admission across network workers.
Acceptance: retained pipeline fixture completes across processes; duplicate delivery cannot duplicate committed effects.

### Phase 5 — Venture, clients and Compose
Status: in implementation
- [x] Expose interactive and one-shot network modes for all six terminal clients. Venture can attach the web stack terminal to Jobs or service logs.
- [ ] Verify full retained interactive parity, including actor discovery and configuration edge cases.
- [x] Add Venture service reconciliation, configuration, visible workspace, health and logs for the managed disk and external Compose profiles.
- [x] Add the Compose baseline, disk profile, development override and no-build startup for preloaded images.
- [ ] Complete hosted candidate image verification and registry release acceptance.
- [ ] Configure API/web bind addresses, Host/origin policy and service reconnection.
Acceptance: Venture launches every client and web mode; client exit leaves services running; explicit stop respects service ownership.

### Phase 6 — Verification automation and repository reorganization
Status: in implementation
- [x] Implement plan, local, pr and full modes around the shared scope planner and explicit hosted dispatch.
- [x] Add dependency-aware selection, a required aggregate check and phase timing artifacts.
- [x] Add ARM64 primary service verification, scoped web and native lanes, and a scheduled full workflow.
- [x] Reorganize active entrypoints into apps and services, shared crates/packages and deployment assets. Name the Venture and Fleetworks product binaries.
- [ ] Remove Recon only after retained dependencies are extracted; retire embedded application storage after parity.
Acceptance: scope fixtures prove narrow changes stay narrow and shared changes widen correctly.

### Phase 7 — Documentation and planning reconciliation
Status: in implementation
- [x] Rebrand active product documentation as Fleetworks and replace old launcher instructions with Venture setup.
- [x] Reconcile current contributor guidance and record older plan and archive dispositions.
- [x] Audit existing plans and archive references. Preserve live compatibility contracts and historical records.
- [x] Audit old SQL and migration references. Keep the active SurrealQL, compile-time dependencies and dependency guards.
- [x] Replace stale active diagram pairs, remove the old CI matrix and archive historical figures. Update current tags, indexes, onboarding and release guidance.
Acceptance: maintained instructions consistently describe Fleetworks and scoped verification; historical evidence is not rewritten as current behavior.

### Phase 8 — Verification and handoff (separate from implementation)
Status: pending
- [ ] Freeze and run agreed affected checks plus full migration parity/recovery suite.
- [x] Verify Venture-owned packaged Compose from source on native Linux AMD64 and ARM64.
- [ ] Verify the new hosted CLI and image candidate workflows, published registry artifacts and the remaining storage/recovery matrix.
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
Review the exact candidate and scoped workflow results. Finish the dedicated model service and retained dependency extraction before retiring the compatibility Recon entrypoints.
