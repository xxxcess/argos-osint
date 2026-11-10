# Fleetworks documentation dispositions

This review covers the tracked plans, archive, maintained contributor guidance and host roles in the Fleetworks cutover checkout. It records where to find each live contract and preserves revision-bound evidence. The [task plan](task_plan.md) owns phase completion.

## Maintained surfaces

| Surface | Disposition |
| --- | --- |
| Root README, docs and wiki indexes, CLI and deployment guides | Use Fleetworks and Venture for current onboarding. Historical Intel launcher and research material lives in the [archive](../../docs/archive/index.md). Keep compatibility package names and flags that still have consumers. |
| Canonical execution, contributing, CI and planning guides | Use the shared scope planner and affected local checks for Fleetworks. Keep the embedded gate for affected compatibility paths, full platform and server matrices in hosted CI, and mutation hosted. See [contributing](../../docs/wiki/code/contributing.md), [CI](../../docs/wiki/code/ci.md) and [planning](../../docs/wiki/code/planning-artifacts.md). |
| Current architecture, storage, API, classifier, providers, React and TUI contracts | Keep the current network paths canonical. Keep affected compatibility UI contracts and evidence discoverable while the existing `argos` executable and its tests still consume them. Keep runtime and UI gaps visible in the task plan. |
| `.opencode/` quick guide, role prompts, commands and skills | Keep the catalog, policy tests and Code Architect boundary. Use the canonical contributor guide for verification. Older role or skill wording that says all local Rust must be embedded still needs reconciliation when the host guidance is updated. Keep the compatibility TUI workflow until the supported network client paths meet parity. |
| `.agents/skills` | Keep the four tracked symlinks for React patterns, React UI, Rust style and TUI verification. All resolve to the canonical `.opencode/skills` entries. |
| `.agy/` how-to and lite/heavy role docs | Keep host role and team distinctions. The how-to already sends Fleetworks and compatibility verification to the relevant scoped paths. |
| Active diagrams, tags, indexes and archived figure pairs | Keep the reviewed Fleetworks HTML/SVG pairs active and keep original archived pairs with their historical references. Use the [diagram catalog](../../docs/diagrams.md) for current navigation. |
| Schema and release verification | Use the [SurrealDB release manifest](../../scripts/surrealdb-release.json) and current [storage authority](../../docs/wiki/code/storage.md#schema-and-runtime-authority). Keep the SQLite/LanceDB dependency guard in `ci.yml`. The audit found no tracked SQL-cleanup workflow or active instruction to run old migrations. Keep current `storage/schema/*.surql`. `schema_telemetry.sql` is still compiled via `store.rs`, so an extension-based deletion would break the build. Review the other legacy SQL files separately before removal. |

## Prior tracked plans

| Plan | Disposition |
| --- | --- |
| [2026-10-11 semantic versioning](../2026-10-11-semantic-versioning/acceptance.md) | Keep the historical setup acceptance. Follow [versioning](../../docs/wiki/code/versioning.md) for current release policy. |
| [2026-10-12 insight reliability](../2026-10-12-insight-reliability/acceptance.md) | Keep revision-bound extraction, retry and publication evidence and its limitations. Use the current storage and Intel contracts for retained work. |
| [2026-10-15 topology mapper](../2026-10-15-topology-mapper/acceptance.md) and its checklist and handoff | Keep the reviewed UI and evidence with the original runtime. Use the [topology contract](../../docs/wiki/concepts/topology-mapper.md) for current behavior. |
| [2026-10-22 SurrealDB migration](../2026-10-22-surrealdb/acceptance.md) and its design and phase 1 design | Keep all three migration records and their unchecked items. Historical SQLite dependencies and server checksum notes no longer define the current runtime or release manifest. Keep current datastore contracts and targeted migration evidence. |
| [2026-10-25 Intel web CI](../2026-10-25-argos-intel/handoff.md) | Keep the approved workflow handoff and exact implementation evidence. Current workflow commands come from the CI guide. |
| [2026-10-26 async navigation and jobs](../2026-10-26-async-navigation-jobs/design.md) | Keep the design record. Use current Intel routes and scoped verification. Design text does not establish a test result. |
| [2026-10-27 async execution audit](../2026-10-27-async-execution-audit/handoff.md) | Keep completed audit findings and open items. Its former Recon file locations and team workflow describe that revision. Carry forward still-relevant cancellation, lease and shutdown constraints. |
| [2026-10-31 Intel Nexus Desk](../2026-10-31-intel-nexus-desk-overhaul/design.md) and acceptance | Keep the implemented design and local evidence, including the unverified visual and performance bounds. Use maintained React and Intel web contracts for current behavior. |
| [2026-11-05 Atlas performance completion](../2026-11-05-atlas-performance-completion/acceptance.md) | Keep synthetic classifier and benchmark evidence tied to its revision and workloads. Live Atlas uses the separate Rust-BERT contract. Keep the original full-gate exit marked unknown. |
| [2026-11-05 TUI log isolation](../2026-11-05-tui-log-isolation/acceptance.md) | Keep the targeted compatibility-terminal evidence and screenshots. The new native clients have their own lifecycle checks. |
| [2026-11-06 classifier quality v2](../2026-11-06-classifier-quality-v2/acceptance.md) | Keep the interrupted verification, failed cancellation result and pending cleanup evidence. Use current classifier and derivative contracts. Preserve the distinction between synthetic results and live classifier quality. |

The archive index retains 22 historical Markdown records and seven HTML/SVG pairs. Keep the brief extraction and provider-routing checklists, Atlas memory checklist, Intel web plan, investigation harness, model routing records, Profile charts, topology design, unified pipeline checklist and dated UI verification records. Their linked screenshots and commands document the original work. Current setup belongs in the Fleetworks and Intel guides. No older records were removed in this review.

## Recon retirement audit

Venture launches the six network clients. The Compose roles, public API and private Brain router expose the retained article, brief, actor, topology, Jobs, Models and Tools operations. They expose no command or route for starting a Recon turn. The generic Jobs retry supports registered index and summary tasks and explicit insight extraction recovery. Keep those retained behaviors separate from new investigation submission.

At `f7d810a` the compatibility `argos` binary refuses `recon new`, `ask`, `ask-new`, `retry`, `resume` and `retry-insights`. The terminal composer and resume action return the same refusal. Historical `list` and `show` commands remain. `argos stack up --tui` can still open the compatibility terminal, and that terminal can still display saved threads. It cannot start or resume a research turn. The public API and private Brain router still expose no research-submission route.

Core still imports Recon-owned graph shapes in Brain browse, the backend compatibility selector in storage and NLP, investigation rule bindings in OSINT, and read/storage types in retained insight, thread and historical report code. The shared model executor is already outside Recon. Keep old persisted types and read paths where required for Jobs, actor graph, insights or reports. The `intel_recon` module also owns the retained Focus Brief backend, so its name alone does not identify retired functionality.

Embedded application storage remains in the compatibility build until the full embedded parity suite passes. Verify legacy insight and report reads before deleting shared tables or types.
