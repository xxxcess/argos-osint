# Unified reliability / summarization / semantic pipelines — completion checklist

Branch: `feat/unified-reliability-semantic-pipelines`
Spec: `specs/argos-unified-reliability-semantic-pipelines-spec.md`

## Phase status

| Phase | Status | Notes |
| --- | --- | --- |
| 1. Preserve data (safe insight replace) | Done | Stage/validate before delete; single TX commit; user-edit preservation |
| 2. Shared execution | Partial | `tasks.rs` SQLite jobs/tasks/attempts/leases + admission + retry caps; provider typed categories in `provider_request.rs`. Full worker pools / cross-process scheduler ownership still open |
| 3. Deadline integration | Partial | `recon/clocks.rs` separate clocks; Recon orchestrator not fully switched yet |
| 4. Role + summary foundation | Done | Fifth role + inheritance; `summarization.rs` nine modes + validators + cache key |
| 5. Existing summary call sites | Partial | InvestigationTitle + GraphExplanation (TUI→summarization role) + PageEvidence (compact_page) wired; FollowUpContext still deterministic fallback |
| 6. Automatic vectors | Partial | Removed 3000 sync barrier; large rebuilds enqueue `argos_index_changes`; claim/complete helpers; generation activate/replay incomplete |
| 7. Shared evidence retrieval | Partial | `evidence.rs` passage chunking + identifier coverage; hybrid/ANN not started |
| 8. Remaining summary modes | Partial | Modes defined; ReportContext/SectionDigest/AtlasBrief/ArticleDescription/ToolObservation/FollowUpContext/PageEvidence/GraphExplanation call-site wiring incomplete |
| 9. Pipeline adoption | Not started | |
| 10. Exploration / tool routing | Not started | |
| 11. Release verification | In progress | Focused unit tests for landed slices; full §19 matrix incomplete |

## Acceptance criteria mapping (landed tests)

- AC1–2, user-edit preserve: `intel_recon/replace_insights.rs` tests
- Attempt caps 2/3 + admission 2: `tasks.rs` tests
- Nine modes / validators / cache key: `summarization.rs` tests
- Schema v18 + tasks tables: store open / migrate
- HTTP categorization: `provider_request.rs` tests
- Clock separation: `recon/clocks.rs` tests
- Role inheritance: provider `role_secret("summarization")` + CLI `defaults show`

Mark unfinished §19 items explicitly in the PR body until green.
