# Storage map

[Code route](index.md) · [Persistence concepts](../concepts/glossary.md#persistence)

`@tag:surreal-storage`

SurrealDB is the only current backend. Default `embedded,nlp` features use RocksDB in-process under `<ARGOS_HOME>/storage/surreal`; `ARGOS_HOME` defaults to `~/.argos`. Server mode is opt-in. Read [build/storage rules](contributing.md#storage-facts) before running anything.

| Responsibility | Source entry |
| --- | --- |
| Module boundary and engine opening | [storage/mod.rs](../../../crates/argos-osint-core/src/storage/mod.rs), [open.rs](../../../crates/argos-osint-core/src/storage/open.rs) |
| Schema ordering and version guard | [bootstrap.rs](../../../crates/argos-osint-core/src/storage/bootstrap.rs), [schema/](../../../crates/argos-osint-core/src/storage/schema) |
| Retrying transactions | [tx.rs](../../../crates/argos-osint-core/src/storage/tx.rs) |
| Memory publication and graph projection | [publisher.rs](../../../crates/argos-osint-core/src/storage/publisher.rs), [memory.rs](../../../crates/argos-osint-core/src/storage/memory.rs), [graph.rs](../../../crates/argos-osint-core/src/storage/graph.rs) |
| Vector and lexical recall | [embeddings.rs](../../../crates/argos-osint-core/src/storage/embeddings.rs), [recall.rs](../../../crates/argos-osint-core/src/storage/recall.rs) |
| Durable jobs | [jobs.rs](../../../crates/argos-osint-core/src/storage/jobs.rs) |
| Contract verification | [tests/](../../../crates/argos-osint-core/tests), [shared harness](../../../crates/argos-osint-core/tests/support), [test_surreal.py](../../../scripts/test_surreal.py) |

[![Current persistence](../../diagrams/persistence.svg)](../../diagrams/persistence.html)

Run storage contracts through the harness; a skipped test is not a pass. Agents run only the embedded matrix locally; server checks belong to CI. Legacy `schema_*.sql` files are migration context, not the current schema authority.

## Recall and source lifecycle

[Brain recall](../../diagrams/brain-recall.html) shows DISKANN candidates followed by lexical top-up. Vector hits must match the current model, generation and memory revision and remain active with an allowed kind. Missing vectors or vector-search failure use lexical recall; vector hits rank first. Results are capped at five memories and 1,500 estimated tokens. See [recall.rs](../../../crates/argos-osint-core/src/storage/recall.rs) and [recall contracts](../../../crates/argos-osint-core/tests/storage_recall_turn.rs).

[Source lifecycle](../../diagrams/brain-source-lifecycle.html) reconciles retained support and durable index work in SurrealDB. The worker rechecks eligibility before index changes. See [lifecycle.rs](../../../crates/argos-osint-core/src/storage/lifecycle.rs), [index_drain.rs](../../../crates/argos-osint-core/src/storage/index_drain.rs) and [lifecycle contracts](../../../crates/argos-osint-core/tests/storage_lifecycle.rs).
