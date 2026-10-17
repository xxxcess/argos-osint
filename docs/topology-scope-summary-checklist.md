# Topology scope summary checklist (PR #48)

Plan: [topology-scope-summary-plan.md](topology-scope-summary-plan.md). Screenshots: `docs/screens/topology-summary/`.

## Phase 1 · Core (`8af7922`)
- [x] Deterministic markdown summary of connections and scope (`topology_summary::deterministic_summary`), visible/filtered nodes only
- [x] Scope key (recenter, depth, cycle, filters), graph version (ignores selection, pan, zoom, order, `memory_changed_seq`), digest
- [x] `topology_scope_summaries` (schema 43): reuse by digest, newest 50 per seed, deleted with the seed

## Phase 2 · Generation (`f2a0a5a`)
- [x] Summarization-role job with `topology_summary.*` events and running/failed records
- [x] Markdown-only prompt; scope data fenced as untrusted; bounded brief (60 nodes / 120 edges)
- [x] Exact digest reuse ≤ 7 days; 10 minute churn throttle; 120 s failure cooldown; explicit retry bypass
- [x] No Summarization route keeps the deterministic text
- [x] A late result for an old scope is saved under its own digest, never displayed

## Phase 3 · TUI (`7abc171`)
- [x] Bordered Scope summary panel (Desktop/Compact), markdown renderer, wrapping
- [x] Independent scrolling: Up/Down, PgUp/PgDn, Home/End, mouse wheel, `▲ a–b/total ▼`; stable across redraws; reset on scope change
- [x] Stacked Summary tab (Graph / Summary / Details / Controls) plus one-line headline; 80x24 and 60x18
- [x] Findings strip folded into the caveat line
- [x] Worker: automatic on snapshot accept, recenter, depth, cycle and filter; obsolete scopes fenced; `[Retry]` / `s`
- [x] States: deterministic, writing…, AI summary, stale, failed, no model
- [x] Docs wording: automatic generation only after the snapshot is accepted, via Summarization, none during draw

## Phase 4 · Hardening
- [x] Failure card: the Brain `summary_card` (details, logs, job, retry, Open Models) reused for topology
- [x] End-to-end worker test through the offline mock provider (write, reuse on return, 401 failure card, record persisted)
- [x] `tui-components.md`, `architecture.md`, new diagram `diagrams/topology-scope-summary.*`, `topology-mapper.md`, `usage.md`
- [x] `graphify update .`
- [x] Matrix (`ARGOS_EMBED` unset): default, `--no-default-features`, `--no-default-features --features nlp`; clippy `-D warnings`, fmt
- [ ] Live run through the installed Summarization route: not run. The route resolves to openrouter / `meta-llama/llama-3.2-3b-instruct:free`, but no API key is configured (`key_present=false`), so it cannot make a request. Run `ARGOS_LIVE_TOPOLOGY=1 cargo test -p argos-osint-core live_topology -- --ignored --nocapture` after connecting the account.

## Known limits
- At 100x32 the failure card fills the 6-row panel; the summary text returns after Retry or on the Summary page of the stacked layouts.
- Node labels are not part of the graph version, so a rename alone does not regenerate the summary.
- A seq-only change with identical visible content is an exact digest reuse (stricter than the 10 minute throttle).
