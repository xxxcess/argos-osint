# Graph Report - argos-osint  (2026-10-07)

## Corpus Check
- 161 files · ~360,864 words
- Verdict: corpus is large enough that graph structure adds value.
- Unclassified: 9 file(s) not represented in the graph (top: (none) 3, .toml 2, .diff 2)

## Summary
- 5821 nodes · 14403 edges · 212 communities (187 shown, 25 thin omitted)
- Extraction: 98% EXTRACTED · 2% INFERRED · 0% AMBIGUOUS · INFERRED: 274 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `15168fee`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- land.rs
- orchestrate.rs
- App
- atlas_memory.rs
- tool_io.rs
- .set_focus
- brain_lance.rs
- FieldId
- map.rs
- recon.rs
- ButtonId
- Store
- IntelligenceCategory
- job_registry.rs
- Call
- directives.rs
- .clear
- absorb_hit
- providers.rs
- atlas_news.rs
- Store
- publication.rs
- gates.rs
- .new
- news_legal.rs
- hardware.rs
- ui.rs
- Store
- embed.rs
- osint.rs
- execute_steps
- .handle_key
- provider.rs
- run_turn
- InvestigationPart
- body.rs
- rule_bindings
- brain_detail.rs
- recon/graph.rs
- atlas.rs
- run_atlas_inner
- contains
- scheduler.rs
- jobs_view.rs
- provider_attempt.rs
- SettingsFile
- picker.rs
- ReportMode
- graph_explanation.rs
- reliability_faults.rs
- graph_explanation/tests.rs
- unix_now
- events.rs
- exec.rs
- home_rows
- ProviderSecret
- ErrorCategory
- grok_oauth.rs
- MemoryKind
- Rect
- brain_resources.rs
- worker.rs
- Target
- src/brain.rs
- intel_recon/jobs.rs
- cli.rs
- atlas_insights.rs
- InformationCredibility
- Category
- body_filter.rs
- synthesize.rs
- modes.rs
- secrets.rs
- rusqlite
- theme.rs
- Overlay
- pipeline.rs
- summarization.rs
- InvestigationSurface
- LogsView
- ScheduledCall
- tui/jobs.rs
- .new
- FeedArticle
- budget.rs
- store.rs
- Command
- WorkEvent
- logs.rs
- src/evidence.rs
- .begin_title
- anyhow
- enqueue_index_work
- F
- wikipedia_rsp.rs
- TurnClock
- How
- H3 Task Packet: State and Persistence Foundation
- summary_card.rs
- Line
- ModuleId
- Region
- explain
- validate.rs
- measure_ann_recall
- all_tools
- .order
- TurnEvent
- TimeoutProfile
- Architecture
- .handle_mouse
- .evaluate
- ClockSet
- ProviderAdmission
- Phase Plan: Home Recon Composer and Investigation Tabs
- AtlasJob
- gsd-v2.js
- AnnPolicy
- briefing_view.rs
- subscription.rs
- enqueue_job_with
- cursor_at
- chat_model
- mem
- ReconCommand
- JobRow
- RouteInput
- poll_device
- Value
- agent
- Functional Requirements
- .on_esc
- super
- ChatBlock
- TaskState
- OsintCommand
- .new
- the_label_updates_when_a_round_adds_calls_and_when_evidence_is_measured
- Argos OSINT — Agent Instructions
- Text
- TaskStatus
- RspStatus
- tasks.rs
- DecisionState
- ToolResult
- RecordKind
- Store
- ClaimRelation
- results.rs
- Store
- ledger.rs
- ProviderPage
- split_vertical
- §9 implementation order
- TurnContinuation
- investigation/evidence.rs
- InvestigationPattern
- Part
- argos-osint-bin
- What Was Learned
- investigation.rs
- Milestone 1 — Core Onboarding (Current)
- paths.rs
- HypothesisRecord
- CycleOutcome
- H2 Contracts: Frozen Interface Decisions
- youtube_pair
- Codebase Map — argos-osint
- PROJECT.md — argos-osint
- Current Phase State
- ReconLimits
- RecoveryAction
- RspIndex
- Docs Ingest — argos-osint
- DecisionAdapterKind
- OpenCode V2 workflow
- Unified reliability / summarization / semantic pipelines — completion checklist
- package.json
- serde
- app.rs
- subject_of
- Argos Adaptive Decision Roles and Strict Output Contracts
- H0-H2 Completion Summary
- Argos OSINT — Unified Investigation Harness
- .default
- json
- Value
- SourceReliability
- select_strategy
- PlanInterval
- IndexOutcome
- Argos UI Interaction Audit
- run_live
- classify_resource
- InsightSource

## God Nodes (most connected - your core abstractions)
1. `App` - 284 edges
2. `ProviderSecret` - 123 edges
3. `ButtonId` - 113 edges
4. `FieldId` - 71 edges
5. `Store` - 65 edges
6. `Target` - 64 edges
7. `ToolResult` - 62 edges
8. `Store` - 61 edges
9. `AtlasArticleRow` - 51 edges
10. `run_atlas_inner()` - 46 edges

## Surprising Connections (you probably didn't know these)
- `every_tool_request_sends_a_non_empty_user_agent()` --references--> `agent`  [INFERRED]
  crates/argos-osint-core/src/osint.rs → .opencode/opencode.json
- `success_is_keyed_and_any_input_change_invalidates_it()` --references--> `edit`  [INFERRED]
  crates/argos-osint-core/src/graph_explanation/tests.rs → .opencode/opencode.json
- `worker_killed_after_the_lance_write_is_recovered_by_revision()` --references--> `edit`  [INFERRED]
  crates/argos-osint-core/src/store/publication.rs → .opencode/opencode.json
- `create_report_job()` --calls--> `body_assertion_candidates()`  [INFERRED]
  crates/argos-osint-core/src/intel_recon/jobs.rs → crates/argos-osint-core/src/intel_recon/ledger.rs
- `paywall_and_snippet_fail_validation()` --calls--> `validate_article_body()`  [INFERRED]
  crates/argos-osint-core/src/intel_recon/tests_acceptance.rs → crates/argos-osint-core/src/intel_recon/validate.rs

## Import Cycles
- 2-file cycle: `crates/argos-osint-core/src/osint.rs -> crates/argos-osint-core/src/osint/results.rs -> crates/argos-osint-core/src/osint.rs`
- 2-file cycle: `crates/argos-osint-bin/src/cli.rs -> crates/argos-osint-core/src/hardware.rs -> crates/argos-osint-bin/src/cli.rs`

## Communities (212 total, 25 thin omitted)

### Community 0 - "land.rs"
Cohesion: 0.01
Nodes (262): CODES, LABELS, R0, R1, R10, R100, R101, R102 (+254 more)

### Community 1 - "orchestrate.rs"
Cohesion: 0.06
Nodes (45): a_follow_up_keeps_names_from_the_previous_synthesis(), a_recon_model_429_trips_the_gate_and_later_calls_skip_the_provider(), ac6_a_courtlistener_429_still_lets_the_turn_synthesize(), ac7_a_pronoun_follow_up_takes_the_thread_subject(), ACCOUNT_TOOLS, ACME, action_call(), binding_ground() (+37 more)

### Community 2 - "App"
Cohesion: 0.05
Nodes (6): App, intel_day_button_label(), open_external_url(), pruned_history_closes_the_open_run(), IntelBody, IntelReport

### Community 3 - "atlas_memory.rs"
Cohesion: 0.05
Nodes (100): Extraction, InsightStats, Settled, atlas_memory_count(), background_indexing_then_refresh_upgrades_a_partial_run(), Barrier, Checkpoint, CheckpointInput (+92 more)

### Community 4 - "tool_io.rs"
Cohesion: 0.04
Nodes (90): Binding, canonical(), dependency_order(), depends_on(), question_bindings_and_dependency_fix(), accept_bindings(), allowed_producer(), best_handle() (+82 more)

### Community 5 - ".set_focus"
Cohesion: 0.09
Nodes (56): a_click_folds_an_event_and_incoming_entries_do_not_move_a_paused_list(), a_finished_tool_call_is_logged_once_and_the_transcript_keeps_a_summary(), all_provider_actions_render_with_hit_areas_at_80x24(), atlas_auto_toggle_persists_the_next_trigger(), atlas_headlines_scroll_and_request_failures_reach_the_system_log(), atlas_history_resume_is_offered_only_for_resumable_cycles_and_repair_starts_once(), atlas_live_feed_opens_intel_brief_and_past_runs_delete(), atlas_map_names_every_highlighted_country() (+48 more)

### Community 6 - "brain_lance.rs"
Cohesion: 0.06
Nodes (38): activate_generation(), batch(), begin_generation(), begin_generation_then_activate(), block_on(), BrainIndex, clear_fingerprint(), current_fingerprint() (+30 more)

### Community 7 - "FieldId"
Cohesion: 0.04
Nodes (55): FieldId, BrainApp, BrainConversation, BrainInsight, BrainQuery, ClaimAssessorModel, ClaimAssessorProvider, ClassifierModel (+47 more)

### Community 8 - "map.rs"
Cohesion: 0.05
Nodes (75): BRAILLE_MAP, canonical(), claim_label(), contains(), country_at(), country_fit(), country_pixel(), draw_country_mini_map() (+67 more)

### Community 9 - "recon.rs"
Cohesion: 0.05
Nodes (57): answer_step_rate_limit_is_explicit_and_keeps_results(), attribute_word(), auxiliary(), BRIEF_SYNTHESIS, broad_question_collapses_to_grounded_lookups(), char_ceil(), char_floor(), COMPACT_SUMMARY_CHARS (+49 more)

### Community 10 - "ButtonId"
Cohesion: 0.02
Nodes (91): ButtonId, Add, AtlasAuto, AtlasDelete, AtlasLive, AtlasNews, AtlasNewsFeed, AtlasRepair (+83 more)

### Community 11 - "Store"
Cohesion: 0.09
Nodes (3): CreditHold, Store, strategy_and_provider_credits_survive_reopen()

### Community 12 - "IntelligenceCategory"
Cohesion: 0.09
Nodes (25): test_all_catalog_tools_mapped_to_categories(), cache_follows_the_provider_plan_interval(), all_59_tools_mapped_to_categories(), argument_contract(), ArgumentBuilderContract, compact_capability_catalog(), CompactToolCapability, IntelligenceCategory (+17 more)

### Community 13 - "job_registry.rs"
Cohesion: 0.05
Nodes (46): begin(), begin_cancellable(), db(), db_path(), finish(), register_canonical(), beat(), BEAT_INTERVAL (+38 more)

### Community 14 - "Call"
Cohesion: 0.11
Nodes (30): build_blocks(), call_stamp(), clip_chars(), coverage_only_counts_explicit_assessments(), coverage_summary(), decision_row_shows_directives_picker_order_bindings_and_fallbacks(), extract_log(), footer_line() (+22 more)

### Community 15 - "directives.rs"
Cohesion: 0.06
Nodes (64): apply_context_targets(), asks(), asks_about_judge(), asks_for_headlines(), clip_words(), context_entity(), CONTEXT_FILLER, context_gate() (+56 more)

### Community 16 - ".clear"
Cohesion: 0.10
Nodes (9): draft_isolation_and_persistence(), duplicate_submission_prevention(), PaletteItem, session_tabs_open_close_reopen(), single_action_launch_from_home(), tab_strip_render_and_hit_test(), IntelArticle, IntelTab (+1 more)

### Community 17 - "absorb_hit"
Cohesion: 0.12
Nodes (22): absorb_hit(), account_platform_host(), Candidate, content_tokens(), distinct_queries(), domain_label(), EntityIdentifier, first_domain() (+14 more)

### Community 18 - "providers.rs"
Cohesion: 0.05
Nodes (71): bitcoin(), bounded(), clip_page(), domain(), email_address(), ip(), linkedin_handle(), number_arg() (+63 more)

### Community 19 - "atlas_news.rs"
Cohesion: 0.07
Nodes (45): api_error(), ATLAS_TOOLS, authors(), clip(), country_arg(), country_code(), CURRENTS_COUNTRIES, CURRENTS_HOST (+37 more)

### Community 20 - "Store"
Cohesion: 0.07
Nodes (18): article_body_from_row(), ArticleBodyRow, element_from_row(), IntelAssessmentRow, IntelEvidenceRow, IntelInvestigationRow, IntelReportJobRow, IntelReportSectionRow (+10 more)

### Community 21 - "publication.rs"
Cohesion: 0.08
Nodes (41): Phase5Report, bump_memories_changed(), change_watcher_sees_only_committed_changes_and_coalesces(), claim(), clear(), clear_index_state(), CoverageReport, delete_run_memory_state() (+33 more)

### Community 22 - "gates.rs"
Cohesion: 0.09
Nodes (12): execute_harness_step(), InvestigationRuntime, GateOutcome, Passed, Rejected, validate_evidence_admission(), validate_publication(), validate_task_admission() (+4 more)

### Community 23 - ".new"
Cohesion: 0.21
Nodes (11): answer_evidence_survives_reopen_and_deleted_thread_rejects_late_calls(), cited(), claims_deduplicate_and_reject_unsupported_sources(), deleting_an_investigation_removes_its_brain_memories_and_graph_summary(), investigation_memory_ids(), now(), persist_claims(), persistence_and_plan() (+3 more)

### Community 24 - "news_legal.rs"
Cohesion: 0.07
Nodes (47): a_rate_limit_switches_to_the_fallback_key_and_a_rejection_does_not(), ac1_catalog_has_55_tools_and_the_news_and_legal_entries(), ac1_each_request_has_its_url_params_and_auth_header_and_no_key_in_the_url(), an_echoed_key_is_redacted_from_the_stored_body_and_error(), article(), articles(), CATEGORIES, choice_arg() (+39 more)

### Community 25 - "hardware.rs"
Cohesion: 0.16
Nodes (17): CACHE_TTL_SECS, classify_arch(), disk_totals(), HardwareProfile, metal_vram_gb(), now_secs(), parse_apple_gpu_cores(), partial_gpu() (+9 more)

### Community 26 - "ui.rs"
Cohesion: 0.05
Nodes (93): ACTION_H, active_popup_area(), atlas_auto_label(), atlas_countdown(), atlas_cycle_stats_room(), atlas_cycle_stats_scroll_max(), atlas_extracting(), atlas_feed_room() (+85 more)

### Community 27 - "Store"
Cohesion: 0.07
Nodes (10): stats_from_stored(), atlas_answer_id(), atlas_brief_id(), AtlasRunRow, AtlasStoredClaim, GraphSummary, has_table(), repair_embed_tables() (+2 more)

### Community 28 - "embed.rs"
Cohesion: 0.07
Nodes (33): record_start(), start_repair(), active(), DIM, disable(), disabled(), DisableGuard, download_file() (+25 more)

### Community 29 - "osint.rs"
Cohesion: 0.06
Nodes (62): a_claimed_email_keeps_no_person_data(), annotate(), bind_request(), CACHE_DAY_SECONDS, CACHE_MONTH_SECONDS, cache_seconds(), CACHE_WEEK_SECONDS, canonical_tool_id() (+54 more)

### Community 30 - "execute_steps"
Cohesion: 0.16
Nodes (28): after_step(), apply_order(), context_block(), context_dispatched(), directive_for(), execute_steps(), expand_per_platform(), extract_bindings() (+20 more)

### Community 31 - ".handle_key"
Cohesion: 0.13
Nodes (5): ctrl_arrows_cycle_app_tabs_forward_and_back(), ctrl_tab_cycles_app_tabs_forward_and_back(), is_picker_field(), multiline_paste_stays_in_composer_and_does_not_run_shortcuts(), provider_label()

### Community 32 - "provider.rs"
Cohesion: 0.05
Nodes (62): chat_body(), ChatMessage, complete(), complete_errors_with_finish_reason_when_response_is_empty(), complete_once(), complete_reads_reasoning_only_non_stream_json(), complete_stream_of_reasoning_deltas_yields_text(), concrete_free_models() (+54 more)

### Community 33 - "run_turn"
Cohesion: 0.15
Nodes (25): call_cached(), cancelled(), classify_turn_mode(), continue_turn(), derive_directives(), derived_note(), enabled_tools(), execute_ordered() (+17 more)

### Community 34 - "InvestigationPart"
Cohesion: 0.10
Nodes (17): event_to_chat_block(), is_thinking_expanded(), InvestigationPart, DirectiveAssessment, EvidencePassage, GateValidation, Handoff, Plan (+9 more)

### Community 35 - "body.rs"
Cohesion: 0.09
Nodes (35): body_fetch_routes(), BodyFetchEvent, Attempt, Failed, InsightsFailed, InsightsRefreshing, InsightsReplaced, Progress (+27 more)

### Community 36 - "rule_bindings"
Cohesion: 0.10
Nodes (33): binder_maps_kinds_to_inputs_and_never_hands_hunter_a_social_host(), binding(), normalize_platform(), question_bindings(), a_gap_filler_only_domain_waits_for_a_primary_observation_before_hunter(), batch_scrape_takes_ranked_urls_and_gates_order_before_hunter(), bind_arguments(), bitcoins_in() (+25 more)

### Community 37 - "brain_detail.rs"
Cohesion: 0.06
Nodes (45): areas(), BrainDetail, DetailAreas, DetailPane, Path, Related, Summary, DetailSnapshot (+37 more)

### Community 38 - "recon/graph.rs"
Cohesion: 0.06
Nodes (59): draw(), draw_path(), draw_summary(), glyph(), inset(), legend_height(), legend_parts(), legend_rows() (+51 more)

### Community 39 - "atlas.rs"
Cohesion: 0.09
Nodes (29): article_card_labels_title_publisher_author_and_classification(), Band, band_sizes(), band_temperature(), category_from_decisions(), CATEGORY_IDS, CLUSTERS, country_label() (+21 more)

### Community 40 - "run_atlas_inner"
Cohesion: 0.09
Nodes (36): article_from_row(), AtlasEvent, Article, Classified, InsightProgress, MemoriesChanged, MemoryProgress, Note (+28 more)

### Community 41 - "contains"
Cohesion: 0.16
Nodes (21): atlas_hit(), atlas_row_at(), contains(), draw_system(), focus_order(), hit_test(), home_line(), intel_body_loading() (+13 more)

### Community 42 - "scheduler.rs"
Cohesion: 0.09
Nodes (22): apply_index_change(), apply_index_change_rebuild_reports_outcome(), apply_index_with(), DEFAULT_LEASE_SECS, drain_index_once(), drain_summary_flush(), drain_summary_flush_cached(), drain_summary_flush_completes_cached_tasks() (+14 more)

### Community 43 - "jobs_view.rs"
Cohesion: 0.11
Nodes (21): summary_text(), active_time_is_live_for_open_attempts_and_unavailable_only_without_a_start(), AttemptRow, event_apps(), event_counts(), EventCounts, get_job(), job() (+13 more)

### Community 44 - "provider_attempt.rs"
Cohesion: 0.11
Nodes (38): Acc, attempt(), AttemptReport, check_final(), complete_stream_and_json_answers_succeed(), Deadlines, delta(), fill() (+30 more)

### Community 45 - "SettingsFile"
Cohesion: 0.05
Nodes (26): LogicalRole, .ALL, ClaimAssessor, Classifier, Controller, EntityResolver, EvidenceCurator, Planner (+18 more)

### Community 46 - "picker.rs"
Cohesion: 0.11
Nodes (28): brain_scrape_options_hide_bare_scrape_and_include_claim_criteria(), CatalogEntry, chat_request(), CONFIDENCE_FLOOR, decisions_request(), directives_phrase(), DONE, eligible_catalog() (+20 more)

### Community 47 - "ReportMode"
Cohesion: 0.12
Nodes (22): HomeDraftState, classifiable_modes(), classify_mode_chat(), classify_mode_decisions(), classify_prompt_mode(), classify_prompt_mode_chat(), classify_prompt_mode_decisions(), classify_recon_mode() (+14 more)

### Community 48 - "graph_explanation.rs"
Cohesion: 0.12
Nodes (15): basic_explanation(), BASIC_HEADING, Cached, None, Stale, Valid, FAILURE_COOLDOWN, Faults (+7 more)

### Community 49 - "reliability_faults.rs"
Cohesion: 0.10
Nodes (9): AttemptOutcome, categorize_http(), execute_with_retries(), other_llm_allows_three(), summarization_stops_at_two_attempts(), cache_invalidates_when_source_revision_changes(), mock_provider_other_llm_allows_three_attempts(), mock_provider_recovers_after_transient_error() (+1 more)

### Community 50 - "graph_explanation/tests.rs"
Cohesion: 0.27
Nodes (16): auth_failure_sends_one_request_gives_guidance_and_redacts(), conn(), count(), failure_is_durable_correlated_bounded_and_harmless(), fixture(), Fx, GOOD, late_completion_for_a_changed_or_deleted_memory_is_not_published() (+8 more)

### Community 51 - "unix_now"
Cohesion: 0.22
Nodes (7): atlas_auto_future_tick_waits_and_past_tick_reschedules(), atlas_auto_while_running_only_arms_the_next_slot(), atlas_history_button_counts_down_while_auto_run_is_on(), atlas_poll_wait(), manual_run_moves_the_next_auto_trigger_out_by_90_minutes(), unix_now(), Atlas

### Community 52 - "events.rs"
Cohesion: 0.09
Nodes (27): starts(), infer_app(), session_event(), clear_events(), DEFAULT_RETENTION_HOURS, EventFilter, get_event(), job_filter_includes_descendants_and_prune_keeps_jobs() (+19 more)

### Community 53 - "exec.rs"
Cohesion: 0.12
Nodes (22): complete_summary(), admission_account(), admission_contention_consumes_no_attempt(), AttemptEvent, Finished, Started, AttemptLog, cancelled() (+14 more)

### Community 54 - "home_rows"
Cohesion: 0.12
Nodes (26): center_row(), composer_parts(), cursor_blink_visible(), draw_composer(), gap_row(), home_composer_and_all_content_centered_vertically_and_horizontally(), home_composer_areas(), home_group() (+18 more)

### Community 55 - "ProviderSecret"
Cohesion: 0.18
Nodes (22): account_secret(), active_text_secret(), authorize(), bearer_token(), catalog_error(), effective_kind(), http(), list_catalog() (+14 more)

### Community 56 - "ErrorCategory"
Cohesion: 0.08
Nodes (24): operation_kind(), backoff_delay(), can_retry(), ErrorCategory, AuthOrQuota, CancelledOrStale, ConfigurationMissing, ContextLimit (+16 more)

### Community 57 - "grok_oauth.rs"
Cohesion: 0.15
Nodes (20): auth_path(), bearer(), bearer_from_path(), check_login(), Entry, entry_from(), failed_login_keeps_the_cli_reason_and_recovery_action(), login() (+12 more)

### Community 58 - "MemoryKind"
Cohesion: 0.09
Nodes (22): AdmissionPolicy, admit_memory(), BrainQuery, pack_context(), AssessmentState, Contradicts, Insufficient, Mentions (+14 more)

### Community 59 - "Rect"
Cohesion: 0.13
Nodes (58): intel_category_short(), Block, draw(), button_areas(), center_text(), cover(), draw(), draw_atlas() (+50 more)

### Community 60 - "brain_resources.rs"
Cohesion: 0.10
Nodes (26): bindings_use_brain_evidence_ids(), BRAIN_SCRAPE_PREFIX, BrainResourceHit, BrainResourceSummary, candidate_json(), CLAIM_CHARS, clip(), format_counts() (+18 more)

### Community 61 - "worker.rs"
Cohesion: 0.15
Nodes (19): IntelReportEvent, InsightsUpdated, JobCreated, JobDone, Section, Stage, cancel_if_needed(), claim_relevant_evidence() (+11 more)

### Community 62 - "Target"
Cohesion: 0.06
Nodes (34): brain_article_source_opens_intel_brief(), FocusEntry, hit(), IntelReconFocus, Section, Start, Tab, LayoutRegistry (+26 more)

### Community 63 - "src/brain.rs"
Cohesion: 0.12
Nodes (21): AGREEMENT_WEIGHT, CATEGORIES, category_hint(), format_injection(), hybrid_recall(), hybrid_recall_adds_paraphrases_and_rewards_agreement(), identity_query_prefers_name_memory(), jaccard() (+13 more)

### Community 64 - "intel_recon/jobs.rs"
Cohesion: 0.15
Nodes (18): active_job_for_mode(), cancel_job(), create_job_inserts_all_verify_sections(), create_report_job(), finish_canonical(), intel_recon_links_its_legacy_job_to_one_canonical_registry_job(), pause_job(), ReportScope (+10 more)

### Community 65 - "cli.rs"
Cohesion: 0.18
Nodes (18): ask(), ask_thread(), atlas_command(), defaults_command(), defaults_json(), defaults_show_includes_the_tool_picker_transport(), dispatch(), login() (+10 more)

### Community 66 - "atlas_insights.rs"
Cohesion: 0.05
Nodes (101): category_tag(), a_country_token_does_not_merge_into_a_longer_name(), a_decisions_model_does_not_extract_claims(), a_description_only_span_is_inference_and_an_added_name_is_dropped(), a_fact_requires_both_spans_in_the_title(), accept_claims(), accept_one(), AcceptMode (+93 more)

### Community 67 - "InformationCredibility"
Cohesion: 0.13
Nodes (17): AdmiraltyCode, best_credibility(), CredibilityInputs, information_credibility(), InformationCredibility, CannotBeJudged, Confirmed, DoubtfullyTrue (+9 more)

### Community 68 - "Category"
Cohesion: 0.05
Nodes (45): bounded(), Category, Auth, Cancelled, Configuration, Empty, InvalidModel, InvalidResult (+37 more)

### Community 69 - "body_filter.rs"
Cohesion: 0.17
Nodes (22): BATCH_SIZE, BodyChunk, chunk_article_body(), CHUNK_MAX, CHUNK_MIN, CHUNK_TARGET, chunks_preserve_absolute_offsets(), classify_chunk_batch() (+14 more)

### Community 70 - "synthesize.rs"
Cohesion: 0.17
Nodes (19): BODY_REFINE_INPUT_CHARS, deterministic_bluf_mentions_title(), deterministic_section(), model_refine_body(), model_synthesize(), parse_json_object(), refine_retrieved_article_body(), refine_without_secret_keeps_validated_scrape() (+11 more)

### Community 71 - "modes.rs"
Cohesion: 0.28
Nodes (11): scoped_section_plan(), chat_response_spec(), chat_response_spec_lists_mode_headings(), chat_section_titles(), every_mode_has_bluf_first_and_sources_last(), investigation_mode_spec(), investigation_mode_spec_guides_directives_and_picker(), normalize_chat_mode() (+3 more)

### Community 72 - "secrets.rs"
Cohesion: 0.16
Nodes (6): accounts_persist_with_owner_only_permissions(), AuthFile, legacy_connection_migrates_without_overwriting_other_accounts(), loading_old_auth_removes_gmail_setup_credentials(), owner_only(), write_private()

### Community 73 - "rusqlite"
Cohesion: 0.13
Nodes (11): record_detail_lines(), ExplanationRecord, GraphSummaryEntry, migrate(), put_record(), SaveOutcome, MemoryChanged, MemoryGone (+3 more)

### Community 74 - "theme.rs"
Cohesion: 0.08
Nodes (33): AccountHealth, AuthRequired, Cooldown, CreditsExhausted, Overloaded, Ready, Restricted, Unverified (+25 more)

### Community 75 - "Overlay"
Cohesion: 0.17
Nodes (12): ChoiceKind, IntelDay, Investigation, Model, Provider, Overlay, Choice, Help (+4 more)

### Community 76 - "pipeline.rs"
Cohesion: 0.18
Nodes (19): apply_recon_directive_coverage(), compare_claims(), coverage_requires_cited_evidence_not_similarity(), directive_coverage(), DirectiveCoverage, event_grouping_keeps_separate_days_apart(), EventGroup, group_atlas_events() (+11 more)

### Community 77 - "summarization.rs"
Cohesion: 0.10
Nodes (35): cache_get(), cache_key_changes_with_revision_and_focus(), cache_put(), cache_round_trip(), CoverageMeta, deterministic_article_description(), deterministic_atlas_brief(), deterministic_follow_up() (+27 more)

### Community 78 - "InvestigationSurface"
Cohesion: 0.09
Nodes (12): InvestigationSurface, HomeComposer, IntelBrief, JobsResume, ReconChat, atlas_only_tools_blocked_on_all_surfaces(), check_need_gate(), sociavault_blocked_on_intel_surface() (+4 more)

### Community 79 - "LogsView"
Cohesion: 0.10
Nodes (10): LevelFilter, All, Error, Info, Warn, LogsView, row_lines(), detail_rows() (+2 more)

### Community 80 - "ScheduledCall"
Cohesion: 0.29
Nodes (9): clock_set_for_turn(), courtlistener_spacing_and_firecrawl_polling_are_counted(), live(), more_calls_mean_more_tool_time_and_cache_hits_add_nothing(), scheduled(), ScheduledCall, tool_allowance_for_deps(), tool_allowance_for_deps_sequential_sums() (+1 more)

### Community 81 - "tui/jobs.rs"
Cohesion: 0.09
Nodes (23): areas(), BASE_COLUMNS, button_label(), buttons(), detail_lines(), hit(), JobsAreas, JobsView (+15 more)

### Community 82 - ".new"
Cohesion: 0.26
Nodes (18): a_spent_primary_quota_uses_the_fallback_key(), country_labels_show_the_name_and_the_code(), daily_cap(), format_run_card(), HttpCall, HttpReply, insights_do_not_call_a_decisions_model(), keys() (+10 more)

### Community 83 - "FeedArticle"
Cohesion: 0.19
Nodes (17): apply_hits(), article_from(), article_row(), canonical_url(), dedup_drops_a_url_an_exact_title_and_keeps_the_higher_domain(), FeedArticle, host_of(), judge() (+9 more)

### Community 84 - "budget.rs"
Cohesion: 0.11
Nodes (15): CUT_NOTE, CUT_SHORT, deadline_seconds(), PHASE_WINDOW_SECONDS, RECON_DEADLINE, RECON_ROUND_SECONDS, STREAM_LOST, synthesis_allowance_capped() (+7 more)

### Community 85 - "store.rs"
Cohesion: 0.08
Nodes (32): article(), atlas_article_from_row(), atlas_articles_for_intel_dedupes_same_article_id_across_runs(), atlas_articles_for_intel_filters_category_day_and_query(), atlas_auto_next_round_trips_through_app_state(), atlas_claims_for_article_returns_linked_claims(), atlas_prune_drops_finished_runs_older_than_the_cutoff(), atlas_recent_articles_lists_retained_rows() (+24 more)

### Community 86 - "Command"
Cohesion: 0.09
Nodes (23): AtlasCommand, Repair, Resume, Verify, Cli, Command, Atlas, Defaults (+15 more)

### Community 87 - "WorkEvent"
Cohesion: 0.06
Nodes (31): ChoiceItem, codex_models(), DefaultsRole, .ALL, ClaimAssessor, Classifier, EntityResolver, EvidenceCurator (+23 more)

### Community 88 - "logs.rs"
Cohesion: 0.19
Nodes (15): stamp(), areas(), button_label(), buttons(), count(), draw(), hit(), in_list() (+7 more)

### Community 89 - "src/evidence.rs"
Cohesion: 0.20
Nodes (14): AGREEMENT_WEIGHT, chunk_text(), chunks_cover_end_of_long_source(), content_hash(), ensure_identifier_coverage(), EvidencePassage, hybrid_bounds_and_prefers_agreement(), hybrid_passage_candidates() (+6 more)

### Community 90 - ".begin_title"
Cohesion: 0.22
Nodes (5): clean_investigation_title(), fallback_investigation_title(), investigation_title(), investigation_titles_drop_labels_and_stay_short(), Thread

### Community 91 - "anyhow"
Cohesion: 0.10
Nodes (25): compile_general_model_prompt(), compile_native(), parse_general_model_response(), parse_native_response(), DecisionContract, template_claim_relation(), template_directive_alignment(), template_entity_binding() (+17 more)

### Community 92 - "enqueue_index_work"
Cohesion: 0.36
Nodes (8): adopt_untracked_index_changes(), enqueue_index_work(), enqueue_task(), ensure_service_job(), index_outbox_coalesces_and_rolls_back_with_its_transaction(), index_work_key(), NewTask, normalized_index_operation()

### Community 93 - "F"
Cohesion: 0.17
Nodes (17): Fault, CallSpec, classification_request(), dispatch(), fetch_with_spare_key(), json_message(), one_line(), parse_fault() (+9 more)

### Community 94 - "wikipedia_rsp.rs"
Cohesion: 0.15
Nodes (16): API, APP_STATE_KEY, CACHE_TTL, index_to_json(), normalize_host(), observation_marks_unlisted(), parse_last_year(), parse_rsp_wikitext() (+8 more)

### Community 96 - "How"
Cohesion: 0.11
Nodes (18): How, CompanyEmail, Coordinates, DomainAsUrl, EntityPhrase, PackageName, PackageParts, Plain (+10 more)

### Community 97 - "H3 Task Packet: State and Persistence Foundation"
Cohesion: 0.07
Nodes (26): Acceptance Checks and Exact Existing Test Targets:, Current Diff / Prior Changes to Preserve:, Effective Agent / Model / Variant:, Exact Existing Test Targets to Reuse/Extend:, Existing APIs / Data Models to Reuse:, Expected Changed Files:, Expected Commands:, Files This Worker Owns: (+18 more)

### Community 98 - "summary_card.rs"
Cohesion: 0.32
Nodes (11): actions(), button_at(), button_cells(), button_row_area(), draw(), height(), is_card_button(), label() (+3 more)

### Community 99 - "Line"
Cohesion: 0.13
Nodes (25): abs_contains(), abs_rect(), AbsRect, draw_clipped_button(), draw_clipped_md_pane(), draw_intel_body_loading(), intel_brief_full_lines(), intel_brief_preview_lines() (+17 more)

### Community 100 - "ModuleId"
Cohesion: 0.15
Nodes (16): ModuleId, .ALL, Atlas, Brain, Intel, Jobs, Logs, Osint (+8 more)

### Community 101 - "Region"
Cohesion: 0.11
Nodes (19): Region, AtlasFeed, AtlasInsights, AtlasNews, AtlasOrigins, AtlasRuns, Chat, Detail (+11 more)

### Community 102 - "explain"
Cohesion: 0.18
Nodes (8): event(), explain(), ExplainOutcome, Failed, Saved, Superseded, ExplainReport, ExplainRequest

### Community 103 - "validate.rs"
Cohesion: 0.19
Nodes (15): BLOCK_MARKERS, BodyQuality, Complete, Partial, Unavailable, Uncertain, BodyValidation, clean_markdown() (+7 more)

### Community 104 - "measure_ann_recall"
Cohesion: 0.33
Nodes (5): AnnMeasurement, AnnThresholds, decide_ann_policy(), measure_ann_recall(), measure_ann_recall_justifies_only_when_thresholds_met()

### Community 105 - "all_tools"
Cohesion: 0.16
Nodes (28): a_429_stops_picker_calls_and_the_fallback_finishes_the_list(), a_dispatch_error_fails_the_step_and_the_loop_goes_on(), a_failed_email_finder_gets_one_fallback_and_no_third_picker_call(), a_fallback_with_no_bindings_still_runs_the_satisfied_steps_and_reaches_synthesis(), a_question_handle_fills_the_handle_steps_without_a_fallback(), all_tools(), an_attribute_question_names_the_person_and_drops_qa_hosts(), an_imperative_social_prompt_names_the_person_not_the_sentence() (+20 more)

### Community 106 - ".order"
Cohesion: 0.25
Nodes (8): checked_serves(), context_additions(), context_tools(), fallback_record(), OrderContext, Picker<'a>, serves_for(), serving()

### Community 107 - "TurnEvent"
Cohesion: 0.14
Nodes (14): AnswerContext, await_completion(), finish_recon_job(), investigation_jobs_finish_truthfully_and_link_their_run(), deltas(), Run, RunLimits, Streamed (+6 more)

### Community 108 - "TimeoutProfile"
Cohesion: 0.29
Nodes (6): TimeoutProfile, .CLASSIFIER, .EXTRACTION, .PLANNING, .SUMMARIZATION, .SYNTHESIS

### Community 109 - "Architecture"
Cohesion: 0.07
Nodes (24): Admiralty source evaluation (WP:RSP), Architecture, Atlas, Investigation flow, Model and UI boundaries, News and Legal context tools (#29), Persistence, Primary providers (+16 more)

### Community 111 - ".evaluate"
Cohesion: 0.16
Nodes (11): DecisionService, DecisionPolicy, EnforcementOutcome, Pass, Reject, Unavailable, Uncertain, ThresholdProfile (+3 more)

### Community 112 - "ClockSet"
Cohesion: 0.23
Nodes (3): ClockSet, queue_time_does_not_spend_active_but_consumes_wall(), sequential_vs_overlapping_tool_budgets()

### Community 113 - "ProviderAdmission"
Cohesion: 0.18
Nodes (6): shared_rate_limit_cooldown_blocks_admission(), AdmissionGuard, note_shared_rate_limit(), provider_admission_caps_at_two(), ProviderAdmission, rate_limit_cooldown_blocks_acquire()

### Community 114 - "Phase Plan: Home Recon Composer and Investigation Tabs"
Cohesion: 0.11
Nodes (17): 1. Outcome, 2. Home Layout (Section 4), 3. Home Composer (Section 5), 4. Submission & Transition (Section 6), 5. Persistent Investigation Tabs (Section 7), 6. Keyboard & Mouse Contract (Section 8), 7. Recovery & QoL (Section 9), Acceptance Criteria (Spec Section 12) (+9 more)

### Community 116 - "gsd-v2.js"
Cohesion: 0.12
Nodes (7): core, home, hooks, names, payload(), run(), setup()

### Community 117 - "AnnPolicy"
Cohesion: 0.33
Nodes (4): AnnPolicy, AnnDeferredUnmeasured, AnnEnabled, ExactSearch

### Community 118 - "briefing_view.rs"
Cohesion: 0.24
Nodes (13): bucket_extracted(), buckets_split_facts_inferences_context_and_links(), claim(), claim_label(), ExtractedBuckets, ExtractedLine, MAX_ACTORS, MAX_CONTEXT (+5 more)

### Community 119 - "subscription.rs"
Cohesion: 0.25
Nodes (8): answer_event(), check_login(), command(), complete(), completion_is_ephemeral_and_does_not_inherit_tools_or_configuration(), exec_args(), is_subscription_status(), login()

### Community 120 - "enqueue_job_with"
Cohesion: 0.67
Nodes (3): enqueue_job_with(), JobMeta, NewJob

### Community 121 - "cursor_at"
Cohesion: 1.00
Nodes (3): cursor_at(), line_col(), viewport()

### Community 122 - "chat_model"
Cohesion: 0.25
Nodes (21): a_dropped_provider_stream_keeps_the_text_already_received(), a_provider_that_rejects_streaming_still_returns_the_answer(), a_streamed_answer_with_a_bad_citation_ends_on_the_repaired_answer(), an_idle_stall_or_the_ceiling_keeps_the_partial_answer(), answer_text(), cancel_during_synthesis_marks_the_run_cancelled_and_keeps_partial_text(), cancel_during_tools_marks_the_run_cancelled(), chat_model() (+13 more)

### Community 123 - "mem"
Cohesion: 0.19
Nodes (22): lease_fencing_rejects_stale_epoch_and_foreign_owner(), claim_complete_and_retry_round_trip(), claim_index_changes(), claim_index_work(), claim_next(), claim_next_in(), complete_index_change(), disabled_outcome_blocks_without_spending_attempts_and_resumes() (+14 more)

### Community 124 - "ReconCommand"
Cohesion: 0.17
Nodes (12): ReconCommand, Ask, AskNew, Delete, Limits, List, New, Rename (+4 more)

### Community 125 - "JobRow"
Cohesion: 0.12
Nodes (11): job_row(), JobFilter, JobRow, JobStatusFilter, Active, All, Completed, Failed (+3 more)

### Community 126 - "RouteInput"
Cohesion: 0.17
Nodes (11): RouteInput, FacebookUrl, Handle, HandleOrUserId, Hashtag, LinkedinCompanyUrl, LinkedinProfileUrl, Query (+3 more)

### Community 127 - "poll_device"
Cohesion: 0.28
Nodes (9): DeviceGrant, Poll, Denied, poll_device(), Pending, SlowDown, Token, start_device() (+1 more)

### Community 128 - "Value"
Cohesion: 0.20
Nodes (16): a_keyword_inside_the_subjects_name_runs_no_news_or_legal_tool(), a_keyword_outside_the_subjects_name_still_runs_news_or_legal(), ac2_without_a_key_news_and_legal_tools_are_never_picked_and_with_a_key_they_are_eligible(), ac4_a_news_prompt_runs_newsapi_search_with_the_entity_and_who_is_runs_none(), ac5_a_sued_prompt_runs_courtlistener_case_or_docket_search_with_the_entity(), ac7_the_relevance_gate_drops_an_off_topic_article(), action(), binding_extraction_drops_values_missing_from_the_observation() (+8 more)

### Community 130 - "agent"
Cohesion: 0.09
Nodes (25): agent, build, compaction, explore, plan, mode, model, permission (+17 more)

### Community 131 - "Functional Requirements"
Cohesion: 0.12
Nodes (15): Atlas (News Pipeline), Brain / Memory, Configuration Requirements, Context Provider Keys, Functional Requirements, Non-Functional Requirements, Non-Goals, OSINT Manual Runs (+7 more)

### Community 132 - ".on_esc"
Cohesion: 0.09
Nodes (5): add_scroll(), keyboard_navigation_esc_and_shortcuts(), summary_system(), RelatedRow, Tool

### Community 133 - "super"
Cohesion: 0.21
Nodes (7): intel_recon_publishes_durably_with_provenance_and_no_atlas_receipt(), ReconInsightUpdate, update(), upsert_adds_and_updates_claims(), upsert_recon_insights(), CODES, NAMES

### Community 134 - "ChatBlock"
Cohesion: 0.17
Nodes (16): ChatBlock, ChatRow, clip_pieces(), disclosure_pieces(), expanded(), face_background(), FrameCache, memory_list_note() (+8 more)

### Community 135 - "TaskState"
Cohesion: 0.18
Nodes (9): TaskState, Cancelled, Completed, Failed, Paused, Queued, RetryScheduled, Running (+1 more)

### Community 136 - "OsintCommand"
Cohesion: 0.22
Nodes (9): OsintCommand, Attach, Describe, Disable, Enable, History, List, Run (+1 more)

### Community 137 - ".new"
Cohesion: 0.24
Nodes (5): a_round_keeps_its_time_and_a_tight_ceiling_still_runs_tools(), aging_the_tool_clock_blocks_new_calls_and_cache_hits_do_not_extend_it(), foreground_expiry_continues_in_background(), sequential_raise_calls_budgets_higher(), TurnCheckpoint

### Community 138 - "the_label_updates_when_a_round_adds_calls_and_when_evidence_is_measured"
Cohesion: 0.28
Nodes (3): format_deadline(), format_span(), the_label_updates_when_a_round_adds_calls_and_when_evidence_is_measured()

### Community 139 - "Argos OSINT — Agent Instructions"
Cohesion: 0.14
Nodes (13): Architecture Notes (non-obvious), Argos OSINT — Agent Instructions, CI (`.github/workflows/ci.yml`), CLI Entry Points (`argos` binary), Developer Commands, Environment Variables, graphify, Key Files to Read for Context (+5 more)

### Community 140 - "Text"
Cohesion: 0.24
Nodes (3): draw_origins(), OriginsView, composer_prompt_text()

### Community 141 - "TaskStatus"
Cohesion: 0.07
Nodes (17): CallProposal, HandoffRecord, TaskRecord, TaskStatus, Cancelled, Completed, Deferred, Failed (+9 more)

### Community 142 - "RspStatus"
Cohesion: 0.15
Nodes (13): clip_summary(), normalize_name(), observation_for(), parse_status(), parses_status_domains_and_maps_reliability(), RspEntry, RspStatus, Blacklisted (+5 more)

### Community 143 - "tasks.rs"
Cohesion: 0.11
Nodes (38): add_missing_columns(), block_claimed(), claim_task(), claim_with(), ClaimedTask, complete_claimed(), complete_task(), DEFAULT_LLM_ATTEMPTS (+30 more)

### Community 144 - "DecisionState"
Cohesion: 0.20
Nodes (4): DecisionState, EvidencePassageState, SubjectStateSummary, TaskStateSummary

### Community 145 - "ToolResult"
Cohesion: 0.13
Nodes (23): ToolResult, ac6_citation_groups_split_validate_each_id_and_normalize(), cacheable(), chat(), citation_groups(), citation_ids(), cut_footer(), cut_short_answer() (+15 more)

### Community 146 - "RecordKind"
Cohesion: 0.29
Nodes (6): RecordKind, Claim, DerivedSummary, Memory, Passage, ToolObservation

### Community 147 - "Store"
Cohesion: 0.18
Nodes (5): ArticleInsightCommit, insight_fingerprint(), active_index_rows(), Store, IndexEnqueue

### Community 148 - "ClaimRelation"
Cohesion: 0.29
Nodes (7): ClaimRelation, AmbiguousEntity, ChangedQuantity, Equivalent, Negation, PlannedVsCompleted, Unrelated

### Community 149 - "results.rs"
Cohesion: 0.16
Nodes (16): classify_output_quality(), extract_observation_items(), extract_search_results(), extracts_from_array_shaped_observations(), extracts_from_object_with_results_array(), normalize_tool_result(), NormalizedToolResult, OutputQuality (+8 more)

### Community 150 - "Store"
Cohesion: 0.53
Nodes (5): charge_newsapi(), charge_quota(), open_key(), quota_open(), utc_day()

### Community 151 - "ledger.rs"
Cohesion: 0.14
Nodes (13): body_assertion_candidates(), coverage_complete(), ElementStatus, Assessed, Excluded, InProgress, Pending, Superseded (+5 more)

### Community 152 - "ProviderPage"
Cohesion: 0.27
Nodes (6): ProviderPage, .ALL, Defaults, Google, Nvidia, OpenRouter

### Community 153 - "split_vertical"
Cohesion: 0.13
Nodes (27): api_key_slot(), ApiKeySlot, atlas_live_areas(), brain_form(), brain_hit(), brain_list(), BrainForm, BrainList (+19 more)

### Community 154 - "§9 implementation order"
Cohesion: 0.15
Nodes (12): §10 acceptance checks, §9 implementation order, Atlas memory completion and System apps — implementation checklist, Phase 1 — what landed, Phase 2 — what landed, Phase 3 — what landed, Phase 4 — what landed, Phase 5a — what landed (+4 more)

### Community 155 - "TurnContinuation"
Cohesion: 0.50
Nodes (4): TurnContinuation, Active, Background, Exhausted

### Community 156 - "investigation/evidence.rs"
Cohesion: 0.15
Nodes (16): assess_claim_against_passages(), ClaimAssessmentOutcome, ClaimStance, Disputed, Insufficient, Mention, Supported, curate_passages_from_result() (+8 more)

### Community 157 - "InvestigationPattern"
Cohesion: 0.10
Nodes (17): InvestigationPattern, ArticleVerification, Bitcoin, BreakingNews, DomainIp, EmailAttribution, FollowUp, GeneralSubject (+9 more)

### Community 164 - "What Was Learned"
Cohesion: 0.15
Nodes (12): CLI Entry Points, Codebase Structure, Investigation Flow, Model Roles (configured independently in Providers → Defaults), Next Commands, Onboarding Summary — argos-osint, Planning Artifacts Created, Primary Providers (+4 more)

### Community 167 - "investigation.rs"
Cohesion: 0.07
Nodes (74): Account, account_hits(), ACCOUNT_PLATFORMS, account_platforms_attach_to_the_subject_and_are_never_entities(), ACCOUNTS, action_order(), actions_are_grounded_capped_and_not_a_sweep(), ADAPTIVE (+66 more)

### Community 168 - "Milestone 1 — Core Onboarding (Current)"
Cohesion: 0.18
Nodes (10): Deliverables, Dependencies, Future Milestones (Planned), Milestone 1 — Core Onboarding (Current), Milestone 2 — CLI Verification, Milestone 3 — Test Coverage, Milestone 4 — Integration & Ship, Objectives (+2 more)

### Community 169 - "paths.rs"
Cohesion: 0.36
Nodes (8): write_cache(), auth_path(), config_path(), db_path(), ensure_home(), hardware_cache_path(), home_dir(), lancedb_dir()

### Community 170 - "HypothesisRecord"
Cohesion: 0.18
Nodes (13): Alternative, classify_hypothesis(), contradiction(), draft_hypotheses(), hypothesis_absence_stays_unresolved(), hypothesis_status(), HypothesisRecord, leading_name() (+5 more)

### Community 171 - "CycleOutcome"
Cohesion: 0.17
Nodes (13): check_dependency_coverage(), CycleOutcome, Blocked, Cancelled, Completed, CompletedWithWarnings, Failed, Partial (+5 more)

### Community 172 - "H2 Contracts: Frozen Interface Decisions"
Cohesion: 0.18
Nodes (10): 1. Home Draft Key, 2. Launch State Machine, 3. Persistence Boundary, 4. Tab Identity, 5. Navigation Intent, 6. State Restoration (Per-Tab), 7. Input Precedence Hierarchy, 8. Layout Measurements (Reference Points) (+2 more)

### Community 173 - "youtube_pair"
Cohesion: 0.33
Nodes (6): https_on_host(), profile_path_token(), facebook_url(), linkedin_url(), youtube_pair(), social_token()

### Community 174 - "Codebase Map — argos-osint"
Cohesion: 0.22
Nodes (8): `argos-osint-bin` — CLI and TUI, `argos-osint-core` — Core Library, Codebase Map — argos-osint, Crates, Documentation Ingest, Primary Data Flow, State & Config (all under `~/.argos`), Tool Input & Binding Kinds (from `tool_io.rs`)

### Community 175 - "PROJECT.md — argos-osint"
Cohesion: 0.22
Nodes (8): Applications, CLI Entry Points, Core Components, Crates, Investigation Flow, PROJECT.md — argos-osint, Project Purpose, State Directory (`~/.argos`)

### Community 176 - "Current Phase State"
Cohesion: 0.25
Nodes (7): Artifacts Status, Configuration State, Current Phase State, Next Steps, Pending Items, Phase: Core Onboarding, STATE.md — argos-osint

### Community 178 - "RecoveryAction"
Cohesion: 0.12
Nodes (12): classify_recovery(), RecoveryAction, AccessRestricted, CircuitCooldown, ContentRefused, CreditsExhausted, FallbackModel, ReduceContext (+4 more)

### Community 181 - "RspIndex"
Cohesion: 0.36
Nodes (9): cache(), cached_index(), CachedIndex, ensure_index(), fetch_index(), index_from_json(), install_index(), RspIndex (+1 more)

### Community 182 - "Docs Ingest — argos-osint"
Cohesion: 0.33
Nodes (5): `docs/architecture.md`, Docs Ingest — argos-osint, `docs/providers.md`, Ingest Status, Ingested Documentation

### Community 184 - "DecisionAdapterKind"
Cohesion: 0.10
Nodes (19): DecisionAdapterKind, JsonMode, NativeDecisions, OutputTool, StrictJsonSchema, ValidatedText, resolve_adapter(), DecisionQuestionType (+11 more)

### Community 185 - "OpenCode V2 workflow"
Cohesion: 0.40
Nodes (4): Focused ECC commands, OpenCode V2 workflow, Start with the graph, Verify setup

### Community 186 - "Unified reliability / summarization / semantic pipelines — completion checklist"
Cohesion: 0.40
Nodes (4): Honest remaining gaps, Phase status (§18), This follow-up, Unified reliability / summarization / semantic pipelines — completion checklist

### Community 201 - "serde"
Cohesion: 0.14
Nodes (11): GraphEdgeKind, ContradictionOrUpdate, Evidenced, Inferred, SemanticSuggestion, related_evidence_view(), RelatedEvidenceHit, semantic_edges_are_not_factual() (+3 more)

### Community 202 - "app.rs"
Cohesion: 0.04
Nodes (54): a_failed_turn_keeps_the_streamed_answer_on_screen(), a_log_or_late_tool_row_does_not_drop_the_live_answer(), ATLAS_AUTO_SECS, atlas_claim(), atlas_countdown_visible(), atlas_extracting_visible(), atlas_live_insights_show_extract_progress(), atlas_log_level() (+46 more)

### Community 203 - "subject_of"
Cohesion: 0.12
Nodes (17): accounts_flow(), accounts_search_query(), clip_query(), complementary_queries(), derived_question_handles(), DiscoveryQuery, investigation_frame(), InvestigationFrame (+9 more)

### Community 205 - "Argos Adaptive Decision Roles and Strict Output Contracts"
Cohesion: 0.25
Nodes (7): Argos Adaptive Decision Roles and Strict Output Contracts, Authoritative 12-Template Registry, Execution Adapters, Overview, State Isolation & Prompt Hardening, Threshold Policy & Uncertainty, TUI Display & Trace

### Community 206 - "H0-H2 Completion Summary"
Cohesion: 0.33
Nodes (5): H0-H2 Completion Summary, H0: Resolve Harness and Current Phase ✓, H1: Map Feature Seams ✓, H2: Freeze Contracts and Packets ✓, Next Step: H3 - State and Persistence Foundation (Build Agent)

### Community 208 - "Argos OSINT — Unified Investigation Harness"
Cohesion: 0.18
Nodes (10): 3-Level Catalog Projection, 3-Level Tool Catalog & 14 Intelligence Categories, 9 Logical Model Roles & Inheritance, Architecture, Argos OSINT — Unified Investigation Harness, Chronological Trace & Persistence, Inheritance Rules, Overview (+2 more)

### Community 209 - ".default"
Cohesion: 0.09
Nodes (35): a_budgeted_call_with_a_dependency_from_an_earlier_batch_runs(), a_claimed_email_removes_its_bindings(), a_gap_filler_domain_feeds_hunter_once_firecrawl_observes_it(), a_goal_naming_a_tool_or_provider_is_repaired_then_falls_back(), a_goal_naming_a_tool_outside_the_prompt_entity_is_still_rejected(), a_handle_named_in_a_derived_question_is_an_unverified_binding(), a_prompt_entity_named_like_a_provider_keeps_the_models_directives(), a_weak_firecrawl_search_adds_one_google_search_with_the_same_query() (+27 more)

### Community 210 - "json"
Cohesion: 0.29
Nodes (3): box(), color(), render()

### Community 211 - "Value"
Cohesion: 0.29
Nodes (10): clip_chars_ellipsis(), clip_long_strings(), compact_page(), compact_page_evidence(), compact_page_system(), long_page_evidence_compacts_to_a_summary_for_synthesis(), page_excerpt(), page_needs_compact() (+2 more)

### Community 214 - "SourceReliability"
Cohesion: 0.22
Nodes (5): SourceReliability, A, B, C, D

### Community 217 - "select_strategy"
Cohesion: 0.40
Nodes (6): has_concrete_identifier(), hypothesis_question(), select_strategy(), strategy_change_reason(), strategy_follows_the_question_and_can_change_without_erasing_work(), StrategyChoice

### Community 218 - "PlanInterval"
Cohesion: 0.33
Nodes (5): PlanInterval, Daily, Monthly, Never, Weekly

### Community 219 - "IndexOutcome"
Cohesion: 0.12
Nodes (7): ReindexReport, IndexOutcome, Disabled, Pending, PermanentFailure, Ready, RetryableFailure

### Community 226 - "Argos UI Interaction Audit"
Cohesion: 0.33
Nodes (5): Argos UI Interaction Audit, Audit Matrix, Field inventory (`field_placeholder`), Remaining limits, Shared contracts

### Community 228 - "run_live"
Cohesion: 0.29
Nodes (6): LiveRun, Fresh, Latest, Run, run_live(), RunInput

### Community 229 - "classify_resource"
Cohesion: 0.40
Nodes (4): classify_resource(), file_link_url(), looks_like_file_path(), video_host()

## Knowledge Gaps
- **1427 isolated node(s):** `$schema`, `model`, `small_model`, `default_agent`, `mode` (+1422 more)
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 1939 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **25 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `App` connect `App` to `.on_esc`, `.set_focus`, `ChatBlock`, `Call`, `.clear`, `ToolResult`, `Store`, `ProviderPage`, `hardware.rs`, `Store`, `.handle_key`, `brain_detail.rs`, `recon/graph.rs`, `run_atlas_inner`, `SettingsFile`, `ReportMode`, `unix_now`, `Target`, `src/brain.rs`, `atlas_insights.rs`, `secrets.rs`, `app.rs`, `Overlay`, `LogsView`, `tui/jobs.rs`, `FeedArticle`, `WorkEvent`, `summary_card.rs`, `ModuleId`, `.handle_mouse`, `briefing_view.rs`?**
  _High betweenness centrality (0.073) - this node is a cross-community bridge._
- **What connects `$schema`, `model`, `small_model` to the rest of the system?**
  _1427 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `land.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.0076045627376425855 - nodes in this community are weakly interconnected._
- **Why does `ProviderSecret` connect `ProviderSecret` to `App`, `atlas_memory.rs`, `ToolResult`, `gates.rs`, `ProviderPage`, `execute_steps`, `provider.rs`, `run_turn`, `body.rs`, `atlas.rs`, `run_atlas_inner`, `scheduler.rs`, `provider_attempt.rs`, `SettingsFile`, `ReportMode`, `graph_explanation.rs`, `graph_explanation/tests.rs`, `exec.rs`, `worker.rs`, `intel_recon/jobs.rs`, `atlas_insights.rs`, `body_filter.rs`, `synthesize.rs`, `secrets.rs`, `summarization.rs`, `Value`, `.begin_title`, `anyhow`, `run_live`, `explain`, `all_tools`, `.order`, `TurnEvent`, `.evaluate`, `subscription.rs`, `chat_model`, `poll_device`?**
  _High betweenness centrality (0.043) - this node is a cross-community bridge._
- **Should `orchestrate.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.05660377358490566 - nodes in this community are weakly interconnected._
- **Why does `ButtonId` connect `ButtonId` to `App`, `summary_card.rs`, `Line`, `.set_focus`, `app.rs`, `tui/jobs.rs`, `WorkEvent`, `logs.rs`, `split_vertical`, `ui.rs`, `Rect`, `Target`?**
  _High betweenness centrality (0.032) - this node is a cross-community bridge._
- **Should `App` be split into smaller, more focused modules?**
  _Cohesion score 0.054527750730282376 - nodes in this community are weakly interconnected._