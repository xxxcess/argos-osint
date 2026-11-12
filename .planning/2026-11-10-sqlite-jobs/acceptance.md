# Embedded Jobs persistence checkpoint

Historical verification record. The user's 2026-11-12 fresh-deployment decision supersedes the rollout/drain prerequisites below; see the [current persistence contract](../../docs/wiki/code/jobs-persistence.md). Test names, results and limitations remain evidence for their original revision.

Validated source checkpoint: `7f579d7` on `feat/atlas-zero-shot`.

## Delivered boundary

- Jobs-owned SQLite operational primitives and standalone logs; Jobs-owned RocksDB delivery queue with authenticated HTTP access.
- Atlas delivery adapter, visibility leases, durable replay and Rabbit-free new Venture/Compose startup. Legacy ownership remains readable for shutdown.
- Planned ownership, migration scopes and six reviewed diagram pairs in the [canonical persistence contract](../../docs/wiki/code/jobs-persistence.md).

This checkpoint does **not** complete SQLite scheduling authority. Existing scheduling, attempts, process recovery, telemetry/Profile reads and domain publication transactions still require the coordinated migration documented there. No physical I/O isolation, throughput result or replicated availability is claimed.

## Executed verification

| Check | Result |
| --- | --- |
| `agent_cargo.py test --locked -p fleetworks-jobs` | 17 passed, including crash/reopen, stale leases, replay and concurrent claims |
| Host `jobs_service::tests`, `server,nlp,classifier` features | 2 passed: authenticated SQLite and persistent queue HTTP |
| Required pinned-server `storage_jobs::embedded_queue_transport_requires_legacy_tasks_to_drain` | 1 passed against isolated SurrealDB 3.3.4 |
| `agent_cargo.py test --locked -p fleetworks-venture` | 12 passed |
| `fleetworks_atlas_smoke.py` with newly built host | Passed authentication, queued cancellation, durable enqueue, completion replay and retry across worker restart |
| Scope planner / candidate image unit tests | 12 / 5 passed |
| Documentation references | 1,402 checked, none broken |
| Diagram QA | Six inline/sibling pairs matched; XML/IDs, geometry budgets, browser overflow and visual inspection passed |
| Resolved Atlas Compose | No Rabbit service/config; Jobs-only persistent mount and healthy startup edges passed |
| Formatting, workflow syntax, dependency policy and shared version | Passed |

All Rust commands used `scripts/agent_cargo.py`, the shared `target/agents` cache and the pinned native toolchain. Host/server checks reused the task-built RocksDB static archive after confirming identical native version/features (`bindgen-runtime`, `static`), avoiding duplicate C++ compilation. Fixtures owned temporary directories, synthetic credentials and loopback ports; they were cleaned up. The user's stack and `target/debug` were untouched.

Initial verification found and fixed SQLite parameter/lifetime issues, native Clang loading, an exhaustive legacy-ownership match and a smoke assertion expecting the framework's default status instead of the service's explicit 400 contract. A later storage regression strengthened cutover to require both task and parent-job completion.

## Limits

Docker images were not rebuilt or deployed. Full embedded compatibility, hosted platform matrices, load/latency benchmarks and fault injection across the future domain/SQLite publication handoff were not run. These focused passes do not establish complete job-write isolation. Existing jobs must drain before activation of a new backend; no live lease migration or production data conversion occurred.
