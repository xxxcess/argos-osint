# Argos documentation

Start here. Docs are Markdown under `docs/`. Open a figure HTML file only when you want the SVG page.

Deep investigation: [architecture.md](architecture.md). Keys and roles: [providers.md](providers.md).

## Product

| Doc | What it is |
| --- | --- |
| [../README.md](../README.md) | Product overview, quick start, doc index |
| [usage.md](usage.md) | TUI navigation, CLI, state directory |
| [architecture.md](architecture.md) | Recon turn, binder, picker, persistence, Atlas, news/legal |
| [providers.md](providers.md) | Model accounts (Defaults, OpenRouter, Google, Nvidia) and OSINT keys |
| [profile-analytics-dashboard.md](profile-analytics-dashboard.md) | Profile's 20 primary views, simultaneous dashboard, shared layout/components and viewport acceptance |
| [profile-dashboard-and-search.md](profile-dashboard-and-search.md) | Profile metric dictionary/history, portable configuration and named search engines |
| [concepts.md](concepts.md) | Glossary: apps vs IDs, roles, bindings, Intel jobs, schema |
| [conventions.md](conventions.md) | TUI, schema, tests, agent scratch, planning files |
| [diagrams.md](diagrams.md) | Editorial diagram language and figures |

## Figures

HTML + SVG under `docs/diagrams/`. Grammar: [cathrynlavery/diagram-design](https://github.com/cathrynlavery/diagram-design). Catalog: [diagrams.md](diagrams.md).

| Figure | Type | File |
| --- | --- | --- |
| Workspace | Architecture | [diagrams/workspace.html](diagrams/workspace.html) |
| TUI shell | Nested | [diagrams/tui-shell.html](diagrams/tui-shell.html) |
| Recon turn | Process | [diagrams/recon-turn.html](diagrams/recon-turn.html) |
| Bindings | Data flow | [diagrams/recon-bindings.html](diagrams/recon-bindings.html) |
| Tool picker | Flowchart | [diagrams/tool-picker.html](diagrams/tool-picker.html) |
| OSINT catalog | Nested | [diagrams/osint-providers.html](diagrams/osint-providers.html) |
| Atlas cycle | Process | [diagrams/atlas-pipeline.html](diagrams/atlas-pipeline.html) |
| Intel report | Data flow | [diagrams/intel-report.html](diagrams/intel-report.html) |
| Intel job states | State machine | [diagrams/intel-job-states.html](diagrams/intel-job-states.html) |
| Brain recall | Architecture | [diagrams/brain-recall.html](diagrams/brain-recall.html) |
| Models and roles | Architecture | [diagrams/model-roles.html](diagrams/model-roles.html) |
| Persistence | Layer stack | [diagrams/persistence.html](diagrams/persistence.html) |
| Report tables | Database schema | [diagrams/schema-core.html](diagrams/schema-core.html) |
| Durable jobs | State machine | [diagrams/jobs-lifecycle.html](diagrams/jobs-lifecycle.html) |
| Investigation harness | Process | [diagrams/investigation-harness.html](diagrams/investigation-harness.html) |
| Agent exploration | Process | [diagrams/agent-graphify.html](diagrams/agent-graphify.html) |

## Agent and workflow

| Doc | What it is |
| --- | --- |
| [../AGENTS.md](../AGENTS.md) | Commands, graphify, planning-with-files, scratch rules |
| [opencode-v2.md](opencode-v2.md) | Primary tool list, graph-first plugins, and ECC commands |
| [tui-verification.md](tui-verification.md) | Agent standard: shared geometry, capture tooling, screenshot/action/metric verification |
| [tui-verification-session-2026-10-10.md](tui-verification-session-2026-10-10.md) | Profile process, actual screenshot evidence, defects and results |
| [tui-overhaul-verification-2026-10-10.md](tui-overhaul-verification-2026-10-10.md) | Shared navigation, imagery, Configs and transcript verification |
| [ui-interaction-audit.md](ui-interaction-audit.md) | TUI surfaces, fields, focus, overflow |
| [investigation-harness.md](investigation-harness.md) | Unified investigation task graph |
| [jev-decision-gates.md](jev-decision-gates.md) | Tool-picker decisions transport |

## Checklists (historical)

Phase checklists under `docs/*-checklist.md` record completed Atlas/Brain/system-app work. Prefer architecture and conventions for current behavior.

TUI presentation: [design contract](tui-design-spec.md) · [component APIs and presets](tui-components.md).
