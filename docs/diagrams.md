# Diagram conventions

Argos figures follow the editorial grammar in [cathrynlavery/diagram-design](https://github.com/cathrynlavery/diagram-design): self-contained HTML with inline SVG. No Mermaid. No shadows. No generic rounded-box grids.

Open a file under `docs/diagrams/` in a browser. Do not rasterize unless someone asks for PNG/SVG export.

## When to draw

Draw when a reader learns more from a picture than from a paragraph or table. Skip one-shape “diagrams” and lists of names.

Pick a **semantic pattern** (queue, lifecycle, trust boundary) then a **visual type**. For this repo the usual types are:

| If you need… | Type | Example |
| --- | --- | --- |
| Components and connections | Architecture | [workspace.html](diagrams/workspace.html) |
| Ordered stages with handoff | Process | [diagrams/recon-turn.html](diagrams/recon-turn.html) |
| Who does what on a pipeline | Data flow | [diagrams/intel-report.html](diagrams/intel-report.html) |
| States and guards | State machine | Intel job `queued` → `running` → terminal |
| Tables and FKs | Database schema | `intel_report_jobs` / `intel_report_attempts` |

## Density

- At most 9 nodes and 12 arrows per figure. Split overview and detail if you exceed that.
- Accent (`#eb6c36`) on **1–2** focal nodes. Everything else is ink or muted.
- Orthogonal connectors only (right-angle elbows). Draw arrows before boxes.
- 4px grid. Node radius 6. No `box-shadow`. No JetBrains Mono.
- Names in a sans (Geist). Ports, URLs, ids in mono. Title in Instrument Serif.

## Accessibility

Each `<svg>` has `role="img"`, `aria-labelledby` pointing at a unique `<title>` and `<desc>` (`<slug>-title`, `<slug>-desc`). Title is the first child of the SVG.

## Tokens (light)

| Role | Hex | Use |
| --- | --- | --- |
| paper | `#f5f5f5` | Page |
| ink | `#2d3142` | Text and primary stroke |
| muted | `#4f5d75` | Default arrows |
| accent | `#eb6c36` | Focal path |
| link | `#2e5aa8` | External HTTP |

These match diagram-design’s default skin so Argos docs stay readable without a brand onboarding pass.

## Agent rule

When adding or rewriting a concept diagram, follow this file and the type table in the upstream skill. Put new figures in `docs/diagrams/` and link them from [README.md](README.md) and the matching architecture section. Do not drop Mermaid fences into `docs/`.
