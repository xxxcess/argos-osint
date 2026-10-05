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
| 6. Automatic vectors | **Partial→advanced** | No 3000 barrier; generations; batched rebuild with fingerprint mix guard; `process_pending_vector_rebuild`; full shadow-table dual-index still open |
| 7. Evidence retrieval | **Partial** | passages + ID coverage |
| 8. Remaining summary modes | **Mostly done** | |
| 9. Pipeline adoption | **Partial** | `pipeline.rs`: directive coverage, Atlas event grouping, claim compare, selective refresh |
| 10. Exploration / tools | **Partial** | `explore.rs` + picker catalog fallback helper; Brain UI wiring still open |
| 11. Verification | **In progress** | Focused offline suites green |

## Honest remaining gaps

- Dual Lance serving/building tables (current batched rebuild upserts into live index)
- Full Brain TUI “why this matched” / related-evidence UI
- Wire directive_coverage into live Recon gap loop (helpers ready)
- Full §19 fault-injection matrix
