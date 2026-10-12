# Concepts

Short names used across the TUI, CLI, and store. Source of truth for investigation mechanics is [architecture.md](architecture.md).

## Apps and internal IDs

Home launches nine surfaces. Labels on screen differ from `ModuleId` values; keep the IDs unless a migration requires otherwise.

[![Argos workspace](diagrams/workspace.svg)](diagrams/workspace.html)

| Label | `ModuleId` | Kind |
| --- | --- | --- |
| Intel | `Intel` | Application |
| Atlas | `Atlas` | Application |
| Brain | `Brain` | Application |
| Recon | `Recon` | Application |
| Jobs | `Jobs` | System |
| Logs | `Logs` | System |
| Tools | `Osint` | System |
| Models | `Providers` | System |
| Profile | `System` | System |

Only Recon has a composer. Intel Briefing starts a **report job** on an Atlas article; that is not a Recon investigation thread.

## Model roles

Each role is provider + model plus an ordered `fallbacks` list in `config.toml`. Accounts live in `auth.json`. Saving a key does not change a role.

[![Models and roles](diagrams/model-roles.svg)](diagrams/model-roles.html)

| Role | Job |
| --- | --- |
| Recon | Directives and bindings |
| Tool picker | One next tool per request (Jev decisions or chat JSON) |
| Synthesis | Cited answer |
| Classifier | Recon report mode from the prompt |
| Summarization | Graph/path summaries |
| Evidence curator, Entity resolver, Claim assessor, Investigation controller | Investigation harness |

Legacy Grok/OpenAI accounts remain until the user assigns a replacement. New setup tabs are Defaults, OpenRouter, Google, Nvidia.

## Primary OSINT providers

Firecrawl, SociaVault, and Hunter are primary. Everything else is a gap-filler. Hunter inputs come only from the prompt, Firecrawl, SociaVault, or earlier Hunter calls.

[![OSINT catalog](diagrams/osint-providers.svg)](diagrams/osint-providers.html)

## Bindings

A binding is a typed value (`domain`, `handle`, `url`, …) with an evidence id. `recon/investigation/tool_io.rs` maps every catalog input to a kind, a prompt extractor, a rule extractor, and producers. Ungrounded inputs skip the step.

[![Binding and grounding](diagrams/recon-bindings.svg)](diagrams/recon-bindings.html)

## Intel report jobs

Durable job on one Atlas article (`intel_report_jobs`). Dispatches are rows in `intel_report_attempts` (created on open at schema 24 if missing).

| UI | Meaning |
| --- | --- |
| `Tools: N calls used · B budget` | N = Argos dispatches including errors. Cache reuse and validation skips are not calls. |
| Summary | Selected revision `bluf` |
| View full report | 70%-width card of all sections |

Busy work on this article hides Summary and the launch control. Another article’s job does not.

## Persistence

- SQLite `~/.argos/argos.db` (`ARGOS_HOME`), `user_version` 26, additive migrations (v26 `recon_model_operations` / `recon_model_attempts`, Intel `lease_epoch`)
- LanceDB `memory_lancedb/` (384-dim MiniLM)
- `ARGOS_EMBED=0` → Jaccard recall

[![Persistence stack](diagrams/persistence.svg)](diagrams/persistence.html)

[![Report tables](diagrams/schema-core.svg)](diagrams/schema-core.html)

## Graph

`graphify-out/graph.json` is the code graph. Query it before broad search. After code edits, `graphify update .`. Track `graph.json`, `manifest.json`, and `GRAPH_REPORT.md` only.

## NLP artifacts and selections

An acquisition artifact preserves retrieved Markdown. A derived prose artifact has its own hash and coordinates. Complete annotations are cached candidates, not validated identities or claims. A selection records exact source ranges and selected/omitted sentence IDs; it does not prove the model read omitted evidence. See [local preprocessing](nlp-preprocessing.md).

## Source claim, inference and insight lifecycle

A **claim** records what a source asserts, including uncertainty, allegation, denial, attribution and time. It does not assert that Argos independently established the proposition. An **inference** is a new conclusion justified by assessed claims and original evidence, with premise IDs, limitations and rationale. **Fidelity** measures faithful extraction; **relation** describes supports/contradicts/insufficient evidence; neither is a retrieval score or probability.

An **insight snapshot** is immutable original evidence, entity bindings, stage coverage, premise assessments and memory revisions for one owner generation. The owner head can be active, pending, previous, insufficient_evidence or blocked. Pending work keeps previous history visible. Valid-empty assessed output differs from failure. Historic fact/inference/context labels remain legacy records and are not automatically promoted. Source-local candidate IDs prevent homonyms from being merged solely by normalized name.

## Related article leads

Atlas collection-v1 uses only Classifier model calls and local/native metadata. Related article pairs represent exact compatible shared headline/description mentions with a category-qualified seed. They supply no corroboration or assessed graph edge; Intel Links remain evidence-grounded propositions. Publisher-based Initial confidence is a source prior, independent from evidence confidence. See [Atlas classification and Related articles](atlas-classification-and-related-articles.md).
