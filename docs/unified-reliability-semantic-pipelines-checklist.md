# Unified reliability / summarization / semantic pipelines — completion checklist

Branch: `feat/unified-reliability-semantic-pipelines`
Spec: `specs/argos-unified-reliability-semantic-pipelines-spec.md`
PR: https://github.com/xxxcess/argos-osint/pull/35

## Phase status (§18)

| Phase | Status | Notes |
| --- | --- | --- |
| 1. Preserve data | **Done** | Safe insight replace; concurrent replace serializes via `BEGIN IMMEDIATE` |
| 2. Shared execution | **Mostly done** | tasks, admission + **shared rate-limit cooldown**, 2/3 retries, elected scheduler; index pool applies upsert/remove/rebuild; summary flush drain (cached) |
| 3. Deadline integration | **Mostly done** | TurnClock embeds ClockSet; FG→background; stream cut preserves partial |
| 4. Role + summary foundation | **Done** | CLI + TUI Summarization |
| 5. Summary call sites | **Mostly done** | Nine modes; deterministic publish + **enqueue flush** (Atlas brief, section digest); live `complete_summary` still call-site specific |
| 6. Automatic vectors | **Mostly done** | Shadow building tables + activate pointer |
| 7. Evidence retrieval | **Mostly done** | Passages + ID coverage + **bounded hybrid candidates** (exact vector only; ANN deferred/unmeasured) |
| 8. Remaining summary modes | **Mostly done** | Flush task path exists; not every mode blocks on LLM polish |
| 9. Pipeline adoption | **Mostly done** | Live Recon `directive_coverage` → `plan.gaps` |
| 10. Exploration / tools | **Mostly done** | explore + Brain related-evidence labels |
| 11. Verification | **Advanced partial** | `reliability_faults` expanded; full §19 matrix still not closed |

## Honest remaining gaps

- **ANN indexes** not created: no measured corpus size / recall-vs-exact / p50-p95 justifying an index policy (spec §19). Exact Lance search + lexical hybrid only.
- Live LLM upgrade of enqueued flush tasks still requires a call site with a provider secret (`complete_summary`); the summary pool completes **cached deterministic** rows without inventing network success.
- Not every deterministic call site enqueues a flush (e.g. tool-observation packet path, report_context input compression).
- Hybrid passage retrieval is library-ready; not every Brain/Recon path consumes `hybrid_passage_candidates` yet (memory recall already uses `hybrid_recall`).
- Fuller §19 items: real HTTP mock providers, multi-node scheduler failover chaos, measured indexing lag.
- Repo-wide `clippy -D warnings` still fails on **pre-existing** issues outside this work (too_many_arguments, dead_code in budget/picker, etc.). Introduced clippy nits in touched modules fixed.

## Diminishing returns

Further polish on this PR has **diminishing returns** relative to the open gaps: ANN needs measurement before enablement; wiring every deterministic site through live LLM needs provider secrets and product decisions about when polish may delay UX; remaining §19 chaos tests need harness investment. Prefer human review of #35 as the verifiable slice, then follow-ups for measured ANN and selective live flush call sites.

## Recent gap closures (this push)

1. Bounded hybrid passage candidates + `AnnPolicy::ExactSearch` (ANN explicitly deferred/unmeasured).
2. `publish_deterministic_and_enqueue` / summary flush tasks; Atlas brief + section digest enqueue; summary flush worker drains cached rows.
3. Index worker applies upsert/remove/rebuild via Store; also drains generation rebuild batches.
4. `reliability_faults`: concurrent insight replace race, stream interruption preserve, shared rate-limit cooldown.
5. Clippy clean on introduced code in touched modules.
