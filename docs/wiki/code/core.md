# Core pipeline map

[Code route](index.md) · [Concept route](../concepts/index.md)

All paths below belong to [argos-osint-core/src](../../../crates/argos-osint-core/src). Read [architecture](architecture.md) alongside source; use [storage](storage.md) for durable state.

## Recon

`@tag:recon-flow` · `@tag:recon-bindings`

Typed bindings gained actor association + original byte spans (span_start/span_end, half-open); actor association set only on subject-owned handles; appearance alone never proves ownership; model extraction quotes verified source evidence.

[recon.rs](../../../crates/argos-osint-core/src/recon.rs) exposes investigations and orchestration. Follow [investigation/](../../../crates/argos-osint-core/src/recon/investigation) for binding/picking and [tool_io.rs](../../../crates/argos-osint-core/src/recon/investigation/tool_io.rs) for tool input/output contracts. Start with the [turn figure](../../diagrams/recon-turn.html) and [binding flow](../../diagrams/recon-bindings.html). Changes to tool contracts need coverage of binding kinds, extractors and producers.

Mode classification is local-first: explicit prompt signals resolve with zero model calls; the `recon_decisions` bounded gate runs only when consequential; gate failures surface as diagnostics in plan binding_notes (no silent fallback).

Plan artifacts produced once per plan: `Directive` carries required_evidence / queries / depends_on; dependencies restricted to earlier directive ids; opening discovery reuses the plan's query candidates (no second generation call); Jev picker request state carries directive depends_on.

Missing inputs trigger one grounded discovery attempt per starved step; material-gap replan runs at most once per turn; fallback stops on repeated no progress (4 consecutive settled calls); compaction is threshold-only (NLP mode no longer forces compaction); turn-start catalog actor discovery note/binding seeding; discovery now searches both actor and article indexes (recon/orchestrate.rs discover_catalog_articles / replan_discovery).

## Discovery services composition

- **`search_actor_names`** — exact normalized form → actor IDs + match reasons + revision; active canonical names only; collisions stay ambiguous; diagnostics: `catalog unavailable` / `query too short` / `unsupported type`; limit 1..=25.
- **`search_actor_aliases`** — aliases with strength `curated`/`catalog`/`learned`, `alias_type`, `provenance`; ambiguous when >1 live owner; exact + bounded prefix tiers on the alias index; owner-union merge.
- **`search_articles`** — two per-field FULLTEXT BM25 statements on schema-v2 title/description indexes; title weighted above description-only; deterministic ties; matched-field attribution; rank = BM25 of matched field — relevance is NOT confidence; bounded excerpt on original byte offsets; dedup by article id; cursor ≤4096, page ≤50; category-membership/day/source/run filters; related candidates exclude the seed and are not corroboration; stale/pending/discarded/deleted hits rejected by construction — canonical corpus is admitted-only.

Turn-start and material-gap discovery now search BOTH actor and article indexes (recon/orchestrate.rs discover_catalog_articles / replan_discovery). Lookup never creates actors or claims; Jev cannot override identity ambiguity.

## Intel

`@tag:intel-brief`

[intel_recon/](../../../crates/argos-osint-core/src/intel_recon) owns article analysis; [brief_pipeline.rs](../../../crates/argos-osint-core/src/intel_recon/brief_pipeline.rs) coordinates brief work, with contract and pipeline tests nearby. Read [job concepts](../concepts/glossary.md#intel-brief-jobs-and-historical-reports) and the [brief chain](../../diagrams/intel-brief-chain.html). Follow persistence into [storage/brief.rs](../../../crates/argos-osint-core/src/storage/brief.rs) and [brief_publish.rs](../../../crates/argos-osint-core/src/storage/brief_publish.rs).

The UI-independent [Intel service](../../../crates/argos-osint-core/src/intel_service.rs) supplies the [local web API](intel-web.md); it reuses repository brief, Related and topology contracts without exporting terminal state.

## Atlas

`@tag:atlas-collection`

[atlas.rs](../../../crates/argos-osint-core/src/atlas.rs) collects and classifies; [atlas_store.rs](../../../crates/argos-osint-core/src/atlas_store.rs) is the store-facing entry. Read the [classification and Related contract](../concepts/atlas-classification-and-related-articles.md) and [cycle figure](../../diagrams/atlas-pipeline.html). Keep tentative article leads distinct from published memory.

Taxonomy v2 (`atlas-taxonomy-v2`): Information replaced by Science (scientific breakthroughs, space exploration, academic research, climate change, conservation, environmental policy) and Health (public health, medical discoveries, pharmaceutical regulations, mental health research, lifestyle wellness guides); geopolitical/economic/military/stability/technology retained; 7 categories + exclusive `unk`; retired `information` reads as `unk` until migration.

## Brain

`@tag:brain-memory`

[brain.rs](../../../crates/argos-osint-core/src/brain.rs) defines memory/provenance concepts and recall scoring; [store.rs](../../../crates/argos-osint-core/src/store.rs) exposes persistence-facing operations. Follow [storage/recall.rs](../../../crates/argos-osint-core/src/storage/recall.rs) for retrieval. Read [source lifecycle](../../diagrams/brain-source-lifecycle.html) and [topology](../concepts/topology-mapper.md) before changing eligibility or associations.

Actor sourcing and Venture seed behavior are defined by the [seeded catalog](actor-catalog.md), implemented in [actor_catalog.rs](../../../crates/argos-osint-core/src/actor_catalog.rs) and [storage/actor_catalog.rs](../../../crates/argos-osint-core/src/storage/actor_catalog.rs).

## Providers and tool adapters

`@tag:osint-providers`

[osint/providers.rs](../../../crates/argos-osint-core/src/osint/providers.rs) implements primary OSINT adapters. [provider_adapters/](../../../crates/argos-osint-core/src/provider_adapters) handles model-provider adapters; [provider.rs](../../../crates/argos-osint-core/src/provider.rs) contains shared model configuration/execution. These are distinct concerns: read [providers](../concepts/providers.md), [catalog](../../diagrams/osint-providers.html) and [roles](../../diagrams/model-roles.html). Tests use offline fixtures and fake executors.

## Local preprocessing

[nlp/](../../../crates/argos-osint-core/src/nlp) contains local preprocessing. [NLP contract](../concepts/nlp-preprocessing.md) and its [flow figure](../../diagrams/nlp-preprocessing.html) describe evidence, selections and cache behavior.
