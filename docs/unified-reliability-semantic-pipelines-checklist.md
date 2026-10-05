# Unified reliability / summarization / semantic pipelines — completion checklist

Branch: `feat/unified-reliability-semantic-pipelines`
Spec: `specs/argos-unified-reliability-semantic-pipelines-spec.md`
PR: https://github.com/xxxcess/argos-osint/pull/35

## Phase status (§18)

| Phase | Status | Notes |
| --- | --- | --- |
| 1. Preserve data | **Done** | Safe insight replace |
| 2. Shared execution | **Partial** | tasks, admission, 2/3 retries, elected scheduler, index pool |
| 3. Deadline integration | **Mostly done** | TurnClock embeds ClockSet; foreground→background + exhausted; checkpoints; sequential `depends_on` budgeting; hard-limit gates recon/tools |
| 4. Role + summary foundation | **Done** | CLI + TUI Summarization |
| 5. Summary call sites | **Mostly done** | All nine modes wired |
| 6. Automatic vectors | **Mostly done** | No 3000 barrier; generations; **shadow building tables** (`brain_memories__{gen}`); activate swaps serving pointer; live upserts still hit serving |
| 7. Evidence retrieval | **Partial** | passages + ID coverage; hybrid/ANN not started |
| 8. Remaining summary modes | **Mostly done** | Many paths still deterministic / not full background LLM flush |
| 9. Pipeline adoption | **Mostly done** | Helpers + **live Recon gap loop** via `refresh_directive_coverage` → `plan.gaps` |
| 10. Exploration / tools | **Mostly done** | `explore.rs` + picker fallback; **Brain TUI related-evidence / why-matched** labels on graph |
| 11. Verification | **Advanced partial** | Focused offline suites + `reliability_faults` (§19 slice); full matrix not closed |

## Honest remaining gaps

- Hybrid / ANN evidence retrieval (§7)
- Full background LLM flush for every summary call site (many still deterministic/`complete_summary`)
- Broader §19 matrix items not yet covered offline (e.g. concurrent multi-worker insight replace races under load, real HTTP mock providers beyond `execute_with_retries`, stream inactivity cutovers)
- Scheduler index worker pool still a skeleton relative to full elected rebuild orchestration
- Clippy `-D warnings` repo-wide still fails on pre-existing issues

## Recent gap closures (this push)

1. Dual Lance serving/building: rebuild batches write `upsert_texts_into(shadow)`; `activate_generation` points `BrainIndex` at shadow and drops prior shadow; Store open restores serving table name.
2. Recon `run_turn` / `continue_turn` call `refresh_directive_coverage` so uncovered directives land in `plan.gaps` before synthesis.
3. Brain graph pane appends bounded related-evidence lines (`explore::related_evidence_view`) with why-matched labels.
4. `reliability_faults` module: mock provider retries, ClockSet expiry, cache revision invalidation, lease fencing, non-retryable auth.
