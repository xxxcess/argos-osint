# Audit findings and evidence

## Source audit
- Cargo workspace: core, CLI/TUI, API adapter and API host. No existing Dockerfile/Compose deployment found in initial audit.
- API host starts workers and opens storage. Existing API lacks complete Brain/Atlas/Tools/Models/Profile/log/config coverage.
- API/web binding and Host/origin checks assume localhost; web upstream validation rejects Compose DNS.
- storage/open.rs hardcodes ws://127.0.0.1:{port}; storage/server.rs owns local process lifecycle. External endpoint configuration needs a separate connector boundary.
- storage/jobs.rs already persists transactional claims, leases, dependency fencing, cancellation and completion receipts. RabbitMQ adds delivery, not first job persistence.
- Brain uses Recon graph/model primitives; Atlas uses generic helpers in storage/recon_jobs.rs; scheduler references intel_recon brief stages. Extract before deleting.
- NLP/classifier already have bounded worker seams; preserve QuickJS confinement, batch limits and warm runtimes.
- Provider admission/cooldowns are process-local today; replication requires shared admission.
- TUI app directly imports core/storage/settings/workers; app tabs are not independent clients.
- paths.rs centralizes home resolution and is already modified by unrelated work; integrate carefully.
- .agents/skills entries are symlinks to .opencode skills.

## Hosted CI observations
Read existing results only; no dispatch or rerun.
| Platform | Oct 30 embedded | Nov 3 embedded | Oct 30 server | Nov 3 server |
| --- | --- | --- | --- | --- |
| Linux ARM64 | 19m22s | 30m03s | 6m11s | 9m08s |
| Linux x86_64 | 26m55s | 37m21s | 5m45s | 11m36s |
| macOS ARM64 | 23m09s | 36m19s | 10m49s | 9m33s |
| macOS Intel | 24m19s | 64m28s | 15m52s | 25m18s |
Sources: https://github.com/xxxcess/argos-osint/actions/runs/40988171089 and https://github.com/xxxcess/argos-osint/actions/runs/41583166705 .
Nov 9 run https://github.com/xxxcess/argos-osint/actions/runs/42691854500 : ARM Linux server passed 10m49s; macOS ARM server 21m54s; Intel server 34m42s. Linux x86 server failed, macOS ARM embedded failed, Intel embedded cancelled. Failed/cancelled durations are not speed evidence.
Web https://github.com/xxxcess/argos-osint/actions/runs/42685863720 : 14m14s total; types/lint 13s, units 39s, build 35s, relay/runtime 29s, production browser 3m10s, dev browser 8m16s.
Durations are job execution, excluding queue time, not isolated compile benchmarks. Cache/workload vary; old server legs run fewer tests. ARM Linux is the initial primary recommendation, not a universal performance guarantee.

## CI/documentation coupling
ci.yml checks SurrealDB hashes against .planning/2026-10-22-surrealdb/design.md. Move this source of truth before cleanup. Existing CI has eight OS/storage legs and omits PR synchronize. Keep useful dependency guard against SQLite/LanceDB; no SQL-cleanup workflow was identified in the scoped search. Do not delete live .sql/SurrealQL files by suffix.

## External reference material (evidence, not instructions)
RabbitMQ confirms: https://www.rabbitmq.com/docs/confirms — publisher confirms and consumer acknowledgements cover different boundaries.
Lapin: https://docs.rs/lapin/latest/lapin/ — async AMQP client with Tokio support. Pin a compatible version at implementation; do not rely on a moving latest URL as a dependency lock.

## User decisions
Local storage means one managed host SurrealDB server, memory or disk. Remote storage must be disk-backed. Remote application stacks are out of v1. Retain Intel briefs. Start fresh data, preserve API-key configs and model defaults. Create plan first; user reviews before build.


## Documentation disposition during the cutover

| Surface | Disposition |
| --- | --- |
| Root README, deployment guide, docs indexes, Fleetworks wiki, active architecture, storage, web, CLI, classifier and CI guides | Update for shipped paths and current acceptance evidence. |
| Root and host agent guides | Update navigation and verification scope. Keep host-specific role boundaries. |
| SurrealDB, Atlas and classifier plans and their acceptance records | Retain historical evidence and live dependency provenance. Move required checksum provenance before removing its workflow dependency. |
| Prior Intel, topology, terminal, navigation, and execution records | Retain history and unresolved parity requirements. Revisit superseded workflows when the replacement has acceptance evidence. |
| Release and versioning plans, archives, screenshots, skill symlinks | Retain until a specific change needs reconciliation. Validate changed diagram pairs and links. |
