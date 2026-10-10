# Graph Report - argos-osint  (2026-10-10)

## Corpus Check
- 239 files · ~495,142 words
- Verdict: corpus is large enough that graph structure adds value.
- Unclassified: 9 file(s) not represented in the graph (top: (none) 3, .toml 2, .diff 2)

## Summary
- 7827 nodes · 19821 edges · 263 communities (229 shown, 34 thin omitted)
- Extraction: 98% EXTRACTED · 2% INFERRED · 0% AMBIGUOUS · INFERRED: 388 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `370dc13c`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- land.rs
- orchestrate.rs
- Binding
- atlas_memory.rs
- tool_io.rs
- app.rs
- BrainIndex
- FieldId
- map.rs
- recon.rs
- ButtonId
- Store
- IntelligenceCategory
- job_registry.rs
- unix_now
- directives.rs
- .handle_key
- investigation.rs
- providers.rs
- atlas_news.rs
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
- provider.rs
- .push_log
- InvestigationPart
- body.rs
- ProviderSecret
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
- .run_configured
- App
- events.rs
- exec.rs
- draw_intel_briefing
- model_exec.rs
- ErrorCategory
- ConfigIssue
- MemoryKind
- Rect
- brain_resources.rs
- worker.rs
- .set_focus
- src/brain.rs
- intel_recon/jobs.rs
- cli.rs
- atlas_insights.rs
- SourceReliability
- provider_diag.rs
- body_filter.rs
- synthesize.rs
- dataset.rs
- Reader
- profile_charts.rs
- atlas_table.rs
- Overlay
- pipeline.rs
- summarization.rs
- serde
- logs.rs
- search_engines.rs
- tui/jobs.rs
- .new
- FeedArticle
- budget.rs
- brain_lance.rs
- Command
- DefaultsRole
- theme.rs
- src/evidence.rs
- ToolRunner
- anyhow
- AtlasArticleRow
- AtlasEvent
- dork_generator.rs
- ProfileView
- How
- H3 Task Packet: State and Persistence Foundation
- summary_card.rs
- actor_review.rs
- ModuleId
- Region
- graph_explanation/tests.rs
- validate.rs
- Category
- split_vertical
- search_engines/tests.rs
- QuotaSettingsFile
- run_turn
- Architecture
- WorkEvent
- store.rs
- ClockSet
- ProviderAdmission
- Phase Plan: Home Recon Composer and Investigation Tabs
- model_roles.rs
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
- secrets.rs
- ServiceSpec
- TurnClock
- opencode.json
- Functional Requirements
- JobRow
- super
- ui.rs
- DateTime
- OsintCommand
- RouteInput
- TelemetryEvent
- Argos OSINT — Agent Instructions
- replace_insights.rs
- Store
- wikipedia_rsp.rs
- graph_explanation.rs
- DecisionState
- .default
- execute_steps
- .is_empty
- QueryExecutionStatus
- Value
- telemetry.rs
- SettingsFile
- DecisionAdapterKind
- contains
- §9 implementation order
- .new
- investigation/evidence.rs
- InvestigationPattern
- Part
- argos-osint-bin
- What Was Learned
- ledger.rs
- SiteOutcomeStatus
- .new
- Milestone 1 — Core Onboarding (Current)
- paths.rs
- Store
- atlas_work.rs
- H2 Contracts: Frozen Interface Decisions
- .new
- Codebase Map — argos-osint
- PROJECT.md — argos-osint
- Current Phase State
- diversity.rs
- RecoveryAction
- grok_oauth.rs
- .memory
- Conventions
- Docs Ingest — argos-osint
- ScheduledCall
- config_transfer.rs
- OpenCode V2 workflow
- Unified reliability / summarization / semantic pipelines — completion checklist
- Concepts
- package.json
- explore.rs
- activate_generation
- profile_config.rs
- Providers
- Decision roles and output contracts
- H0-H2 Completion Summary
- Terminal UI
- Profile dashboard and search
- RunStats
- BrainIndex
- normalize_destination
- render_tui_cells.py
- .order
- intel_recon/brain.rs
- Unified Investigation Harness
- Json
- .evaluate
- TurnContinuation
- Argos OSINT
- tool_runner.rs
- EventKind
- Argos documentation
- Diagram conventions
- AcceptMode
- Argos UI Interaction Audit
- ChainReport<T>
- TaskStatus
- PickRequest
- InsightStats
- brain_lance_off.rs
- Cell
- search_engines_fixtures.rs
- Supplied<T>
- .rebuild_vectors
- parse_serp_response
- home_rows
- atlas_actions.rs
- provider_metrics.rs
- names_subject
- SerpOutcome
- .recon_outcomes
- .open_job_source
- Trigger
- ExecuteOptions
- BrainResourceSummary
- Investigation flow
- AtlasPage
- ReportOutcome
- DispatchError
- run_live
- serde_json
- PlanInterval
- LayoutRegistry
- OperationScope<'a>

## God Nodes (most connected - your core abstractions)
1. `App` - 315 edges
2. `Value` - 277 edges
3. `ProviderSecret` - 143 edges
4. `ButtonId` - 130 edges
5. `ToolResult` - 78 edges
6. `FieldId` - 76 edges
7. `Store` - 73 edges
8. `Target` - 65 edges
9. `Store` - 64 edges
10. `AtlasArticleRow` - 57 edges

## Surprising Connections (you probably didn't know these)
- `Module layout` --references--> `widget_lines()`  [EXTRACTED]
  docs/architecture.md → crates/argos-osint-bin/src/tui/profile.rs
- `every_tool_request_sends_a_non_empty_user_agent()` --references--> `agent`  [INFERRED]
  crates/argos-osint-core/src/osint.rs → .opencode/opencode.json
- `create_report_job()` --calls--> `body_assertion_candidates()`  [INFERRED]
  crates/argos-osint-core/src/intel_recon/jobs.rs → crates/argos-osint-core/src/intel_recon/ledger.rs
- `all_four_modes_create_distinct_section_plans()` --calls--> `section_plan()`  [INFERRED]
  crates/argos-osint-core/src/intel_recon/tests_acceptance.rs → crates/argos-osint-core/src/intel_recon/modes.rs
- `paywall_and_snippet_fail_validation()` --calls--> `validate_article_body()`  [INFERRED]
  crates/argos-osint-core/src/intel_recon/tests_acceptance.rs → crates/argos-osint-core/src/intel_recon/validate.rs

## Import Cycles
- 1-file cycle: `crates/argos-osint-core/src/config_transfer.rs -> crates/argos-osint-core/src/config_transfer.rs`
- 2-file cycle: `crates/argos-osint-core/src/osint.rs -> crates/argos-osint-core/src/osint/results.rs -> crates/argos-osint-core/src/osint.rs`
- 2-file cycle: `crates/argos-osint-bin/src/cli.rs -> crates/argos-osint-core/src/hardware.rs -> crates/argos-osint-bin/src/cli.rs`
- 4-file cycle: `crates/argos-osint-core/src/config_commit.rs -> crates/argos-osint-core/src/provider_attempt.rs -> crates/argos-osint-core/src/provider_diag.rs -> crates/argos-osint-core/src/config_transfer.rs -> crates/argos-osint-core/src/config_commit.rs`

## Communities (263 total, 34 thin omitted)

### Community 0 - "land.rs"
Cohesion: 0.01
Nodes (262): CODES, LABELS, R0, R1, R10, R100, R101, R102 (+254 more)

### Community 1 - "orchestrate.rs"
Cohesion: 0.04
Nodes (92): opinion(), a_claimed_email_removes_its_bindings(), a_follow_up_keeps_names_from_the_previous_synthesis(), a_gap_filler_domain_feeds_hunter_once_firecrawl_observes_it(), a_keyword_inside_the_subjects_name_runs_no_news_or_legal_tool(), a_keyword_outside_the_subjects_name_still_runs_news_or_legal(), a_weak_firecrawl_search_adds_one_google_search_with_the_same_query(), a_zero_email_count_skips_the_paid_domain_search() (+84 more)

### Community 2 - "Binding"
Cohesion: 0.14
Nodes (29): Binding, is_brain_binding(), canonical(), dependency_order(), depends_on(), question_bindings_and_dependency_fix(), allowed_producer(), best_handle() (+21 more)

### Community 3 - "atlas_memory.rs"
Cohesion: 0.05
Nodes (96): Extraction, atlas_memory_count(), background_indexing_then_refresh_upgrades_a_partial_run(), Barrier, Checkpoint, claim(), claims(), clear() (+88 more)

### Community 4 - "tool_io.rs"
Cohesion: 0.03
Nodes (96): leftovers(), names(), PROMPT_TARGETS, normalize_platform(), question_bindings(), a_gap_filler_only_domain_waits_for_a_primary_observation_before_hunter(), accept_bindings(), batch_scrape_takes_ranked_urls_and_gates_order_before_hunter() (+88 more)

### Community 5 - "app.rs"
Cohesion: 0.06
Nodes (81): a_click_folds_an_event_and_incoming_entries_do_not_move_a_paused_list(), a_finished_tool_call_is_logged_once_and_the_transcript_keeps_a_summary(), all_provider_actions_render_with_hit_areas_at_80x24(), ATLAS_AUTO_SECS, atlas_auto_toggle_persists_the_next_trigger(), atlas_claim(), atlas_headlines_scroll_and_request_failures_reach_the_system_log(), atlas_history_resume_is_offered_only_for_resumable_cycles_and_repair_starts_once() (+73 more)

### Community 6 - "BrainIndex"
Cohesion: 0.14
Nodes (5): block_on(), BrainIndex, id_filter(), quote(), TABLE

### Community 7 - "FieldId"
Cohesion: 0.03
Nodes (60): FieldId, BrainApp, BrainConversation, BrainInsight, BrainQuery, ClaimAssessorModel, ClaimAssessorProvider, ClassifierModel (+52 more)

### Community 8 - "map.rs"
Cohesion: 0.05
Nodes (77): BRAILLE_MAP, canonical(), claim_label(), contains(), country_at(), country_fit(), country_pixel(), default_world_scale_is_enlarged_and_fits_small_terminals() (+69 more)

### Community 9 - "recon.rs"
Cohesion: 0.04
Nodes (76): ac6_citation_groups_split_validate_each_id_and_normalize(), answer_step_rate_limit_is_explicit_and_keeps_results(), attribute_word(), auxiliary(), BRIEF_SYNTHESIS, broad_question_collapses_to_grounded_lookups(), char_ceil(), char_floor() (+68 more)

### Community 10 - "ButtonId"
Cohesion: 0.02
Nodes (106): ButtonId, Add, AddFallback, AtlasAuto, AtlasDelete, AtlasLive, AtlasNews, AtlasNewsFeed (+98 more)

### Community 11 - "Store"
Cohesion: 0.06
Nodes (21): answer_evidence_survives_reopen_and_deleted_thread_rejects_late_calls(), cited(), claims_deduplicate_and_reject_unsupported_sources(), CreditHold, deleting_an_investigation_removes_its_brain_memories_and_graph_summary(), explicit_entities(), investigation_memory_ids(), Message (+13 more)

### Community 12 - "IntelligenceCategory"
Cohesion: 0.08
Nodes (27): test_all_catalog_tools_mapped_to_categories(), cache_follows_the_provider_plan_interval(), all_catalog_tools_mapped_to_categories(), argument_contract(), ArgumentBuilderContract, compact_capability_catalog(), CompactToolCapability, IntelligenceCategory (+19 more)

### Community 13 - "job_registry.rs"
Cohesion: 0.05
Nodes (48): begin(), begin_cancellable(), db(), db_path(), finish(), beat(), BEAT_INTERVAL, beats() (+40 more)

### Community 14 - "unix_now"
Cohesion: 0.18
Nodes (7): atlas_auto_future_tick_waits_and_past_tick_reschedules(), atlas_auto_while_running_only_arms_the_next_slot(), atlas_history_button_counts_down_while_auto_run_is_on(), atlas_poll_wait(), manual_run_moves_the_next_auto_trigger_out_by_90_minutes(), unix_now(), Atlas

### Community 15 - "directives.rs"
Cohesion: 0.06
Nodes (63): paint_logo(), accounts_search_query(), apply_context_targets(), asks(), asks_about_judge(), asks_for_headlines(), clip_words(), context_entity() (+55 more)

### Community 16 - ".handle_key"
Cohesion: 0.09
Nodes (12): backspace_after_a_sent_question_deletes_one_character(), ctrl_arrows_cycle_app_tabs_forward_and_back(), ctrl_tab_cycles_app_tabs_forward_and_back(), draft_isolation_and_persistence(), duplicate_submission_prevention(), home_order_renames_and_nine_routes_agree(), PaletteItem, session_tabs_open_close_reopen() (+4 more)

### Community 17 - "investigation.rs"
Cohesion: 0.05
Nodes (67): absorb_hit(), account_platform_host(), ACCOUNT_PLATFORMS, ACCOUNTS, accounts_flow(), ADAPTIVE, additional_tools(), additional_tools_name_three_more_unused_tools() (+59 more)

### Community 18 - "providers.rs"
Cohesion: 0.04
Nodes (79): annotate(), bounded(), clip_page(), domain(), email_address(), https_on_host(), ip(), linkedin_handle() (+71 more)

### Community 19 - "atlas_news.rs"
Cohesion: 0.07
Nodes (44): api_error(), ATLAS_TOOLS, authors(), clip(), country_arg(), country_code(), CURRENTS_COUNTRIES, CURRENTS_HOST (+36 more)

### Community 20 - "Store"
Cohesion: 0.06
Nodes (26): article_body_from_row(), ArticleBodyRow, cited_evidence_keeps_provenance_and_revision_identity(), element_from_row(), full_assessment_parents_are_flagged_for_child_report_totals(), IntelAssessmentRow, IntelEvidenceRow, IntelInvestigationRow (+18 more)

### Community 21 - "publication.rs"
Cohesion: 0.08
Nodes (42): AtlasInsightClaim, bump_memories_changed(), change_watcher_sees_only_committed_changes_and_coalesces(), claim(), clear(), clear_index_state(), CoverageReport, delete_run_memory_state() (+34 more)

### Community 22 - "result"
Cohesion: 0.09
Nodes (10): execute_harness_step(), InvestigationRuntime, GateOutcome, Passed, Rejected, validate_evidence_admission(), validate_publication(), validate_task_admission() (+2 more)

### Community 23 - "whoxy.rs"
Cohesion: 0.11
Nodes (35): adjacent_changes(), AdjacentChange, balance_request_url(), bounded_model_view(), check_balance(), contact(), ContactCard, date_and_limit_validation() (+27 more)

### Community 24 - "news_legal.rs"
Cohesion: 0.07
Nodes (44): a_rate_limit_switches_to_the_fallback_key_and_a_rejection_does_not(), ac1_each_request_has_its_url_params_and_auth_header_and_no_key_in_the_url(), an_echoed_key_is_redacted_from_the_stored_body_and_error(), article(), articles(), CATEGORIES, choice_arg(), context_kind() (+36 more)

### Community 25 - "hardware.rs"
Cohesion: 0.13
Nodes (20): system_host_lines(), CACHE_TTL_SECS, classify_arch(), disk_totals(), HardwareProfile, metal_vram_gb(), now_secs(), parse_apple_gpu_cores() (+12 more)

### Community 26 - "App"
Cohesion: 0.06
Nodes (74): api_key_slot(), ApiKeySlot, atlas_auto_label(), atlas_cycle_stats_room(), atlas_cycle_stats_scroll_max(), atlas_extracting(), atlas_feed_room(), atlas_feed_room_for() (+66 more)

### Community 27 - "Store"
Cohesion: 0.07
Nodes (10): atlas_answer_id(), atlas_brief_id(), atlas_prune_drops_finished_runs_older_than_the_cutoff(), AtlasRunRow, GraphSummary, has_table(), intel_link_explanations_round_trip_and_cleanup(), repair_embed_tables() (+2 more)

### Community 28 - "embed.rs"
Cohesion: 0.08
Nodes (31): active(), DIM, disable(), disabled(), DisableGuard, download_file(), DOWNLOAD_TIMEOUT_SECS, embed_batch() (+23 more)

### Community 29 - "osint.rs"
Cohesion: 0.06
Nodes (62): a_claimed_email_keeps_no_person_data(), bitcoin(), bounded_recovery_is_offered_once_for_an_unrecognised_page_only(), CACHE_DAY_SECONDS, cache_identity(), CACHE_MONTH_SECONDS, cache_seconds(), CACHE_WEEK_SECONDS (+54 more)

### Community 30 - "holehe/mod.rs"
Cohesion: 0.08
Nodes (34): cache_key(), cache_stores_only_definitive(), CACHE_TTL, CacheEntry, cancel_marks_inconclusive(), cap_reports_omitted(), check_implemented(), classify_http() (+26 more)

### Community 31 - "config_commit.rs"
Cohesion: 0.06
Nodes (73): a_batch_with_a_duplicate_or_empty_slot_is_refused_before_any_write(), a_committed_journal_needs_no_recovery(), a_second_lock_is_refused_while_one_is_held(), a_stale_lock_is_taken_over(), an_abandoned_unfinished_lockfile_is_recovered(), backup_path(), canonical_root(), commit_files_is_all_or_nothing() (+65 more)

### Community 32 - "provider.rs"
Cohesion: 0.04
Nodes (55): a_blank_osint_user_agent_loads_as_unset(), complete_errors_with_finish_reason_when_response_is_empty(), complete_stream_of_reasoning_deltas_yields_text(), concrete_free_models(), decide(), DecisionAnswer, decisions_client_posts_to_alpha_decisions_and_parses_answers(), DECISIONS_MODELS (+47 more)

### Community 34 - "InvestigationPart"
Cohesion: 0.10
Nodes (17): event_to_chat_block(), is_thinking_expanded(), InvestigationPart, DirectiveAssessment, EvidencePassage, GateValidation, Handoff, Plan (+9 more)

### Community 35 - "body.rs"
Cohesion: 0.09
Nodes (34): body_fetch_routes(), BodyFetchEvent, Attempt, Failed, InsightsFailed, InsightsRefreshing, InsightsReplaced, Progress (+26 more)

### Community 36 - "ProviderSecret"
Cohesion: 0.12
Nodes (35): resolve_actor_reviewer_secret(), account_secret(), active_text_secret(), authorize(), bearer_token(), catalog_error(), chat_body(), complete() (+27 more)

### Community 37 - "brain_detail.rs"
Cohesion: 0.05
Nodes (49): areas(), BrainDetail, DetailAreas, DetailPane, Path, Related, Summary, DetailSnapshot (+41 more)

### Community 38 - "profile_stats.rs"
Cohesion: 0.05
Nodes (73): a_snapshot_never_panics_on_a_partially_migrated_store(), an_empty_database_returns_empty_sections_not_zeros(), atlas_carries_data(), AtlasStats, BacklogRow, band_for(), CategoryTrigger, classify_model_failure() (+65 more)

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
Cohesion: 0.10
Nodes (21): summary_text(), active_time_is_live_for_open_attempts_and_unavailable_only_without_a_start(), AttemptRow, event_apps(), event_counts(), EventCounts, get_job(), job() (+13 more)

### Community 44 - "provider_attempt.rs"
Cohesion: 0.13
Nodes (28): Acc, attempt(), attempt_with_observer(), AttemptReport, check_final(), Deadlines, fill(), is_subscription() (+20 more)

### Community 45 - "LogicalRole"
Cohesion: 0.10
Nodes (11): LogicalRole, .ALL, ClaimAssessor, Classifier, Controller, EntityResolver, EvidenceCurator, Planner (+3 more)

### Community 46 - "picker.rs"
Cohesion: 0.13
Nodes (22): brain_scrape_options_hide_bare_scrape_and_include_claim_criteria(), CatalogEntry, CONFIDENCE_FLOOR, decisions_request(), directives_phrase(), DONE, eligible_catalog(), empty_brain_resources_omitted_from_state() (+14 more)

### Community 47 - "ReportMode"
Cohesion: 0.09
Nodes (35): HomeDraftState, brief_rating_reuses_the_existing_mean_semantics(), classifiable_modes(), classify_mode_chat(), classify_mode_decisions(), classify_prompt_mode(), classify_prompt_mode_chat(), classify_prompt_mode_decisions() (+27 more)

### Community 48 - "provider_chain.rs"
Cohesion: 0.15
Nodes (22): AttemptRecord, cancellation_stops_the_chain(), ChainReport, execute(), FALLBACK_ATTEMPTS, fallback_emits_three_attempts_with_10_20_waits(), FALLBACK_WAITS, http_stream_and_parse_failures_all_retry() (+14 more)

### Community 49 - "atomic"
Cohesion: 0.08
Nodes (15): AttemptOutcome, categorize_http(), execute_with_retries(), other_llm_allows_three(), summarization_stops_at_two_attempts(), TimeoutProfile, .CLASSIFIER, .EXTRACTION (+7 more)

### Community 50 - ".run_configured"
Cohesion: 0.08
Nodes (30): bind_request(), body_cap(), credential_key(), Executor, exhausted_keys(), get(), header_for(), read_limited() (+22 more)

### Community 51 - "App"
Cohesion: 0.03
Nodes (40): a_failed_turn_keeps_the_streamed_answer_on_screen(), a_log_or_late_tool_row_does_not_drop_the_live_answer(), App, atlas_countdown_visible(), atlas_extracting_visible(), BrainListMode, Create, Graph (+32 more)

### Community 52 - "events.rs"
Cohesion: 0.07
Nodes (33): LevelFilter, All, Error, Info, Warn, session_event(), clear_events(), DEFAULT_RETENTION_HOURS (+25 more)

### Community 53 - "exec.rs"
Cohesion: 0.14
Nodes (20): admission_account(), admission_contention_consumes_no_attempt(), AttemptEvent, Finished, Started, AttemptLog, cancelled(), complete_summary_report() (+12 more)

### Community 54 - "draw_intel_briefing"
Cohesion: 0.10
Nodes (40): abs_contains(), abs_rect(), AbsRect, draw_centered_loading_card(), draw_clipped_button(), draw_clipped_intel_loading(), draw_clipped_md_pane(), draw_intel_body_loading() (+32 more)

### Community 55 - "model_exec.rs"
Cohesion: 0.10
Nodes (33): adapter_reported_first_response_is_kept_when_no_chunk_arrived(), attempt_outcome_keeps_the_failure_category_for_the_reason_dimension(), AttemptFacts, AttemptOutcome, AttemptTimings, canonical_attempt_id(), canonical_attempt_id_is_stable_across_replay(), DecisionsAdapter (+25 more)

### Community 56 - "ErrorCategory"
Cohesion: 0.05
Nodes (33): operation_kind(), backoff_delay(), can_retry(), ErrorCategory, AuthOrQuota, CancelledOrStale, ConfigurationMissing, ContextLimit (+25 more)

### Community 57 - "ConfigIssue"
Cohesion: 0.29
Nodes (27): array_value(), check_known_fields(), check_limit(), check_required(), check_role_compatibility(), check_route_reference(), check_scheme(), child() (+19 more)

### Community 58 - "MemoryKind"
Cohesion: 0.09
Nodes (22): AdmissionPolicy, admit_memory(), BrainQuery, pack_context(), AssessmentState, Contradicts, Insufficient, Mentions (+14 more)

### Community 59 - "Rect"
Cohesion: 0.12
Nodes (58): intel_category_short(), Block, draw(), active_popup_area(), add_fallback_popup_area(), button_areas(), choice_list_room(), configs_area() (+50 more)

### Community 60 - "brain_resources.rs"
Cohesion: 0.09
Nodes (25): source_anchor_label(), BRAIN_SCRAPE_PREFIX, CLAIM_CHARS, classify_resource(), file_link_url(), hit(), is_brain_scrape_pick(), looks_like_api_endpoint() (+17 more)

### Community 61 - "worker.rs"
Cohesion: 0.15
Nodes (20): IntelReportEvent, InsightsUpdated, JobCreated, JobDone, Section, Stage, IntelReportJobRow, cancel_if_needed() (+12 more)

### Community 62 - ".set_focus"
Cohesion: 0.04
Nodes (39): add_scroll(), hit(), IntelReconFocus, Section, Start, Tab, keyboard_navigation_esc_and_shortcuts(), Target (+31 more)

### Community 63 - "src/brain.rs"
Cohesion: 0.12
Nodes (21): AGREEMENT_WEIGHT, CATEGORIES, category_hint(), format_injection(), hybrid_recall(), hybrid_recall_adds_paraphrases_and_rewards_agreement(), identity_query_prefers_name_memory(), jaccard() (+13 more)

### Community 64 - "intel_recon/jobs.rs"
Cohesion: 0.14
Nodes (21): active_job_for_mode(), cancel_job(), create_job_inserts_all_verify_sections(), create_report_job(), finish_canonical(), intel_recon_links_its_legacy_job_to_one_canonical_registry_job(), pause_job(), register_canonical() (+13 more)

### Community 65 - "cli.rs"
Cohesion: 0.15
Nodes (21): ask(), ask_thread(), atlas_command(), defaults_command(), defaults_json(), defaults_show_includes_the_tool_picker_transport(), DefaultsCommand, Set (+13 more)

### Community 66 - "atlas_insights.rs"
Cohesion: 0.06
Nodes (64): a_country_token_does_not_merge_into_a_longer_name(), a_decisions_model_does_not_extract_claims(), a_description_only_span_is_inference_and_an_added_name_is_dropped(), a_fact_requires_both_spans_in_the_title(), accept_claims(), aliases_merge_on_shared_tokens_when_the_longer_form_is_a_span(), article(), article_with_body_spans() (+56 more)

### Community 67 - "SourceReliability"
Cohesion: 0.09
Nodes (24): admiralty_scales_claim_confidence_from_rsp_and_peers(), apply_admiralty_evaluation(), article_information_credibility(), AdmiraltyCode, best_credibility(), CredibilityInputs, information_credibility(), InformationCredibility (+16 more)

### Community 68 - "provider_diag.rs"
Cohesion: 0.09
Nodes (27): text(), bounded(), cause_chain(), classify_status(), classify_text(), endpoint_strips_query_and_userinfo(), find_url(), http_failure() (+19 more)

### Community 69 - "body_filter.rs"
Cohesion: 0.18
Nodes (22): BATCH_SIZE, BodyChunk, chunk_article_body(), CHUNK_MAX, CHUNK_MIN, CHUNK_TARGET, chunks_preserve_absolute_offsets(), classify_chunk_batch() (+14 more)

### Community 70 - "synthesize.rs"
Cohesion: 0.18
Nodes (19): BODY_REFINE_INPUT_CHARS, deterministic_bluf_mentions_title(), deterministic_section(), model_refine_body(), model_synthesize(), parse_json_object(), refine_retrieved_article_body(), refine_without_secret_keeps_validated_scrape() (+11 more)

### Community 71 - "dataset.rs"
Cohesion: 0.13
Nodes (24): acquire_refresh_lease(), active_manifest_path(), ACTIVE_REFRESHES, dataset_root(), DatasetManifest, DatasetStatus, get_status(), import_from_file() (+16 more)

### Community 72 - "Reader"
Cohesion: 0.12
Nodes (19): ArticleFact, atlas_origin_rows(), category_triggers(), DiversityRow, dominant_label(), engine_health(), EventRow, fingerprint() (+11 more)

### Community 73 - "profile_charts.rs"
Cohesion: 0.08
Nodes (38): BLOCKS, cap_eighths(), cell_width(), cells(), column_cell(), duration_ms(), eighths(), eighths_of_ratio() (+30 more)

### Community 74 - "atlas_table.rs"
Cohesion: 0.17
Nodes (22): border_style(), borders_and_cells_have_matching_display_widths(), bottom_border_line(), char_width(), clip_to_width(), column_widths(), columns_consume_inner_width(), columns_distribute_proportionally_when_not_flex_last() (+14 more)

### Community 75 - "Overlay"
Cohesion: 0.08
Nodes (17): ChoiceKind, IntelDay, Investigation, Model, Provider, Overlay, AddFallback, Choice (+9 more)

### Community 76 - "pipeline.rs"
Cohesion: 0.11
Nodes (26): apply_recon_directive_coverage(), ClaimRelation, AmbiguousEntity, ChangedQuantity, Equivalent, Negation, PlannedVsCompleted, Unrelated (+18 more)

### Community 77 - "summarization.rs"
Cohesion: 0.10
Nodes (37): cache_get(), cache_key_changes_with_revision_and_focus(), cache_put(), cache_round_trip(), complete_summary(), CoverageMeta, deterministic_article_description(), deterministic_atlas_brief() (+29 more)

### Community 78 - "serde"
Cohesion: 0.09
Nodes (14): CallProposal, HandoffRecord, InvestigationSurface, HomeComposer, IntelBrief, JobsResume, ReconChat, atlas_only_tools_blocked_on_all_surfaces() (+6 more)

### Community 79 - "logs.rs"
Cohesion: 0.12
Nodes (16): areas(), button_label(), buttons(), count(), draw(), hit(), in_list(), list_geometry() (+8 more)

### Community 80 - "search_engines.rs"
Cohesion: 0.07
Nodes (43): Candidate, card_snippet(), classify_status_region(), clip_diag(), clip_item_text(), collapse_ws(), collect_candidates(), contains_phrase() (+35 more)

### Community 81 - "tui/jobs.rs"
Cohesion: 0.08
Nodes (25): areas(), BASE_COLUMNS, button_label(), buttons(), detail_lines(), hit(), JobsAreas, JobsView (+17 more)

### Community 82 - ".new"
Cohesion: 0.25
Nodes (19): a_spent_primary_quota_uses_the_fallback_key(), classification_request(), country_labels_show_the_name_and_the_code(), daily_cap(), format_run_card(), HttpCall, HttpReply, insights_do_not_call_a_decisions_model() (+11 more)

### Community 83 - "FeedArticle"
Cohesion: 0.17
Nodes (19): apply_hits(), article_from(), article_row(), canonical_url(), category_tag(), dedup_drops_a_url_an_exact_title_and_keeps_the_higher_domain(), FeedArticle, host_of() (+11 more)

### Community 84 - "budget.rs"
Cohesion: 0.12
Nodes (14): CUT_NOTE, CUT_SHORT, PHASE_WINDOW_SECONDS, RECON_DEADLINE, RECON_ROUND_SECONDS, STREAM_LOST, synthesis_allowance_capped(), synthesis_allowance_seconds() (+6 more)

### Community 85 - "brain_lance.rs"
Cohesion: 0.12
Nodes (16): current_fingerprint(), DUPLICATE_THRESHOLD, EMBED_CHUNK, ensure_ann_index_respects_exact_policy(), fingerprint_matches(), GENERATION_BATCH, LAST_ERROR, LAYOUT (+8 more)

### Community 86 - "Command"
Cohesion: 0.10
Nodes (20): AtlasCommand, Repair, Resume, Verify, Cli, Command, Atlas, Defaults (+12 more)

### Community 87 - "DefaultsRole"
Cohesion: 0.06
Nodes (20): ChoiceItem, codex_models(), DefaultsRole, .ALL, ClaimAssessor, Classifier, EntityResolver, EvidenceCurator (+12 more)

### Community 88 - "theme.rs"
Cohesion: 0.15
Nodes (22): ACCENT, BG, BORDER, card(), card_accent(), card_dim(), card_text(), CODE_BG (+14 more)

### Community 89 - "src/evidence.rs"
Cohesion: 0.09
Nodes (29): AGREEMENT_WEIGHT, AnnMeasurement, AnnPolicy, AnnDeferredUnmeasured, AnnEnabled, ExactSearch, AnnThresholds, chunk_text() (+21 more)

### Community 90 - "ToolRunner"
Cohesion: 0.11
Nodes (13): attribution_is_explicit_never_inferred_from_the_prompt(), failure_reason(), InvocationFact, named_engine(), test_tool_runner_cache_hit(), test_tool_runner_concurrency_and_dedup(), ToolAttribution, ToolRunner (+5 more)

### Community 91 - "anyhow"
Cohesion: 0.11
Nodes (25): compile_general_model_prompt(), compile_native(), parse_general_model_response(), parse_native_response(), DecisionContract, template_claim_relation(), template_directive_alignment(), template_entity_binding() (+17 more)

### Community 92 - "AtlasArticleRow"
Cohesion: 0.11
Nodes (38): accept_one(), apply_peer_support(), brief_rating_mean(), cap_claims(), catalog_json(), classifier_peers_are_preferred_over_token_overlap(), classify_peers_chat(), classify_peers_decisions() (+30 more)

### Community 93 - "AtlasEvent"
Cohesion: 0.12
Nodes (24): AtlasEvent, Article, Classified, Fault, InsightProgress, MemoriesChanged, MemoryProgress, Note (+16 more)

### Community 94 - "dork_generator.rs"
Cohesion: 0.11
Nodes (31): ACTIVE_SNAPSHOT, compose_single_query(), compute_query_id(), compute_template_id(), DATASET_NAME, DEFAULT_MAX_QUERIES, DorkCatalog, DorkCategory (+23 more)

### Community 95 - "ProfileView"
Cohesion: 0.08
Nodes (26): draw_filter_strip(), draw_overview(), draw_profile(), draw_section_body(), draw_section_navigator(), draw_status_strip(), draw_system_tab(), draw_tab_strip() (+18 more)

### Community 96 - "How"
Cohesion: 0.11
Nodes (18): How, CompanyEmail, Coordinates, DomainAsUrl, EntityPhrase, PackageName, PackageParts, Plain (+10 more)

### Community 97 - "H3 Task Packet: State and Persistence Foundation"
Cohesion: 0.07
Nodes (26): Acceptance Checks and Exact Existing Test Targets:, Current Diff / Prior Changes to Preserve:, Effective Agent / Model / Variant:, Exact Existing Test Targets to Reuse/Extend:, Existing APIs / Data Models to Reuse:, Expected Changed Files:, Expected Commands:, Files This Worker Owns: (+18 more)

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
Cohesion: 0.15
Nodes (23): explain(), ExplainOutcome, Failed, Saved, Superseded, ExplainReport, ExplainRequest, auth_failure_consumes_primary_budget_gives_guidance_and_redacts() (+15 more)

### Community 103 - "validate.rs"
Cohesion: 0.18
Nodes (16): BLOCK_MARKERS, BodyQuality, Complete, Partial, Unavailable, Uncertain, BodyValidation, clean_markdown() (+8 more)

### Community 104 - "Category"
Cohesion: 0.08
Nodes (21): Category, Auth, Cancelled, Configuration, Empty, InvalidModel, InvalidResult, MalformedPayload (+13 more)

### Community 105 - "split_vertical"
Cohesion: 0.13
Nodes (23): add_fallback_layout(), brain_form(), brain_list(), BrainForm, BrainList, chat_areas(), cursor_at(), dashboard_areas() (+15 more)

### Community 106 - "search_engines/tests.rs"
Cohesion: 0.10
Nodes (43): build_scrape_body(), build_serp_url(), a_non_serp_path_on_the_engine_host_is_not_a_serp_page(), a_noscript_wrapped_consent_meta_refresh_is_consent(), a_standalone_zero_count_in_a_status_region_is_a_verified_zero(), a_zero_phrase_outside_a_status_region_is_not_a_verified_zero(), an_off_host_final_url_with_cards_keeps_the_items_but_is_not_valid(), an_off_host_final_url_without_cards_is_never_a_zero() (+35 more)

### Community 107 - "QuotaSettingsFile"
Cohesion: 0.13
Nodes (15): commit_profile_config(), ConfigChange, ConfigurationSnapshot, describe_changes(), export_to_path(), import_from_path(), ImportPlan, merge_key() (+7 more)

### Community 108 - "run_turn"
Cohesion: 0.12
Nodes (33): a_budgeted_call_with_a_dependency_from_an_earlier_batch_runs(), ac8_status_errors_and_401_403_fail_readably_and_are_not_cached(), advance_stage(), BudgetedOutcome, call_cached(), cancel_mid_dispatch_releases_credit_holds(), cancelled(), classify_turn_mode() (+25 more)

### Community 109 - "Architecture"
Cohesion: 0.14
Nodes (14): Admiralty source evaluation (WP:RSP), Architecture, Atlas, Config transfer and the commit, Intel reports, Model and UI boundaries, Module layout, News and Legal context tools (#29) (+6 more)

### Community 110 - "WorkEvent"
Cohesion: 0.07
Nodes (23): work_event(), WorkEvent, AnswerDelta, AnswerNote, AnswerReplacement, AnswerReset, AtlasDone, BrainRelatedExplained (+15 more)

### Community 111 - "store.rs"
Cohesion: 0.10
Nodes (23): ArticleInsightCommit, AUTO_REBUILD_HINT, boost_with_passage_hybrid(), deleting_a_memory_removes_its_graph_summary(), embedding_disabled_degrades_to_jaccard(), embedding_failure_mid_session_degrades_to_jaccard(), every_memory_has_provenance_and_category(), fingerprint_mismatch_rebuilds_and_drift_is_reconciled() (+15 more)

### Community 112 - "ClockSet"
Cohesion: 0.21
Nodes (3): ClockSet, queue_time_does_not_spend_active_but_consumes_wall(), sequential_vs_overlapping_tool_budgets()

### Community 113 - "ProviderAdmission"
Cohesion: 0.18
Nodes (6): shared_rate_limit_cooldown_blocks_admission(), AdmissionGuard, note_shared_rate_limit(), provider_admission_caps_at_two(), ProviderAdmission, rate_limit_cooldown_blocks_acquire()

### Community 114 - "Phase Plan: Home Recon Composer and Investigation Tabs"
Cohesion: 0.11
Nodes (17): 1. Outcome, 2. Home Layout (Section 4), 3. Home Composer (Section 5), 4. Submission & Transition (Section 6), 5. Persistent Investigation Tabs (Section 7), 6. Keyboard & Mouse Contract (Section 8), 7. Recovery & QoL (Section 9), Acceptance Criteria (Spec Section 12) (+9 more)

### Community 115 - "model_roles.rs"
Cohesion: 0.13
Nodes (13): AccountHealth, AuthRequired, Cooldown, CreditsExhausted, Overloaded, Ready, Restricted, Unverified (+5 more)

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
Cohesion: 0.15
Nodes (41): amplification_lines(), attribution_lines(), backlog_lines(), capacity_lines(), cause_lines(), confidence_lines(), cycle_lines(), cycle_time_lines() (+33 more)

### Community 121 - "IndexOutcome"
Cohesion: 0.15
Nodes (6): IndexOutcome, Disabled, Pending, PermanentFailure, Ready, RetryableFailure

### Community 122 - "recon/graph.rs"
Cohesion: 0.06
Nodes (60): draw(), draw_path(), draw_summary(), glyph(), inset(), legend_height(), legend_parts(), legend_rows() (+52 more)

### Community 123 - "tasks.rs"
Cohesion: 0.07
Nodes (71): lease_fencing_rejects_stale_epoch_and_foreign_owner(), add_missing_columns(), adopt_untracked_index_changes(), block_claimed(), claim_complete_and_retry_round_trip(), claim_index_changes(), claim_index_work(), claim_next() (+63 more)

### Community 124 - "ReconCommand"
Cohesion: 0.17
Nodes (12): ReconCommand, Ask, AskNew, Delete, Limits, List, New, Rename (+4 more)

### Community 125 - "config_transfer_tests.rs"
Cohesion: 0.14
Nodes (40): apply_import(), export_document(), parse_document(), a_failed_validation_writes_nothing(), a_subscription_account_keeps_its_device_endpoints_through_an_import(), a_syntax_error_reports_line_and_column(), an_error_message_never_echoes_a_key(), an_unresolved_env_reference_warns_instead_of_importing_an_empty_key() (+32 more)

### Community 126 - "Service"
Cohesion: 0.11
Nodes (23): AnswerContext, await_completion(), compact_page_evidence(), cut_footer(), cut_short_answer(), directive_goals_line(), evidence_summary(), finish_recon_job() (+15 more)

### Community 127 - "secrets.rs"
Cohesion: 0.11
Nodes (15): DeviceGrant, Poll, Denied, poll_device(), Pending, SlowDown, Token, start_device() (+7 more)

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

### Community 132 - "JobRow"
Cohesion: 0.12
Nodes (11): job_row(), JobFilter, JobRow, JobStatusFilter, Active, All, Completed, Failed (+3 more)

### Community 133 - "super"
Cohesion: 0.10
Nodes (12): CODES, NAMES, ADAPTER_VERSION, HOST, ID, positive_negative_and_unknown(), request_url(), ADAPTER_VERSION (+4 more)

### Community 134 - "ui.rs"
Cohesion: 0.05
Nodes (76): ACTION_H, atlas_countdown(), atlas_history_live_label(), atlas_source_anchors_are_not_labeled_deleted_origin(), build_blocks(), call_stamp(), center_line(), center_text() (+68 more)

### Community 135 - "DateTime"
Cohesion: 0.10
Nodes (30): adaptive_bucket_seconds(), BriefFact, Bucket, bucket_count(), bucket_floor(), bucket_format(), bucket_start(), CoverageFact (+22 more)

### Community 136 - "OsintCommand"
Cohesion: 0.14
Nodes (14): DatasetCommand, Import, Refresh, Status, OsintCommand, Attach, Dataset, Describe (+6 more)

### Community 137 - "RouteInput"
Cohesion: 0.17
Nodes (11): RouteInput, FacebookUrl, Handle, HandleOrUserId, Hashtag, LinkedinCompanyUrl, LinkedinProfileUrl, Query (+3 more)

### Community 138 - "TelemetryEvent"
Cohesion: 0.13
Nodes (10): clamp(), coverage_reports_observed_since_and_counts(), db(), events_persist_and_roll_up_once(), id_is_stable_so_replay_never_double_counts(), logical_attempt_and_cache_rows_stay_distinct_under_replay(), record(), safe_payload() (+2 more)

### Community 139 - "Argos OSINT — Agent Instructions"
Cohesion: 0.11
Nodes (18): Architecture notes, Argos OSINT — Agent Instructions, CI (`.github/workflows/ci.yml`), CLI Entry Points (`argos` binary), Developer Commands, diagrams, Environment Variables, graphify (+10 more)

### Community 140 - "replace_insights.rs"
Cohesion: 0.35
Nodes (11): commit_refined_body(), article(), claim(), commit_replaces_atomically_and_keeps_shared_support(), failed_validation_leaves_prior_insights(), replace_article_insights_from_body(), ReplaceOutcome, snapshot_counts_are_stable_before_commit() (+3 more)

### Community 141 - "Store"
Cohesion: 0.13
Nodes (4): TaskRecord, ClaimAssessment, validate_claim_assessment(), Store

### Community 142 - "wikipedia_rsp.rs"
Cohesion: 0.09
Nodes (38): API, APP_STATE_KEY, cache(), CACHE_TTL, cached_index(), CachedIndex, clip_summary(), ensure_index() (+30 more)

### Community 143 - "graph_explanation.rs"
Cohesion: 0.07
Nodes (25): BASIC_HEADING, Cached, None, Stale, Valid, FAILURE_COOLDOWN, Faults, Gate (+17 more)

### Community 144 - "DecisionState"
Cohesion: 0.20
Nodes (4): DecisionState, EvidencePassageState, SubjectStateSummary, TaskStateSummary

### Community 145 - ".default"
Cohesion: 0.09
Nodes (59): a_429_stops_picker_calls_and_the_fallback_finishes_the_list(), a_dispatch_error_fails_the_step_and_the_loop_goes_on(), a_dropped_provider_stream_keeps_the_text_already_received(), a_failed_email_finder_gets_one_fallback_and_no_third_picker_call(), a_fallback_with_no_bindings_still_runs_the_satisfied_steps_and_reaches_synthesis(), a_goal_naming_a_tool_or_provider_is_repaired_then_falls_back(), a_goal_naming_a_tool_outside_the_prompt_entity_is_still_rejected(), a_handle_named_in_a_derived_question_is_an_unverified_binding() (+51 more)

### Community 146 - "execute_steps"
Cohesion: 0.15
Nodes (30): after_step(), apply_order(), binding_ground(), context_block(), context_dispatched(), directive_for(), execute_steps(), expand_dork_children() (+22 more)

### Community 147 - ".is_empty"
Cohesion: 0.11
Nodes (16): delta_text(), message_json(), message_reasoning(), message_text(), ModelAssignment, ModelRoute, parse_completion(), parse_completion_errors_clearly_when_truly_empty() (+8 more)

### Community 148 - "QueryExecutionStatus"
Cohesion: 0.17
Nodes (11): QueryExecutionStatus, Cancelled, Completed, Failed, Generated, NoResults, Queued, Running (+3 more)

### Community 149 - "Value"
Cohesion: 0.08
Nodes (33): Value, classify_output_quality(), extract_observation_items(), extract_search_results(), extracts_from_array_shaped_observations(), extracts_from_object_with_results_array(), normalize_tool_result(), NormalizedToolResult (+25 more)

### Community 150 - "telemetry.rs"
Cohesion: 0.11
Nodes (22): bin_index(), Coverage, duration_bins_are_bounded_and_mergeable(), DURATION_BINS_MS, ensure_observed_since(), event_bins(), get_meta(), ID_SEQ (+14 more)

### Community 151 - "SettingsFile"
Cohesion: 0.11
Nodes (5): ReconLimits, set_saved_key_round_trips_every_keyed_tool_provider(), SettingsFile, RoleRuntime, cost_map()

### Community 152 - "DecisionAdapterKind"
Cohesion: 0.10
Nodes (20): Item, DecisionAdapterKind, JsonMode, NativeDecisions, OutputTool, StrictJsonSchema, ValidatedText, resolve_adapter() (+12 more)

### Community 153 - "contains"
Cohesion: 0.16
Nodes (22): atlas_hit(), atlas_row_at(), brain_hit(), contains(), focus_order(), hit_test(), home_line(), in_pane() (+14 more)

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

### Community 167 - ".new"
Cohesion: 0.09
Nodes (61): Account, account_hits(), account_platforms_attach_to_the_subject_and_are_never_entities(), action_order(), actions_are_grounded_capped_and_not_a_sweep(), adaptive_expansion_spends_one_scarce_lookup_then_reranks(), adaptive_step(), attach_accounts() (+53 more)

### Community 168 - "Milestone 1 — Core Onboarding (Current)"
Cohesion: 0.18
Nodes (10): Deliverables, Dependencies, Future Milestones (Planned), Milestone 1 — Core Onboarding (Current), Milestone 2 — CLI Verification, Milestone 3 — Test Coverage, Milestone 4 — Integration & Ship, Objectives (+2 more)

### Community 169 - "paths.rs"
Cohesion: 0.25
Nodes (11): auth_path(), config_journal_path(), config_lock_path(), config_path(), dataset_dir(), datasets_dir(), db_path(), ensure_home() (+3 more)

### Community 170 - "Store"
Cohesion: 0.26
Nodes (3): active_index_rows(), Store, IndexEnqueue

### Community 171 - "atlas_work.rs"
Cohesion: 0.06
Nodes (36): dropped_from_receipt(), articles_rev(), AttemptRecord, check_dependency_coverage(), CycleOutcome, Blocked, Cancelled, Completed (+28 more)

### Community 172 - "H2 Contracts: Frozen Interface Decisions"
Cohesion: 0.18
Nodes (10): 1. Home Draft Key, 2. Launch State Machine, 3. Persistence Boundary, 4. Tab Identity, 5. Navigation Intent, 6. State Restoration (Per-Tab), 7. Input Precedence Hierarchy, 8. Layout Measurements (Reference Points) (+2 more)

### Community 173 - ".new"
Cohesion: 0.11
Nodes (26): active_jobs(), add_cycle_outcome(), AmplificationBucket, AttemptFact, AttemptSummary, count_rows(), CycleOutcomeBucket, distinct_values() (+18 more)

### Community 174 - "Codebase Map — argos-osint"
Cohesion: 0.22
Nodes (8): `argos-osint-bin` — CLI and TUI, `argos-osint-core` — Core Library, Codebase Map — argos-osint, Crates, Documentation Ingest, Primary Data Flow, State & Config (all under `~/.argos`), Tool Input & Binding Kinds (from `tool_io.rs`)

### Community 175 - "PROJECT.md — argos-osint"
Cohesion: 0.22
Nodes (8): Applications, CLI Entry Points, Core Components, Crates, Investigation Flow, PROJECT.md — argos-osint, Project Purpose, State Directory (`~/.argos`)

### Community 176 - "Current Phase State"
Cohesion: 0.25
Nodes (7): Artifacts Status, Configuration State, Current Phase State, Next Steps, Pending Items, Phase: Core Onboarding, STATE.md — argos-osint

### Community 177 - "diversity.rs"
Cohesion: 0.17
Nodes (24): coverage_gaps_summary(), coverage_record_id(), coverage_stats(), CoverageCandidate, CoverageRecord, CoverageStats, covered_record(), credentials_available() (+16 more)

### Community 178 - "RecoveryAction"
Cohesion: 0.12
Nodes (12): classify_recovery(), RecoveryAction, AccessRestricted, CircuitCooldown, ContentRefused, CreditsExhausted, FallbackModel, ReduceContext (+4 more)

### Community 179 - "grok_oauth.rs"
Cohesion: 0.13
Nodes (20): auth_path(), bearer(), bearer_from_path(), check_login(), Entry, entry_from(), failed_login_keeps_the_cli_reason_and_recovery_action(), login() (+12 more)

### Community 180 - ".memory"
Cohesion: 0.18
Nodes (12): article(), atlas_articles_for_intel_dedupes_same_article_id_across_runs(), atlas_articles_for_intel_filters_category_day_and_query(), atlas_auto_next_round_trips_through_app_state(), atlas_claims_for_article_returns_linked_claims(), atlas_recent_articles_lists_retained_rows(), atlas_run_days_lists_distinct_days_newest_first(), dedupe_articles_by_id() (+4 more)

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
Cohesion: 0.06
Nodes (40): apply_provider(), apply_role(), apply_tool_slot(), Credential, CREDENTIAL_FIELDS, CredentialSource, Env, Inline (+32 more)

### Community 185 - "OpenCode V2 workflow"
Cohesion: 0.25
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

### Community 202 - "activate_generation"
Cohesion: 0.21
Nodes (15): activate_generation(), begin_generation(), begin_generation_then_activate(), clear_fingerprint(), fingerprint_round_trips_through_memory_embed_meta(), generation_table_name(), GenerationProgress, migrate_generations() (+7 more)

### Community 203 - "profile_config.rs"
Cohesion: 0.07
Nodes (21): a_directory_target_is_an_actionable_error_not_a_crash(), a_validation_error_reports_a_pointer_and_never_a_value(), an_oversize_document_is_refused_before_anything_is_applied(), char_offset(), ConfigTab, Export, Import, ConfigView (+13 more)

### Community 204 - "Providers"
Cohesion: 0.25
Nodes (8): Budgeted executor, Graph explanation jobs, News and legal keys, OSINT data providers, Providers, Tool picker transport, Typed provider diagnostics, Verification and testing

### Community 205 - "Decision roles and output contracts"
Cohesion: 0.29
Nodes (6): 12-template registry, Adapters, Decision roles and output contracts, State isolation, Thresholds, TUI

### Community 206 - "H0-H2 Completion Summary"
Cohesion: 0.33
Nodes (5): H0-H2 Completion Summary, H0: Resolve Harness and Current Phase ✓, H1: Map Feature Seams ✓, H2: Freeze Contracts and Packets ✓, Next Step: H3 - State and Persistence Foundation (Build Agent)

### Community 207 - "Terminal UI"
Cohesion: 0.22
Nodes (9): Atlas, CLI, Intel, Other apps, Profile, Recon, State, Terminal UI (+1 more)

### Community 209 - "Profile dashboard and search"
Cohesion: 0.12
Nodes (17): Atlas — 6 widgets, Filter strip, Import semantics, Intel — 7 widgets, Metric dictionary, Models — 8 widgets, Named search engines, Profile dashboard and search (+9 more)

### Community 210 - "RunStats"
Cohesion: 0.13
Nodes (13): a_resumed_cycle_continues_candidate_numbering(), cycle_event_id_is_the_run_key_so_replay_never_double_counts(), CycleTelemetry, db(), each_candidate_occurrence_gets_exactly_one_disposition(), OpenStage, origin_snapshots_are_one_row_per_origin_per_cycle(), parse_stats() (+5 more)

### Community 212 - "normalize_destination"
Cohesion: 0.14
Nodes (17): EngineSearchResult, extract_items(), host_in_any(), host_is(), is_ad_host(), is_private_host(), normalize_destination(), ParserInput (+9 more)

### Community 213 - "render_tui_cells.py"
Cohesion: 0.27
Nodes (3): box(), color(), render()

### Community 214 - ".order"
Cohesion: 0.26
Nodes (9): checked_serves(), context_additions(), context_tools(), fallback_record(), OrderContext, Picker<'a>, serves_for(), serving() (+1 more)

### Community 215 - "intel_recon/brain.rs"
Cohesion: 0.57
Nodes (5): intel_recon_publishes_durably_with_provenance_and_no_atlas_receipt(), ReconInsightUpdate, update(), upsert_adds_and_updates_claims(), upsert_recon_insights()

### Community 216 - "Unified Investigation Harness"
Cohesion: 0.33
Nodes (6): Catalog projection, Model roles, Persistence, Reasoning channels, Unified Investigation Harness, Validation gates

### Community 217 - "Json"
Cohesion: 0.20
Nodes (16): json_array_len(), json_bool(), json_str(), complete_stream_and_json_answers_succeed(), delta(), http_errors_carry_status_code_and_redacted_message(), ok_json(), premature_eof_before_and_after_content_is_typed() (+8 more)

### Community 218 - ".evaluate"
Cohesion: 0.17
Nodes (11): DecisionService, DecisionPolicy, EnforcementOutcome, Pass, Reject, Unavailable, Uncertain, ThresholdProfile (+3 more)

### Community 219 - "TurnContinuation"
Cohesion: 0.40
Nodes (4): TurnContinuation, Active, Background, Exhausted

### Community 220 - "Argos OSINT"
Cohesion: 0.33
Nodes (6): Applications, Argos OSINT, Documentation, Figures, Limits, Quick start

### Community 221 - "tool_runner.rs"
Cohesion: 0.07
Nodes (31): serp_tool_outcome(), ToolDefinition, a_synthesized_timeout_is_a_failure_with_a_tool_timeout_reason(), cache_ttl_for(), cancelled_and_cache_modes_are_not_remote_requests(), definition(), every_other_tool_keeps_its_catalog_cache_lifetime(), failure_outcome() (+23 more)

### Community 222 - "EventKind"
Cohesion: 0.10
Nodes (19): EventKind, .ALL, AtlasCandidate, AtlasCycle, AtlasOriginSnapshot, AtlasStage, DirectiveAssessed, EvidenceItem (+11 more)

### Community 223 - "Argos documentation"
Cohesion: 0.40
Nodes (5): Agent and workflow, Argos documentation, Checklists (historical), Figures, Product

### Community 224 - "Diagram conventions"
Cohesion: 0.50
Nodes (4): Agent rule, Catalog, Diagram conventions, When to draw

### Community 225 - "AcceptMode"
Cohesion: 0.67
Nodes (3): AcceptMode, Context, Lead

### Community 226 - "Argos UI Interaction Audit"
Cohesion: 0.40
Nodes (5): Argos UI Interaction Audit, Audit Matrix, Field inventory (`field_placeholder`), Remaining limits, Shared contracts

### Community 228 - "TaskStatus"
Cohesion: 0.14
Nodes (11): TaskStatus, Cancelled, Completed, Deferred, Failed, Partial, Planned, Ready (+3 more)

### Community 229 - "PickRequest"
Cohesion: 0.23
Nodes (8): chat_request(), Ordered, parse_chat_pick(), PickReply, PickRequest, PickRequestAdapter, PickRequestAdapter<'r, 'b>, state()

### Community 230 - "InsightStats"
Cohesion: 0.18
Nodes (12): insight_header(), insight_row(), insight_row_line(), insight_stats_line(), insight_table_lines(), InsightRow, InsightStats, older_run_stats_without_insights_still_parse() (+4 more)

### Community 231 - "brain_lance_off.rs"
Cohesion: 0.15
Nodes (16): begin_generation(), block_on(), clear_fingerprint(), current_fingerprint(), DUPLICATE_THRESHOLD, fingerprint_matches(), GenerationProgress, LAST_ERROR (+8 more)

### Community 232 - "Cell"
Cohesion: 0.16
Nodes (4): Cell, Dims, dims_from_key(), Reader<'a>

### Community 233 - "search_engines_fixtures.rs"
Cohesion: 0.37
Nodes (12): every_fixture_matches_its_recorded_outcome(), field(), fixture_cases(), fixtures_dir(), no_fixture_claimed_a_verified_zero_without_a_supported_phrase(), opt_str(), opt_usize(), read_json() (+4 more)

### Community 237 - "parse_serp_response"
Cohesion: 0.18
Nodes (19): cache_fragment(), parse_serp_response(), a_redirected_final_url_is_not_used_to_rebuild_the_query(), a_single_retry_is_allowed_only_for_a_parser_mismatch_with_a_dom(), an_empty_dom_without_any_payload_is_a_parser_mismatch(), api_http_404_is_an_upstream_failure(), api_http_404_with_a_success_envelope_is_an_upstream_failure(), cache_fragment_carries_both_versions_and_changes_with_query_and_limit() (+11 more)

### Community 238 - "home_rows"
Cohesion: 0.14
Nodes (23): center_row(), composer_parts(), composer_prompt_text(), cursor_blink_visible(), draw_composer(), gap_row(), home_composer_and_all_content_centered_vertically_and_horizontally(), home_composer_areas() (+15 more)

### Community 239 - "atlas_actions.rs"
Cohesion: 0.29
Nodes (4): record_start(), start_repair(), starts(), infer_app()

### Community 240 - "provider_metrics.rs"
Cohesion: 0.24
Nodes (9): cache_capacity_rows(), cached_rows_become_an_available_snapshot(), capacity_snapshot(), CapacityRow, CapacitySnapshot, memory(), missing_companion_snapshot_is_unavailable_not_zero(), QueueRow (+1 more)

### Community 242 - "names_subject"
Cohesion: 0.50
Nodes (4): relevance_gate(), result_mentions(), names_subject(), owned_handle()

### Community 243 - "SerpOutcome"
Cohesion: 0.15
Nodes (11): retry_allowed(), SerpOutcome, Challenge, Consent, ParserMismatch, RateLimited, ResponseTooLarge, UpstreamFailure (+3 more)

### Community 244 - ".recon_outcomes"
Cohesion: 0.20
Nodes (9): normalize_run_outcome(), ReconOutcomeBucket, RunOutcome, Cancelled, CompletedWithEvidence, CompletedZeroEvidence, Failed, Partial (+1 more)

### Community 245 - ".open_job_source"
Cohesion: 0.40
Nodes (4): AtlasRun, JobSource, AtlasRun, ReconThread

### Community 246 - "Trigger"
Cohesion: 0.17
Nodes (11): TELEMETRY_TRIGGER, is_forbidden_key(), payload_number(), Trigger, AtlasCycle, IntelBrief, ManualTool, ReconPrompt (+3 more)

### Community 247 - "ExecuteOptions"
Cohesion: 0.38
Nodes (5): cancellable_sleep(), cancelled(), ExecuteOptions, sleep_recorded(), wait_cancelled()

### Community 248 - "BrainResourceSummary"
Cohesion: 0.20
Nodes (10): bindings_use_brain_evidence_ids(), BrainResourceHit, BrainResourceSummary, candidate_json(), clip(), format_counts(), is_http_url(), parse_brain_scrape_index() (+2 more)

### Community 249 - "Investigation flow"
Cohesion: 0.29
Nodes (7): Binder and executor, Brain recall, Directives, Investigation flow, Pipeline, Synthesis, Tool picker

### Community 250 - "AtlasPage"
Cohesion: 0.67
Nodes (3): AtlasPage, Live, Runs

### Community 253 - "ReportOutcome"
Cohesion: 0.22
Nodes (8): normalize_report_outcome(), ReportOutcome, Blocked, Cancelled, Completed, Failed, Partial, Waiting

### Community 255 - "DispatchError"
Cohesion: 0.25
Nodes (5): DispatchError, RetryDisposition, NextRoute, RetryRoute, Stop

### Community 256 - "run_live"
Cohesion: 0.29
Nodes (6): LiveRun, Fresh, Latest, Run, run_live(), RunInput

### Community 257 - "serde_json"
Cohesion: 0.32
Nodes (5): ADAPTER_VERSION, HOST, ID, positive_negative_and_source_field(), request_url()

### Community 260 - "PlanInterval"
Cohesion: 0.33
Nodes (5): PlanInterval, Daily, Monthly, Never, Weekly

## Knowledge Gaps
- **1801 isolated node(s):** `$schema`, `default_agent`, `subagent_depth`, `timeout`, `chunkTimeout` (+1796 more)
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 2469 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **34 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `App` connect `App` to `app.rs`, `LayoutRegistry`, `ui.rs`, `Store`, `unix_now`, `.handle_key`, `Store`, `Value`, `SettingsFile`, `hardware.rs`, `Store`, `.push_log`, `ProviderSecret`, `brain_detail.rs`, `ReportMode`, `worker.rs`, `.set_focus`, `src/brain.rs`, `Overlay`, `profile_config.rs`, `logs.rs`, `tui/jobs.rs`, `RunStats`, `FeedArticle`, `DefaultsRole`, `AtlasArticleRow`, `ProfileView`, `summary_card.rs`, `ModuleId`, `recon/graph.rs`, `WorkEvent`, `.open_job_source`, `briefing_view.rs`, `AtlasPage`?**
  _High betweenness centrality (0.083) - this node is a cross-community bridge._
- **What connects `$schema`, `default_agent`, `subagent_depth` to the rest of the system?**
  _1801 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `land.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.0076045627376425855 - nodes in this community are weakly interconnected._
- **Why does `Value` connect `Value` to `orchestrate.rs`, `Binding`, `atlas_memory.rs`, `tool_io.rs`, `ui.rs`, `recon.rs`, `TelemetryEvent`, `Store`, `IntelligenceCategory`, `Store`, `wikipedia_rsp.rs`, `directives.rs`, `DecisionState`, `.default`, `providers.rs`, `atlas_news.rs`, `.is_empty`, `execute_steps`, `telemetry.rs`, `whoxy.rs`, `news_legal.rs`, `SettingsFile`, `osint.rs`, `holehe/mod.rs`, `provider.rs`, `body.rs`, `ProviderSecret`, `.new`, `provider_attempt.rs`, `picker.rs`, `ReportMode`, `.run_configured`, `grok_oauth.rs`, `events.rs`, `model_exec.rs`, `config_transfer.rs`, `ConfigIssue`, `cli.rs`, `atlas_insights.rs`, `provider_diag.rs`, `body_filter.rs`, `synthesize.rs`, `summarization.rs`, `serde`, `search_engines.rs`, `.new`, `RunStats`, `src/evidence.rs`, `ToolRunner`, `anyhow`, `AtlasArticleRow`, `AtlasEvent`, `dork_generator.rs`, `PickRequest`, `search_engines_fixtures.rs`, `search_engines/tests.rs`, `Supplied<T>`, `run_turn`, `parse_serp_response`, `names_subject`, `ServiceResult`, `subscription.rs`, `BrainResourceSummary`, `config_transfer_tests.rs`, `Service`?**
  _High betweenness centrality (0.061) - this node is a cross-community bridge._
- **Should `orchestrate.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.038585858585858585 - nodes in this community are weakly interconnected._
- **Why does `FieldId` connect `FieldId` to `app.rs`, `split_vertical`, `DefaultsRole`, `App`, `Rect`, `.set_focus`?**
  _High betweenness centrality (0.037) - this node is a cross-community bridge._
- **Should `Binding` be split into smaller, more focused modules?**
  _Cohesion score 0.14193548387096774 - nodes in this community are weakly interconnected._