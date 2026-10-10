# Graph Report - argos-osint  (2026-10-10)

## Corpus Check
- 253 files · ~558,580 words
- Verdict: corpus is large enough that graph structure adds value.
- Unclassified: 5 file(s) not represented in the graph (top: (none) 3, .toml 2)

## Summary
- 8098 nodes · 20857 edges · 293 communities (254 shown, 39 thin omitted)
- Extraction: 98% EXTRACTED · 2% INFERRED · 0% AMBIGUOUS · INFERRED: 386 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `864bffbe`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- land.rs
- orchestrate.rs
- ProfileSnapshot
- Store
- tool_io.rs
- app.rs
- brain_lance.rs
- FieldId
- map.rs
- recon.rs
- ButtonId
- Store
- serde_json
- job_registry.rs
- unix_now
- directives.rs
- article_image.rs
- absorb_hit
- Value
- atlas_news.rs
- Store
- publication.rs
- gates.rs
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
- telemetry.rs
- Call
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
- tui/jobs.rs
- App
- events.rs
- exec.rs
- Frame
- model_exec.rs
- ErrorCategory
- MemoryPhase
- MemoryKind
- draw_brain
- brain_resources.rs
- worker.rs
- .set_focus
- src/brain.rs
- intel_recon/jobs.rs
- cli.rs
- atlas_insights.rs
- SourceReliability
- .new
- body_filter.rs
- synthesize.rs
- dataset.rs
- Loaded
- profile_charts.rs
- atlas_table.rs
- Overlay
- pipeline.rs
- summarization.rs
- theme.rs
- .reload_memories
- search_engines.rs
- logs.rs
- .new
- FeedArticle
- budget.rs
- validate.rs
- Command
- DefaultsRole
- graph_explanation.rs
- src/evidence.rs
- ToolRunner
- DecisionContract
- AtlasArticleRow
- AtlasEvent
- dork_generator.rs
- Section
- How
- .memory
- summary_card.rs
- actor_review.rs
- ModuleId
- Region
- diversity.rs
- .default
- Category
- Rect
- search_engines/tests.rs
- profile_config.rs
- execute_ordered
- Architecture
- WorkEvent
- store.rs
- ClockSet
- ProviderAdmission
- components.rs
- ToolResult
- gsd-v2.js
- ServiceResult
- briefing_view.rs
- subscription.rs
- Line
- DecisionAdapterKind
- recon/graph.rs
- tasks.rs
- ReconCommand
- config_transfer_tests.rs
- run
- SettingsFile
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
- Service
- Argos OSINT — Agent Instructions
- ProviderSecret
- InvestigationSurface
- wikipedia_rsp.rs
- rusqlite
- DecisionState
- ProviderFailure
- execute_steps
- ledger.rs
- QueryExecutionStatus
- results.rs
- TelemetryEvent
- SerpOutcome
- profile.rs
- App
- tui_review.py
- .new
- investigation/evidence.rs
- InvestigationPattern
- Part
- argos-osint-bin
- What Was Learned
- Supplied<T>
- SiteOutcomeStatus
- investigation.rs
- Milestone 1 — Core Onboarding (Current)
- paths.rs
- model_roles.rs
- atlas_work.rs
- rule_bindings
- tool_runner.rs
- Codebase Map — argos-osint
- PROJECT.md — argos-osint
- Current Phase State
- Trigger
- RecoveryAction
- grok_oauth.rs
- mem
- Conventions
- Docs Ingest — argos-osint
- ScheduledCall
- config_transfer.rs
- OpenCode V2 workflow
- home_rows
- Concepts
- package.json
- explore.rs
- TUI components
- ConfigView
- Providers
- Decision roles and output contracts
- LayoutResult
- Terminal UI
- Profile dashboard and search
- RunStats
- BrainIndex
- extract_items
- now
- .order
- intel_recon/brain.rs
- Unified Investigation Harness
- RendererKind
- .evaluate
- TurnContinuation
- Argos OSINT
- ToolOutcome
- EventKind
- Argos documentation
- run_turn
- KeptClaim
- Argos UI Interaction Audit
- ChainReport<T>
- ImportBuffer
- synthesize
- serde
- brain_lance_off.rs
- model_facts
- profile_components.rs
- atlas_memory.rs
- parse_serp_response
- seeded
- atlas_actions.rs
- provider_metrics.rs
- modes.rs
- ChatMessage
- .recon_outcomes
- .job_source
- PlanCall
- AtlasInsightClaim
- BrainResourceSummary
- PickReply
- provider_diag.rs
- graph_explanation/tests.rs
- SystemTab
- .intel_reports
- §9 implementation order
- DispatchError
- accept_claims
- run_primary
- run_live
- note_directives
- draw
- Target
- CycleOutcome
- .related_memories
- QuestionSpec
- OperationScope<'a>
- Profile analytics dashboard
- CheckSignal
- TaskState
- Profile TUI implementation session — 2026-10-10
- Diagram conventions
- ModelExecEvent
- PlanInterval
- TUI implementation and verification
- DecisionsAdapter
- Unified reliability / summarization / semantic pipelines — completion checklist
- AtlasCommand
- RepairStatus
- poll_device
- HypothesisRecord
- Json
- select_strategy
- TUI overhaul verification — 10 October 2026
- collections
- parse
- PublicationReceipt

## God Nodes (most connected - your core abstractions)
1. `App` - 321 edges
2. `Value` - 286 edges
3. `ProviderSecret` - 143 edges
4. `ButtonId` - 132 edges
5. `Target` - 89 edges
6. `ToolResult` - 78 edges
7. `FieldId` - 76 edges
8. `Store` - 74 edges
9. `Store` - 64 edges
10. `AtlasArticleRow` - 57 edges

## Surprising Connections (you probably didn't know these)
- `Module layout` --references--> `widget_lines()`  [EXTRACTED]
  docs/architecture.md → crates/argos-osint-bin/src/tui/profile.rs
- `Other application presets` --references--> `draw_system_tab()`  [EXTRACTED]
  docs/tui-design-spec.md → crates/argos-osint-bin/src/tui/profile.rs
- `Profile edit map` --references--> `LayoutResult`  [EXTRACTED]
  docs/tui-design-spec.md → crates/argos-osint-bin/src/tui/profile_layout.rs
- `Other application presets` --references--> `draw_recon_chat()`  [EXTRACTED]
  docs/tui-design-spec.md → crates/argos-osint-bin/src/tui/ui.rs
- `Other application presets` --references--> `draw_osint()`  [EXTRACTED]
  docs/tui-design-spec.md → crates/argos-osint-bin/src/tui/ui.rs

## Import Cycles
- 1-file cycle: `crates/argos-osint-core/src/config_transfer.rs -> crates/argos-osint-core/src/config_transfer.rs`
- 2-file cycle: `crates/argos-osint-core/src/osint.rs -> crates/argos-osint-core/src/osint/results.rs -> crates/argos-osint-core/src/osint.rs`
- 2-file cycle: `crates/argos-osint-bin/src/cli.rs -> crates/argos-osint-core/src/hardware.rs -> crates/argos-osint-bin/src/cli.rs`
- 4-file cycle: `crates/argos-osint-core/src/config_commit.rs -> crates/argos-osint-core/src/provider_attempt.rs -> crates/argos-osint-core/src/provider_diag.rs -> crates/argos-osint-core/src/config_transfer.rs -> crates/argos-osint-core/src/config_commit.rs`

## Communities (293 total, 39 thin omitted)

### Community 0 - "land.rs"
Cohesion: 0.01
Nodes (262): CODES, LABELS, R0, R1, R10, R100, R101, R102 (+254 more)

### Community 1 - "orchestrate.rs"
Cohesion: 0.05
Nodes (58): a_follow_up_keeps_names_from_the_previous_synthesis(), a_handle_named_in_a_derived_question_is_an_unverified_binding(), a_keyword_inside_the_subjects_name_runs_no_news_or_legal_tool(), a_keyword_outside_the_subjects_name_still_runs_news_or_legal(), ac2_without_a_key_news_and_legal_tools_are_never_picked_and_with_a_key_they_are_eligible(), ac4_a_news_prompt_runs_newsapi_search_with_the_entity_and_who_is_runs_none(), ac5_a_sued_prompt_runs_courtlistener_case_or_docket_search_with_the_entity(), ac6_caps_hold_and_a_courtlistener_429_skips_the_rest() (+50 more)

### Community 2 - "ProfileSnapshot"
Cohesion: 0.12
Nodes (14): adaptive_bucket_seconds(), AttentionRow, bucket_count(), GlobalStatus, Period, .ALL, Custom, H1 (+6 more)

### Community 3 - "Store"
Cohesion: 0.19
Nodes (26): disk_store(), finished_run(), legacy_published(), legacy_receipt(), load_checkpoint(), load_progress(), mark_repair_interrupted(), prepare_reextraction() (+18 more)

### Community 4 - "tool_io.rs"
Cohesion: 0.04
Nodes (99): leftovers(), names(), Binding, canonical(), dependency_order(), depends_on(), normalize_platform(), question_bindings_and_dependency_fix() (+91 more)

### Community 5 - "app.rs"
Cohesion: 0.06
Nodes (96): a_finished_tool_call_is_logged_once_and_the_transcript_keeps_a_summary(), all_provider_actions_render_with_hit_areas_at_80x24(), analytics_fixture(), analytics_viewports_and_data_states(), ATLAS_AUTO_SECS, atlas_auto_toggle_persists_the_next_trigger(), atlas_claim(), atlas_headlines_scroll_and_request_failures_reach_the_system_log() (+88 more)

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
Nodes (68): answer_step_rate_limit_is_explicit_and_keeps_results(), attribute_word(), auxiliary(), await_completion(), BRIEF_SYNTHESIS, broad_question_collapses_to_grounded_lookups(), char_ceil(), char_floor() (+60 more)

### Community 10 - "ButtonId"
Cohesion: 0.02
Nodes (108): ButtonId, Add, AddFallback, AtlasAuto, AtlasDelete, AtlasLive, AtlasNews, AtlasNewsFeed (+100 more)

### Community 11 - "Store"
Cohesion: 0.09
Nodes (5): CreditHold, Store, strategy_and_provider_credits_survive_reopen(), Thread, whoxy_prepaid_pool_does_not_reset_monthly()

### Community 12 - "serde_json"
Cohesion: 0.07
Nodes (28): all_catalog_tools_mapped_to_categories(), argument_contract(), ArgumentBuilderContract, compact_capability_catalog(), CompactToolCapability, IntelligenceCategory, Bitcoin, DomainNetwork (+20 more)

### Community 13 - "job_registry.rs"
Cohesion: 0.05
Nodes (48): begin(), begin_cancellable(), db(), db_path(), finish(), beat(), BEAT_INTERVAL, beats() (+40 more)

### Community 14 - "unix_now"
Cohesion: 0.11
Nodes (16): atlas_auto_future_tick_waits_and_past_tick_reschedules(), atlas_auto_while_running_only_arms_the_next_slot(), atlas_history_button_counts_down_while_auto_run_is_on(), atlas_poll_wait(), BrainListMode, Create, Graph, List (+8 more)

### Community 15 - "directives.rs"
Cohesion: 0.06
Nodes (66): paint_logo(), accounts_search_query(), apply_context_targets(), asks(), asks_about_judge(), asks_for_headlines(), clip_words(), context_entity() (+58 more)

### Community 16 - "article_image.rs"
Cohesion: 0.10
Nodes (16): absent_invalid_failed_images_collapse(), ArticleImages, Completion, Decoded, Encoded, decode(), encode(), Entry (+8 more)

### Community 17 - "absorb_hit"
Cohesion: 0.09
Nodes (27): absorb_hit(), account_platform_host(), accounts_flow(), Candidate, complementary_queries(), content_tokens(), DiscoveryQuery, distinct_queries() (+19 more)

### Community 18 - "Value"
Cohesion: 0.03
Nodes (98): NoDuplicates, Value, CallProposal, HandoffRecord, annotate(), clip_page(), https_on_host(), number_arg() (+90 more)

### Community 19 - "atlas_news.rs"
Cohesion: 0.07
Nodes (44): api_error(), ATLAS_TOOLS, authors(), clip(), country_arg(), country_code(), CURRENTS_COUNTRIES, CURRENTS_HOST (+36 more)

### Community 20 - "Store"
Cohesion: 0.06
Nodes (25): article_body_from_row(), ArticleBodyRow, cited_evidence_keeps_provenance_and_revision_identity(), element_from_row(), full_assessment_parents_are_flagged_for_child_report_totals(), IntelAssessmentRow, IntelInvestigationRow, IntelReportSectionRow (+17 more)

### Community 21 - "publication.rs"
Cohesion: 0.08
Nodes (41): active_index_rows(), bump_memories_changed(), change_watcher_sees_only_committed_changes_and_coalesces(), claim(), clear(), clear_index_state(), CoverageReport, delete_run_memory_state() (+33 more)

### Community 22 - "gates.rs"
Cohesion: 0.13
Nodes (12): execute_harness_step(), InvestigationRuntime, ClaimAssessment, GateOutcome, Passed, Rejected, validate_claim_assessment(), validate_evidence_admission() (+4 more)

### Community 23 - "whoxy.rs"
Cohesion: 0.11
Nodes (35): adjacent_changes(), AdjacentChange, balance_request_url(), bounded_model_view(), check_balance(), contact(), ContactCard, date_and_limit_validation() (+27 more)

### Community 24 - "news_legal.rs"
Cohesion: 0.06
Nodes (46): a_rate_limit_switches_to_the_fallback_key_and_a_rejection_does_not(), ac1_catalog_has_55_tools_and_the_news_and_legal_entries(), ac1_each_request_has_its_url_params_and_auth_header_and_no_key_in_the_url(), an_echoed_key_is_redacted_from_the_stored_body_and_error(), article(), articles(), CATEGORIES, choice_arg() (+38 more)

### Community 25 - "hardware.rs"
Cohesion: 0.14
Nodes (19): system_host_lines(), CACHE_TTL_SECS, classify_arch(), disk_totals(), HardwareProfile, metal_vram_gb(), now_secs(), parse_apple_gpu_cores() (+11 more)

### Community 26 - "App"
Cohesion: 0.07
Nodes (80): active_popup_area(), add_fallback_popup_area(), atlas_cycle_stats_room(), atlas_cycle_stats_scroll_max(), atlas_feed_room(), atlas_feed_room_for(), atlas_news_room(), atlas_news_room_for() (+72 more)

### Community 27 - "Store"
Cohesion: 0.05
Nodes (15): UnitManifest, atlas_answer_id(), atlas_brief_id(), AtlasStoredClaim, has_table(), intel_link_explanations_round_trip_and_cleanup(), repair_embed_tables(), Store (+7 more)

### Community 28 - "embed.rs"
Cohesion: 0.08
Nodes (31): active(), DIM, disable(), disabled(), DisableGuard, download_file(), DOWNLOAD_TIMEOUT_SECS, embed_batch() (+23 more)

### Community 29 - "osint.rs"
Cohesion: 0.04
Nodes (103): test_all_catalog_tools_mapped_to_categories(), a_claimed_email_keeps_no_person_data(), bind_request(), bitcoin(), body_cap(), bounded(), bounded_recovery_is_offered_once_for_an_unrecognised_page_only(), CACHE_DAY_SECONDS (+95 more)

### Community 30 - "holehe/mod.rs"
Cohesion: 0.09
Nodes (23): CACHE_TTL, cap_reports_omitted(), DEFAULT_MAX_SITES, default_selection_uses_implemented_adapters(), email_hash(), email_preserves_local_part(), fetch_once(), GLOBAL_CONCURRENCY (+15 more)

### Community 31 - "config_commit.rs"
Cohesion: 0.06
Nodes (73): a_batch_with_a_duplicate_or_empty_slot_is_refused_before_any_write(), a_committed_journal_needs_no_recovery(), a_second_lock_is_refused_while_one_is_held(), a_stale_lock_is_taken_over(), an_abandoned_unfinished_lockfile_is_recovered(), backup_path(), canonical_root(), commit_files_is_all_or_nothing() (+65 more)

### Community 32 - ".default"
Cohesion: 0.07
Nodes (33): a_blank_osint_user_agent_loads_as_unset(), default_credit_reset(), default_firecrawl_credits(), default_hunter_credits(), default_max_calls(), default_max_rounds(), default_one_cost(), default_opening_cap() (+25 more)

### Community 33 - "telemetry.rs"
Cohesion: 0.12
Nodes (21): bin_index(), Coverage, duration_bins_are_bounded_and_mergeable(), DURATION_BINS_MS, ensure_observed_since(), event_bins(), get_meta(), ID_SEQ (+13 more)

### Community 34 - "Call"
Cohesion: 0.07
Nodes (36): event_to_chat_block(), is_thinking_expanded(), InvestigationPart, DirectiveAssessment, EvidencePassage, GateValidation, Handoff, Plan (+28 more)

### Community 35 - "body.rs"
Cohesion: 0.09
Nodes (36): body_fetch_routes(), BodyFetchEvent, Attempt, Failed, InsightsFailed, InsightsRefreshing, InsightsReplaced, Progress (+28 more)

### Community 36 - "provider.rs"
Cohesion: 0.06
Nodes (36): concrete_free_models(), decide(), DecisionAnswer, decisions_client_posts_to_alpha_decisions_and_parses_answers(), DECISIONS_MODELS, decisions_url(), DecisionsResponse, default_grok_model() (+28 more)

### Community 37 - "brain_detail.rs"
Cohesion: 0.06
Nodes (50): areas(), areas_for(), BrainDetail, DetailAreas, DetailPane, Path, Related, Summary (+42 more)

### Community 38 - "profile_stats.rs"
Cohesion: 0.05
Nodes (70): add_cycle_outcome(), atlas_carries_data(), AtlasStats, AttemptSummary, BacklogRow, band_for(), category_triggers(), CategoryTrigger (+62 more)

### Community 39 - "atlas.rs"
Cohesion: 0.09
Nodes (30): article_card_labels_title_publisher_author_and_classification(), Band, band_sizes(), band_temperature(), category_from_decisions(), CATEGORY_IDS, CLUSTERS, country_label() (+22 more)

### Community 40 - "run_atlas_inner"
Cohesion: 0.09
Nodes (32): article_from_row(), Stats, Status, AtlasJob, charge_newsapi(), charge_quota(), Cursor, cursor_at() (+24 more)

### Community 41 - "whatsmyname.rs"
Cohesion: 0.05
Nodes (53): D, AccountTuple, ACTIVE_SNAPSHOT, ADAPTER_VERSION, apply_strip_bad_char(), benchmark_14_4_parse_index_and_selection(), cache_get(), cache_set() (+45 more)

### Community 42 - "scheduler.rs"
Cohesion: 0.09
Nodes (22): apply_index_change(), apply_index_change_rebuild_reports_outcome(), apply_index_with(), DEFAULT_LEASE_SECS, drain_index_once(), drain_summary_flush(), drain_summary_flush_cached(), drain_summary_flush_completes_cached_tasks() (+14 more)

### Community 43 - "jobs_view.rs"
Cohesion: 0.09
Nodes (24): summary_text(), active_time_is_live_for_open_attempts_and_unavailable_only_without_a_start(), AttemptRow, event_apps(), event_counts(), get_job(), job(), job_apps() (+16 more)

### Community 44 - "provider_attempt.rs"
Cohesion: 0.11
Nodes (36): Acc, attempt(), attempt_with_observer(), AttemptReport, check_final(), complete_stream_and_json_answers_succeed(), Deadlines, delta() (+28 more)

### Community 45 - "LogicalRole"
Cohesion: 0.10
Nodes (11): LogicalRole, .ALL, ClaimAssessor, Classifier, Controller, EntityResolver, EvidenceCurator, Planner (+3 more)

### Community 46 - "picker.rs"
Cohesion: 0.12
Nodes (26): brain_scrape_options_hide_bare_scrape_and_include_claim_criteria(), CatalogEntry, chat_request(), CONFIDENCE_FLOOR, decisions_request(), directives_phrase(), DONE, eligible_catalog() (+18 more)

### Community 47 - "ReportMode"
Cohesion: 0.12
Nodes (23): HomeDraftState, brief_rating_reuses_the_existing_mean_semantics(), classifiable_modes(), classify_mode_chat(), classify_mode_decisions(), classify_prompt_mode(), classify_prompt_mode_chat(), classify_prompt_mode_decisions() (+15 more)

### Community 48 - "provider_chain.rs"
Cohesion: 0.13
Nodes (27): AttemptRecord, cancellable_sleep(), cancellation_stops_the_chain(), cancelled(), ChainReport, execute(), ExecuteOptions, FALLBACK_ATTEMPTS (+19 more)

### Community 49 - "atomic"
Cohesion: 0.08
Nodes (15): AttemptOutcome, categorize_http(), execute_with_retries(), other_llm_allows_three(), summarization_stops_at_two_attempts(), TimeoutProfile, .CLASSIFIER, .EXTRACTION (+7 more)

### Community 50 - "tui/jobs.rs"
Cohesion: 0.08
Nodes (24): areas(), BASE_COLUMNS, button_label(), buttons(), detail_lines(), draw(), hit(), JobsAreas (+16 more)

### Community 51 - "App"
Cohesion: 0.03
Nodes (28): a_click_folds_an_event_and_incoming_entries_do_not_move_a_paused_list(), a_failed_turn_keeps_the_streamed_answer_on_screen(), a_log_or_late_tool_row_does_not_drop_the_live_answer(), add_scroll(), App, atlas_log_level(), AtlasPage, Live (+20 more)

### Community 52 - "events.rs"
Cohesion: 0.07
Nodes (33): LevelFilter, All, Error, Info, Warn, session_event(), clear_events(), DEFAULT_RETENTION_HOURS (+25 more)

### Community 53 - "exec.rs"
Cohesion: 0.14
Nodes (20): admission_account(), admission_contention_consumes_no_attempt(), AttemptEvent, Finished, Started, AttemptLog, cancelled(), complete_summary_report() (+12 more)

### Community 54 - "Frame"
Cohesion: 0.11
Nodes (43): intel_category_short(), AbsRect, atlas_auto_label(), atlas_extracting(), atlas_run_label(), draw_article_image(), draw_atlas(), draw_atlas_insights() (+35 more)

### Community 55 - "model_exec.rs"
Cohesion: 0.15
Nodes (25): adapter_reported_first_response_is_kept_when_no_chunk_arrived(), attempt_outcome_keeps_the_failure_category_for_the_reason_dimension(), AttemptFacts, AttemptOutcome, AttemptTimings, canonical_attempt_id(), canonical_attempt_id_is_stable_across_replay(), ensure_operation() (+17 more)

### Community 56 - "ErrorCategory"
Cohesion: 0.08
Nodes (24): operation_kind(), backoff_delay(), can_retry(), ErrorCategory, AuthOrQuota, CancelledOrStale, ConfigurationMissing, ContextLimit (+16 more)

### Community 57 - "MemoryPhase"
Cohesion: 0.11
Nodes (19): CycleOutcome, Blocked, Completed, Failed, Partial, Waiting, memory_cycle_payload(), memory_step() (+11 more)

### Community 58 - "MemoryKind"
Cohesion: 0.09
Nodes (22): AdmissionPolicy, admit_memory(), BrainQuery, pack_context(), AssessmentState, Contradicts, Insufficient, Mentions (+14 more)

### Community 59 - "draw_brain"
Cohesion: 0.21
Nodes (14): cursor_at(), draw_brain(), draw_field(), draw_osint(), draw_see_more(), field_placeholder(), field_value_area(), focused_pane() (+6 more)

### Community 60 - "brain_resources.rs"
Cohesion: 0.09
Nodes (26): source_anchor_label(), BRAIN_SCRAPE_PREFIX, CLAIM_CHARS, classify_resource(), file_link_url(), hit(), is_brain_binding(), is_brain_scrape_pick() (+18 more)

### Community 61 - "worker.rs"
Cohesion: 0.15
Nodes (20): IntelReportEvent, InsightsUpdated, JobCreated, JobDone, Section, Stage, IntelReportJobRow, cancel_if_needed() (+12 more)

### Community 62 - ".set_focus"
Cohesion: 0.06
Nodes (21): backspace_after_a_sent_question_deletes_one_character(), draft_isolation_and_persistence(), duplicate_submission_prevention(), intel_opens_bulletin_filters_and_opens_briefing(), keyboard_navigation_esc_and_shortcuts(), multiline_paste_stays_in_composer_and_does_not_run_shortcuts(), overhaul_tool_category_collapse_preserves_identity_and_search_state(), pointer_and_chords_do_not_switch_apps_on_their_own() (+13 more)

### Community 63 - "src/brain.rs"
Cohesion: 0.12
Nodes (21): AGREEMENT_WEIGHT, CATEGORIES, category_hint(), format_injection(), hybrid_recall(), hybrid_recall_adds_paraphrases_and_rewards_agreement(), identity_query_prefers_name_memory(), jaccard() (+13 more)

### Community 64 - "intel_recon/jobs.rs"
Cohesion: 0.15
Nodes (19): active_job_for_mode(), cancel_job(), create_job_inserts_all_verify_sections(), create_report_job(), finish_canonical(), intel_recon_links_its_legacy_job_to_one_canonical_registry_job(), pause_job(), register_canonical() (+11 more)

### Community 65 - "cli.rs"
Cohesion: 0.18
Nodes (18): ask(), ask_thread(), atlas_command(), defaults_command(), defaults_json(), defaults_show_includes_the_tool_picker_transport(), dispatch(), login() (+10 more)

### Community 66 - "atlas_insights.rs"
Cohesion: 0.07
Nodes (50): a_country_token_does_not_merge_into_a_longer_name(), a_decisions_model_does_not_extract_claims(), aliases_merge_on_shared_tokens_when_the_longer_form_is_a_span(), BODY_CLAIM_LIMIT, BODY_SPAN_CHARS, brief_rating_mean(), COL_CLASS, COL_ENTITY (+42 more)

### Community 67 - "SourceReliability"
Cohesion: 0.09
Nodes (22): article_information_credibility(), AdmiraltyCode, best_credibility(), CredibilityInputs, information_credibility(), InformationCredibility, CannotBeJudged, Confirmed (+14 more)

### Community 68 - ".new"
Cohesion: 0.21
Nodes (27): a_snapshot_never_panics_on_a_partially_migrated_store(), active_jobs(), an_empty_database_returns_empty_sections_not_zeros(), atlas_raw_candidate_occurrences_are_not_added_twice(), count_rows(), distinct_article_body_cohort_deduplicates_tags_and_excludes_empty_cache_rows(), distinct_values(), durable_model_rows_and_supplemental_telemetry_count_once() (+19 more)

### Community 69 - "body_filter.rs"
Cohesion: 0.18
Nodes (22): BATCH_SIZE, BodyChunk, chunk_article_body(), CHUNK_MAX, CHUNK_MIN, CHUNK_TARGET, chunks_preserve_absolute_offsets(), classify_chunk_batch() (+14 more)

### Community 70 - "synthesize.rs"
Cohesion: 0.16
Nodes (20): IntelEvidenceRow, BODY_REFINE_INPUT_CHARS, deterministic_bluf_mentions_title(), deterministic_section(), model_refine_body(), model_synthesize(), parse_json_object(), refine_retrieved_article_body() (+12 more)

### Community 71 - "dataset.rs"
Cohesion: 0.15
Nodes (19): acquire_refresh_lease(), active_manifest_path(), ACTIVE_REFRESHES, dataset_root(), DatasetManifest, DatasetStatus, get_status(), import_from_file() (+11 more)

### Community 72 - "Loaded"
Cohesion: 0.12
Nodes (19): atlas_origin_rows(), Dims, dominant_label(), engine_health(), EventRow, evidence_call(), evidence_cited(), evidence_details() (+11 more)

### Community 73 - "profile_charts.rs"
Cohesion: 0.06
Nodes (61): absolute_stacks_compare_one_and_one_hundred_on_one_scale(), BLOCKS, cap_eighths(), cells(), coarsen_counts(), column_cell(), connect_cells(), count_coarsening_preserves_totals_boundaries_and_unknown_history() (+53 more)

### Community 74 - "atlas_table.rs"
Cohesion: 0.17
Nodes (22): border_style(), borders_and_cells_have_matching_display_widths(), bottom_border_line(), char_width(), clip_to_width(), column_widths(), columns_consume_inner_width(), columns_distribute_proportionally_when_not_flex_last() (+14 more)

### Community 75 - "Overlay"
Cohesion: 0.09
Nodes (21): ChoiceKind, IntelDay, Investigation, Model, Provider, LastViewSession, Overlay, AddFallback (+13 more)

### Community 76 - "pipeline.rs"
Cohesion: 0.11
Nodes (26): apply_recon_directive_coverage(), ClaimRelation, AmbiguousEntity, ChangedQuantity, Equivalent, Negation, PlannedVsCompleted, Unrelated (+18 more)

### Community 77 - "summarization.rs"
Cohesion: 0.10
Nodes (37): cache_get(), cache_key_changes_with_revision_and_focus(), cache_put(), cache_round_trip(), complete_summary(), CoverageMeta, deterministic_article_description(), deterministic_atlas_brief() (+29 more)

### Community 78 - "theme.rs"
Cohesion: 0.13
Nodes (24): Block, ACCENT, BG, BORDER, card(), card_accent(), card_dim(), card_text() (+16 more)

### Community 79 - ".reload_memories"
Cohesion: 0.29
Nodes (7): brain_summary_failure_card_explains_links_and_retries_in_place(), dump_phase7_screens(), graph_jobs(), saved_summary_is_reused_until_its_inputs_change_then_shown_as_earlier(), settle_summary(), summary_fixture(), SummaryFx

### Community 80 - "search_engines.rs"
Cohesion: 0.08
Nodes (35): build_scrape_body(), Candidate, card_snippet(), classify_status_region(), clip_item_text(), collapse_ws(), collect_candidates(), contains_phrase() (+27 more)

### Community 81 - "logs.rs"
Cohesion: 0.11
Nodes (19): stamp(), areas(), button_label(), buttons(), count(), draw(), hit(), in_list() (+11 more)

### Community 82 - ".new"
Cohesion: 0.25
Nodes (19): a_spent_primary_quota_uses_the_fallback_key(), classification_request(), country_labels_show_the_name_and_the_code(), daily_cap(), format_run_card(), HttpCall, HttpReply, insights_do_not_call_a_decisions_model() (+11 more)

### Community 83 - "FeedArticle"
Cohesion: 0.17
Nodes (19): apply_hits(), article_from(), article_row(), canonical_url(), category_tag(), dedup_drops_a_url_an_exact_title_and_keeps_the_higher_domain(), FeedArticle, host_of() (+11 more)

### Community 84 - "budget.rs"
Cohesion: 0.12
Nodes (14): CUT_NOTE, CUT_SHORT, PHASE_WINDOW_SECONDS, RECON_DEADLINE, RECON_ROUND_SECONDS, STREAM_LOST, synthesis_allowance_capped(), synthesis_allowance_seconds() (+6 more)

### Community 85 - "validate.rs"
Cohesion: 0.17
Nodes (17): paywall_and_snippet_fail_validation(), BLOCK_MARKERS, BodyQuality, Complete, Partial, Unavailable, Uncertain, BodyValidation (+9 more)

### Community 86 - "Command"
Cohesion: 0.11
Nodes (19): Cli, Command, Atlas, Defaults, Hardware, Insights, Login, Logout (+11 more)

### Community 87 - "DefaultsRole"
Cohesion: 0.08
Nodes (17): ChoiceItem, codex_models(), DefaultsRole, .ALL, ClaimAssessor, Classifier, EntityResolver, EvidenceCurator (+9 more)

### Community 88 - "graph_explanation.rs"
Cohesion: 0.14
Nodes (13): BASIC_HEADING, explain(), ExplainOutcome, Failed, Saved, Superseded, ExplainReport, ExplainRequest (+5 more)

### Community 89 - "src/evidence.rs"
Cohesion: 0.09
Nodes (30): AGREEMENT_WEIGHT, AnnMeasurement, AnnPolicy, AnnDeferredUnmeasured, AnnEnabled, ExactSearch, AnnThresholds, chunk_text() (+22 more)

### Community 90 - "ToolRunner"
Cohesion: 0.09
Nodes (18): ToolDefinition, attribution_is_explicit_never_inferred_from_the_prompt(), failure_reason(), InvocationFact, named_engine(), osint_cacheable(), test_tool_runner_cache_hit(), test_tool_runner_concurrency_and_dedup() (+10 more)

### Community 91 - "DecisionContract"
Cohesion: 0.12
Nodes (25): compile_general_model_prompt(), compile_native(), parse_general_model_response(), parse_native_response(), DecisionContract, template_claim_relation(), template_directive_alignment(), template_entity_binding() (+17 more)

### Community 92 - "AtlasArticleRow"
Cohesion: 0.15
Nodes (26): accept_one(), apply_peer_support(), article_with_body_spans(), articles_have_span(), body_lead_prompt(), catalog_json(), classifier_peers_are_preferred_over_token_overlap(), classify_peers_chat() (+18 more)

### Community 93 - "AtlasEvent"
Cohesion: 0.12
Nodes (24): AtlasEvent, Article, Classified, Fault, InsightProgress, MemoriesChanged, MemoryProgress, Note (+16 more)

### Community 94 - "dork_generator.rs"
Cohesion: 0.09
Nodes (36): ACTIVE_SNAPSHOT, compose_single_query(), compute_query_id(), compute_template_id(), DATASET_NAME, DEFAULT_MAX_QUERIES, DorkCatalog, DorkCategory (+28 more)

### Community 95 - "Section"
Cohesion: 0.18
Nodes (8): every_section_registers_its_reviewed_widget_count(), Section, Atlas, Intel, Models, Recon, Tools, widgets_render_in_the_specs_presentation_priority_order()

### Community 96 - "How"
Cohesion: 0.11
Nodes (18): How, CompanyEmail, Coordinates, DomainAsUrl, EntityPhrase, PackageName, PackageParts, Plain (+10 more)

### Community 97 - ".memory"
Cohesion: 0.13
Nodes (14): article(), atlas_articles_for_intel_dedupes_same_article_id_across_runs(), atlas_articles_for_intel_filters_category_day_and_query(), atlas_auto_next_round_trips_through_app_state(), atlas_claims_for_article_returns_linked_claims(), atlas_prune_drops_finished_runs_older_than_the_cutoff(), atlas_recent_articles_lists_retained_rows(), atlas_run_days_lists_distinct_days_newest_first() (+6 more)

### Community 98 - "summary_card.rs"
Cohesion: 0.32
Nodes (11): actions(), button_at(), button_cells(), button_row_area(), draw(), height(), is_card_button(), label() (+3 more)

### Community 99 - "actor_review.rs"
Cohesion: 0.29
Nodes (11): ActorReviewItem, ActorReviewResult, apply_reviewed_actors(), deterministic_review_actors(), is_meaningful_actor(), JUNK_ACTORS, model_review_actors(), parse_actor_review_response() (+3 more)

### Community 100 - "ModuleId"
Cohesion: 0.15
Nodes (16): ModuleId, .ALL, Atlas, Brain, Intel, Jobs, Logs, Osint (+8 more)

### Community 101 - "Region"
Cohesion: 0.11
Nodes (19): Region, AtlasInsights, AtlasNews, AtlasOrigins, AtlasRuns, Chat, Detail, IntelBrief (+11 more)

### Community 102 - "diversity.rs"
Cohesion: 0.13
Nodes (25): ReconLimits, coverage_gaps_summary(), coverage_record_id(), coverage_stats(), CoverageCandidate, CoverageRecord, CoverageStats, covered_record() (+17 more)

### Community 103 - ".default"
Cohesion: 0.17
Nodes (34): a_429_stops_picker_calls_and_the_fallback_finishes_the_list(), a_dispatch_error_fails_the_step_and_the_loop_goes_on(), a_failed_email_finder_gets_one_fallback_and_no_third_picker_call(), a_fallback_with_no_bindings_still_runs_the_satisfied_steps_and_reaches_synthesis(), a_goal_naming_a_tool_or_provider_is_repaired_then_falls_back(), a_goal_naming_a_tool_outside_the_prompt_entity_is_still_rejected(), a_prompt_entity_named_like_a_provider_keeps_the_models_directives(), a_question_handle_fills_the_handle_steps_without_a_fallback() (+26 more)

### Community 104 - "Category"
Cohesion: 0.08
Nodes (21): Category, Auth, Cancelled, Configuration, Empty, InvalidModel, InvalidResult, MalformedPayload (+13 more)

### Community 105 - "Rect"
Cohesion: 0.10
Nodes (59): abs_contains(), add_fallback_layout(), api_key_slot(), article_image_area(), atlas_hit(), atlas_live_areas(), atlas_news_areas(), atlas_row_at() (+51 more)

### Community 106 - "search_engines/tests.rs"
Cohesion: 0.10
Nodes (42): build_serp_url(), a_non_serp_path_on_the_engine_host_is_not_a_serp_page(), a_noscript_wrapped_consent_meta_refresh_is_consent(), a_single_retry_is_allowed_only_for_a_parser_mismatch_with_a_dom(), a_standalone_zero_count_in_a_status_region_is_a_verified_zero(), a_zero_phrase_outside_a_status_region_is_not_a_verified_zero(), an_off_host_final_url_with_cards_keeps_the_items_but_is_not_valid(), an_off_host_final_url_without_cards_is_never_a_zero() (+34 more)

### Community 107 - "profile_config.rs"
Cohesion: 0.13
Nodes (11): a_validation_error_reports_a_pointer_and_never_a_value(), an_oversize_document_is_refused_before_anything_is_applied(), confirmed_export_overwrites_the_same_destination(), direct_buffer_change_invalidates_commit_revision(), draw(), EditorError, locate(), SECRET_WARNING (+3 more)

### Community 108 - "execute_ordered"
Cohesion: 0.13
Nodes (19): a_budgeted_call_with_a_dependency_from_an_earlier_batch_runs(), ac3_no_key_reaches_inputs_cache_keys_plan_json_raw_bodies_or_logs(), ac6_a_courtlistener_429_still_lets_the_turn_synthesize(), ac8_status_errors_and_401_403_fail_readably_and_are_not_cached(), advance_stage(), BudgetedOutcome, cancel_mid_dispatch_releases_credit_holds(), citing_synthesis() (+11 more)

### Community 109 - "Architecture"
Cohesion: 0.10
Nodes (21): Admiralty source evaluation (WP:RSP), Architecture, Atlas, Binder and executor, Brain recall, Config transfer and the commit, Directives, Intel reports (+13 more)

### Community 110 - "WorkEvent"
Cohesion: 0.08
Nodes (22): work_event(), WorkEvent, AnswerDelta, AnswerNote, AnswerReplacement, AnswerReset, AtlasDone, BrainRelatedExplained (+14 more)

### Community 111 - "store.rs"
Cohesion: 0.10
Nodes (22): ArticleInsightCommit, AUTO_REBUILD_HINT, deleting_a_memory_removes_its_graph_summary(), embedding_disabled_degrades_to_jaccard(), embedding_failure_mid_session_degrades_to_jaccard(), every_memory_has_provenance_and_category(), fingerprint_mismatch_rebuilds_and_drift_is_reconciled(), GraphSummary (+14 more)

### Community 112 - "ClockSet"
Cohesion: 0.21
Nodes (3): ClockSet, queue_time_does_not_spend_active_but_consumes_wall(), sequential_vs_overlapping_tool_budgets()

### Community 113 - "ProviderAdmission"
Cohesion: 0.18
Nodes (6): shared_rate_limit_cooldown_blocks_admission(), AdmissionGuard, note_shared_rate_limit(), provider_admission_caps_at_two(), ProviderAdmission, rate_limit_cooldown_blocks_acquire()

### Community 114 - "components.rs"
Cohesion: 0.19
Nodes (13): action_rects(), analytics_card(), clip_text(), compact_records_keep_trailing_numeric_values(), detail_records(), detail_table(), editor_height(), measured_lines() (+5 more)

### Community 115 - "ToolResult"
Cohesion: 0.12
Nodes (18): ToolResult, ac6_citation_groups_split_validate_each_id_and_normalize(), cacheable(), citation_groups(), citation_ids(), cited(), evidence_summary(), infer_uncited_evidence() (+10 more)

### Community 116 - "gsd-v2.js"
Cohesion: 0.10
Nodes (15): core, home, hooks, names, payload(), run(), setup(), root (+7 more)

### Community 117 - "ServiceResult"
Cohesion: 0.15
Nodes (17): cache_key(), cache_stores_only_definitive(), cached_result(), CacheEntry, cancel_marks_inconclusive(), check_implemented(), classify_http(), clear_caches() (+9 more)

### Community 118 - "briefing_view.rs"
Cohesion: 0.21
Nodes (18): bucket_extracted(), bucket_extracted_with_explanations(), buckets_split_facts_inferences_context_and_links(), claim(), claim_label(), explain_intel_links_with_model(), explain_relation_link(), ExtractedBuckets (+10 more)

### Community 119 - "subscription.rs"
Cohesion: 0.24
Nodes (8): answer_event(), check_login(), command(), complete(), completion_is_ephemeral_and_does_not_inherit_tools_or_configuration(), exec_args(), is_subscription_status(), login()

### Community 120 - "Line"
Cohesion: 0.17
Nodes (39): amplification_lines(), attribution_lines(), backlog_lines(), capacity_lines(), cause_lines(), confidence_lines(), cycle_lines(), cycle_time_lines() (+31 more)

### Community 121 - "DecisionAdapterKind"
Cohesion: 0.16
Nodes (14): DecisionAdapterKind, JsonMode, NativeDecisions, OutputTool, StrictJsonSchema, ValidatedText, resolve_adapter(), DecisionValidationStatus (+6 more)

### Community 122 - "recon/graph.rs"
Cohesion: 0.06
Nodes (59): draw(), draw_path(), draw_summary(), glyph(), inset(), legend_height(), legend_parts(), legend_rows() (+51 more)

### Community 123 - "tasks.rs"
Cohesion: 0.09
Nodes (48): add_missing_columns(), adopt_untracked_index_changes(), block_claimed(), claim_index_changes(), claim_task(), claim_with(), ClaimedTask, complete_claimed() (+40 more)

### Community 124 - "ReconCommand"
Cohesion: 0.17
Nodes (12): ReconCommand, Ask, AskNew, Delete, Limits, List, New, Rename (+4 more)

### Community 125 - "config_transfer_tests.rs"
Cohesion: 0.09
Nodes (52): apply_import(), export_document(), export_kind(), export_to_path(), merge_key(), parse_document(), ProfileConfig, provider_entry() (+44 more)

### Community 126 - "run"
Cohesion: 0.11
Nodes (10): atlas_countdown_visible(), atlas_extracting_visible(), intel_body_loading_visible(), intel_insights_loading_visible(), intel_summary_loading_visible(), is_picker_field(), mouse_dirties(), provider_label() (+2 more)

### Community 127 - "SettingsFile"
Cohesion: 0.09
Nodes (18): commit_profile_config(), ConfigChange, ConfigurationSnapshot, describe_changes(), import_from_path(), ImportPlan, resolve_actor_reviewer_secret(), SettingsFile (+10 more)

### Community 128 - "ServiceSpec"
Cohesion: 0.18
Nodes (9): by_id(), CATALOG, CATALOG_LEN, UPSTREAM_COMMIT, ServiceSpec, ServiceState, Enabled, Experimental (+1 more)

### Community 129 - "TurnClock"
Cohesion: 0.10
Nodes (6): a_round_keeps_its_time_and_a_tight_ceiling_still_runs_tools(), deadline_seconds(), format_deadline(), format_span(), the_label_updates_when_a_round_adds_calls_and_when_evidence_is_measured(), TurnClock

### Community 130 - "opencode.json"
Cohesion: 0.07
Nodes (31): agents, build, general, plan, reasoningEffort, thinkingBudget, mode, model (+23 more)

### Community 131 - "Functional Requirements"
Cohesion: 0.12
Nodes (15): Atlas (News Pipeline), Brain / Memory, Configuration Requirements, Context Provider Keys, Functional Requirements, Non-Functional Requirements, Non-Goals, OSINT Manual Runs (+7 more)

### Community 132 - "JobStatusFilter"
Cohesion: 0.25
Nodes (6): JobStatusFilter, Active, All, Completed, Failed, Retrying

### Community 133 - "super"
Cohesion: 0.11
Nodes (12): CODES, NAMES, ADAPTER_VERSION, HOST, ID, positive_negative_and_unknown(), request_url(), ADAPTER_VERSION (+4 more)

### Community 134 - "ui.rs"
Cohesion: 0.05
Nodes (77): TranscriptBlock, abs_rect(), ACTION_H, ApiKeySlot, atlas_countdown(), atlas_history_live_label(), atlas_source_anchors_are_not_labeled_deleted_origin(), call_stamp() (+69 more)

### Community 135 - "DateTime"
Cohesion: 0.13
Nodes (23): amplification_is_a_raw_multiplier_with_an_eligible_denominator(), AmplificationBucket, ArticleFact, AttemptFact, BriefFact, Bucket, bucket_floor(), bucket_format() (+15 more)

### Community 136 - "OsintCommand"
Cohesion: 0.14
Nodes (14): DatasetCommand, Import, Refresh, Status, OsintCommand, Attach, Dataset, Describe (+6 more)

### Community 137 - "normalize_destination"
Cohesion: 0.21
Nodes (14): detect_interstitial(), host_in_any(), host_is(), host_is_captcha(), host_is_consent(), is_ad_host(), is_private_host(), meta_refresh_is_consent() (+6 more)

### Community 138 - "Service"
Cohesion: 0.11
Nodes (17): AnswerContext, compact_page_evidence(), cut_footer(), cut_short_answer(), finish_recon_job(), credit_map(), deltas(), page_needs_compact() (+9 more)

### Community 139 - "Argos OSINT — Agent Instructions"
Cohesion: 0.11
Nodes (19): Agent Cargo isolation, Architecture notes, Argos OSINT — Agent Instructions, CI (`.github/workflows/ci.yml`), CLI Entry Points (`argos` binary), Developer Commands, diagrams, Environment Variables (+11 more)

### Community 140 - "ProviderSecret"
Cohesion: 0.18
Nodes (22): account_secret(), active_text_secret(), authorize(), bearer_token(), catalog_error(), detect_kind_from_url(), effective_kind(), http() (+14 more)

### Community 141 - "InvestigationSurface"
Cohesion: 0.06
Nodes (21): InvestigationSurface, HomeComposer, IntelBrief, JobsResume, ReconChat, TaskRecord, TaskStatus, Cancelled (+13 more)

### Community 142 - "wikipedia_rsp.rs"
Cohesion: 0.08
Nodes (44): admiralty_scales_claim_confidence_from_rsp_and_peers(), article(), raw_initial_confidence_is_captured_before_source_scaling(), recover_legacy_packet(), recover_legacy_packet_has_no_initial_confidence(), the_packet_keeps_four_and_reports_the_full_gate(), API, APP_STATE_KEY (+36 more)

### Community 143 - "rusqlite"
Cohesion: 0.09
Nodes (19): Cached, None, Stale, Valid, Gate, CoolingDown, Ready, Running (+11 more)

### Community 144 - "DecisionState"
Cohesion: 0.20
Nodes (4): DecisionState, EvidencePassageState, SubjectStateSummary, TaskStateSummary

### Community 145 - "ProviderFailure"
Cohesion: 0.11
Nodes (14): cause_chain(), classify_text(), Leaf, nested_causes_and_endpoints_are_kept_and_redacted(), ProviderFailure, Stage, Admission, Configuration (+6 more)

### Community 146 - "execute_steps"
Cohesion: 0.15
Nodes (27): after_step(), binding_ground(), context_block(), context_dispatched(), directive_for(), execute_steps(), expand_dork_children(), expand_per_platform() (+19 more)

### Community 147 - "ledger.rs"
Cohesion: 0.13
Nodes (13): body_assertion_candidates(), coverage_complete(), ElementStatus, Assessed, Excluded, InProgress, Pending, Superseded (+5 more)

### Community 148 - "QueryExecutionStatus"
Cohesion: 0.17
Nodes (11): QueryExecutionStatus, Cancelled, Completed, Failed, Generated, NoResults, Queued, Running (+3 more)

### Community 149 - "results.rs"
Cohesion: 0.13
Nodes (20): classify_output_quality(), extract_observation_items(), extract_search_results(), extracts_from_array_shaped_observations(), extracts_from_object_with_results_array(), normalize_tool_result(), NormalizedToolResult, OutputQuality (+12 more)

### Community 150 - "TelemetryEvent"
Cohesion: 0.13
Nodes (10): clamp(), coverage_reports_observed_since_and_counts(), db(), events_persist_and_roll_up_once(), id_is_stable_so_replay_never_double_counts(), logical_attempt_and_cache_rows_stay_distinct_under_replay(), record(), safe_payload() (+2 more)

### Community 151 - "SerpOutcome"
Cohesion: 0.17
Nodes (10): SerpOutcome, Challenge, Consent, ParserMismatch, RateLimited, ResponseTooLarge, UpstreamFailure, Valid (+2 more)

### Community 152 - "profile.rs"
Cohesion: 0.13
Nodes (30): applicable_filters(), ATTENTION, attention_lines(), bucket_end(), bucket_start(), build_report_detail_lines(), categorical_plot(), compare_cells() (+22 more)

### Community 153 - "App"
Cohesion: 0.12
Nodes (33): activate(), capacity_preserves_unknown_disabled_and_overflow_values(), content_layout(), dashboard_layout(), draw_actions(), draw_filter_strip(), draw_picker(), draw_profile() (+25 more)

### Community 154 - "tui_review.py"
Cohesion: 0.14
Nodes (11): cargo_command(), cargo_env(), main(), box(), color(), render(), capture(), digest() (+3 more)

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

### Community 166 - "SiteOutcomeStatus"
Cohesion: 0.18
Nodes (10): deserialize_flexible_bool(), SiteOutcomeStatus, Ambiguous, Blocked, Error, Found, NotFound, RateLimited (+2 more)

### Community 167 - "investigation.rs"
Cohesion: 0.06
Nodes (90): Account, account_hits(), ACCOUNT_PLATFORMS, account_platforms_attach_to_the_subject_and_are_never_entities(), ACCOUNTS, action_order(), actions_are_grounded_capped_and_not_a_sweep(), ADAPTIVE (+82 more)

### Community 168 - "Milestone 1 — Core Onboarding (Current)"
Cohesion: 0.18
Nodes (10): Deliverables, Dependencies, Future Milestones (Planned), Milestone 1 — Core Onboarding (Current), Milestone 2 — CLI Verification, Milestone 3 — Test Coverage, Milestone 4 — Integration & Ship, Objectives (+2 more)

### Community 169 - "paths.rs"
Cohesion: 0.24
Nodes (12): auth_path(), config_journal_path(), config_lock_path(), config_path(), dataset_dir(), datasets_dir(), db_path(), ensure_home() (+4 more)

### Community 170 - "model_roles.rs"
Cohesion: 0.15
Nodes (13): AccountHealth, AuthRequired, Cooldown, CreditsExhausted, Overloaded, Ready, Restricted, Unverified (+5 more)

### Community 171 - "atlas_work.rs"
Cohesion: 0.09
Nodes (23): articles_rev(), AttemptRecord, check_dependency_coverage(), DependencyCoverage, DISPOSITION_EMPTY, DISPOSITION_FAILED, DISPOSITION_INCOMPLETE, DISPOSITION_REJECTED (+15 more)

### Community 172 - "rule_bindings"
Cohesion: 0.11
Nodes (27): binder_maps_kinds_to_inputs_and_never_hands_hunter_a_social_host(), binding(), PROMPT_TARGETS, question_bindings(), a_gap_filler_only_domain_waits_for_a_primary_observation_before_hunter(), batch_scrape_takes_ranked_urls_and_gates_order_before_hunter(), bind_arguments(), bitcoins_in() (+19 more)

### Community 173 - "tool_runner.rs"
Cohesion: 0.16
Nodes (12): a_synthesized_timeout_is_a_failure_with_a_tool_timeout_reason(), cancelled_and_cache_modes_are_not_remote_requests(), definition(), every_other_tool_keeps_its_catalog_cache_lifetime(), failure_outcome(), named_serp_cache_policy_is_semantic(), NAMED_SERP_TOOLS, remote_attempts() (+4 more)

### Community 174 - "Codebase Map — argos-osint"
Cohesion: 0.22
Nodes (8): `argos-osint-bin` — CLI and TUI, `argos-osint-core` — Core Library, Codebase Map — argos-osint, Crates, Documentation Ingest, Primary Data Flow, State & Config (all under `~/.argos`), Tool Input & Binding Kinds (from `tool_io.rs`)

### Community 175 - "PROJECT.md — argos-osint"
Cohesion: 0.22
Nodes (8): Applications, CLI Entry Points, Core Components, Crates, Investigation Flow, PROJECT.md — argos-osint, Project Purpose, State Directory (`~/.argos`)

### Community 176 - "Current Phase State"
Cohesion: 0.25
Nodes (7): Artifacts Status, Configuration State, Current Phase State, Next Steps, Pending Items, Phase: Core Onboarding, STATE.md — argos-osint

### Community 177 - "Trigger"
Cohesion: 0.17
Nodes (11): TELEMETRY_TRIGGER, is_forbidden_key(), payload_number(), Trigger, AtlasCycle, IntelBrief, ManualTool, ReconPrompt (+3 more)

### Community 178 - "RecoveryAction"
Cohesion: 0.12
Nodes (12): classify_recovery(), RecoveryAction, AccessRestricted, CircuitCooldown, ContentRefused, CreditsExhausted, FallbackModel, ReduceContext (+4 more)

### Community 179 - "grok_oauth.rs"
Cohesion: 0.14
Nodes (20): auth_path(), bearer(), bearer_from_path(), check_login(), Entry, entry_from(), failed_login_keeps_the_cli_reason_and_recovery_action(), login() (+12 more)

### Community 180 - "mem"
Cohesion: 0.17
Nodes (23): lease_fencing_rejects_stale_epoch_and_foreign_owner(), claim_complete_and_retry_round_trip(), claim_index_work(), claim_next(), claim_next_in(), disabled_outcome_blocks_without_spending_attempts_and_resumes(), enqueue_index_change(), enqueue_job() (+15 more)

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
Cohesion: 0.25
Nodes (8): Builds, ECC commands, Graph first, OpenCode V2 workflow, Phase edits, Primary tools, TUI verification setup, Verify setup

### Community 186 - "home_rows"
Cohesion: 0.15
Nodes (22): center_row(), composer_prompt_text(), cursor_blink_visible(), draw_composer(), draw_home(), gap_row(), home_composer_and_all_content_centered_vertically_and_horizontally(), home_group() (+14 more)

### Community 187 - "Concepts"
Cohesion: 0.25
Nodes (8): Apps and internal IDs, Bindings, Concepts, Graph, Intel report jobs, Model roles, Persistence, Primary OSINT providers

### Community 201 - "explore.rs"
Cohesion: 0.20
Nodes (11): GraphEdgeKind, ContradictionOrUpdate, Evidenced, Inferred, SemanticSuggestion, related_evidence_view(), RelatedEvidenceHit, semantic_edges_are_not_factual() (+3 more)

### Community 202 - "TUI components"
Cohesion: 0.17
Nodes (12): Acceptance, ActionBar and Overlay, AnalyticsCard and Dashboard preset, Application presets, DetailTable and Report preset, Measured text, MeasuredEditor and TranscriptBlock, RankedBars, ComparisonBars and Meter (+4 more)

### Community 203 - "ConfigView"
Cohesion: 0.20
Nodes (4): a_directory_target_is_an_actionable_error_not_a_crash(), ConfigView, editing_the_buffer_disarms_the_import_button(), expand_home()

### Community 204 - "Providers"
Cohesion: 0.25
Nodes (8): Budgeted executor, Graph explanation jobs, News and legal keys, OSINT data providers, Providers, Tool picker transport, Typed provider diagnostics, Verification and testing

### Community 205 - "Decision roles and output contracts"
Cohesion: 0.29
Nodes (6): 12-template registry, Adapters, Decision roles and output contracts, State isolation, Thresholds, TUI

### Community 206 - "LayoutResult"
Cohesion: 0.19
Nodes (10): DashboardPage, Atlas, Other, Recon, Summary, LayoutResult, PanelRect, reference_geometry_and_responsive_panels() (+2 more)

### Community 207 - "Terminal UI"
Cohesion: 0.22
Nodes (9): Atlas, CLI, Intel, Other apps, Profile, Recon, State, Terminal UI (+1 more)

### Community 209 - "Profile dashboard and search"
Cohesion: 0.17
Nodes (12): Analytics controls, Import semantics, Metric dictionary, Named search engines, Profile dashboard and search, Provider orchestration companion, Retention and coverage, The 20 primary views (+4 more)

### Community 210 - "RunStats"
Cohesion: 0.13
Nodes (13): a_resumed_cycle_continues_candidate_numbering(), cycle_event_id_is_the_run_key_so_replay_never_double_counts(), CycleTelemetry, db(), each_candidate_occurrence_gets_exactly_one_disposition(), OpenStage, origin_snapshots_are_one_row_per_origin_per_cycle(), parse_stats() (+5 more)

### Community 212 - "extract_items"
Cohesion: 0.19
Nodes (10): EngineSearchResult, extract_items(), ParserInput, CleanHtml, LinksOnly, Missing, RawHtml, retry_allowed() (+2 more)

### Community 213 - "now"
Cohesion: 0.17
Nodes (13): answer_evidence_survives_reopen_and_deleted_thread_rejects_late_calls(), claims_deduplicate_and_reject_unsupported_sources(), deleting_an_investigation_removes_its_brain_memories_and_graph_summary(), investigation_memory_ids(), Message, now(), persist_claims(), persistence_and_plan() (+5 more)

### Community 214 - ".order"
Cohesion: 0.23
Nodes (10): checked_serves(), context_additions(), context_tools(), fallback_record(), OrderContext, Ordered, Picker<'a>, serves_for() (+2 more)

### Community 215 - "intel_recon/brain.rs"
Cohesion: 0.57
Nodes (5): intel_recon_publishes_durably_with_provenance_and_no_atlas_receipt(), ReconInsightUpdate, update(), upsert_adds_and_updates_claims(), upsert_recon_insights()

### Community 216 - "Unified Investigation Harness"
Cohesion: 0.33
Nodes (6): Catalog projection, Model roles, Persistence, Reasoning channels, Unified Investigation Harness, Validation gates

### Community 217 - "RendererKind"
Cohesion: 0.29
Nodes (7): renderer_kind(), RendererKind, Comparison, Meter, Ranked, Table, Time

### Community 218 - ".evaluate"
Cohesion: 0.16
Nodes (11): DecisionService, DecisionPolicy, EnforcementOutcome, Pass, Reject, Unavailable, Uncertain, ThresholdProfile (+3 more)

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
Cohesion: 0.09
Nodes (23): applicable_filter_dimensions(), bounded_label(), dims_from_key(), Reader<'a>, EventKind, .ALL, AtlasCandidate, AtlasCycle (+15 more)

### Community 223 - "Argos documentation"
Cohesion: 0.40
Nodes (5): Agent and workflow, Argos documentation, Checklists (historical), Figures, Product

### Community 224 - "run_turn"
Cohesion: 0.14
Nodes (27): call_cached(), cancelled(), classify_turn_mode(), continue_turn(), derive_directives(), enabled_tools(), extract_bindings(), model_bindings() (+19 more)

### Community 225 - "KeptClaim"
Cohesion: 0.20
Nodes (18): apply_admiralty_evaluation(), brief_text(), cap_claims(), countries_differ(), dedupe_claims(), entity_path(), fingerprint(), finish_insights() (+10 more)

### Community 226 - "Argos UI Interaction Audit"
Cohesion: 0.40
Nodes (5): Argos UI Interaction Audit, Audit Matrix, Field inventory (`field_placeholder`), Remaining limits, Shared contracts

### Community 229 - "synthesize"
Cohesion: 0.22
Nodes (19): a_dropped_provider_stream_keeps_the_text_already_received(), a_provider_that_rejects_streaming_fails_without_a_hidden_retry(), a_streamed_answer_with_a_bad_citation_ends_on_the_repaired_answer(), an_idle_stall_or_the_ceiling_keeps_the_partial_answer(), answer_text(), cancel_during_synthesis_marks_the_run_cancelled_and_keeps_partial_text(), chat_server(), ChatReply (+11 more)

### Community 230 - "serde"
Cohesion: 0.15
Nodes (4): atlas_only_tools_blocked_on_all_surfaces(), check_need_gate(), sociavault_blocked_on_intel_surface(), test_scarce_provider_preflight_protections()

### Community 231 - "brain_lance_off.rs"
Cohesion: 0.14
Nodes (16): begin_generation(), block_on(), clear_fingerprint(), current_fingerprint(), DUPLICATE_THRESHOLD, fingerprint_matches(), GenerationProgress, LAST_ERROR (+8 more)

### Community 232 - "model_facts"
Cohesion: 0.13
Nodes (18): Cell, FallbackRow, model_blocked(), model_cancelled(), model_duration_cell(), model_facts(), model_fallback(), model_finished() (+10 more)

### Community 233 - "profile_components.rs"
Cohesion: 0.23
Nodes (7): clipped(), Kpi, panel(), table_row(), count_kpi(), kpis(), rate_kpi()

### Community 235 - "atlas_memory.rs"
Cohesion: 0.10
Nodes (18): atlas_memory_count(), Barrier, index_and_verify(), INDEXING_DISABLED_LINE, LEGACY_REVISION, memory_phase_lines_and_outcomes_are_truthful(), MEMORY_STAGE_EXTRACTION, MEMORY_STAGE_INDEXING (+10 more)

### Community 237 - "parse_serp_response"
Cohesion: 0.15
Nodes (21): clip_diag(), is_serp_path(), num_at(), parse_serp_response(), str_at(), a_redirected_final_url_is_not_used_to_rebuild_the_query(), an_empty_dom_without_any_payload_is_a_parser_mismatch(), api_http_404_is_an_upstream_failure() (+13 more)

### Community 238 - "seeded"
Cohesion: 0.45
Nodes (20): background_indexing_then_refresh_upgrades_a_partial_run(), claims(), clear(), crash_after_checkpoint_resumes_without_reextracting(), cycle_jobs(), disabled_embeddings_save_memories_and_say_so_honestly(), embedding_failure_keeps_memories_visible_reports_partial_and_retry_verifies(), enabling_embeddings_resumes_outstanding_work_and_upgrades_the_run() (+12 more)

### Community 239 - "atlas_actions.rs"
Cohesion: 0.29
Nodes (4): record_start(), start_repair(), starts(), infer_app()

### Community 240 - "provider_metrics.rs"
Cohesion: 0.15
Nodes (15): cache_capacity_rows(), cached_rows_become_an_available_snapshot(), capacity_snapshot(), CapacityRow, CapacitySnapshot, memory(), missing_companion_snapshot_is_unavailable_not_zero(), QueueRow (+7 more)

### Community 242 - "modes.rs"
Cohesion: 0.27
Nodes (13): scoped_section_plan(), chat_response_spec(), chat_response_spec_embeds_section_guidelines_and_style_guide(), chat_response_spec_lists_mode_headings(), chat_section_titles(), every_mode_has_bluf_first_and_sources_last(), every_mode_section_has_guideline(), investigation_mode_spec() (+5 more)

### Community 243 - "ChatMessage"
Cohesion: 0.17
Nodes (16): chat_body(), ChatMessage, complete(), complete_errors_with_finish_reason_when_response_is_empty(), complete_once(), complete_one(), complete_reads_reasoning_only_non_stream_json(), complete_stream_of_reasoning_deltas_yields_text() (+8 more)

### Community 244 - ".recon_outcomes"
Cohesion: 0.20
Nodes (9): normalize_run_outcome(), ReconOutcomeBucket, RunOutcome, Cancelled, CompletedWithEvidence, CompletedZeroEvidence, Failed, Partial (+1 more)

### Community 245 - ".job_source"
Cohesion: 0.50
Nodes (4): AtlasRun, JobSource, AtlasRun, ReconThread

### Community 246 - "PlanCall"
Cohesion: 0.24
Nodes (11): action_call(), bound(), isolation_lines(), isolation_lines_show_what_ran_what_was_held_and_why(), run_wave(), search_call(), served(), step() (+3 more)

### Community 247 - "AtlasInsightClaim"
Cohesion: 0.24
Nodes (16): Checkpoint, CheckpointInput, claim(), save_checkpoint(), article(), claim(), commit_replaces_atomically_and_keeps_shared_support(), failed_validation_leaves_prior_insights() (+8 more)

### Community 248 - "BrainResourceSummary"
Cohesion: 0.20
Nodes (10): bindings_use_brain_evidence_ids(), BrainResourceHit, BrainResourceSummary, candidate_json(), clip(), format_counts(), is_http_url(), parse_brain_scrape_index() (+2 more)

### Community 249 - "PickReply"
Cohesion: 0.43
Nodes (3): parse_chat_pick(), PickReply, PickRequestAdapter<'r, 'b>

### Community 250 - "provider_diag.rs"
Cohesion: 0.22
Nodes (14): text(), bounded(), classify_status(), endpoint_strips_query_and_userinfo(), find_url(), http_failure(), provider_error(), request_id() (+6 more)

### Community 251 - "graph_explanation/tests.rs"
Cohesion: 0.27
Nodes (16): auth_failure_consumes_primary_budget_gives_guidance_and_redacts(), conn(), count(), failure_is_durable_correlated_bounded_and_harmless(), fixture(), Fx, GOOD, late_completion_for_a_changed_or_deleted_memory_is_not_published() (+8 more)

### Community 252 - "SystemTab"
Cohesion: 0.33
Nodes (4): SystemTab, Configs, Overview, System

### Community 253 - ".intel_reports"
Cohesion: 0.18
Nodes (10): mean(), normalize_report_outcome(), report_key(), ReportOutcome, Blocked, Cancelled, Completed, Failed (+2 more)

### Community 254 - "§9 implementation order"
Cohesion: 0.15
Nodes (12): §10 acceptance checks, §9 implementation order, Atlas memory completion and System apps — implementation checklist, Phase 1 — what landed, Phase 2 — what landed, Phase 3 — what landed, Phase 4 — what landed, Phase 5a — what landed (+4 more)

### Community 255 - "DispatchError"
Cohesion: 0.25
Nodes (5): DispatchError, RetryDisposition, NextRoute, RetryRoute, Stop

### Community 257 - "accept_claims"
Cohesion: 0.15
Nodes (18): a_description_only_span_is_inference_and_an_added_name_is_dropped(), a_fact_requires_both_spans_in_the_title(), accept_claims(), AcceptMode, Context, Lead, ask_claims(), AskedClaims (+10 more)

### Community 258 - "run_primary"
Cohesion: 0.16
Nodes (17): a_claimed_email_removes_its_bindings(), a_gap_filler_domain_feeds_hunter_once_firecrawl_observes_it(), a_weak_firecrawl_search_adds_one_google_search_with_the_same_query(), a_zero_email_count_skips_the_paid_domain_search(), ac3_seo_results_that_never_mention_the_subject_add_no_domain_org_email_or_url(), ac4_google_search_always_sends_the_replaced_firecrawl_query(), ac5_every_executed_input_is_grounded_and_an_ungrounded_step_is_skipped(), ac7_a_pronoun_follow_up_takes_the_thread_subject() (+9 more)

### Community 259 - "run_live"
Cohesion: 0.29
Nodes (6): LiveRun, Fresh, Latest, Run, run_live(), RunInput

### Community 260 - "note_directives"
Cohesion: 0.28
Nodes (8): call_serves(), directive_assessments(), DirectiveAssessment, note_directives(), now_stamp(), run_facts(), RunFacts, stage_attempt()

### Community 261 - "draw"
Cohesion: 0.15
Nodes (16): Chrome, composer_height(), draw(), draw_header(), draw_slash_hint(), draw_system(), draw_tab_strip(), ensure_registered_layout() (+8 more)

### Community 262 - "Target"
Cohesion: 0.04
Nodes (46): brain_article_source_opens_intel_brief(), FocusEntry, hit(), IntelReconFocus, Section, Start, Tab, LayoutRegistry (+38 more)

### Community 263 - "CycleOutcome"
Cohesion: 0.18
Nodes (11): CycleOutcome, Blocked, Cancelled, Completed, CompletedWithWarnings, Failed, Partial, Pending (+3 more)

### Community 264 - ".related_memories"
Cohesion: 0.18
Nodes (3): atlas_article_from_row(), like_needle(), memory_row()

### Community 265 - "QuestionSpec"
Cohesion: 0.25
Nodes (5): DecisionQuestionType, Choice, Noul, Score, QuestionSpec

### Community 267 - "Profile analytics dashboard"
Cohesion: 0.33
Nodes (6): Dashboard preset, Primary inventory, Profile analytics dashboard, State and interaction, Verification and delivery, Visual and metric rules

### Community 268 - "CheckSignal"
Cohesion: 0.22
Nodes (8): CheckSignal, Blocked, Error, Inconclusive, NotRegistered, RateLimited, Registered, Unsupported

### Community 269 - "TaskState"
Cohesion: 0.18
Nodes (9): TaskState, Cancelled, Completed, Failed, Paused, Queued, RetryScheduled, Running (+1 more)

### Community 270 - "Profile TUI implementation session — 2026-10-10"
Cohesion: 0.22
Nodes (9): Artifact retention, Defects found and corrected, Fixture and screenshot construction, Gate results and environment handling, Ownership and implementation, Profile TUI implementation session — 2026-10-10, Scope and starting evidence, Screenshot evidence (+1 more)

### Community 276 - "Diagram conventions"
Cohesion: 0.50
Nodes (4): Agent rule, Catalog, Diagram conventions, When to draw

### Community 277 - "ModelExecEvent"
Cohesion: 0.29
Nodes (7): ModelExecEvent, AttemptFinish, AttemptReset, AttemptStart, FinalReplacement, ProvisionalDelta, RetryStatus

### Community 278 - "PlanInterval"
Cohesion: 0.33
Nodes (5): PlanInterval, Daily, Monthly, Never, Weekly

### Community 279 - "TUI implementation and verification"
Cohesion: 0.29
Nodes (7): Build fixtures and assertions, Capture actual cells, Finish and retain evidence, Plan the contract, Review, fix and recapture, Run the check gate, TUI implementation and verification

### Community 281 - "Unified reliability / summarization / semantic pipelines — completion checklist"
Cohesion: 0.40
Nodes (4): Honest remaining gaps, Phase status (§18), This follow-up, Unified reliability / summarization / semantic pipelines — completion checklist

### Community 282 - "AtlasCommand"
Cohesion: 0.50
Nodes (4): AtlasCommand, Repair, Resume, Verify

### Community 283 - "RepairStatus"
Cohesion: 0.22
Nodes (9): RepairStatus, Active, IndexingDisabled, IndexQueued, MissingPayload, NoInsights, Partial, Repaired (+1 more)

### Community 285 - "poll_device"
Cohesion: 0.28
Nodes (9): DeviceGrant, Poll, Denied, poll_device(), Pending, SlowDown, Token, start_device() (+1 more)

### Community 286 - "HypothesisRecord"
Cohesion: 0.32
Nodes (8): Alternative, classify_hypothesis(), contradiction(), draft_hypotheses(), hypothesis_absence_stays_unresolved(), hypothesis_status(), HypothesisRecord, hypothesis_view()

### Community 287 - "Json"
Cohesion: 0.38
Nodes (6): DiversityRow, json_array_len(), json_at(), json_bool(), json_str(), Json

### Community 288 - "select_strategy"
Cohesion: 0.40
Nodes (6): has_concrete_identifier(), hypothesis_question(), select_strategy(), strategy_change_reason(), strategy_follows_the_question_and_can_change_without_erasing_work(), StrategyChoice

### Community 289 - "TUI overhaul verification — 10 October 2026"
Cohesion: 0.33
Nodes (6): Defects found and corrected, Interaction and metric evidence, Limits, Reproduction, TUI overhaul verification — 10 October 2026, Visual evidence

## Knowledge Gaps
- **1815 isolated node(s):** `$schema`, `default_agent`, `subagent_depth`, `timeout`, `chunkTimeout` (+1810 more)
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 2491 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **39 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `App` connect `App` to `app.rs`, `Target`, `ui.rs`, `InvestigationSurface`, `unix_now`, `article_image.rs`, `Store`, `App`, `hardware.rs`, `Call`, `brain_detail.rs`, `ReportMode`, `tui/jobs.rs`, `worker.rs`, `.set_focus`, `src/brain.rs`, `Overlay`, `ConfigView`, `.reload_memories`, `logs.rs`, `RunStats`, `FeedArticle`, `now`, `DefaultsRole`, `AtlasArticleRow`, `.memory`, `summary_card.rs`, `ModuleId`, `WorkEvent`, `ToolResult`, `.job_source`, `briefing_view.rs`, `recon/graph.rs`, `run`, `SettingsFile`?**
  _High betweenness centrality (0.084) - this node is a cross-community bridge._
- **What connects `$schema`, `default_agent`, `subagent_depth` to the rest of the system?**
  _1815 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `land.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.0076045627376425855 - nodes in this community are weakly interconnected._
- **Why does `Value` connect `Value` to `accept_claims`, `run_primary`, `orchestrate.rs`, `tool_io.rs`, `ui.rs`, `recon.rs`, `Service`, `Store`, `serde_json`, `InvestigationSurface`, `wikipedia_rsp.rs`, `directives.rs`, `DecisionState`, `ProviderFailure`, `execute_steps`, `atlas_news.rs`, `results.rs`, `TelemetryEvent`, `whoxy.rs`, `profile.rs`, `App`, `news_legal.rs`, `DecisionsAdapter`, `osint.rs`, `holehe/mod.rs`, `.default`, `telemetry.rs`, `Call`, `body.rs`, `provider.rs`, `investigation.rs`, `rule_bindings`, `picker.rs`, `ReportMode`, `grok_oauth.rs`, `events.rs`, `config_transfer.rs`, `MemoryPhase`, `cli.rs`, `atlas_insights.rs`, `body_filter.rs`, `synthesize.rs`, `summarization.rs`, `search_engines.rs`, `.new`, `RunStats`, `now`, `src/evidence.rs`, `ToolRunner`, `DecisionContract`, `AtlasArticleRow`, `AtlasEvent`, `dork_generator.rs`, `run_turn`, `diversity.rs`, `.default`, `search_engines/tests.rs`, `parse_serp_response`, `components.rs`, `ToolResult`, `ChatMessage`, `ServiceResult`, `PlanCall`, `subscription.rs`, `BrainResourceSummary`, `PickReply`, `provider_diag.rs`, `config_transfer_tests.rs`?**
  _High betweenness centrality (0.076) - this node is a cross-community bridge._
- **Should `orchestrate.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.048131080389144903 - nodes in this community are weakly interconnected._
- **Why does `FieldId` connect `FieldId` to `app.rs`, `ui.rs`, `Target`, `Rect`, `DefaultsRole`, `draw_brain`, `run`?**
  _High betweenness centrality (0.039) - this node is a cross-community bridge._
- **Should `ProfileSnapshot` be split into smaller, more focused modules?**
  _Cohesion score 0.11904761904761904 - nodes in this community are weakly interconnected._