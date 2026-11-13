# Diagram conventions

Fleetworks figures follow the editorial grammar in [cathrynlavery/diagram-design](https://github.com/cathrynlavery/diagram-design): self-contained HTML with inline SVG. No Mermaid. No shadows. No generic rounded-box grids.

Open a file under `docs/diagrams/` in a browser for the HTML page. Matching `.svg` files embed in Markdown. Do not rasterize unless someone asks for PNG.

## Catalog

| Aspect | Type | HTML | SVG |
| --- | --- | --- | --- |
| Backend NLP preprocessing | Data flow | [nlp-preprocessing.html](diagrams/nlp-preprocessing.html) | [nlp-preprocessing.svg](diagrams/nlp-preprocessing.svg) |
| Fleetworks service boundaries | Architecture | [fleetworks.html](diagrams/fleetworks.html) | [fleetworks.svg](diagrams/fleetworks.svg) |
| Native navigation and selected content | Layout | [native-workstation-layout.html](diagrams/native-workstation-layout.html) | [native-workstation-layout.svg](diagrams/native-workstation-layout.svg) |
| Native identity and shared backend | Architecture | [workspace.html](diagrams/workspace.html) | [workspace.svg](diagrams/workspace.svg) |
| Backend resources and workstation origin | Architecture | [workspace-scope.html](diagrams/workspace-scope.html) | [workspace-scope.svg](diagrams/workspace-scope.svg) |
| Protected workstation setup and recovery | Process | [workspace-lifecycle.html](diagrams/workspace-lifecycle.html) | [workspace-lifecycle.svg](diagrams/workspace-lifecycle.svg) |
| Native connection and registered browser handoff | Process | [native-connections.html](diagrams/native-connections.html) | [native-connections.svg](diagrams/native-connections.svg) |
| Partial readiness and explicit maintenance | Data flow | [workstation-readiness.html](diagrams/workstation-readiness.html) | [workstation-readiness.svg](diagrams/workstation-readiness.svg) |
| Workstation registry and active bindings | Architecture | [connect-registry.html](diagrams/connect-registry.html) | [connect-registry.svg](diagrams/connect-registry.svg) |
| Intel Connect authority and central gate | Process | [intel-connect-gate.html](diagrams/intel-connect-gate.html) | [intel-connect-gate.svg](diagrams/intel-connect-gate.svg) |
| Durable request origin and scoped activity | Data flow | [brief-job-context.html](diagrams/brief-job-context.html) | [brief-job-context.svg](diagrams/brief-job-context.svg) |
| Intel web request and readiness boundaries | Process | [intel-web.html](diagrams/intel-web.html) | [intel-web.svg](diagrams/intel-web.svg) |
| Intel Focus Brief reading order | Layout | [intel-focus-layout.html](diagrams/intel-focus-layout.html) | [intel-focus-layout.svg](diagrams/intel-focus-layout.svg) |
| OSINT catalog | Nested | [osint-providers.html](diagrams/osint-providers.html) | [osint-providers.svg](diagrams/osint-providers.svg) |
| Atlas collection, admission, and derivatives | Process | [atlas-pipeline.html](diagrams/atlas-pipeline.html) | [atlas-pipeline.svg](diagrams/atlas-pipeline.svg) |
| Pinned Rust-BERT classification | Data flow | [classifier-runtime.html](diagrams/classifier-runtime.html) | [classifier-runtime.svg](diagrams/classifier-runtime.svg) |
| Private Brain read boundaries | Data flow | [brain-browse.html](diagrams/brain-browse.html) | [brain-browse.svg](diagrams/brain-browse.svg) |
| Brain source lifecycle | Data flow | [brain-source-lifecycle.html](diagrams/brain-source-lifecycle.html) | [brain-source-lifecycle.svg](diagrams/brain-source-lifecycle.svg) |
| Actor catalog | Process | [actor-catalog.html](diagrams/actor-catalog.html) | [actor-catalog.svg](diagrams/actor-catalog.svg) |
| Private actor projection and canonical discovery | Data flow | [actor-search.html](diagrams/actor-search.html) | [actor-search.svg](diagrams/actor-search.svg) |
| Actor identity maintenance | Process | [actor-identity.html](diagrams/actor-identity.html) | [actor-identity.svg](diagrams/actor-identity.svg) |
| Brain topology and neighborhood summaries | Architecture | [brain-topology.html](diagrams/brain-topology.html) | [brain-topology.svg](diagrams/brain-topology.svg) |
| Topology neighborhood summary states | State machine | [topology-summary-states.html](diagrams/topology-summary-states.html) | [topology-summary-states.svg](diagrams/topology-summary-states.svg) |
| Intel brief stage states | State machine | [intel-job-states.html](diagrams/intel-job-states.html) | [intel-job-states.svg](diagrams/intel-job-states.svg) |
| Intel Focus Brief stages and retained publication | Process | [intel-brief-chain.html](diagrams/intel-brief-chain.html) | [intel-brief-chain.svg](diagrams/intel-brief-chain.svg) |
| Brain recall | Architecture | [brain-recall.html](diagrams/brain-recall.html) | [brain-recall.svg](diagrams/brain-recall.svg) |
| Model routing | Data flow | [model-roles.html](diagrams/model-roles.html) | [model-roles.svg](diagrams/model-roles.svg) |
| Provider request scheduling | Data flow | [model-request-scheduling.html](diagrams/model-request-scheduling.html) | [model-request-scheduling.svg](diagrams/model-request-scheduling.svg) |
| Planned Jobs persistence and embedded delivery | Data flow | [jobs-persistence.html](diagrams/jobs-persistence.html) | [jobs-persistence.svg](diagrams/jobs-persistence.svg) |
| Network storage selections | Architecture | [persistence.html](diagrams/persistence.html) | [persistence.svg](diagrams/persistence.svg) |
| Durable jobs and terminal operations | State machine | [jobs-lifecycle.html](diagrams/jobs-lifecycle.html) | [jobs-lifecycle.svg](diagrams/jobs-lifecycle.svg) |
| Fleetworks documentation routes | Hierarchy | [wiki-navigation.html](diagrams/wiki-navigation.html) | [wiki-navigation.svg](diagrams/wiki-navigation.svg) |
| Development iteration loops | Process | [dev-loops.html](diagrams/dev-loops.html) | [dev-loops.svg](diagrams/dev-loops.svg) |
| Build tools, artifacts and persistent-data boundaries | Artifact map | [build-artifacts.html](diagrams/build-artifacts.html) | [build-artifacts.svg](diagrams/build-artifacts.svg) |
| Native app development loop | Process | [dev-loop-native.html](diagrams/dev-loop-native.html) | [dev-loop-native.svg](diagrams/dev-loop-native.svg) |
| Runtime service image development loop | Process | [dev-loop-runtime.html](diagrams/dev-loop-runtime.html) | [dev-loop-runtime.svg](diagrams/dev-loop-runtime.svg) |
| Web development container and browser loop | Data flow | [dev-loop-web.html](diagrams/dev-loop-web.html) | [dev-loop-web.svg](diagrams/dev-loop-web.svg) |
| Dockerfile targets and packaged image loop | Architecture | [dev-loop-images.html](diagrams/dev-loop-images.html) | [dev-loop-images.svg](diagrams/dev-loop-images.svg) |
| Fleetworks development scopes | Data flow | [ci-scopes.html](diagrams/ci-scopes.html) | [ci-scopes.svg](diagrams/ci-scopes.svg) |
| Native review artifacts and trusted promotion gates | Process | [native-release.html](diagrams/native-release.html) | [native-release.svg](diagrams/native-release.svg) |
| Native Runtime, capture and separate acceptance | Data flow | [native-verification.html](diagrams/native-verification.html) | [native-verification.svg](diagrams/native-verification.svg) |
| Agent handoffs | Process | [agent-team.html](diagrams/agent-team.html) | [agent-team.svg](diagrams/agent-team.svg) |

The catalog covers current Fleetworks contracts and explicitly labeled target architectures. The [Jobs persistence plan](wiki/code/jobs-persistence.md) distinguishes implemented foundations from the pending authority cutover. Earlier design figures and dated verification evidence are collected in [the archive](archive/index.md).

## Historical figures

These CLI-only workflows were retired on 2026-11-12. Their diagrams preserve the former architecture; current replacements are linked below and the [retirement record](archive/cli-retirement.md) records disposition.

| Historical figure | HTML | SVG | Current replacement |
| --- | --- | --- | --- |
| Retired Venture lifecycle | [venture-navigation.html](diagrams/venture-navigation.html) | [venture-navigation.svg](diagrams/venture-navigation.svg) | [Native connections](diagrams/native-connections.html) and [Docker image loop](diagrams/dev-loop-images.html) |
| Retired CLI development loop | [dev-loop-cli.html](diagrams/dev-loop-cli.html) | [dev-loop-cli.svg](diagrams/dev-loop-cli.svg) | [Native app development loop](diagrams/dev-loop-native.html) |

## When to draw

Draw when a reader learns more from a picture than from a paragraph or table. Skip one-shape “diagrams” and lists of names.

Pick a **semantic pattern** then a **visual type**. Accent (`#eb6c36`) on **1–2** focal nodes. At most 9 nodes and 12 arrows. Orthogonal connectors. 4px grid. Node radius 6. No `box-shadow`. No JetBrains Mono. Names in Geist sans; ports and ids in Geist Mono; titles in Instrument Serif.

Each `<svg>` has `role="img"` and unique `<title>` / `<desc>` ids (`<slug>-title`, `<slug>-desc`).

## Agent rule

New or significant architectural/conceptual changes follow
[the documentation policy](wiki/code/conventions.md#architecture-and-concepts): update affected
figures in the same change as code and prose. Check final ownership, flow, states,
labels and failure/retry paths against the implementation. Keep HTML/inline SVG and
sibling SVG consistent; update catalog/index/prose links for added, replaced or
retired figures. An index-only refresh does not replace these maintained figures.

Follow this file and [cathrynlavery/diagram-design](https://github.com/cathrynlavery/diagram-design). Put new figures in `docs/diagrams/` as HTML plus a sibling SVG, then link them from this catalog and the matching wiki page. Do not drop Mermaid fences into `docs/`.
