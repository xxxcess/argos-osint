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

Only Recon has a composer. Intel Briefing starts a **focus brief job** on an Atlas article; independent Recon investigations retain their own threads.

## Model roles

Each role is provider + model plus an ordered `fallbacks` list in `config.toml`. Accounts live in `auth.json`. Saving a key does not change a role.

[![Models and roles](diagrams/model-roles.svg)](diagrams/model-roles.html)

| Role | Job |
| --- | --- |
| Recon | Directives and bindings |
| Tool picker | One next tool per request (Jev decisions or chat JSON) |
| Synthesis | Cited answer |
| Classifier | Atlas article categories and bounded article-cleanup decisions |
| Summarization | Graph/path summaries |
| Evidence curator, Entity resolver, Claim assessor, Investigation controller | Investigation harness |

Legacy Grok/OpenAI accounts remain until the user assigns a replacement. New setup tabs are Defaults, OpenRouter, Google, Nvidia.

## Primary OSINT providers

Firecrawl, SociaVault, and Hunter are primary. Everything else is a gap-filler. Hunter inputs come only from the prompt, Firecrawl, SociaVault, or earlier Hunter calls.

[![OSINT catalog](diagrams/osint-providers.svg)](diagrams/osint-providers.html)

## Bindings

A binding is a typed value (`domain`, `handle`, `url`, …) with an evidence id. `recon/investigation/tool_io.rs` maps every catalog input to a kind, a prompt extractor, a rule extractor, and producers. Ungrounded inputs skip the step.

[![Binding and grounding](diagrams/recon-bindings.svg)](diagrams/recon-bindings.html)

## Intel brief jobs and historical reports

A Focus Brief retrieves the full article and extracts Claims, then joins both for claim-specific Inferences. Local actor-mention sentences become candidates; Classifier selects support, context and counterevidence; Synthesis analyzes each claim with its actors and selected original sentences (up to three concurrent analysis requests). Repeated conclusions, including paraphrases, are deduplicated before Links. Claims, Inferences and Links publish separately as each stage completes, through the same index outbox. Article-link leads enter as tentative Related memories with verification gaps. Later failures preserve earlier published results. Its Jobs pane shows durable stage progress and publication/index status. Actors appear above Claims; Claim and Inference records open topology from their exact publication identities. Related beneath Full Article reflects retained ingest associations requiring at least two distinct shared entity mentions.

Article Recon report execution is retired. Historical `intel_report_jobs`, attempts, sections and provenance remain stored; admission, resume and retry reject new execution, and unfinished report jobs and descendants are cancelled and fenced. Historical reports do not take precedence over normal brief publication. Independent Recon investigations remain available.

Brain browsing exposes Claims and unique Actor-name groups without merging identities. Actor tabs show Claims, Recall-enabled Threads and directly mentioned retained Articles. Claim tabs show Actors and original Sources; provenance is independent of Related eligibility. Other memory kinds remain stored for recall and internal use.

## Brain Memory Topology and Scope Analysis

Brain topology is a seed-scoped read projection, defaulting to two hops (1..3)
and all retained origins, with optional current-cycle scope. Brain claim/actor
graphs and Intel Focus Brief claim/actor records supply exact typed identities.
Selecting a record changes inspection; recentering changes the seed and saves
history. Retained-source entries remain truthful when canonical indexing is absent.

- **Presentation nodes**: Actor, explicit ActorGroup, Claim, Inference, derived Topic, Article and Source. A Topic groups recorded topic/category attributes and is not a canonical Brain memory kind. Actor name groups do not assert shared identity; inference premises do not assert world truth.
- **Edges**: Link/Related payloads use typed `EndpointRef::Source`, `EndpointRef::Article` and `EndpointRef::Memory` destinations. Snapshot edges carry stable IDs, endpoint node keys and qualified direction (`concerns`, `mentioned_in`, `source`, `supports`, `premise`, `possible context`). Derived mentions/topics retain their own recorded receipts.
- **Scope Worker**: Explicit candidate evaluation runs on dedicated worker pool `POOL_BRAIN_SCOPE` (`brain_scope`). It uses a lightweight Store and separately persists validated inference memory/metadata and Links; it does not use atomic brief publication. Retained-source projections cannot become canonical premises through browsing.

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

Actor spelling normalization removes terminal English possessives and collapses whitespace while retaining original mention spans. Source-scoped records remain provisional. Background identity maintenance repairs historical spelling variants locally and uses contextual Entity Resolver decisions for broader aliases; cross-source redirects require an explicit `same` assessment. Related article overlap remains independent of actor identity.
