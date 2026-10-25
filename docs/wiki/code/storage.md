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

## Schema and runtime authority

[Bootstrap](../../../crates/argos-osint-core/src/storage/bootstrap.rs) applies the registered module `SCHEMA` constants (including `.surql` files) in dependency order within one retryable transaction. It records `storage_meta:schema.version` and refuses a database newer than the build. The current constant is `storage::bootstrap::SCHEMA_VERSION` (currently 1), independent of the old `store::SCHEMA_VERSION` (43). The SurrealDB bootstrap does not import a legacy SQLite/LanceDB database or run its numbered migration ladder. See [bootstrap contracts](../../../crates/argos-osint-core/tests/storage_bootstrap.rs).

[`Store`](../../../crates/argos-osint-core/src/store.rs) is a lightweight handle onto the process [runtime](../../../crates/argos-osint-core/src/storage/runtime.rs). `open_ui`, `open_light` and `open_profile_reader` do not establish separate SQLite or read-only database connections. The legacy `argos.db` path accepted by `Store::open` supplies its parent directory; the method does not open that file. Engine ownership and configuration belong to [open.rs](../../../crates/argos-osint-core/src/storage/open.rs).

SurrealQL statements, record IDs, indexes and transactions in current repository code are valid current SQL-like references. Legacy `schema_*.sql`, plural SQLite table names and numbered migration comments are not the current schema. Follow the actual `DEFINE TABLE`/`DEFINE FIELD` declarations and callers, rather than migration-era module comments.

| Concern | Current records and implementation | Verification |
| --- | --- | --- |
| Atlas metadata | `atlas_article_metadata`, `atlas_article_mention`, `atlas_related_article` in [atlas_metadata.rs](../../../crates/argos-osint-core/src/storage/atlas_metadata.rs) | [metadata contracts](../../../crates/argos-osint-core/tests/storage_atlas_metadata.rs) |
| Brief stages and publication | `brief_run`, `stage_output`, `intel_brief_publication`, `intel_brief_stage_receipt`; [brief_stages.rs](../../../crates/argos-osint-core/src/storage/brief_stages.rs), [brief_publish.rs](../../../crates/argos-osint-core/src/storage/brief_publish.rs) | [stage contracts](../../../crates/argos-osint-core/tests/storage_brief.rs), [publication contracts](../../../crates/argos-osint-core/tests/storage_brief_publish.rs) |
| Insight execution | `insight_stage_checkpoint`, `insight_refresh_execution`, `insight_refresh_task`, `insight_supersession`; [pipeline_exec.rs](../../../crates/argos-osint-core/src/storage/pipeline_exec.rs), [refresh.rs](../../../crates/argos-osint-core/src/storage/refresh.rs) | [execution contracts](../../../crates/argos-osint-core/tests/storage_pipeline_exec.rs), [refresh contracts](../../../crates/argos-osint-core/tests/storage_refresh.rs) |
| Model telemetry | `recon_model_operation`, `recon_model_attempt`; [recon_jobs.rs](../../../crates/argos-osint-core/src/storage/recon_jobs.rs) | [model/job contracts](../../../crates/argos-osint-core/tests/storage_recon_jobs.rs) |
| Profile | `telemetry_event`, `telemetry_hourly`, `telemetry_daily`; [profile_view.rs](../../../crates/argos-osint-core/src/storage/profile_view.rs) | [profile contracts](../../../crates/argos-osint-core/tests/storage_profile_view.rs) |
| NLP and summary cache | `nlp_artifact`, `nlp_source_ref`, `nlp_feature`, `derived_summary`, `summary_flush_request`; [nlp_cache.rs](../../../crates/argos-osint-core/src/storage/nlp_cache.rs), [summary_flush.rs](../../../crates/argos-osint-core/src/storage/summary_flush.rs) | [summary contracts](../../../crates/argos-osint-core/tests/storage_summary_flush.rs) |

Stage completion and canonical publication are separate commits: `brief_stages::commit` first commits the leased stage output, then requests publication. A canonical publication failure rolls back that transaction and reports an index/publication failure without undoing completed extraction. Atlas phase-6 batching likewise does not create one 100-article transaction: each occurrence's relationships and receipt commit together, and successful occurrences remain available if another fails.
