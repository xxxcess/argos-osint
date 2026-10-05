# Unified reliability / summarization / semantic pipelines — completion checklist

Branch: `feat/ann-live-summary-flush`
Spec: `specs/argos-unified-reliability-semantic-pipelines-spec.md` (local path on agent box)
Base: `main` @ `8e9e144` (PR #35 squash-merge)
PR: (this follow-up)

## Phase status (§18)

| Phase | Status | Notes |
| --- | --- | --- |
| 1. Preserve data | **Done** | (from #35) |
| 2. Shared execution | **Mostly done** | Live summary flush drain with optional secret |
| 3. Deadline integration | **Mostly done** | (from #35) |
| 4. Role + summary foundation | **Done** | |
| 5. Summary call sites | **Mostly done** | Deterministic publish + enqueue; live upgrade when secret configured |
| 6. Automatic vectors | **Mostly done** | Shadow Lance + optional IVF-Flat when policy enables |
| 7. Evidence retrieval | **Mostly done** | Hybrid passages + **measured ANN policy** (Exact default) |
| 8. Remaining summary modes | **Mostly done** | Flush enqueue on tool-obs, report_context, follow-up, article desc, atlas brief, section digest |
| 9. Pipeline adoption | **Mostly done** | |
| 10. Exploration / tools | **Mostly done** | Store::recall boosts via hybrid_passage_candidates |
| 11. Verification | **Advanced partial** | Synthetic ANN harness; live flush without inventing network success |

## This follow-up

1. **Measured ANN** — `AnnThresholds` / `measure_ann_recall` / `decide_ann_policy`; ExactSearch remains default until thresholds met (or force). `BrainIndex::ensure_ann_index` builds IVF-Flat only when `AnnPolicy::AnnEnabled`.
2. **Live LLM flush** — persist flush request JSON; `try_live_summary_upgrade`; scheduler loads summarization secret when present; never invents success without secrets.
3. **Enqueue coverage** — tool-observation, report_context, follow-up context, article description (+ prior atlas brief / section digest).
4. **Hybrid adoption** — `Store::recall` re-ranks long memories via `hybrid_passage_candidates`.

## Honest remaining gaps

- ANN still **off by default** until a real measurement on a large enough corpus justifies it; unit tests prove the gate and that IVF can be built when enabled.
- Live LLM upgrade only runs when auth/settings expose a summarization secret; offline CI always takes the no-secret path.
- Full §19 chaos / HTTP mocks still out of scope.
- Repo-wide clippy `-D warnings` on pre-existing issues still out of scope.
