# Unified reliability / summarization / semantic pipelines — completion checklist

Branch: `feat/unified-reliability-semantic-pipelines`
Spec: `specs/argos-unified-reliability-semantic-pipelines-spec.md`
PR: https://github.com/xxxcess/argos-osint/pull/35

## Phase status (§18)

| Phase | Status | Notes |
| --- | --- | --- |
| 1. Preserve data | **Done** | Safe insight replace; user-edit + shared-source preservation; zero-claim success |
| 2. Shared execution | **Partial** | tasks + admission + 2/3 attempts; `execute_with_retries`; elected `scheduler` lease; index worker pool skeleton; full LLM/network pools & cross-process stress still open |
| 3. Deadline integration | **Partial** | `ClockSet` + budget bridges (`clock_set_for_turn`, `tool_allowance_for_deps`); TurnClock not fully replaced in orchestrator |
| 4. Role + summary foundation | **Done** | Fifth role + inheritance; nine modes; cache get/put; CLI + TUI Defaults |
| 5. Existing summary call sites | **Mostly done** | InvestigationTitle, GraphExplanation, PageEvidence, FollowUpContext (cache+deterministic), ToolObservation digest |
| 6. Automatic vectors | **Partial** | No 3000 barrier; index-change queue; `argos_index_generations` begin/activate; batch rebuild/replay incomplete |
| 7. Shared evidence retrieval | **Partial** | `evidence.rs` passages + identifier coverage; hybrid/ANN not started |
| 8. Remaining summary modes | **Mostly done** | ReportContext + SectionDigest + AtlasBrief + ArticleDescription wired (deterministic/service path); live model paths still optional via `complete_summary` |
| 9. Pipeline adoption | **Not started** | Directive coverage / Atlas events / selective Intel refresh |
| 10. Exploration / tool routing | **Not started** | Brain semantic UX / catalog ranking fallback |
| 11. Release verification | **In progress** | Focused offline tests green; full §19 matrix incomplete |

## Mode wiring

| Mode | Call site | Status |
| --- | --- | --- |
| PageEvidence | `recon::compact_page` | Prompt + summarization secret |
| GraphExplanation | TUI `write_graph_summary` | Summarization role + validate |
| FollowUpContext | `previous_synthesis` / `compact_prior_synthesis` | Cache lookup + deterministic |
| ToolObservation | `packet_observation` | Deterministic structured digest |
| InvestigationTitle | `investigation_title` | Summarization system prompt |
| ReportContext | `intel_recon/synthesize` model packet | Deterministic digests for body/upstream |
| SectionDigest | `save_section` | Digest after analytical save |
| AtlasBrief | `atlas_insights::brief_text` | Deterministic AtlasBrief |
| ArticleDescription | `packet_json` | Deterministic when oversized |

## Honest gaps

- Live LLM Summarization for FollowUp/Tool/Atlas/Article still mostly deterministic (no background job flush yet)
- Recon orchestrator still on TurnClock; ClockSet is bridge-only
- Lance generation activate does not yet stream batch embeds into a shadow table
- Phases 9–10 not started
