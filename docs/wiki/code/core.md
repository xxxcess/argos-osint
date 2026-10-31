# Core pipeline map

[Code route](index.md) · [Concept route](../concepts/index.md)

All paths below belong to [argos-osint-core/src](../../../crates/argos-osint-core/src). Read [architecture](architecture.md) alongside source; use [storage](storage.md) for durable state.

## Recon

`@tag:recon-flow` · `@tag:recon-bindings`

[recon.rs](../../../crates/argos-osint-core/src/recon.rs) exposes investigations and orchestration. Follow [investigation/](../../../crates/argos-osint-core/src/recon/investigation) for binding/picking and [tool_io.rs](../../../crates/argos-osint-core/src/recon/investigation/tool_io.rs) for tool input/output contracts. Start with the [turn figure](../../diagrams/recon-turn.html) and [binding flow](../../diagrams/recon-bindings.html). Changes to tool contracts need coverage of binding kinds, extractors and producers.

## Intel

`@tag:intel-brief`

[intel_recon/](../../../crates/argos-osint-core/src/intel_recon) owns article analysis; [brief_pipeline.rs](../../../crates/argos-osint-core/src/intel_recon/brief_pipeline.rs) coordinates brief work, with contract and pipeline tests nearby. Read [job concepts](../concepts/glossary.md#intel-brief-jobs-and-historical-reports) and the [brief chain](../../diagrams/intel-brief-chain.html). Follow persistence into [storage/brief.rs](../../../crates/argos-osint-core/src/storage/brief.rs) and [brief_publish.rs](../../../crates/argos-osint-core/src/storage/brief_publish.rs).

The UI-independent [Intel service](../../../crates/argos-osint-core/src/intel_service.rs) supplies the [local web API](intel-web.md); it reuses repository brief, Related and topology contracts without exporting terminal state.

## Atlas

`@tag:atlas-collection`

[atlas.rs](../../../crates/argos-osint-core/src/atlas.rs) collects and classifies; [atlas_store.rs](../../../crates/argos-osint-core/src/atlas_store.rs) is the store-facing entry. Read the [classification and Related contract](../concepts/atlas-classification-and-related-articles.md) and [cycle figure](../../diagrams/atlas-pipeline.html). Keep tentative article leads distinct from published memory.

## Brain

`@tag:brain-memory`

[brain.rs](../../../crates/argos-osint-core/src/brain.rs) defines memory/provenance concepts and recall scoring; [store.rs](../../../crates/argos-osint-core/src/store.rs) exposes persistence-facing operations. Follow [storage/recall.rs](../../../crates/argos-osint-core/src/storage/recall.rs) for retrieval. Read [source lifecycle](../../diagrams/brain-source-lifecycle.html) and [topology](../concepts/topology-mapper.md) before changing eligibility or associations.

## Providers and tool adapters

`@tag:osint-providers`

[osint/providers.rs](../../../crates/argos-osint-core/src/osint/providers.rs) implements primary OSINT adapters. [provider_adapters/](../../../crates/argos-osint-core/src/provider_adapters) handles model-provider adapters; [provider.rs](../../../crates/argos-osint-core/src/provider.rs) contains shared model configuration/execution. These are distinct concerns: read [providers](../concepts/providers.md), [catalog](../../diagrams/osint-providers.html) and [roles](../../diagrams/model-roles.html). Tests use offline fixtures and fake executors.

## Local preprocessing

[nlp/](../../../crates/argos-osint-core/src/nlp) contains local preprocessing. [NLP contract](../concepts/nlp-preprocessing.md) and its [flow figure](../../diagrams/nlp-preprocessing.html) describe evidence, selections and cache behavior.
