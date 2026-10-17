# Topology neighborhood summary checklist

See the [final design](topology-scope-summary-plan.md) and [specification](topology-mapper.md#neighborhood-summary).

## Phase 1 — brief, packet, cache (`d1c11c0`)
- [x] Deterministic `NeighborhoodBrief` and typed `NeighborhoodInput` built in the topology read transaction
- [x] Blocks omitted when unsupported; no placeholders; no invented confidence, reliability, time or likelihood
- [x] `basis_key` over seed, options, content, revisions, coverage, format
- [x] Validator: schema, references, kinds, direction, qualifications, unsupported assertions
- [x] Fallback publication, deduplicated enqueue, exact cache hit, retry in place
- [x] No new table or migration; no canonical writes

## Phase 2 — scheduler and delivery (`b55fcc1`)
- [x] Role-aware dispatch for `topology_neighborhood`; other modes unchanged
- [x] Explicit response contract and Summarization attempt cap honored
- [x] Lease renewal and epoch-fenced publish/complete
- [x] Truthful outcomes (`fallback=true` is not success); typed terminal failures
- [x] `TopologySummaryLoaded` event; delayed-reply, cancellation, cache and queue failure tests

## Phase 3 — TUI (`0b08923`)
- [x] Summary/Record tabs; Summary default; Relationships tabs independent
- [x] Basis activation opens Record, no recenter or filter change; focus alone does not activate
- [x] Independent Summary scroll (wheel, PgUp/PgDn, arrows, Home/End) with indicator
- [x] Atomic replacement, no focus steal, obsolete results fenced
- [x] Retry keeps content; generating/generated/failed/blocked/stale/limited labels
- [x] History and tab/scroll restoration; compact Details and Back in too-small mode
- [x] Screenshots at 160×50, 120×40, 100×32, 80×24, 60×18, 60×20, 40×20

## Phase 4 — docs and verification
- [x] topology-mapper.md, diagram pair, architecture.md, tui-components.md, plan, this checklist
- [x] graphify update
- [x] PR #48 body rewritten; screenshots pinned to the final SHA
- [x] Matrix: default, `--no-default-features`, `--no-default-features --features nlp` (fmt, Clippy, tests)
- [x] Optional live Summarization check: result recorded in the PR body

## Known limits
- 60×20 leaves two Summary content rows
- Visual review was by image only; no human or live-terminal pass
- Fixture generated text is canned
