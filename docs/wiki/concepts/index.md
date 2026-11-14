# Explore Fleetworks concepts

[Wiki](../../index.md) · [Explore code](../code/index.md)

Start with [usage](usage.md) for the native macOS app and Intel Web with independently operated backend services. The [glossary](glossary.md) explains service and evidence terms.

## Follow the work

| Concept | Meaning and reading | Diagram | Implementation |
| --- | --- | --- | --- |
| Intel | Retained article briefs, claim analysis and staged publication. [Brief jobs](glossary.md#intel-brief-jobs-and-historical-reports) | [Brief chain](../../diagrams/intel-brief-chain.html) | [Core](../code/core.md#intel) |
| Atlas | Durable headline collection and six-domain admission. [Classification and Related](atlas-classification-and-related-articles.md) · [Taxonomy](strategic-domains-taxonomy.md) | [Atlas cycle](../../diagrams/atlas-pipeline.html) | [Core](../code/core.md#atlas) |
| Brain | Sourced memory, direct associations and topology. [Browsing](../code/architecture.md#brain-browsing-and-direct-associations) · [Topology](topology-mapper.md) | [Source lifecycle](../../diagrams/brain-source-lifecycle.html) | [Core](../code/core.md#brain) |
| Jobs and Logs | Durable work and operational events. [Workstation activity](usage.md#jobs-and-logs) | [Jobs lifecycle](../../diagrams/jobs-lifecycle.html) | [Architecture](../code/architecture.md#fleetworks-service-boundaries) |

## Shared contracts

| Concern | Read next | Trace |
| --- | --- | --- |
| Model roles and data providers | [Providers](providers.md) · [Model roles](../../diagrams/model-roles.html) | `@tag:osint-providers` |
| Storage and durability | [Persistence](glossary.md#persistence) · [Storage source map](../code/storage.md) | `@tag:surreal-storage` |
| Clients and navigation | [Usage](usage.md) · [Interfaces](../code/interfaces.md) | `@tag:web-routing` |
| Local preprocessing | [NLP](nlp-preprocessing.md) · [Data flow](../../diagrams/nlp-preprocessing.svg) | [Core](../code/core.md#local-preprocessing) |

Historical checklists and dated evidence live in the [archive](../../archive/index.md). Use the [code route](../code/index.md) for source and verification.
