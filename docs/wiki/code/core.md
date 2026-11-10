# Core pipeline map

[Code route](index.md) · [Concept route](../concepts/index.md)

All paths below belong to [argos-osint-core/src](../../../crates/argos-osint-core/src). Read [architecture](architecture.md) alongside source; use [storage](storage.md) for durable state. The [Fleetworks architecture](architecture.md#fleetworks-service-boundaries) maps these core workflows to the API host and its worker roles.

## Retained provider and tool execution

`@tag:recon-flow` · `@tag:recon-bindings`

[model_exec.rs](../../../crates/argos-osint-core/src/model_exec.rs) owns shared model execution, including route fallback, cancellation, and attempt reporting. The tool executor and [tool contracts](../../../crates/argos-osint-core/src/recon/investigation/tool_io.rs) still share the investigation infrastructure for typed bindings, source spans, and validated producer relationships. The [provider guide](../concepts/providers.md) covers current route selection, pacing and admission.

## Discovery services composition

- **`search_actor_names`** — exact normalized form → actor IDs + match reasons + revision; active canonical names only; collisions stay ambiguous; diagnostics: `catalog unavailable` / `query too short` / `unsupported type`; limit 1..=25.
- **`search_actor_aliases`** — aliases with strength `curated`/`catalog`/`learned`, `alias_type`, `provenance`; ambiguous when >1 live owner; exact + bounded prefix tiers on the alias index; owner-union merge.
- **`search_articles`** — two per-field FULLTEXT BM25 statements on schema-v2 title/description indexes; title weighted above description-only; deterministic ties; matched-field attribution; rank = BM25 of matched field — relevance is NOT confidence; bounded excerpt on original byte offsets; dedup by article id; cursor ≤4096, page ≤50; category-membership/day/source/run filters; related candidates exclude the seed and are not corroboration; stale/pending/discarded/deleted hits rejected by construction — canonical corpus is admitted-only.

Discovery lookup reads existing actor and article records. It preserves identity ambiguity and performs no actor or claim publication.

## Intel

`@tag:intel-brief`

[intel_recon/](../../../crates/argos-osint-core/src/intel_recon) owns article analysis; [brief_pipeline.rs](../../../crates/argos-osint-core/src/intel_recon/brief_pipeline.rs) coordinates brief work, with contract and pipeline tests nearby. Read [job concepts](../concepts/glossary.md#intel-brief-jobs-and-historical-reports) and the [brief chain](../../diagrams/intel-brief-chain.html). Follow persistence into [storage/brief.rs](../../../crates/argos-osint-core/src/storage/brief.rs) and [brief_publish.rs](../../../crates/argos-osint-core/src/storage/brief_publish.rs).

[intelligence_contracts.rs](../../../crates/argos-osint-core/src/intelligence_contracts.rs) owns the feature-independent domain packet, inference kinds, six directed Link kinds and eight summary facets. Brief roots capture metadata actors and domain context once; body identity is joined only after completed Body output. Canonical inference payloads retain kind, relevant domains, true support and conditions separately from extraction fidelity. Link publication rechecks exact source/target evidence and the actual semantic decision in the same transaction; a changed publication locator alone is not a changed assertion.

The UI-independent [Intel service](../../../crates/argos-osint-core/src/intel_service.rs) supplies the [local web API](intel-web.md); it reuses repository brief, Related and topology contracts without exporting terminal state.

## Atlas

`@tag:atlas-collection`

[atlas.rs](../../../crates/argos-osint-core/src/atlas.rs) collects and classifies; [atlas_store.rs](../../../crates/argos-osint-core/src/atlas_store.rs) is the store-facing entry. Read the [classification and Related contract](../concepts/atlas-classification-and-related-articles.md) and [cycle figure](../../diagrams/atlas-pipeline.html). Keep tentative article leads distinct from published memory.

Atlas scores the six `strategic-domains-v1` domains independently. Admission requires a qualifying primary and can retain a second qualifying domain. Native readers require current taxonomy/admission/publication authority before interpreting those labels. `atlas-taxonomy-v2` and its seven-topic strings remain explicit historical data; legacy `economic` does not establish the new Economic domain. Historical rejection preserves original tags and published evidence.

## Classifier

@tag:classifier · @tag:atlas-taxonomy

[classifier/mod.rs](../../../crates/argos-osint-core/src/classifier/mod.rs) exposes the independent classifier feature and re-exports the taxonomy leaf. New Atlas cycles pin a native Rust-BERT worker and its checkpoint before acquisition. CLI and Venture probe that worker without opening storage. TUI Profile stores dataset and classifier paths. Legacy Natural management retains QuickJS, bounded dataset parsing and immutable artifacts for historical replay. The feature is wired across all four workspace crates. Disabled builds report unavailable. Durable historical admission and provider monitoring run in separate pools. See the [classifier implementation](classifier.md) and its [acceptance evidence and limitations](../../../.planning/2026-11-05-atlas-performance-completion/acceptance.md).

[atlas_taxonomy.rs](../../../crates/argos-osint-core/src/atlas_taxonomy.rs) — leaf owning the six domain IDs + official names + `strategic-domains-v1` + explicit legacy identifiers + membership/validation helpers; integration direction `atlas_taxonomy ← {classifier, atlas, related_articles, storage, intel_service, article_discovery}`; the leaf does not import the classifier engine, nlp, storage, or orchestration.

Read the [taxonomy concept](../concepts/strategic-domains-taxonomy.md), [classifier code map](classifier.md) and public `tests/classifier_{contract,artifact,training_calibration,dataset,readiness}.rs` contracts together.

Classifier status vocabulary: `missing / ready / training / invalid / incompatible / unavailable`; rejection reasons vs. model/queue/storage faults — faults are blocked or retryable, never `unk`.

## Brain

`@tag:brain-memory`

[brain.rs](../../../crates/argos-osint-core/src/brain.rs) defines memory/provenance concepts and recall scoring; [store.rs](../../../crates/argos-osint-core/src/store.rs) exposes persistence-facing operations. Follow [storage/recall.rs](../../../crates/argos-osint-core/src/storage/recall.rs) for retrieval. Read [source lifecycle](../../diagrams/brain-source-lifecycle.html) and [topology](../concepts/topology-mapper.md) before changing eligibility or associations.

Actor sourcing and catalog ingestion are defined by the [seeded catalog](actor-catalog.md), implemented in [actor_catalog.rs](../../../crates/argos-osint-core/src/actor_catalog.rs) and [storage/actor_catalog.rs](../../../crates/argos-osint-core/src/storage/actor_catalog.rs).

## Providers and tool adapters

`@tag:osint-providers`

[osint/providers.rs](../../../crates/argos-osint-core/src/osint/providers.rs) implements primary OSINT adapters. [provider_adapters/](../../../crates/argos-osint-core/src/provider_adapters) handles model-provider adapters; [provider.rs](../../../crates/argos-osint-core/src/provider.rs) contains shared model configuration/execution. These are distinct concerns: read [providers](../concepts/providers.md), [catalog](../../diagrams/osint-providers.html) and [roles](../../diagrams/model-roles.html). Tests use offline fixtures and fake executors.

## Local preprocessing

[nlp/](../../../crates/argos-osint-core/src/nlp) contains local preprocessing. [NLP contract](../concepts/nlp-preprocessing.md) and its [flow figure](../../diagrams/nlp-preprocessing.svg) describe evidence, selections and cache behavior.


### Intel execution measurements

`intel_recon/brief_timing.rs` supplies one task-local clock per real Surreal Brief stage; concurrent stage workers install their own clock inside the spawned execution. Scoped-thread adapters explicitly carry the captured clock. Local capture, model, fidelity and fenced persistence phases emit a numeric-only `intel_stage` telemetry event keyed by task and lease epoch. This event is separate from report publication totals, and telemetry write failures remain best effort. Summary fallback/packet construction, read-only cache lookups, provider generation, host validation and fenced summary publication emit structured `argos::intel_performance` traces. Timings never enter evidence or cache identities.

Unobserved phases, queue admission, driver round trips and process RSS remain unavailable rather than zero. Existing model-operation/attempt telemetry remains authoritative for provider calls and retries. Stage wall durations can overlap nested/concurrent work and must not be summed as exclusive CPU time. The [acceptance record](../../../.planning/2026-11-05-atlas-performance-completion/acceptance.md) records executed workload comparisons, native embedded verification and their limits; the measurements do not establish an end-to-end speedup.


New body-grounded Brief and Link publication uses `storage::live_evidence::validate_for_publication_in`: a shared per-article body-input epoch serializes body-row/source registration with a deduplicated verified current-head check in the publication transaction. The epoch is operational and never enters semantic assertion/cache identities. Historical confirmed Link liveness retains exact retained-evidence validation; a later acquisition alone does not retire an otherwise valid confirmed relationship.

Foreground admission atomically queues independent per-occurrence work in [atlas_derivative_worker.rs](../../../crates/argos-osint-core/src/atlas_derivative_worker.rs). Collection completion follows guaranteed handoff; native actor publication and lifecycle reconciliation use separate job leases, bounded activations and current-generation readiness. [Derivative invalidation](../../../crates/argos-osint-core/src/storage/atlas_derivative_invalidation.rs) discovers late body/catalog/policy changes in bounded pages. Historical admission uses [atlas_admission_maintenance.rs](../../../crates/argos-osint-core/src/atlas_admission_maintenance.rs) and its storage adapter, preserving retained source identity and completing scoped derivatives in bounded activations. [Provider capture](../../../crates/argos-osint-core/src/classifier/provider_capture.rs), [story families](../../../crates/argos-osint-core/src/atlas_calibration.rs) and [calibration storage](../../../crates/argos-osint-core/src/storage/classifier_calibration.rs) own the separate monitoring lane. See the [classifier contract](classifier.md#historical-admission-and-provider-monitoring).
