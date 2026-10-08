# Diagram conventions

Argos figures follow the editorial grammar in [cathrynlavery/diagram-design](https://github.com/cathrynlavery/diagram-design): self-contained HTML with inline SVG. No Mermaid. No shadows. No generic rounded-box grids.

Open a file under `docs/diagrams/` in a browser. Matching `.svg` files exist for Markdown embeds (GitHub README). Do not rasterize unless someone asks for PNG export.

## Catalog

| Aspect | Type | HTML | SVG |
| --- | --- | --- | --- |
| Workspace | Architecture | [workspace.html](diagrams/workspace.html) | [workspace.svg](diagrams/workspace.svg) |
| TUI shell | Nested | [tui-shell.html](diagrams/tui-shell.html) | [tui-shell.svg](diagrams/tui-shell.svg) |
| Recon turn | Process | [recon-turn.html](diagrams/recon-turn.html) | [recon-turn.svg](diagrams/recon-turn.svg) |
| Bindings | Data flow | [recon-bindings.html](diagrams/recon-bindings.html) | [recon-bindings.svg](diagrams/recon-bindings.svg) |
| Tool picker | Flowchart | [tool-picker.html](diagrams/tool-picker.html) | [tool-picker.svg](diagrams/tool-picker.svg) |
| OSINT catalog | Nested | [osint-providers.html](diagrams/osint-providers.html) | [osint-providers.svg](diagrams/osint-providers.svg) |
| Atlas cycle | Process | [atlas-pipeline.html](diagrams/atlas-pipeline.html) | [atlas-pipeline.svg](diagrams/atlas-pipeline.svg) |
| Intel report | Data flow | [intel-report.html](diagrams/intel-report.html) | [intel-report.svg](diagrams/intel-report.svg) |
| Intel job states | State machine | [intel-job-states.html](diagrams/intel-job-states.html) | [intel-job-states.svg](diagrams/intel-job-states.svg) |
| Brain recall | Architecture | [brain-recall.html](diagrams/brain-recall.html) | [brain-recall.svg](diagrams/brain-recall.svg) |
| Models and roles | Architecture | [model-roles.html](diagrams/model-roles.html) | [model-roles.svg](diagrams/model-roles.svg) |
| Persistence | Layer stack | [persistence.html](diagrams/persistence.html) | [persistence.svg](diagrams/persistence.svg) |
| Report tables | Database schema | [schema-core.html](diagrams/schema-core.html) | [schema-core.svg](diagrams/schema-core.svg) |
| Durable jobs | State machine | [jobs-lifecycle.html](diagrams/jobs-lifecycle.html) | [jobs-lifecycle.svg](diagrams/jobs-lifecycle.svg) |
| Investigation harness | Process | [investigation-harness.html](diagrams/investigation-harness.html) | [investigation-harness.svg](diagrams/investigation-harness.svg) |
| Agent exploration | Process | [agent-graphify.html](diagrams/agent-graphify.html) | [agent-graphify.svg](diagrams/agent-graphify.svg) |

## When to draw

Draw when a reader learns more from a picture than from a paragraph or table. Skip one-shape “diagrams” and lists of names.

Pick a **semantic pattern** then a **visual type**. Accent (`#eb6c36`) on **1–2** focal nodes. At most 9 nodes and 12 arrows. Orthogonal connectors. 4px grid. Node radius 6. No `box-shadow`. No JetBrains Mono. Names in Geist sans; ports and ids in Geist Mono; titles in Instrument Serif.

Each `<svg>` has `role="img"` and unique `<title>` / `<desc>` ids (`<slug>-title`, `<slug>-desc`).

## Agent rule

Follow this file and [cathrynlavery/diagram-design](https://github.com/cathrynlavery/diagram-design). Put new figures in `docs/diagrams/` as HTML plus a sibling SVG, then link them from [README.md](README.md) and the matching prose doc. Do not drop Mermaid fences into `docs/`.
