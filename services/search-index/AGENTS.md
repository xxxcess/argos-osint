# Private actor search service

Read [root guidance](../../AGENTS.md), [contributing](../../docs/wiki/code/contributing.md),
[conventions](../../docs/wiki/code/conventions.md) and the Rust style skill before
edits. The integration owner controls manifests/lock/toolchain and the shared
agent Cargo cache. Preserve unrelated work and all persistent index evidence.

- This is a private Linux backend engine. Keep Tantivy out of desktop, shared
  client DTOs, inference and unrelated build graphs. No database replacement,
  Docker supervisor, arbitrary command/URL/path endpoint or public admin service.
- SurrealDB actor/alias/lifecycle records are truth. Producers atomically record
  outbox intent with source changes. Only the leased authenticated coordinator
  sends bounded projection batches; it acknowledges only reader-visible receipts.
- Preserve source/deployment/schema/analyzer/index-format/generation/revision,
  tombstone and coordinator-epoch fences. Commit payload and documents share the
  same Tantivy commit. Equal revision with different content must fail.
- One process/root lock and one active writer budget; never remove lock files to
  make another replica start. No automatic index deletion or silent repair.
  Shadow replacement retains the last verified serving reader until promotion.
- Enforce count/byte/rate/concurrency bounds before work; no unbounded queue,
  per-actor HTTP fan-out, query parser/regex or all-catalog memory load. Blocking
  search/write/commit work stays off Tokio executors and keeps capacity permits
  through uncertain timeouts. The coordinator alone owns retries and bulk pacing.
- Internal results are candidates. Gateway code must authorize current canonical
  actors before exposing labels/results and fetch dossiers separately. Scores are
  retrieval ranking, never identity confidence or evidence strength.
- Tests use synthetic actors/credentials and owned temporary disk roots. Preserve
  real index/worker/gateway/recovery evidence separately from mocks and native
  PNGs. No live providers, user home/volumes or local mutation tests.
- Keep README limitations honest. Missing outbox/gateway/retention/pressure work,
  unexecuted tests and unmeasured Linux performance cannot become passing claims.
