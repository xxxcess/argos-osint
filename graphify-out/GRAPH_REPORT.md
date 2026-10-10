# Graph Report - argos-osint  (2026-10-10)

## Corpus Check
- 239 files · ~502,907 words
- Verdict: corpus is large enough that graph structure adds value.
- Unclassified: 6 file(s) not represented in the graph (top: (none) 3, .toml 2, .orig 1)

## Summary
- 7886 nodes · 20129 edges · 265 communities (232 shown, 33 thin omitted)
- Extraction: 98% EXTRACTED · 2% INFERRED · 0% AMBIGUOUS · INFERRED: 388 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `acf63e74`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- land.rs
- orchestrate.rs
- Call
- atlas_memory.rs
- tool_io.rs
- app.rs
- brain_lance.rs
- FieldId
- map.rs
- recon.rs
- ButtonId
- Store
- IntelligenceCategory
- job_registry.rs
- unix_now
- directives.rs
- .activate_button
- SearchHit
- providers.rs
- Value
- Store
- publication.rs
- result
- whoxy.rs
- news_legal.rs
- hardware.rs
- App
- Store
- embed.rs
- osint.rs
- holehe/mod.rs
- config_commit.rs
- .default
- TurnEvent
- InvestigationPart
- body.rs
- provider.rs
- brain_detail.rs
- profile_stats.rs
- atlas.rs
- run_atlas_inner
- whatsmyname.rs
- scheduler.rs
- jobs_view.rs
- provider_attempt.rs
- LogicalRole
- picker.rs
- ReportMode
- provider_chain.rs
- atomic
- JobsView
- App
- events.rs
- exec.rs
- draw_intel_briefing
- model_exec.rs
- ErrorCategory
- now
- MemoryKind
- Frame
- brain_resources.rs
- worker.rs
- .handle_key
- src/brain.rs
- intel_recon/jobs.rs
- cli.rs
- atlas_insights.rs
- SourceReliability
- provider_diag.rs
- body_filter.rs
- synthesize.rs
- dataset.rs
- .new
- profile_charts.rs
- theme.rs
- Overlay
- pipeline.rs
- summarization.rs
- InvestigationSurface
- logs.rs
- search_engines.rs
- tui/jobs.rs
- .new
- FeedArticle
- budget.rs
- Field
- Command
- DefaultsRole
- graph_explanation.rs
- src/evidence.rs
- ToolResult
- DecisionContract
- AtlasArticleRow
- F
- dork_generator.rs
- ProfileView
- How
- synthesize
- summary_card.rs
- actor_review.rs
- ModuleId
- Region
- graph_explanation/tests.rs
- validate.rs
- Category
- Rect
- search_engines/tests.rs
- profile_config.rs
- run_turn
- Architecture
- WorkEvent
- store.rs
- ClockSet
- ProviderAdmission
- components.rs
- KeptClaim
- gsd-v2.js
- ServiceResult
- briefing_view.rs
- subscription.rs
- profile.rs
- IndexOutcome
- recon/graph.rs
- tasks.rs
- ReconCommand
- config_transfer_tests.rs
- Service
- AuthFile
- ServiceSpec
- TurnClock
- opencode.json
- Functional Requirements
- JobStatusFilter
- super
- ui.rs
- DateTime
- OsintCommand
- normalize_destination
- TelemetryEvent
- Argos OSINT — Agent Instructions
- logical_attempt_and_cache_rows_stay_distinct_under_replay
- TaskStatus
- wikipedia_rsp.rs
- rusqlite
- serde
- .default
- execute_steps
- .is_empty
- QueryExecutionStatus
- results.rs
- telemetry.rs
- SettingsFile
- anyhow
- modes.rs
- §9 implementation order
- .new
- investigation/evidence.rs
- InvestigationPattern
- Part
- argos-osint-bin
- What Was Learned
- ledger.rs
- SiteOutcomeStatus
- investigation.rs
- Milestone 1 — Core Onboarding (Current)
- paths.rs
- Connection
- atlas_work.rs
- JobRow
- decide
- Codebase Map — argos-osint
- PROJECT.md — argos-osint
- Current Phase State
- QuestionSpec
- RecoveryAction
- grok_oauth.rs
- TaskState
- Conventions
- Docs Ingest — argos-osint
- ScheduledCall
- config_transfer.rs
- OpenCode V2 workflow
- Unified reliability / summarization / semantic pipelines — completion checklist
- Concepts
- package.json
- explore.rs
- TUI components
- ConfigView
- Providers
- Decision roles and output contracts
- AnalyticsLayout
- Terminal UI
- docs/README.md
- Profile dashboard and search
- RunStats
- BrainIndex
- extract_items
- render_tui_cells.py
- .order
- intel_recon/brain.rs
- Unified Investigation Harness
- Json
- .evaluate
- TurnContinuation
- Argos OSINT
- ToolOutcome
- EventKind
- Argos documentation
- ReconLimits
- accept_claims
- Argos UI Interaction Audit
- ChainReport<T>
- ImportBuffer
- PickRequest
- enqueue_job
- brain_lance_off.rs
- Cell
- serde_json
- Supplied<T>
- ACTIVE_SNAPSHOT
- parse_serp_response
- home_rows
- atlas_actions.rs
- provider_metrics.rs
- Store
- SerpOutcome
- .recon_outcomes
- .job_source
- Trigger
- ExecuteOptions
- BrainResourceSummary
- select_strategy
- ConfigTab
- LaunchState
- SystemTab
- ReportOutcome
- BrainListMode
- DispatchError
- PlanInterval
- Target
- OperationScope<'a>

## God Nodes (most connected - your core abstractions)
1. `App` - 316 edges
2. `Value` - 280 edges
3. `ProviderSecret` - 143 edges
4. `ButtonId` - 130 edges
5. `ToolResult` - 78 edges
6. `Target` - 77 edges
7. `FieldId` - 76 edges
8. `Store` - 74 edges
9. `Store` - 64 edges
10. `AtlasArticleRow` - 57 edges

## Surprising Connections (you probably didn't know these)
- `Module layout` --references--> `widget_lines()`  [EXTRACTED]
  docs/architecture.md → crates/argos-osint-bin/src/tui/profile.rs
- `8. Implementation order and completion checks` --references--> `LayoutRegistry`  [EXTRACTED]
  docs/tui-design-spec.md → crates/argos-osint-bin/src/tui/app.rs
- `6. Apply the baseline across Argos` --references--> `draw_system_tab()`  [EXTRACTED]
  docs/tui-design-spec.md → crates/argos-osint-bin/src/tui/profile.rs
- `3. Current audit → required fixes` --references--> `draw_section_body()`  [EXTRACTED]
  docs/tui-design-spec.md → crates/argos-osint-bin/src/tui/profile.rs
- `6. Apply the baseline across Argos` --references--> `home_layout_metrics()`  [EXTRACTED]
  docs/tui-design-spec.md → crates/argos-osint-bin/src/tui/ui.rs

## Import Cycles
- 1-file cycle: `crates/argos-osint-core/src/config_transfer.rs -> crates/argos-osint-core/src/config_transfer.rs`
- 2-file cycle: `crates/argos-osint-core/src/osint.rs -> crates/argos-osint-core/src/osint/results.rs -> crates/argos-osint-core/src/osint.rs`
- 2-file cycle: `crates/argos-osint-bin/src/cli.rs -> crates/argos-osint-core/src/hardware.rs -> crates/argos-osint-bin/src/cli.rs`
- 4-file cycle: `crates/argos-osint-core/src/config_commit.rs -> crates/argos-osint-core/src/provider_attempt.rs -> crates/argos-osint-core/src/provider_diag.rs -> crates/argos-osint-core/src/config_transfer.rs -> crates/argos-osint-core/src/config_commit.rs`

## Communities (265 total, 33 thin omitted)

### Community 0 - "land.rs"
Cohesion: 0.01
Nodes (262): CODES, LABELS, R0, R1, R10, R100, R101, R102 (+254 more)

### Community 1 - "orchestrate.rs"
Cohesion: 0.04
Nodes (76): a_follow_up_keeps_names_from_the_previous_synthesis(), a_handle_named_in_a_derived_question_is_an_unverified_binding(), a_keyword_inside_the_subjects_name_runs_no_news_or_legal_tool(), a_keyword_outside_the_subjects_name_still_runs_news_or_legal(), ac2_without_a_key_news_and_legal_tools_are_never_picked_and_with_a_key_they_are_eligible(), ac3_seo_results_that_never_mention_the_subject_add_no_domain_org_email_or_url(), ac4_a_news_prompt_runs_newsapi_search_with_the_entity_and_who_is_runs_none(), ac4_google_search_always_sends_the_replaced_firecrawl_query() (+68 more)

### Community 2 - "Call"
Cohesion: 0.08
Nodes (42): TranscriptBlock, build_blocks(), call_stamp(), ChatBlock, ChatRow, clip_pieces(), coverage_only_counts_explicit_assessments(), coverage_summary() (+34 more)

### Community 3 - "atlas_memory.rs"
Cohesion: 0.05
Nodes (97): atlas_memory_count(), background_indexing_then_refresh_upgrades_a_partial_run(), Barrier, Checkpoint, CheckpointInput, claim(), claims(), clear() (+89 more)

### Community 4 - "tool_io.rs"
Cohesion: 0.03
Nodes (124): header_for(), Binding, canonical(), dependency_order(), depends_on(), PROMPT_TARGETS, normalize_platform(), question_bindings() (+116 more)

### Community 5 - "app.rs"
Cohesion: 0.06
Nodes (80): a_click_folds_an_event_and_incoming_entries_do_not_move_a_paused_list(), a_finished_tool_call_is_logged_once_and_the_transcript_keeps_a_summary(), all_provider_actions_render_with_hit_areas_at_80x24(), analytics_fixture(), analytics_viewports_and_data_states(), ATLAS_AUTO_SECS, atlas_auto_toggle_persists_the_next_trigger(), atlas_claim() (+72 more)

### Community 6 - "brain_lance.rs"
Cohesion: 0.06
Nodes (38): activate_generation(), batch(), begin_generation(), begin_generation_then_activate(), block_on(), BrainIndex, clear_fingerprint(), current_fingerprint() (+30 more)

### Community 7 - "FieldId"
Cohesion: 0.03
Nodes (60): FieldId, BrainApp, BrainConversation, BrainInsight, BrainQuery, ClaimAssessorModel, ClaimAssessorProvider, ClassifierModel (+52 more)

### Community 8 - "map.rs"
Cohesion: 0.05
Nodes (77): BRAILLE_MAP, canonical(), claim_label(), contains(), country_at(), country_fit(), country_pixel(), default_world_scale_is_enlarged_and_fits_small_terminals() (+69 more)

### Community 9 - "recon.rs"
Cohesion: 0.05
Nodes (74): ac6_citation_groups_split_validate_each_id_and_normalize(), answer_step_rate_limit_is_explicit_and_keeps_results(), attribute_word(), auxiliary(), await_completion(), BRIEF_SYNTHESIS, broad_question_collapses_to_grounded_lookups(), cacheable() (+66 more)

### Community 10 - "ButtonId"
Cohesion: 0.02
Nodes (106): ButtonId, Add, AddFallback, AtlasAuto, AtlasDelete, AtlasLive, AtlasNews, AtlasNewsFeed (+98 more)

### Community 11 - "Store"
Cohesion: 0.08
Nodes (5): CreditHold, Store, strategy_and_provider_credits_survive_reopen(), Thread, whoxy_prepaid_pool_does_not_reset_monthly()

### Community 12 - "IntelligenceCategory"
Cohesion: 0.08
Nodes (23): all_catalog_tools_mapped_to_categories(), argument_contract(), ArgumentBuilderContract, compact_capability_catalog(), CompactToolCapability, IntelligenceCategory, Bitcoin, DomainNetwork (+15 more)

### Community 13 - "job_registry.rs"
Cohesion: 0.05
Nodes (48): begin(), begin_cancellable(), db(), db_path(), finish(), beat(), BEAT_INTERVAL, beats() (+40 more)

### Community 14 - "unix_now"
Cohesion: 0.22
Nodes (7): atlas_auto_future_tick_waits_and_past_tick_reschedules(), atlas_auto_while_running_only_arms_the_next_slot(), atlas_history_button_counts_down_while_auto_run_is_on(), atlas_poll_wait(), manual_run_moves_the_next_auto_trigger_out_by_90_minutes(), unix_now(), Atlas

### Community 15 - "directives.rs"
Cohesion: 0.06
Nodes (64): accounts_search_query(), binder_maps_kinds_to_inputs_and_never_hands_hunter_a_social_host(), binding(), apply_context_targets(), asks(), asks_about_judge(), asks_for_headlines(), clip_words() (+56 more)

### Community 16 - ".activate_button"
Cohesion: 0.06
Nodes (9): draft_isolation_and_persistence(), duplicate_submission_prevention(), open_external_url(), pruned_history_closes_the_open_run(), session_tabs_open_close_reopen(), single_action_launch_from_home(), tab_strip_render_and_hit_test(), IntelBody (+1 more)

### Community 17 - "SearchHit"
Cohesion: 0.10
Nodes (27): absorb_hit(), account_platform_host(), Candidate, content_tokens(), relevance_gate(), result_mentions(), distinct_queries(), domain_label() (+19 more)

### Community 18 - "providers.rs"
Cohesion: 0.03
Nodes (91): annotate(), bitcoin(), bounded(), clip_page(), domain(), email_address(), https_on_host(), ip() (+83 more)

### Community 19 - "Value"
Cohesion: 0.07
Nodes (48): NoDuplicates, Value, api_error(), ATLAS_TOOLS, authors(), clip(), country_arg(), country_code() (+40 more)

### Community 20 - "Store"
Cohesion: 0.06
Nodes (25): article_body_from_row(), ArticleBodyRow, cited_evidence_keeps_provenance_and_revision_identity(), element_from_row(), full_assessment_parents_are_flagged_for_child_report_totals(), IntelAssessmentRow, IntelInvestigationRow, IntelReportSectionRow (+17 more)

### Community 21 - "publication.rs"
Cohesion: 0.07
Nodes (47): Phase5Report, MemorySource, active_index_rows(), bump_memories_changed(), change_watcher_sees_only_committed_changes_and_coalesces(), claim(), clear(), clear_index_state() (+39 more)

### Community 22 - "result"
Cohesion: 0.09
Nodes (9): execute_harness_step(), InvestigationRuntime, GateOutcome, Passed, Rejected, validate_claim_assessment(), validate_evidence_admission(), validate_publication() (+1 more)

### Community 23 - "whoxy.rs"
Cohesion: 0.11
Nodes (35): adjacent_changes(), AdjacentChange, balance_request_url(), bounded_model_view(), check_balance(), contact(), ContactCard, date_and_limit_validation() (+27 more)

### Community 24 - "news_legal.rs"
Cohesion: 0.06
Nodes (46): a_rate_limit_switches_to_the_fallback_key_and_a_rejection_does_not(), ac1_catalog_has_55_tools_and_the_news_and_legal_entries(), ac1_each_request_has_its_url_params_and_auth_header_and_no_key_in_the_url(), an_echoed_key_is_redacted_from_the_stored_body_and_error(), article(), articles(), CATEGORIES, choice_arg() (+38 more)

### Community 25 - "hardware.rs"
Cohesion: 0.13
Nodes (20): system_host_lines(), CACHE_TTL_SECS, classify_arch(), disk_totals(), HardwareProfile, metal_vram_gb(), now_secs(), parse_apple_gpu_cores() (+12 more)

### Community 26 - "App"
Cohesion: 0.08
Nodes (64): active_popup_area(), atlas_cycle_stats_room(), atlas_cycle_stats_scroll_max(), atlas_feed_room(), atlas_feed_room_for(), atlas_news_room(), atlas_news_room_for(), atlas_runs_room() (+56 more)

### Community 27 - "Store"
Cohesion: 0.06
Nodes (11): atlas_answer_id(), atlas_brief_id(), atlas_claims_for_article_returns_linked_claims(), AtlasRunRow, AtlasStoredClaim, GraphSummary, has_table(), intel_link_explanations_round_trip_and_cleanup() (+3 more)

### Community 28 - "embed.rs"
Cohesion: 0.08
Nodes (31): active(), DIM, disable(), disabled(), DisableGuard, download_file(), DOWNLOAD_TIMEOUT_SECS, embed_batch() (+23 more)

### Community 29 - "osint.rs"
Cohesion: 0.04
Nodes (92): test_all_catalog_tools_mapped_to_categories(), a_claimed_email_keeps_no_person_data(), bind_request(), body_cap(), bounded_recovery_is_offered_once_for_an_unrecognised_page_only(), CACHE_DAY_SECONDS, cache_follows_the_provider_plan_interval(), cache_identity() (+84 more)

### Community 30 - "holehe/mod.rs"
Cohesion: 0.08
Nodes (34): cache_key(), cache_stores_only_definitive(), CACHE_TTL, CacheEntry, cancel_marks_inconclusive(), cap_reports_omitted(), check_implemented(), classify_http() (+26 more)

### Community 31 - "config_commit.rs"
Cohesion: 0.05
Nodes (75): a_batch_with_a_duplicate_or_empty_slot_is_refused_before_any_write(), a_committed_journal_needs_no_recovery(), a_second_lock_is_refused_while_one_is_held(), a_stale_lock_is_taken_over(), an_abandoned_unfinished_lockfile_is_recovered(), backup_path(), canonical_root(), commit_files_is_all_or_nothing() (+67 more)

### Community 32 - ".default"
Cohesion: 0.09
Nodes (28): a_blank_osint_user_agent_loads_as_unset(), default_credit_reset(), default_firecrawl_credits(), default_hunter_credits(), default_max_calls(), default_max_rounds(), default_one_cost(), default_opening_cap() (+20 more)

### Community 33 - "TurnEvent"
Cohesion: 0.13
Nodes (20): compact_page_evidence(), cut_footer(), cut_short_answer(), Directive, directive_goals_line(), EntityView, evidence_summary(), Grounding (+12 more)

### Community 34 - "InvestigationPart"
Cohesion: 0.10
Nodes (17): event_to_chat_block(), is_thinking_expanded(), InvestigationPart, DirectiveAssessment, EvidencePassage, GateValidation, Handoff, Plan (+9 more)

### Community 35 - "body.rs"
Cohesion: 0.05
Nodes (69): body_fetch_routes(), BodyFetchEvent, Attempt, Failed, InsightsFailed, InsightsRefreshing, InsightsReplaced, Progress (+61 more)

### Community 36 - "provider.rs"
Cohesion: 0.06
Nodes (60): account_secret(), active_text_secret(), authorize(), bearer_token(), catalog_error(), chat_body(), ChatMessage, complete() (+52 more)

### Community 37 - "brain_detail.rs"
Cohesion: 0.06
Nodes (50): areas(), areas_for(), BrainDetail, DetailAreas, DetailPane, Path, Related, Summary (+42 more)

### Community 38 - "profile_stats.rs"
Cohesion: 0.05
Nodes (78): a_snapshot_never_panics_on_a_partially_migrated_store(), add_cycle_outcome(), AmplificationBucket, an_empty_database_returns_empty_sections_not_zeros(), atlas_carries_data(), AtlasStats, AttemptSummary, BacklogRow (+70 more)

### Community 39 - "atlas.rs"
Cohesion: 0.08
Nodes (32): article_card_labels_title_publisher_author_and_classification(), article_from(), Band, band_sizes(), band_temperature(), category_from_decisions(), CATEGORY_IDS, CLUSTERS (+24 more)

### Community 40 - "run_atlas_inner"
Cohesion: 0.08
Nodes (36): AtlasEvent, Classified, Fault, InsightProgress, MemoriesChanged, MemoryProgress, Note, Replaced (+28 more)

### Community 41 - "whatsmyname.rs"
Cohesion: 0.05
Nodes (52): DatasetStatus, status(), D, AccountTuple, ADAPTER_VERSION, apply_strip_bad_char(), benchmark_14_4_parse_index_and_selection(), cache_get() (+44 more)

### Community 42 - "scheduler.rs"
Cohesion: 0.09
Nodes (22): apply_index_change(), apply_index_change_rebuild_reports_outcome(), apply_index_with(), DEFAULT_LEASE_SECS, drain_index_once(), drain_summary_flush(), drain_summary_flush_cached(), drain_summary_flush_completes_cached_tasks() (+14 more)

### Community 43 - "jobs_view.rs"
Cohesion: 0.11
Nodes (20): summary_text(), active_time_is_live_for_open_attempts_and_unavailable_only_without_a_start(), AttemptRow, event_apps(), event_counts(), EventCounts, job(), job_apps() (+12 more)

### Community 44 - "provider_attempt.rs"
Cohesion: 0.11
Nodes (35): Acc, attempt(), attempt_with_observer(), AttemptReport, check_final(), complete_stream_and_json_answers_succeed(), Deadlines, delta() (+27 more)

### Community 45 - "LogicalRole"
Cohesion: 0.10
Nodes (11): LogicalRole, .ALL, ClaimAssessor, Classifier, Controller, EntityResolver, EvidenceCurator, Planner (+3 more)

### Community 46 - "picker.rs"
Cohesion: 0.12
Nodes (22): brain_scrape_options_hide_bare_scrape_and_include_claim_criteria(), CatalogEntry, CONFIDENCE_FLOOR, decisions_request(), directives_phrase(), DONE, eligible_catalog(), empty_brain_resources_omitted_from_state() (+14 more)

### Community 47 - "ReportMode"
Cohesion: 0.12
Nodes (23): HomeDraftState, brief_rating_reuses_the_existing_mean_semantics(), classifiable_modes(), classify_mode_chat(), classify_mode_decisions(), classify_prompt_mode(), classify_prompt_mode_chat(), classify_prompt_mode_decisions() (+15 more)

### Community 48 - "provider_chain.rs"
Cohesion: 0.15
Nodes (22): AttemptRecord, cancellation_stops_the_chain(), ChainReport, execute(), FALLBACK_ATTEMPTS, fallback_emits_three_attempts_with_10_20_waits(), FALLBACK_WAITS, http_stream_and_parse_failures_all_retry() (+14 more)

### Community 49 - "atomic"
Cohesion: 0.08
Nodes (14): AttemptOutcome, categorize_http(), execute_with_retries(), other_llm_allows_three(), summarization_stops_at_two_attempts(), TimeoutProfile, .CLASSIFIER, .EXTRACTION (+6 more)

### Community 50 - "JobsView"
Cohesion: 0.15
Nodes (8): buttons(), detail_lines(), JobsView, progress(), state_style(), table_lines(), title(), format_duration()

### Community 51 - "App"
Cohesion: 0.04
Nodes (27): a_failed_turn_keeps_the_streamed_answer_on_screen(), a_log_or_late_tool_row_does_not_drop_the_live_answer(), add_scroll(), App, atlas_countdown_visible(), atlas_extracting_visible(), atlas_log_level(), AtlasPage (+19 more)

### Community 52 - "events.rs"
Cohesion: 0.08
Nodes (28): session_event(), clear_events(), DEFAULT_RETENTION_HOURS, event_row(), EventFilter, EventRow, get_event(), job_filter_includes_descendants_and_prune_keeps_jobs() (+20 more)

### Community 53 - "exec.rs"
Cohesion: 0.14
Nodes (20): admission_account(), admission_contention_consumes_no_attempt(), AttemptEvent, Finished, Started, AttemptLog, cancelled(), complete_summary_report() (+12 more)

### Community 54 - "draw_intel_briefing"
Cohesion: 0.11
Nodes (37): abs_rect(), AbsRect, draw_centered_loading_card(), draw_clipped_button(), draw_clipped_intel_loading(), draw_clipped_md_pane(), draw_intel_body_loading(), draw_intel_briefing() (+29 more)

### Community 55 - "model_exec.rs"
Cohesion: 0.10
Nodes (33): adapter_reported_first_response_is_kept_when_no_chunk_arrived(), attempt_outcome_keeps_the_failure_category_for_the_reason_dimension(), AttemptFacts, AttemptOutcome, AttemptTimings, canonical_attempt_id(), canonical_attempt_id_is_stable_across_replay(), DecisionsAdapter (+25 more)

### Community 56 - "ErrorCategory"
Cohesion: 0.09
Nodes (23): operation_kind(), backoff_delay(), can_retry(), ErrorCategory, AuthOrQuota, CancelledOrStale, ConfigurationMissing, ContextLimit (+15 more)

### Community 57 - "now"
Cohesion: 0.17
Nodes (12): answer_evidence_survives_reopen_and_deleted_thread_rejects_late_calls(), claims_deduplicate_and_reject_unsupported_sources(), deleting_an_investigation_removes_its_brain_memories_and_graph_summary(), investigation_memory_ids(), Message, now(), persist_claims(), persistence_and_plan() (+4 more)

### Community 58 - "MemoryKind"
Cohesion: 0.09
Nodes (22): AdmissionPolicy, admit_memory(), BrainQuery, pack_context(), AssessmentState, Contradicts, Insufficient, Mentions (+14 more)

### Community 59 - "Frame"
Cohesion: 0.14
Nodes (49): intel_category_short(), Block, draw(), button_areas(), cover(), draw(), draw_add_fallback(), draw_atlas() (+41 more)

### Community 60 - "brain_resources.rs"
Cohesion: 0.09
Nodes (26): source_anchor_label(), BRAIN_SCRAPE_PREFIX, CLAIM_CHARS, classify_resource(), file_link_url(), hit(), is_brain_binding(), is_brain_scrape_pick() (+18 more)

### Community 61 - "worker.rs"
Cohesion: 0.15
Nodes (20): IntelReportEvent, InsightsUpdated, JobCreated, JobDone, Section, Stage, IntelReportJobRow, cancel_if_needed() (+12 more)

### Community 62 - ".handle_key"
Cohesion: 0.06
Nodes (17): ctrl_arrows_cycle_app_tabs_forward_and_back(), ctrl_tab_cycles_app_tabs_forward_and_back(), IntelReconFocus, Section, Start, Tab, is_picker_field(), PaletteItem (+9 more)

### Community 63 - "src/brain.rs"
Cohesion: 0.14
Nodes (19): AGREEMENT_WEIGHT, CATEGORIES, category_hint(), format_injection(), hybrid_recall(), hybrid_recall_adds_paraphrases_and_rewards_agreement(), identity_query_prefers_name_memory(), jaccard() (+11 more)

### Community 64 - "intel_recon/jobs.rs"
Cohesion: 0.15
Nodes (19): active_job_for_mode(), cancel_job(), create_job_inserts_all_verify_sections(), create_report_job(), finish_canonical(), intel_recon_links_its_legacy_job_to_one_canonical_registry_job(), pause_job(), register_canonical() (+11 more)

### Community 65 - "cli.rs"
Cohesion: 0.15
Nodes (21): ask(), ask_thread(), atlas_command(), defaults_command(), defaults_json(), defaults_show_includes_the_tool_picker_transport(), DefaultsCommand, Set (+13 more)

### Community 66 - "atlas_insights.rs"
Cohesion: 0.07
Nodes (50): a_country_token_does_not_merge_into_a_longer_name(), a_decisions_model_does_not_extract_claims(), aliases_merge_on_shared_tokens_when_the_longer_form_is_a_span(), BODY_CLAIM_LIMIT, BODY_SPAN_CHARS, brief_rating_mean(), COL_CLASS, COL_ENTITY (+42 more)

### Community 67 - "SourceReliability"
Cohesion: 0.09
Nodes (22): article_information_credibility(), AdmiraltyCode, best_credibility(), CredibilityInputs, information_credibility(), InformationCredibility, CannotBeJudged, Confirmed (+14 more)

### Community 68 - "provider_diag.rs"
Cohesion: 0.18
Nodes (15): text(), bounded(), classify_status(), endpoint_strips_query_and_userinfo(), find_url(), http_failure(), Leaf, provider_error() (+7 more)

### Community 69 - "body_filter.rs"
Cohesion: 0.18
Nodes (22): BATCH_SIZE, BodyChunk, chunk_article_body(), CHUNK_MAX, CHUNK_MIN, CHUNK_TARGET, chunks_preserve_absolute_offsets(), classify_chunk_batch() (+14 more)

### Community 70 - "synthesize.rs"
Cohesion: 0.16
Nodes (20): IntelEvidenceRow, BODY_REFINE_INPUT_CHARS, deterministic_bluf_mentions_title(), deterministic_section(), model_refine_body(), model_synthesize(), parse_json_object(), refine_retrieved_article_body() (+12 more)

### Community 71 - "dataset.rs"
Cohesion: 0.16
Nodes (20): acquire_refresh_lease(), active_manifest_path(), ACTIVE_REFRESHES, dataset_root(), DatasetManifest, get_status(), import_from_file(), load_active_manifest() (+12 more)

### Community 72 - ".new"
Cohesion: 0.10
Nodes (27): ArticleFact, atlas_origin_rows(), AttemptFact, category_triggers(), dominant_label(), engine_health(), EventRow, fingerprint() (+19 more)

### Community 73 - "profile_charts.rs"
Cohesion: 0.05
Nodes (59): BLOCKS, cap_eighths(), cells(), coarsen_counts(), column_cell(), count_coarsening_preserves_totals_boundaries_and_unknown_history(), duration_ms(), eighths() (+51 more)

### Community 74 - "theme.rs"
Cohesion: 0.05
Nodes (57): border_style(), borders_and_cells_have_matching_display_widths(), bottom_border_line(), char_width(), clip_to_width(), column_widths(), columns_consume_inner_width(), columns_distribute_proportionally_when_not_flex_last() (+49 more)

### Community 75 - "Overlay"
Cohesion: 0.08
Nodes (23): ChoiceKind, IntelDay, Investigation, Model, Provider, LastViewSession, Overlay, AddFallback (+15 more)

### Community 76 - "pipeline.rs"
Cohesion: 0.11
Nodes (26): apply_recon_directive_coverage(), ClaimRelation, AmbiguousEntity, ChangedQuantity, Equivalent, Negation, PlannedVsCompleted, Unrelated (+18 more)

### Community 77 - "summarization.rs"
Cohesion: 0.10
Nodes (39): cache_invalidates_when_source_revision_changes(), cache_get(), cache_key_changes_with_revision_and_focus(), cache_put(), cache_round_trip(), complete_summary(), CoverageMeta, deterministic_article_description() (+31 more)

### Community 78 - "InvestigationSurface"
Cohesion: 0.09
Nodes (16): CallProposal, HandoffRecord, InvestigationSurface, HomeComposer, IntelBrief, JobsResume, ReconChat, validate_tool_preflight() (+8 more)

### Community 79 - "logs.rs"
Cohesion: 0.09
Nodes (21): areas(), button_label(), buttons(), count(), draw(), hit(), in_list(), LevelFilter (+13 more)

### Community 80 - "search_engines.rs"
Cohesion: 0.08
Nodes (35): build_scrape_body(), Candidate, card_snippet(), classify_status_region(), clip_item_text(), collapse_ws(), collect_candidates(), contains_phrase() (+27 more)

### Community 81 - "tui/jobs.rs"
Cohesion: 0.12
Nodes (17): areas(), BASE_COLUMNS, button_label(), hit(), JobsAreas, LOGS_COLUMN, MIN_TITLE, optional_columns_drop_instead_of_truncating_headers() (+9 more)

### Community 82 - ".new"
Cohesion: 0.25
Nodes (19): a_spent_primary_quota_uses_the_fallback_key(), classification_request(), country_labels_show_the_name_and_the_code(), daily_cap(), format_run_card(), HttpCall, HttpReply, insights_do_not_call_a_decisions_model() (+11 more)

### Community 83 - "FeedArticle"
Cohesion: 0.11
Nodes (24): apply_hits(), article_from_row(), article_row(), Article, canonical_url(), category_tag(), dedup_drops_a_url_an_exact_title_and_keeps_the_higher_domain(), FeedArticle (+16 more)

### Community 84 - "budget.rs"
Cohesion: 0.12
Nodes (14): CUT_NOTE, CUT_SHORT, PHASE_WINDOW_SECONDS, RECON_DEADLINE, RECON_ROUND_SECONDS, STREAM_LOST, synthesis_allowance_capped(), synthesis_allowance_seconds() (+6 more)

### Community 85 - "Field"
Cohesion: 0.15
Nodes (18): backspace_after_a_sent_question_deletes_one_character(), brain_anchors_follow_memory_focus_and_scroll_stops_at_ends(), DefaultRole, defaults_pick_provider_and_model_from_account_access(), defaults_tool_picker_saves_only_its_role(), intel_day_button_label(), intel_opens_bulletin_filters_and_opens_briefing(), keyboard_navigation_esc_and_shortcuts() (+10 more)

### Community 86 - "Command"
Cohesion: 0.10
Nodes (20): AtlasCommand, Repair, Resume, Verify, Cli, Command, Atlas, Defaults (+12 more)

### Community 87 - "DefaultsRole"
Cohesion: 0.07
Nodes (17): ChoiceItem, codex_models(), DefaultsRole, .ALL, ClaimAssessor, Classifier, EntityResolver, EvidenceCurator (+9 more)

### Community 88 - "graph_explanation.rs"
Cohesion: 0.14
Nodes (14): basic_explanation(), BASIC_HEADING, explain(), ExplainOutcome, Failed, Saved, Superseded, ExplainReport (+6 more)

### Community 89 - "src/evidence.rs"
Cohesion: 0.09
Nodes (29): AGREEMENT_WEIGHT, AnnMeasurement, AnnPolicy, AnnDeferredUnmeasured, AnnEnabled, ExactSearch, AnnThresholds, chunk_text() (+21 more)

### Community 90 - "ToolResult"
Cohesion: 0.06
Nodes (35): ToolResult, BudgetedOutcome, evidence_notes(), run_has_evidence(), sample_evidence(), signatures(), StepOutcome, NotRun (+27 more)

### Community 91 - "DecisionContract"
Cohesion: 0.13
Nodes (23): compile_general_model_prompt(), compile_native(), parse_native_response(), DecisionContract, template_claim_relation(), template_directive_alignment(), template_entity_binding(), template_evidence_relevance() (+15 more)

### Community 92 - "AtlasArticleRow"
Cohesion: 0.15
Nodes (26): accept_one(), apply_peer_support(), article_with_body_spans(), articles_have_span(), body_lead_prompt(), catalog_json(), classifier_peers_are_preferred_over_token_overlap(), classify_peers_chat() (+18 more)

### Community 93 - "F"
Cohesion: 0.23
Nodes (13): CallSpec, dispatch(), fetch_with_spare_key(), json_message(), phase1_call(), phase2_call(), pretty_body(), provider_fault() (+5 more)

### Community 94 - "dork_generator.rs"
Cohesion: 0.10
Nodes (32): ACTIVE_SNAPSHOT, compose_single_query(), compute_query_id(), compute_template_id(), DATASET_NAME, DEFAULT_MAX_QUERIES, DorkCatalog, DorkCategory (+24 more)

### Community 95 - "ProfileView"
Cohesion: 0.10
Nodes (30): activate(), content_layout(), draw_actions(), draw_filter_strip(), draw_picker(), draw_profile(), draw_report(), draw_section_body() (+22 more)

### Community 96 - "How"
Cohesion: 0.11
Nodes (18): How, CompanyEmail, Coordinates, DomainAsUrl, EntityPhrase, PackageName, PackageParts, Plain (+10 more)

### Community 97 - "synthesize"
Cohesion: 0.19
Nodes (20): a_dropped_provider_stream_keeps_the_text_already_received(), a_provider_that_rejects_streaming_fails_without_a_hidden_retry(), a_streamed_answer_with_a_bad_citation_ends_on_the_repaired_answer(), an_idle_stall_or_the_ceiling_keeps_the_partial_answer(), answer_text(), cancel_during_synthesis_marks_the_run_cancelled_and_keeps_partial_text(), chat_server(), ChatReply (+12 more)

### Community 98 - "summary_card.rs"
Cohesion: 0.32
Nodes (11): actions(), button_at(), button_cells(), button_row_area(), draw(), height(), is_card_button(), label() (+3 more)

### Community 99 - "actor_review.rs"
Cohesion: 0.26
Nodes (11): ActorReviewItem, ActorReviewResult, apply_reviewed_actors(), deterministic_review_actors(), is_meaningful_actor(), JUNK_ACTORS, model_review_actors(), parse_actor_review_response() (+3 more)

### Community 100 - "ModuleId"
Cohesion: 0.15
Nodes (16): ModuleId, .ALL, Atlas, Brain, Intel, Jobs, Logs, Osint (+8 more)

### Community 101 - "Region"
Cohesion: 0.11
Nodes (19): Region, AtlasInsights, AtlasNews, AtlasOrigins, AtlasRuns, Chat, Detail, IntelBrief (+11 more)

### Community 102 - "graph_explanation/tests.rs"
Cohesion: 0.20
Nodes (19): auth_failure_consumes_primary_budget_gives_guidance_and_redacts(), conn(), count(), failure_is_durable_correlated_bounded_and_harmless(), fixture(), Fx, GOOD, late_completion_for_a_changed_or_deleted_memory_is_not_published() (+11 more)

### Community 103 - "validate.rs"
Cohesion: 0.17
Nodes (17): paywall_and_snippet_fail_validation(), BLOCK_MARKERS, BodyQuality, Complete, Partial, Unavailable, Uncertain, BodyValidation (+9 more)

### Community 104 - "Category"
Cohesion: 0.06
Nodes (33): Category, Auth, Cancelled, Configuration, Empty, InvalidModel, InvalidResult, MalformedPayload (+25 more)

### Community 105 - "Rect"
Cohesion: 0.10
Nodes (57): abs_contains(), add_fallback_layout(), add_fallback_popup_area(), api_key_slot(), atlas_hit(), atlas_live_areas(), atlas_news_areas(), atlas_row_at() (+49 more)

### Community 106 - "search_engines/tests.rs"
Cohesion: 0.10
Nodes (42): build_serp_url(), a_non_serp_path_on_the_engine_host_is_not_a_serp_page(), a_noscript_wrapped_consent_meta_refresh_is_consent(), a_single_retry_is_allowed_only_for_a_parser_mismatch_with_a_dom(), a_standalone_zero_count_in_a_status_region_is_a_verified_zero(), a_zero_phrase_outside_a_status_region_is_not_a_verified_zero(), an_off_host_final_url_with_cards_keeps_the_items_but_is_not_valid(), an_off_host_final_url_without_cards_is_never_a_zero() (+34 more)

### Community 107 - "profile_config.rs"
Cohesion: 0.22
Nodes (10): a_validation_error_reports_a_pointer_and_never_a_value(), credential_summary(), draw(), draw_export(), draw_import(), EditorError, locate(), rows_at() (+2 more)

### Community 108 - "run_turn"
Cohesion: 0.08
Nodes (44): a_budgeted_call_with_a_dependency_from_an_earlier_batch_runs(), ac3_no_key_reaches_inputs_cache_keys_plan_json_raw_bodies_or_logs(), ac6_a_courtlistener_429_still_lets_the_turn_synthesize(), ac8_status_errors_and_401_403_fail_readably_and_are_not_cached(), advance_stage(), bounded_reason(), call_cached(), cancel_mid_dispatch_releases_credit_holds() (+36 more)

### Community 109 - "Architecture"
Cohesion: 0.10
Nodes (21): Admiralty source evaluation (WP:RSP), Architecture, Atlas, Binder and executor, Brain recall, Config transfer and the commit, Directives, Intel reports (+13 more)

### Community 110 - "WorkEvent"
Cohesion: 0.08
Nodes (22): work_event(), WorkEvent, AnswerDelta, AnswerNote, AnswerReplacement, AnswerReset, AtlasDone, BrainRelatedExplained (+14 more)

### Community 111 - "store.rs"
Cohesion: 0.06
Nodes (37): article(), ArticleInsightCommit, atlas_article_from_row(), atlas_articles_for_intel_dedupes_same_article_id_across_runs(), atlas_articles_for_intel_filters_category_day_and_query(), atlas_auto_next_round_trips_through_app_state(), atlas_prune_drops_finished_runs_older_than_the_cutoff(), atlas_recent_articles_lists_retained_rows() (+29 more)

### Community 112 - "ClockSet"
Cohesion: 0.21
Nodes (3): ClockSet, queue_time_does_not_spend_active_but_consumes_wall(), sequential_vs_overlapping_tool_budgets()

### Community 113 - "ProviderAdmission"
Cohesion: 0.18
Nodes (6): shared_rate_limit_cooldown_blocks_admission(), AdmissionGuard, note_shared_rate_limit(), provider_admission_caps_at_two(), ProviderAdmission, rate_limit_cooldown_blocks_acquire()

### Community 114 - "components.rs"
Cohesion: 0.17
Nodes (11): action_rects(), analytics_card(), clip_text(), compact_records_keep_trailing_numeric_values(), detail_records(), detail_table(), editor_height(), measured_lines() (+3 more)

### Community 115 - "KeptClaim"
Cohesion: 0.20
Nodes (18): apply_admiralty_evaluation(), brief_text(), cap_claims(), countries_differ(), dedupe_claims(), entity_path(), fingerprint(), finish_insights() (+10 more)

### Community 116 - "gsd-v2.js"
Cohesion: 0.10
Nodes (12): core, home, hooks, names, payload(), run(), setup(), config (+4 more)

### Community 117 - "ServiceResult"
Cohesion: 0.14
Nodes (14): cached_result(), CheckSignal, Blocked, Error, Inconclusive, NotRegistered, RateLimited, Registered (+6 more)

### Community 118 - "briefing_view.rs"
Cohesion: 0.21
Nodes (18): bucket_extracted(), bucket_extracted_with_explanations(), buckets_split_facts_inferences_context_and_links(), claim(), claim_label(), explain_intel_links_with_model(), explain_relation_link(), ExtractedBuckets (+10 more)

### Community 119 - "subscription.rs"
Cohesion: 0.35
Nodes (8): answer_event(), check_login(), command(), complete(), completion_is_ephemeral_and_does_not_inherit_tools_or_configuration(), exec_args(), is_subscription_status(), login()

### Community 120 - "profile.rs"
Cohesion: 0.09
Nodes (65): amplification_lines(), analytics_lines(), applicable_filters(), attribution_lines(), backlog_lines(), bucket_end(), bucket_start(), capacity_lines() (+57 more)

### Community 121 - "IndexOutcome"
Cohesion: 0.12
Nodes (6): IndexOutcome, Disabled, Pending, PermanentFailure, Ready, RetryableFailure

### Community 122 - "recon/graph.rs"
Cohesion: 0.06
Nodes (59): draw(), draw_path(), draw_summary(), glyph(), inset(), legend_height(), legend_parts(), legend_rows() (+51 more)

### Community 123 - "tasks.rs"
Cohesion: 0.08
Nodes (63): add_missing_columns(), adopt_untracked_index_changes(), block_claimed(), claim_index_changes(), claim_index_work(), claim_next_in(), claim_task(), claim_with() (+55 more)

### Community 124 - "ReconCommand"
Cohesion: 0.17
Nodes (12): ReconCommand, Ask, AskNew, Delete, Limits, List, New, Rename (+4 more)

### Community 125 - "config_transfer_tests.rs"
Cohesion: 0.07
Nodes (58): apply_import(), commit_profile_config(), ConfigChange, ConfigurationSnapshot, describe_changes(), export_document(), export_kind(), export_to_path() (+50 more)

### Community 126 - "Service"
Cohesion: 0.14
Nodes (8): AnswerContext, finish_recon_job(), credit_map(), note_cache(), Run, Service, snapshot_secret(), turn_clock()

### Community 127 - "AuthFile"
Cohesion: 0.13
Nodes (10): resolve_actor_reviewer_secret(), RoleRuntime, accounts_persist_with_owner_only_permissions(), assert_no_staged_files(), auth_save_commits_only_the_auth_slot(), AuthFile, legacy_connection_migrates_without_overwriting_other_accounts(), loading_old_auth_removes_gmail_setup_credentials() (+2 more)

### Community 128 - "ServiceSpec"
Cohesion: 0.18
Nodes (9): by_id(), CATALOG, CATALOG_LEN, UPSTREAM_COMMIT, ServiceSpec, ServiceState, Enabled, Experimental (+1 more)

### Community 129 - "TurnClock"
Cohesion: 0.10
Nodes (6): a_round_keeps_its_time_and_a_tight_ceiling_still_runs_tools(), deadline_seconds(), format_deadline(), format_span(), the_label_updates_when_a_round_adds_calls_and_when_evidence_is_measured(), TurnClock

### Community 130 - "opencode.json"
Cohesion: 0.07
Nodes (30): agent, build, general, plan, mode, model, options, permissions (+22 more)

### Community 131 - "Functional Requirements"
Cohesion: 0.12
Nodes (15): Atlas (News Pipeline), Brain / Memory, Configuration Requirements, Context Provider Keys, Functional Requirements, Non-Functional Requirements, Non-Goals, OSINT Manual Runs (+7 more)

### Community 132 - "JobStatusFilter"
Cohesion: 0.18
Nodes (8): JobFilter, JobStatusFilter, Active, All, Completed, Failed, Retrying, list_jobs()

### Community 133 - "super"
Cohesion: 0.10
Nodes (12): CODES, NAMES, ADAPTER_VERSION, HOST, ID, positive_negative_and_unknown(), request_url(), ADAPTER_VERSION (+4 more)

### Community 134 - "ui.rs"
Cohesion: 0.04
Nodes (63): ACTION_H, ApiKeySlot, atlas_auto_label(), atlas_countdown(), atlas_extracting(), atlas_history_live_label(), atlas_run_card(), atlas_run_label() (+55 more)

### Community 135 - "DateTime"
Cohesion: 0.08
Nodes (40): active_jobs(), adaptive_bucket_seconds(), BriefFact, Bucket, bucket_count(), bucket_floor(), bucket_format(), bucket_start() (+32 more)

### Community 136 - "OsintCommand"
Cohesion: 0.14
Nodes (14): DatasetCommand, Import, Refresh, Status, OsintCommand, Attach, Dataset, Describe (+6 more)

### Community 137 - "normalize_destination"
Cohesion: 0.21
Nodes (14): detect_interstitial(), host_in_any(), host_is(), host_is_captcha(), host_is_consent(), is_ad_host(), is_private_host(), meta_refresh_is_consent() (+6 more)

### Community 138 - "TelemetryEvent"
Cohesion: 0.18
Nodes (3): clamp(), safe_payload(), TelemetryEvent

### Community 139 - "Argos OSINT — Agent Instructions"
Cohesion: 0.11
Nodes (18): Architecture notes, Argos OSINT — Agent Instructions, CI (`.github/workflows/ci.yml`), CLI Entry Points (`argos` binary), Developer Commands, diagrams, Environment Variables, graphify (+10 more)

### Community 140 - "logical_attempt_and_cache_rows_stay_distinct_under_replay"
Cohesion: 0.29
Nodes (7): coverage_reports_observed_since_and_counts(), db(), events_persist_and_roll_up_once(), id_is_stable_so_replay_never_double_counts(), logical_attempt_and_cache_rows_stay_distinct_under_replay(), record(), tool_invocation()

### Community 141 - "TaskStatus"
Cohesion: 0.07
Nodes (14): TaskRecord, TaskStatus, Cancelled, Completed, Deferred, Failed, Partial, Planned (+6 more)

### Community 142 - "wikipedia_rsp.rs"
Cohesion: 0.08
Nodes (44): admiralty_scales_claim_confidence_from_rsp_and_peers(), article(), raw_initial_confidence_is_captured_before_source_scaling(), recover_legacy_packet(), recover_legacy_packet_has_no_initial_confidence(), the_packet_keeps_four_and_reports_the_full_gate(), API, APP_STATE_KEY (+36 more)

### Community 143 - "rusqlite"
Cohesion: 0.09
Nodes (19): Cached, None, Stale, Valid, Gate, CoolingDown, Ready, Running (+11 more)

### Community 144 - "serde"
Cohesion: 0.18
Nodes (4): DecisionState, EvidencePassageState, SubjectStateSummary, TaskStateSummary

### Community 145 - ".default"
Cohesion: 0.13
Nodes (42): a_429_stops_picker_calls_and_the_fallback_finishes_the_list(), a_claimed_email_removes_its_bindings(), a_dispatch_error_fails_the_step_and_the_loop_goes_on(), a_failed_email_finder_gets_one_fallback_and_no_third_picker_call(), a_fallback_with_no_bindings_still_runs_the_satisfied_steps_and_reaches_synthesis(), a_gap_filler_domain_feeds_hunter_once_firecrawl_observes_it(), a_goal_naming_a_tool_or_provider_is_repaired_then_falls_back(), a_goal_naming_a_tool_outside_the_prompt_entity_is_still_rejected() (+34 more)

### Community 146 - "execute_steps"
Cohesion: 0.13
Nodes (32): after_step(), binding_ground(), context_block(), context_dispatched(), directive_for(), execute_steps(), expand_dork_children(), expand_per_platform() (+24 more)

### Community 147 - ".is_empty"
Cohesion: 0.13
Nodes (18): delta_text(), message_reasoning(), message_text(), parse_completion(), parse_completion_errors_clearly_when_truly_empty(), parse_completion_prefers_content_over_reasoning(), parse_completion_separates_reasoning_content(), Poll (+10 more)

### Community 148 - "QueryExecutionStatus"
Cohesion: 0.18
Nodes (11): QueryExecutionStatus, Cancelled, Completed, Failed, Generated, NoResults, Queued, Running (+3 more)

### Community 149 - "results.rs"
Cohesion: 0.13
Nodes (20): classify_output_quality(), extract_observation_items(), extract_search_results(), extracts_from_array_shaped_observations(), extracts_from_object_with_results_array(), normalize_tool_result(), NormalizedToolResult, OutputQuality (+12 more)

### Community 150 - "telemetry.rs"
Cohesion: 0.11
Nodes (13): bin_index(), duration_bins_are_bounded_and_mergeable(), DURATION_BINS_MS, event_bins(), ID_SEQ, is_forbidden_key(), MAX_PAYLOAD_CHARS, measured_elapsed_is_never_negative() (+5 more)

### Community 151 - "SettingsFile"
Cohesion: 0.22
Nodes (3): set_saved_key_round_trips_every_keyed_tool_provider(), SettingsFile, cost_map()

### Community 152 - "anyhow"
Cohesion: 0.14
Nodes (15): DecisionAdapterKind, JsonMode, NativeDecisions, OutputTool, StrictJsonSchema, ValidatedText, parse_general_model_response(), resolve_adapter() (+7 more)

### Community 153 - "modes.rs"
Cohesion: 0.27
Nodes (13): scoped_section_plan(), chat_response_spec(), chat_response_spec_embeds_section_guidelines_and_style_guide(), chat_response_spec_lists_mode_headings(), chat_section_titles(), every_mode_has_bluf_first_and_sources_last(), every_mode_section_has_guideline(), investigation_mode_spec() (+5 more)

### Community 154 - "§9 implementation order"
Cohesion: 0.15
Nodes (12): §10 acceptance checks, §9 implementation order, Atlas memory completion and System apps — implementation checklist, Phase 1 — what landed, Phase 2 — what landed, Phase 3 — what landed, Phase 4 — what landed, Phase 5a — what landed (+4 more)

### Community 155 - ".new"
Cohesion: 0.28
Nodes (5): aging_the_tool_clock_blocks_new_calls_and_cache_hits_do_not_extend_it(), clock_set_for_turn(), foreground_expiry_continues_in_background(), sequential_raise_calls_budgets_higher(), TurnCheckpoint

### Community 156 - "investigation/evidence.rs"
Cohesion: 0.12
Nodes (24): accepted_evidence_events_are_stable_and_keep_provenance(), assess_claim_against_passages(), ClaimAssessmentOutcome, ClaimStance, Disputed, Insufficient, Mention, Supported (+16 more)

### Community 157 - "InvestigationPattern"
Cohesion: 0.10
Nodes (17): InvestigationPattern, ArticleVerification, Bitcoin, BreakingNews, DomainIp, EmailAttribution, FollowUp, GeneralSubject (+9 more)

### Community 164 - "What Was Learned"
Cohesion: 0.15
Nodes (12): CLI Entry Points, Codebase Structure, Investigation Flow, Model Roles (configured independently in Providers → Defaults), Next Commands, Onboarding Summary — argos-osint, Planning Artifacts Created, Primary Providers (+4 more)

### Community 165 - "ledger.rs"
Cohesion: 0.13
Nodes (13): body_assertion_candidates(), coverage_complete(), ElementStatus, Assessed, Excluded, InProgress, Pending, Superseded (+5 more)

### Community 166 - "SiteOutcomeStatus"
Cohesion: 0.18
Nodes (10): deserialize_flexible_bool(), SiteOutcomeStatus, Ambiguous, Blocked, Error, Found, NotFound, RateLimited (+2 more)

### Community 167 - "investigation.rs"
Cohesion: 0.05
Nodes (101): Account, account_hits(), ACCOUNT_PLATFORMS, account_platforms_attach_to_the_subject_and_are_never_entities(), ACCOUNTS, accounts_flow(), action_order(), actions_are_grounded_capped_and_not_a_sweep() (+93 more)

### Community 168 - "Milestone 1 — Core Onboarding (Current)"
Cohesion: 0.18
Nodes (10): Deliverables, Dependencies, Future Milestones (Planned), Milestone 1 — Core Onboarding (Current), Milestone 2 — CLI Verification, Milestone 3 — Test Coverage, Milestone 4 — Integration & Ship, Objectives (+2 more)

### Community 169 - "paths.rs"
Cohesion: 0.25
Nodes (11): auth_path(), config_journal_path(), config_lock_path(), config_path(), dataset_dir(), datasets_dir(), db_path(), ensure_home() (+3 more)

### Community 170 - "Connection"
Cohesion: 0.26
Nodes (11): Coverage, ensure_observed_since(), get_meta(), insert(), next_id(), prune(), rollup(), rusqlite_result() (+3 more)

### Community 171 - "atlas_work.rs"
Cohesion: 0.06
Nodes (35): articles_rev(), AttemptRecord, check_dependency_coverage(), CycleOutcome, Blocked, Cancelled, Completed, CompletedWithWarnings (+27 more)

### Community 172 - "JobRow"
Cohesion: 0.26
Nodes (4): get_job(), job_row(), JobRow, parse()

### Community 173 - "decide"
Cohesion: 0.20
Nodes (8): decide(), DecisionAnswer, decisions_client_posts_to_alpha_decisions_and_parses_answers(), decisions_url(), DecisionsResponse, live_decisions_smoke(), normalize_base(), parse_decisions()

### Community 174 - "Codebase Map — argos-osint"
Cohesion: 0.22
Nodes (8): `argos-osint-bin` — CLI and TUI, `argos-osint-core` — Core Library, Codebase Map — argos-osint, Crates, Documentation Ingest, Primary Data Flow, State & Config (all under `~/.argos`), Tool Input & Binding Kinds (from `tool_io.rs`)

### Community 175 - "PROJECT.md — argos-osint"
Cohesion: 0.22
Nodes (8): Applications, CLI Entry Points, Core Components, Crates, Investigation Flow, PROJECT.md — argos-osint, Project Purpose, State Directory (`~/.argos`)

### Community 176 - "Current Phase State"
Cohesion: 0.25
Nodes (7): Artifacts Status, Configuration State, Current Phase State, Next Steps, Pending Items, Phase: Core Onboarding, STATE.md — argos-osint

### Community 177 - "QuestionSpec"
Cohesion: 0.25
Nodes (5): DecisionQuestionType, Choice, Noul, Score, QuestionSpec

### Community 178 - "RecoveryAction"
Cohesion: 0.12
Nodes (12): classify_recovery(), RecoveryAction, AccessRestricted, CircuitCooldown, ContentRefused, CreditsExhausted, FallbackModel, ReduceContext (+4 more)

### Community 179 - "grok_oauth.rs"
Cohesion: 0.13
Nodes (20): auth_path(), bearer(), bearer_from_path(), check_login(), Entry, entry_from(), failed_login_keeps_the_cli_reason_and_recovery_action(), login() (+12 more)

### Community 180 - "TaskState"
Cohesion: 0.18
Nodes (9): TaskState, Cancelled, Completed, Failed, Paused, Queued, RetryScheduled, Running (+1 more)

### Community 181 - "Conventions"
Cohesion: 0.22
Nodes (9): Agent scratch, Checks, Conventions, Diagrams, Graphify, Planning files, Schema, Tests (+1 more)

### Community 182 - "Docs Ingest — argos-osint"
Cohesion: 0.33
Nodes (5): `docs/architecture.md`, Docs Ingest — argos-osint, `docs/providers.md`, Ingest Status, Ingested Documentation

### Community 183 - "ScheduledCall"
Cohesion: 0.38
Nodes (8): courtlistener_spacing_and_firecrawl_polling_are_counted(), live(), more_calls_mean_more_tool_time_and_cache_hits_add_nothing(), scheduled(), ScheduledCall, tool_allowance_for_deps(), tool_allowance_for_deps_sequential_sums(), tool_allowance_seconds()

### Community 184 - "config_transfer.rs"
Cohesion: 0.08
Nodes (64): apply_provider(), apply_role(), apply_tool_slot(), array_value(), check_known_fields(), check_limit(), check_required(), check_role_compatibility() (+56 more)

### Community 185 - "OpenCode V2 workflow"
Cohesion: 0.29
Nodes (7): Builds, ECC commands, Graph first, OpenCode V2 workflow, Phase edits, Primary tools, Verify setup

### Community 186 - "Unified reliability / summarization / semantic pipelines — completion checklist"
Cohesion: 0.40
Nodes (4): Honest remaining gaps, Phase status (§18), This follow-up, Unified reliability / summarization / semantic pipelines — completion checklist

### Community 187 - "Concepts"
Cohesion: 0.25
Nodes (8): Apps and internal IDs, Bindings, Concepts, Graph, Intel report jobs, Model roles, Persistence, Primary OSINT providers

### Community 201 - "explore.rs"
Cohesion: 0.20
Nodes (11): GraphEdgeKind, ContradictionOrUpdate, Evidenced, Inferred, SemanticSuggestion, related_evidence_view(), RelatedEvidenceHit, semantic_edges_are_not_factual() (+3 more)

### Community 202 - "TUI components"
Cohesion: 0.18
Nodes (11): Acceptance, ActionBar and Overlay, AnalyticsCard and grid preset, Application presets, DetailTable and report preset, Measured text, MeasuredEditor and TranscriptBlock, RankedBars, ComparisonBars and Meter (+3 more)

### Community 203 - "ConfigView"
Cohesion: 0.14
Nodes (6): a_directory_target_is_an_actionable_error_not_a_crash(), an_oversize_document_is_refused_before_anything_is_applied(), ConfigView, editing_the_buffer_disarms_the_import_button(), expand_home(), the_pasted_secret_buffer_is_cleared_on_close()

### Community 204 - "Providers"
Cohesion: 0.25
Nodes (8): Budgeted executor, Graph explanation jobs, News and legal keys, OSINT data providers, Providers, Tool picker transport, Typed provider diagnostics, Verification and testing

### Community 205 - "Decision roles and output contracts"
Cohesion: 0.29
Nodes (6): 12-template registry, Adapters, Decision roles and output contracts, State isolation, Thresholds, TUI

### Community 206 - "AnalyticsLayout"
Cohesion: 0.40
Nodes (3): AnalyticsLayout, grid_boundary_and_short_viewports(), ReportLayout

### Community 207 - "Terminal UI"
Cohesion: 0.22
Nodes (9): Atlas, CLI, Intel, Other apps, Profile, Recon, State, Terminal UI (+1 more)

### Community 208 - "docs/README.md"
Cohesion: 0.30
Nodes (4): Agent rule, Catalog, Diagram conventions, When to draw

### Community 209 - "Profile dashboard and search"
Cohesion: 0.12
Nodes (17): Analytics controls, Atlas — 6 widgets, Import semantics, Intel — 7 widgets, Metric dictionary, Models — 8 widgets, Named search engines, Profile dashboard and search (+9 more)

### Community 210 - "RunStats"
Cohesion: 0.13
Nodes (13): a_resumed_cycle_continues_candidate_numbering(), cycle_event_id_is_the_run_key_so_replay_never_double_counts(), CycleTelemetry, db(), each_candidate_occurrence_gets_exactly_one_disposition(), OpenStage, origin_snapshots_are_one_row_per_origin_per_cycle(), parse_stats() (+5 more)

### Community 212 - "extract_items"
Cohesion: 0.19
Nodes (10): EngineSearchResult, extract_items(), ParserInput, CleanHtml, LinksOnly, Missing, RawHtml, retry_allowed() (+2 more)

### Community 213 - "render_tui_cells.py"
Cohesion: 0.27
Nodes (3): box(), color(), render()

### Community 214 - ".order"
Cohesion: 0.24
Nodes (10): checked_serves(), context_additions(), context_tools(), fallback_record(), OrderContext, Ordered, Picker<'a>, serves_for() (+2 more)

### Community 215 - "intel_recon/brain.rs"
Cohesion: 0.57
Nodes (5): intel_recon_publishes_durably_with_provenance_and_no_atlas_receipt(), ReconInsightUpdate, update(), upsert_adds_and_updates_claims(), upsert_recon_insights()

### Community 216 - "Unified Investigation Harness"
Cohesion: 0.29
Nodes (6): Catalog projection, Model roles, Persistence, Reasoning channels, Unified Investigation Harness, Validation gates

### Community 217 - "Json"
Cohesion: 0.43
Nodes (6): DiversityRow, json_array_len(), json_at(), json_bool(), json_str(), Json

### Community 218 - ".evaluate"
Cohesion: 0.16
Nodes (12): DecisionService, DecisionPolicy, EnforcementOutcome, Pass, Reject, Unavailable, Uncertain, ThresholdProfile (+4 more)

### Community 219 - "TurnContinuation"
Cohesion: 0.40
Nodes (4): TurnContinuation, Active, Background, Exhausted

### Community 220 - "Argos OSINT"
Cohesion: 0.33
Nodes (6): Applications, Argos OSINT, Documentation, Figures, Limits, Quick start

### Community 221 - "ToolOutcome"
Cohesion: 0.11
Nodes (15): serp_tool_outcome(), cache_ttl_for(), is_named_serp(), SerpCacheClass, Failure, Valid, VerifiedZero, ToolOutcome (+7 more)

### Community 222 - "EventKind"
Cohesion: 0.10
Nodes (19): EventKind, .ALL, AtlasCandidate, AtlasCycle, AtlasOriginSnapshot, AtlasStage, DirectiveAssessed, EvidenceItem (+11 more)

### Community 223 - "Argos documentation"
Cohesion: 0.40
Nodes (5): Agent and workflow, Argos documentation, Checklists (historical), Figures, Product

### Community 225 - "accept_claims"
Cohesion: 0.15
Nodes (18): a_description_only_span_is_inference_and_an_added_name_is_dropped(), a_fact_requires_both_spans_in_the_title(), accept_claims(), AcceptMode, Context, Lead, ask_claims(), AskedClaims (+10 more)

### Community 226 - "Argos UI Interaction Audit"
Cohesion: 0.40
Nodes (5): Argos UI Interaction Audit, Audit Matrix, Field inventory (`field_placeholder`), Remaining limits, Shared contracts

### Community 229 - "PickRequest"
Cohesion: 0.26
Nodes (7): chat_request(), parse_chat_pick(), PickReply, PickRequest, PickRequestAdapter, PickRequestAdapter<'r, 'b>, state()

### Community 230 - "enqueue_job"
Cohesion: 0.36
Nodes (7): lease_fencing_rejects_stale_epoch_and_foreign_owner(), claim_complete_and_retry_round_trip(), claim_next(), enqueue_job(), enqueue_job_with(), JobMeta, NewJob

### Community 231 - "brain_lance_off.rs"
Cohesion: 0.15
Nodes (16): begin_generation(), block_on(), clear_fingerprint(), current_fingerprint(), DUPLICATE_THRESHOLD, fingerprint_matches(), GenerationProgress, LAST_ERROR (+8 more)

### Community 232 - "Cell"
Cohesion: 0.15
Nodes (4): Cell, Dims, dims_from_key(), Reader<'a>

### Community 233 - "serde_json"
Cohesion: 0.19
Nodes (17): ADAPTER_VERSION, HOST, ID, positive_negative_and_source_field(), request_url(), every_fixture_matches_its_recorded_outcome(), field(), fixture_cases() (+9 more)

### Community 236 - "ACTIVE_SNAPSHOT"
Cohesion: 0.29
Nodes (4): ACTIVE_SNAPSHOT, HOST_PACER, HostPacer, SITE_CACHE

### Community 237 - "parse_serp_response"
Cohesion: 0.15
Nodes (21): clip_diag(), is_serp_path(), num_at(), parse_serp_response(), str_at(), a_redirected_final_url_is_not_used_to_rebuild_the_query(), an_empty_dom_without_any_payload_is_a_parser_mismatch(), api_http_404_is_an_upstream_failure() (+13 more)

### Community 238 - "home_rows"
Cohesion: 0.23
Nodes (15): center_row(), gap_row(), home_group(), home_line_text(), home_pads_titles_and_application_order(), home_rows(), HomeKind, Gap (+7 more)

### Community 239 - "atlas_actions.rs"
Cohesion: 0.29
Nodes (4): record_start(), start_repair(), starts(), infer_app()

### Community 240 - "provider_metrics.rs"
Cohesion: 0.24
Nodes (9): cache_capacity_rows(), cached_rows_become_an_available_snapshot(), capacity_snapshot(), CapacityRow, CapacitySnapshot, memory(), missing_companion_snapshot_is_unavailable_not_zero(), QueueRow (+1 more)

### Community 242 - "Store"
Cohesion: 0.53
Nodes (5): charge_newsapi(), charge_quota(), open_key(), quota_open(), utc_day()

### Community 243 - "SerpOutcome"
Cohesion: 0.15
Nodes (11): SerpOutcome, Challenge, Consent, ParserMismatch, RateLimited, ResponseTooLarge, UpstreamFailure, Valid (+3 more)

### Community 244 - ".recon_outcomes"
Cohesion: 0.20
Nodes (9): normalize_run_outcome(), ReconOutcomeBucket, RunOutcome, Cancelled, CompletedWithEvidence, CompletedZeroEvidence, Failed, Partial (+1 more)

### Community 245 - ".job_source"
Cohesion: 0.50
Nodes (4): AtlasRun, JobSource, AtlasRun, ReconThread

### Community 246 - "Trigger"
Cohesion: 0.22
Nodes (9): TELEMETRY_TRIGGER, Trigger, AtlasCycle, IntelBrief, ManualTool, ReconPrompt, Repair, Scheduled (+1 more)

### Community 247 - "ExecuteOptions"
Cohesion: 0.38
Nodes (5): cancellable_sleep(), cancelled(), ExecuteOptions, sleep_recorded(), wait_cancelled()

### Community 248 - "BrainResourceSummary"
Cohesion: 0.20
Nodes (10): bindings_use_brain_evidence_ids(), BrainResourceHit, BrainResourceSummary, candidate_json(), clip(), format_counts(), is_http_url(), parse_brain_scrape_index() (+2 more)

### Community 249 - "select_strategy"
Cohesion: 0.40
Nodes (6): has_concrete_identifier(), hypothesis_question(), select_strategy(), strategy_change_reason(), strategy_follows_the_question_and_can_change_without_erasing_work(), StrategyChoice

### Community 250 - "ConfigTab"
Cohesion: 0.40
Nodes (3): ConfigTab, Export, Import

### Community 251 - "LaunchState"
Cohesion: 0.40
Nodes (5): LaunchState, Accepted, Accepting, Editable, RecoverableFailure

### Community 252 - "SystemTab"
Cohesion: 0.40
Nodes (3): SystemTab, Overview, System

### Community 253 - "ReportOutcome"
Cohesion: 0.22
Nodes (8): normalize_report_outcome(), ReportOutcome, Blocked, Cancelled, Completed, Failed, Partial, Waiting

### Community 254 - "BrainListMode"
Cohesion: 0.50
Nodes (4): BrainListMode, Create, Graph, List

### Community 255 - "DispatchError"
Cohesion: 0.25
Nodes (5): DispatchError, RetryDisposition, NextRoute, RetryRoute, Stop

### Community 260 - "PlanInterval"
Cohesion: 0.33
Nodes (5): PlanInterval, Daily, Monthly, Never, Weekly

### Community 262 - "Target"
Cohesion: 0.06
Nodes (33): FocusEntry, hit(), LayoutRegistry, Target, App, AtlasCycleStats, BrainMark, BrainRecall (+25 more)

## Knowledge Gaps
- **1782 isolated node(s):** `$schema`, `default_agent`, `subagent_depth`, `timeout`, `chunkTimeout` (+1777 more)
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 2448 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **33 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `App` connect `App` to `Call`, `app.rs`, `Target`, `unix_now`, `.activate_button`, `Store`, `SettingsFile`, `hardware.rs`, `Store`, `brain_detail.rs`, `ReportMode`, `JobsView`, `now`, `worker.rs`, `.handle_key`, `src/brain.rs`, `Overlay`, `ConfigView`, `InvestigationSurface`, `logs.rs`, `RunStats`, `FeedArticle`, `Field`, `DefaultsRole`, `ToolResult`, `AtlasArticleRow`, `ProfileView`, `summary_card.rs`, `ModuleId`, `recon/graph.rs`, `WorkEvent`, `.job_source`, `briefing_view.rs`, `ConfigTab`, `LaunchState`, `BrainListMode`, `AuthFile`?**
  _High betweenness centrality (0.082) - this node is a cross-community bridge._
- **What connects `$schema`, `default_agent`, `subagent_depth` to the rest of the system?**
  _1782 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `land.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.0076045627376425855 - nodes in this community are weakly interconnected._
- **Why does `Value` connect `Value` to `orchestrate.rs`, `Call`, `atlas_memory.rs`, `tool_io.rs`, `ui.rs`, `recon.rs`, `TelemetryEvent`, `Store`, `IntelligenceCategory`, `TaskStatus`, `wikipedia_rsp.rs`, `directives.rs`, `serde`, `SearchHit`, `providers.rs`, `.is_empty`, `.default`, `results.rs`, `execute_steps`, `whoxy.rs`, `news_legal.rs`, `telemetry.rs`, `osint.rs`, `holehe/mod.rs`, `TurnEvent`, `body.rs`, `provider.rs`, `investigation.rs`, `run_atlas_inner`, `provider_attempt.rs`, `decide`, `picker.rs`, `ReportMode`, `grok_oauth.rs`, `events.rs`, `model_exec.rs`, `config_transfer.rs`, `now`, `cli.rs`, `atlas_insights.rs`, `provider_diag.rs`, `body_filter.rs`, `synthesize.rs`, `summarization.rs`, `InvestigationSurface`, `search_engines.rs`, `.new`, `RunStats`, `src/evidence.rs`, `ToolResult`, `DecisionContract`, `AtlasArticleRow`, `F`, `dork_generator.rs`, `ReconLimits`, `accept_claims`, `BrainResourceSummary`, `PickRequest`, `serde_json`, `search_engines/tests.rs`, `run_turn`, `parse_serp_response`, `components.rs`, `SerpOutcome`, `ServiceResult`, `subscription.rs`, `profile.rs`, `config_transfer_tests.rs`, `Service`?**
  _High betweenness centrality (0.072) - this node is a cross-community bridge._
- **Should `orchestrate.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.0441358024691358 - nodes in this community are weakly interconnected._
- **Why does `FieldId` connect `FieldId` to `app.rs`, `ui.rs`, `Target`, `Rect`, `DefaultsRole`, `Frame`, `.handle_key`?**
  _High betweenness centrality (0.037) - this node is a cross-community bridge._
- **Should `Call` be split into smaller, more focused modules?**
  _Cohesion score 0.07632850241545894 - nodes in this community are weakly interconnected._