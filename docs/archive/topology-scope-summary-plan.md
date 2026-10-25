# Topology neighborhood summary: final design

> Historical record. For current behavior, start at the [wiki](../index.md).

Status: implemented on `feat/topology-mapper-scope-summary` (PR #48, draft). This replaces the earlier model-written scope-summary proposal, which was reverted in `9c43c2c` (new table, worker, throttle and failure card). Spec baseline: `main` `66878bc`. Progress is tracked in `.planning/2026-10-15-topology-mapper/checklist.md`; behavior is specified in [topology-mapper.md](../wiki/concepts/topology-mapper.md#neighborhood-summary).

## Outcome

For each topology seed, show a deterministic neighborhood brief immediately, automatically generate a better explanation with the Summarization role from a frozen typed packet, and atomically replace the fallback only after the complete answer validates. Failure keeps the fallback. Scope is the **loaded neighborhood**: selection, display filters, pan and zoom never regenerate it; recenter, depth, cycle scope, changed data or a changed route configuration do.

## Design decisions

- One read transaction builds the brief and the packet; hydration is bounded (160 payload reads) and typed. No label or ID parsing.
- Reuse `argos_derived_summaries`, the summary flush request and the deduplicated `argos_tasks` row. **No new table, no migration.**
- New mode `SummarizationMode::TopologyNeighborhood` / operation `topology_neighborhood` in `POOL_SUMMARY`; role-aware dispatch through `execute_chat` with the explicit `GeneratedJson` contract and the Summarization attempt cap and timeout. Other summary modes are unchanged.
- The chain is the single retry owner. Terminal failures complete the task with `topology_failed:<kind>`; Retry resets the same request in place.
- Lease renewal while waiting; cache publication and task completion fenced by lease epoch in a short `BEGIN IMMEDIATE`.
- UI: own summary generation counter plus seed, depth, cycle and `basis_key` fences, so a failed refresh that retains the snapshot cannot orphan a live watcher. The watcher polls every 750 ms, never calls a model, and is detached when obsolete.
- Cache identity: `basis_key` (seed, options, admitted content, hydrated revisions, coverage, format version), packet hash, prompt/output versions, budget, and a secret-free route fingerprint.

## Mapping: spec concept → Argos record

| Spec concept | Argos record | Rule in the brief |
| --- | --- | --- |
| Actor / ActorGroup | `ActorPayload`, `ActorRef`; explicit group members | Names, subtype, aliases, binding state; members stay distinct; names never merge identities |
| Claim | `ClaimPayload` | Text, actors, modality, passages; assessment is extraction support, not truth |
| Inference | `InferencePayload` | Stored conclusion shown as a stored interpretation with premises, reasoning, limitations; `supported` is not independently verified |
| Link | `LinkPayload` (edge) | Directed endpoints, exact predicate, rationale, receipts |
| Related | `RelatedPayload` (edge) | Tentative potential context with basis and verification gap; never corroboration |
| Article / Source | Exact occurrence/revision projections, derivations | Provenance only; distinct publishers do not establish independence |
| Topic | Derived `insight_claims.topic` / Atlas category attribute | Context only; no canonical Topic memory |
| Coverage | Snapshot counters (admitted, discovered, omitted, unresolved, exhausted) | Limits block and footer; status Ready, Limited, Isolated or Missing |
| Basis reference | `BasisRef { n, BasisTarget::Node/Edge }` | Typed; opens Record, never recenters |
| Confidence, origin groups, reliability, valid time, likelihood | **Not present in Argos** | Never shown or invented |

## Summary tab sizing

The details panel takes 75% of the inspector height (min 8), leaving Relationships at least 5 rows; Record keeps 55% (min 7). Inside: tab bar 1 row, wrapped state label 1–2 rows, scroll content, overflow indicator 1 row (only when needed), button row 1 row (only on failure).

| Terminal | Details panel (with borders) | Notes |
| --- | --- | --- |
| 160×50 | 32 rows | side by side with the graph |
| 120×40 | 24 rows | |
| 100×32 | 18 rows | |
| 80×24 | 12 rows | stacked Details page |
| 60×20 | 8 rows | real minimum; two content rows, cramped but scrollable |
| 60×18, 40×20 | not drawn | below minimum: size notice and Back |

## Phases (PR #48)

| Phase | Commit | Content |
| --- | --- | --- |
| 1 | `d1c11c0` | Brief builder, typed packet, validator, cache/executor, snapshot fields |
| 2 | `b55fcc1` | Scheduler role dispatch, lease/epoch fencing, truthful outcomes, `TopologySummaryLoaded`, delayed mock server |
| 3 | `0b08923` | TUI tabs, watcher and states, history, tests, screenshots |
| 4 | `1c6ca92` | Docs, diagram pair, plan, checklist, graphify |
| 5 | `8920477`, `d692708`, `d7cd6f8`, `c83df54`, `dba2fff` | CI fixes (lease heartbeat, bounded watcher, worker-thread config load, lazy idle tick); render cache; docs trim |

## Verification

CI (macOS Apple Silicon and Intel, Linux x86_64 and aarch64) is the gate of record; see the PR for its result. Offline by default: fmt, strict Clippy and full tests with `ARGOS_EMBED` unset and `--locked` for default features, `--no-default-features` and `--no-default-features --features nlp`. Screenshots: `dump_brain_topology_screens` / `topology-summary-*` in [screens/topology-summary](../screens/topology-summary/README.md), with visual review recorded separately from interaction tests. No live provider call is part of the default gate.
