# Topology neighborhood summary checklist

See the [final design](../../docs/topology-scope-summary-plan.md) and [specification](../../docs/topology-mapper.md#neighborhood-summary).

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
- [x] Representative screenshots (10) from the 160×50 to 60×18 matrix

## Phase 4 — docs and verification
- [x] topology-mapper.md, diagram pair, architecture.md, tui-components.md, plan, this checklist
- [x] graphify update
- [x] PR #48 body rewritten; screenshots pinned to the final SHA
- [x] CI matrix green on all four targets (macOS arm64/x86_64, Linux x86_64/aarch64) at `dba2fff` (run 39047777535) and at `4758691` (run 39050891830: Linux x86_64/aarch64, macOS arm64/Intel, policy all pass)
- [x] Local matrix (default, `--no-default-features`, nlp) fmt/clippy/tests via `scripts/agent_cargo.py` (run before the last heartbeat commit; CI covers `dba2fff`)
- [ ] Live Summarization check (not run: no Summarization account configured)

## Known limits
- 60×20 leaves two Summary content rows
- Visual review was by image only; no human or live-terminal pass
- Fixture generated text is canned

## Phase 5 — review fixes
- [x] Lease heartbeat load-tolerant; watcher bounded (deadline, poll errors, typed failures)
- [x] Auth/settings/runtime load off the UI thread; idle tick loads nothing
- [x] Rows cached; styled lines from the typed brief
- [x] Docs trimmed, state diagram added, representative screenshots only
- [x] Lease heartbeat stress: 20 runs of the executor and `sqlite_contention` tests under 16 busy loops with `--test-threads=32`, 0/20 failures
- Note: the `lease_secs` 3 to 6 bump in `intel_recon/brief_stages_tests.rs` is a mitigation, not a root-cause fix
