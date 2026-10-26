# Explore Argos concepts

[Wiki](../../index.md) · [Explore code](../code/index.md)

Argos combines interactive investigations, article analysis, headline collection and shared memory. Begin with [usage](usage.md) for interaction and the [glossary](glossary.md) for app labels, internal IDs and model roles.

## 1. Follow the work

| Concept | Meaning and reading | Diagram | Implementation |
| --- | --- | --- | --- |
| Recon | A persistent question → tool evidence → synthesis investigation; [flow](../code/architecture.md#investigation-flow) | [Recon turn](../../diagrams/recon-turn.html) | [Core](../code/core.md#recon) |
| Intel | Article briefs, claim analysis and staged publication; [brief jobs](glossary.md#intel-brief-jobs-and-historical-reports) | [Brief chain](../../diagrams/intel-brief-chain.html) | [Core](../code/core.md#intel) |
| Atlas | Collect and classify headlines; Related articles are tentative leads, not published Brain memories; [contract](atlas-classification-and-related-articles.md) | [Atlas cycle](../../diagrams/atlas-pipeline.html) | [Core](../code/core.md#atlas) |
| Brain | Sourced memory, recall, direct associations and topology; [browse](../code/architecture.md#brain-browsing-and-direct-associations), [topology](topology-mapper.md) | [Source lifecycle](../../diagrams/brain-source-lifecycle.html) | [Core](../code/core.md#brain) |

## 2. Understand shared contracts

| Concern | Read next | Trace |
| --- | --- | --- |
| Tool inputs and evidence | [Bindings](glossary.md#bindings), [tool contracts](../code/architecture.md#tool-inputs-and-bindings) | `@tag:recon-bindings` |
| Model roles vs OSINT services | [Providers](providers.md), [model roles figure](../../diagrams/model-roles.html) | `@tag:osint-providers` |
| Storage and durability | [Current persistence](glossary.md#persistence), [storage source map](../code/storage.md) | `@tag:surreal-storage` |
| Interface behavior | [Usage](usage.md), [TUI design](../code/tui-design-spec.md), [shell figure](../../diagrams/tui-shell.html) | `@tag:tui-components` |
| Local preprocessing | [NLP](nlp-preprocessing.md), [data flow](../../diagrams/nlp-preprocessing.html) | [Core map](../code/core.md#local-preprocessing) |

## 3. Go deeper

These links are the canonical concept references. Past implementation evidence lives in the [archive](../../archive/index.md). Historical checklists explain past work; the diagram catalog contains current contracts. Switch to the [code route](../code/index.md) to inspect an implementation or plan a change.
