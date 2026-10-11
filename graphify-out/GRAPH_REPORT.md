# Graph Report - argos-osint  (2026-10-11)

## Corpus Check
- 261 files · ~594,329 words
- Verdict: corpus is large enough that graph structure adds value.
- Unclassified: 4 file(s) not represented in the graph (top: .toml 2, (none) 2)

## Summary
- 8576 nodes · 21409 edges · 278 communities (238 shown, 40 thin omitted)
- Extraction: 98% EXTRACTED · 2% INFERRED · 0% AMBIGUOUS · INFERRED: 387 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `fca363af`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- land.rs
- orchestrate.rs
- telemetry.rs
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
- execute_steps
- directives.rs
- article_image.rs
- Binding
- Value
- atlas_news.rs
- Store
- publication.rs
- TaskStatus
- whoxy.rs
- news_legal.rs
- hardware.rs
- App
- Store
- embed.rs
- osint.rs
- holehe/mod.rs
- config_commit.rs
- .activate_target
- TelemetryEvent
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
- source_sha256
- atomic
- tui/jobs.rs
- App
- events.rs
- exec.rs
- contains
- model_exec.rs
- ErrorCategory
- JobRow
- MemoryKind
- atlas_table.rs
- brain_resources.rs
- worker.rs
- JobHandle
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
- theme.rs
- Overlay
- pipeline.rs
- summarization.rs
- .new_thread
- .clear
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
- anyhow
- AtlasArticleRow
- BrainResourceSummary
- dork_generator.rs
- Section
- How
- LogsView
- summary_card.rs
- explore.rs
- SettingsFile
- Region
- diversity.rs
- Plan
- model_roles.rs
- Frame
- search_engines/tests.rs
- profile_config.rs
- AtlasEvent
- Architecture
- WorkEvent
- store.rs
- ClockSet
- ProviderAdmission
- components.rs
- ParserInput
- release_check.mjs
- ServiceResult
- briefing_view.rs
- subscription.rs
- profile.rs
- TurnEvent
- recon/graph.rs
- tasks.rs
- ReconCommand
- config_transfer_tests.rs
- run
- ProviderSecret
- ServiceSpec
- TurnClock
- opencode.json
- provider_diag.rs
- ProfileView
- ConfigView
- ToolResult
- DateTime
- OsintCommand
- source_sha256
- brain_lance_off.rs
- Argos OSINT — Agent Instructions
- modes.rs
- investigation.rs
- wikipedia_rsp.rs
- intel-brief-image-column-2026-10-11/manifest.json
- serde_json
- Category
- tracked.rs
- Rect
- QueryExecutionStatus
- results.rs
- gates.rs
- ToolOutcome
- WidgetSpec
- App
- tui_review.py
- .new
- investigation/evidence.rs
- serde
- Part
- argos-osint-bin
- ImportBuffer
- .new
- SiteOutcomeStatus
- .new
- CancelRequest
- paths.rs
- ModuleId
- atlas_work.rs
- rusqlite
- ModelAssignment
- ReconLimits
- Finish
- atlas_answer_id
- ui.rs
- RecoveryAction
- grok_oauth.rs
- LaunchState
- Conventions
- Argos UI Interaction Audit
- ScheduledCall
- config_transfer.rs
- OpenCode V2 workflow
- Call
- Concepts
- package.json
- IntelPage
- TUI components
- engine.rs
- Providers
- Decision roles and output contracts
- LayoutResult
- Terminal UI
- docs/README.md
- Profile dashboard and search
- RunStats
- BrainIndex
- Changelog
- .field
- .order
- super
- Unified Investigation Harness
- RendererKind
- TurnContinuation
- Argos OSINT
- EventKind
- Argos documentation
- ChainReport<T>
- OriginStat
- Json
- IndexOutcome
- model_facts
- profile_components.rs
- manifest.json
- StatFilters
- parse_serp_response
- fetch_once
- provider_metrics.rs
- Trigger
- .recon_outcomes
- BrainListMode
- replace_insights.rs
- search_engines_fixtures.rs
- PickReply
- graph_explanation/tests.rs
- SystemTab
- .intel_reports
- §9 implementation order
- InvestigationSurface
- Item
- actor_review.rs
- run_live
- tool_runner.rs
- result_cache
- Target
- .open_job_source
- OperationScope<'a>
- Profile analytics dashboard
- CheckSignal
- atlas_actions.rs
- Profile TUI implementation session — 2026-10-10
- Diagram conventions
- AtlasPage
- TUI implementation and verification
- Unified reliability / summarization / semantic pipelines — completion checklist
- poll_device
- Semantic-versioning setup acceptance
- TUI overhaul verification — 10 October 2026
- Versioning, commits and releases
- pull_request_template.md
- release-please-config.json
- intel-brief-image-column-2026-10-11/README.md

## God Nodes (most connected - your core abstractions)
1. `App` - 323 edges
2. `Value` - 286 edges
3. `source_sha256` - 222 edges
4. `source_sha256` - 222 edges
5. `ProviderSecret` - 143 edges
6. `ButtonId` - 132 edges
7. `Target` - 89 edges
8. `ToolResult` - 78 edges
9. `FieldId` - 76 edges
10. `Store` - 74 edges

## Surprising Connections (you probably didn't know these)
- `Module layout` --references--> `widget_lines()`  [EXTRACTED]
  docs/architecture.md → crates/argos-osint-bin/src/tui/profile.rs
- `Other application presets` --references--> `draw_system_tab()`  [EXTRACTED]
  docs/tui-design-spec.md → crates/argos-osint-bin/src/tui/profile.rs
- `Profile edit map` --references--> `LayoutResult`  [EXTRACTED]
  docs/tui-design-spec.md → crates/argos-osint-bin/src/tui/profile_layout.rs
- `Other application presets` --references--> `home_layout_metrics()`  [EXTRACTED]
  docs/tui-design-spec.md → crates/argos-osint-bin/src/tui/ui.rs
- `Other application presets` --references--> `intel_briefing_areas()`  [EXTRACTED]
  docs/tui-design-spec.md → crates/argos-osint-bin/src/tui/ui.rs

## Import Cycles
- 1-file cycle: `crates/argos-osint-core/src/config_transfer.rs -> crates/argos-osint-core/src/config_transfer.rs`
- 2-file cycle: `crates/argos-osint-core/src/osint.rs -> crates/argos-osint-core/src/osint/results.rs -> crates/argos-osint-core/src/osint.rs`
- 2-file cycle: `crates/argos-osint-bin/src/cli.rs -> crates/argos-osint-core/src/hardware.rs -> crates/argos-osint-bin/src/cli.rs`
- 4-file cycle: `crates/argos-osint-core/src/config_commit.rs -> crates/argos-osint-core/src/provider_attempt.rs -> crates/argos-osint-core/src/provider_diag.rs -> crates/argos-osint-core/src/config_transfer.rs -> crates/argos-osint-core/src/config_commit.rs`

## Communities (278 total, 40 thin omitted)

### Community 0 - "land.rs"
Cohesion: 0.01
Nodes (262): CODES, LABELS, R0, R1, R10, R100, R101, R102 (+254 more)

### Community 1 - "orchestrate.rs"
Cohesion: 0.05
Nodes (120): a_429_stops_picker_calls_and_the_fallback_finishes_the_list(), a_claimed_email_removes_its_bindings(), a_dispatch_error_fails_the_step_and_the_loop_goes_on(), a_dropped_provider_stream_keeps_the_text_already_received(), a_failed_email_finder_gets_one_fallback_and_no_third_picker_call(), a_fallback_with_no_bindings_still_runs_the_satisfied_steps_and_reaches_synthesis(), a_gap_filler_domain_feeds_hunter_once_firecrawl_observes_it(), a_goal_naming_a_tool_or_provider_is_repaired_then_falls_back() (+112 more)

### Community 2 - "telemetry.rs"
Cohesion: 0.12
Nodes (21): bin_index(), Coverage, duration_bins_are_bounded_and_mergeable(), DURATION_BINS_MS, ensure_observed_since(), event_bins(), get_meta(), ID_SEQ (+13 more)

### Community 3 - "atlas_memory.rs"
Cohesion: 0.05
Nodes (98): atlas_memory_count(), background_indexing_then_refresh_upgrades_a_partial_run(), Barrier, Checkpoint, CheckpointInput, claim(), claims(), clear() (+90 more)

### Community 4 - "tool_io.rs"
Cohesion: 0.03
Nodes (97): leftovers(), names(), PROMPT_TARGETS, normalize_platform(), question_bindings(), a_gap_filler_only_domain_waits_for_a_primary_observation_before_hunter(), accept_bindings(), batch_scrape_takes_ranked_urls_and_gates_order_before_hunter() (+89 more)

### Community 5 - "app.rs"
Cohesion: 0.05
Nodes (109): a_finished_tool_call_is_logged_once_and_the_transcript_keeps_a_summary(), all_provider_actions_render_with_hit_areas_at_80x24(), analytics_fixture(), analytics_viewports_and_data_states(), ATLAS_AUTO_SECS, atlas_auto_toggle_persists_the_next_trigger(), atlas_claim(), atlas_headlines_scroll_and_request_failures_reach_the_system_log() (+101 more)

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
Nodes (65): ac6_citation_groups_split_validate_each_id_and_normalize(), answer_step_rate_limit_is_explicit_and_keeps_results(), attribute_word(), auxiliary(), await_completion(), BRIEF_SYNTHESIS, broad_question_collapses_to_grounded_lookups(), cacheable() (+57 more)

### Community 10 - "ButtonId"
Cohesion: 0.02
Nodes (108): ButtonId, Add, AddFallback, AtlasAuto, AtlasDelete, AtlasLive, AtlasNews, AtlasNewsFeed (+100 more)

### Community 11 - "Store"
Cohesion: 0.07
Nodes (5): CreditHold, Store, strategy_and_provider_credits_survive_reopen(), Thread, whoxy_prepaid_pool_does_not_reset_monthly()

### Community 12 - "IntelligenceCategory"
Cohesion: 0.09
Nodes (23): all_catalog_tools_mapped_to_categories(), argument_contract(), ArgumentBuilderContract, compact_capability_catalog(), CompactToolCapability, IntelligenceCategory, Bitcoin, DomainNetwork (+15 more)

### Community 13 - "job_registry.rs"
Cohesion: 0.18
Nodes (16): BEAT_INTERVAL, begin_optional(), cancel_is_cooperative_and_only_for_cancellable_running_jobs(), cross_process_cancel_reaches_the_owner_through_its_heartbeat(), db(), dropped_or_panicking_work_reaches_a_failed_state(), jobs_of_exited_processes_become_interrupted_and_live_ones_stay(), JobSpec (+8 more)

### Community 14 - "execute_steps"
Cohesion: 0.08
Nodes (48): a_follow_up_keeps_names_from_the_previous_synthesis(), after_step(), binding_ground(), call_cached(), context_block(), context_dispatched(), derive_directives(), Derived (+40 more)

### Community 15 - "directives.rs"
Cohesion: 0.06
Nodes (66): paint_logo(), accounts_search_query(), apply_context_targets(), asks(), asks_about_judge(), asks_for_headlines(), clip_words(), context_entity() (+58 more)

### Community 16 - "article_image.rs"
Cohesion: 0.09
Nodes (22): absent_invalid_failed_images_collapse(), ArticleImages, centered_image_rect(), Completion, Decoded, Encoded, decode(), decode_preserves_source_resolution_and_pixels() (+14 more)

### Community 17 - "Binding"
Cohesion: 0.14
Nodes (29): Binding, canonical(), dependency_order(), depends_on(), question_bindings_and_dependency_fix(), allowed_producer(), best_handle(), bind_step() (+21 more)

### Community 18 - "Value"
Cohesion: 0.04
Nodes (88): Value, annotate(), country_arg(), optional_text(), query_arg(), request(), clip_page(), https_on_host() (+80 more)

### Community 19 - "atlas_news.rs"
Cohesion: 0.07
Nodes (40): api_error(), ATLAS_TOOLS, authors(), clip(), country_code(), CURRENTS_COUNTRIES, CURRENTS_HOST, CURRENTS_LATEST (+32 more)

### Community 20 - "Store"
Cohesion: 0.06
Nodes (24): article_body_from_row(), ArticleBodyRow, cited_evidence_keeps_provenance_and_revision_identity(), full_assessment_parents_are_flagged_for_child_report_totals(), IntelAssessmentRow, IntelInvestigationRow, IntelReportSectionRow, IntelReportTaskRow (+16 more)

### Community 21 - "publication.rs"
Cohesion: 0.07
Nodes (43): Phase5Report, active_index_rows(), bump_memories_changed(), change_watcher_sees_only_committed_changes_and_coalesces(), claim(), clear(), clear_index_state(), CoverageReport (+35 more)

### Community 22 - "TaskStatus"
Cohesion: 0.07
Nodes (15): TaskRecord, TaskStatus, Cancelled, Completed, Deferred, Failed, Partial, Planned (+7 more)

### Community 23 - "whoxy.rs"
Cohesion: 0.11
Nodes (35): adjacent_changes(), AdjacentChange, balance_request_url(), bounded_model_view(), check_balance(), contact(), ContactCard, date_and_limit_validation() (+27 more)

### Community 24 - "news_legal.rs"
Cohesion: 0.06
Nodes (48): a_rate_limit_switches_to_the_fallback_key_and_a_rejection_does_not(), ac1_catalog_has_55_tools_and_the_news_and_legal_entries(), ac1_each_request_has_its_url_params_and_auth_header_and_no_key_in_the_url(), an_echoed_key_is_redacted_from_the_stored_body_and_error(), article(), articles(), CATEGORIES, choice_arg() (+40 more)

### Community 25 - "hardware.rs"
Cohesion: 0.14
Nodes (18): system_host_lines(), CACHE_TTL_SECS, classify_arch(), disk_totals(), HardwareProfile, metal_vram_gb(), now_secs(), parse_apple_gpu_cores() (+10 more)

### Community 26 - "App"
Cohesion: 0.08
Nodes (64): api_key_slot(), atlas_cycle_stats_room(), atlas_cycle_stats_scroll_max(), atlas_feed_room(), atlas_feed_room_for(), atlas_live_areas(), atlas_news_areas(), atlas_news_room() (+56 more)

### Community 27 - "Store"
Cohesion: 0.06
Nodes (9): atlas_brief_id(), GraphSummary, has_table(), intel_link_explanations_round_trip_and_cleanup(), profile_reader_cannot_write_or_create_a_database(), ReindexReport, repair_embed_tables(), Store (+1 more)

### Community 28 - "embed.rs"
Cohesion: 0.08
Nodes (31): active(), DIM, disable(), disabled(), DisableGuard, download_file(), DOWNLOAD_TIMEOUT_SECS, embed_batch() (+23 more)

### Community 29 - "osint.rs"
Cohesion: 0.04
Nodes (103): test_all_catalog_tools_mapped_to_categories(), a_claimed_email_keeps_no_person_data(), bind_request(), bitcoin(), body_cap(), bounded(), bounded_recovery_is_offered_once_for_an_unrecognised_page_only(), CACHE_DAY_SECONDS (+95 more)

### Community 30 - "holehe/mod.rs"
Cohesion: 0.10
Nodes (21): CACHE_TTL, cap_reports_omitted(), DEFAULT_MAX_SITES, default_selection_uses_implemented_adapters(), email_hash(), email_preserves_local_part(), GLOBAL_CONCURRENCY, lookup() (+13 more)

### Community 31 - "config_commit.rs"
Cohesion: 0.06
Nodes (73): a_batch_with_a_duplicate_or_empty_slot_is_refused_before_any_write(), a_committed_journal_needs_no_recovery(), a_second_lock_is_refused_while_one_is_held(), a_stale_lock_is_taken_over(), an_abandoned_unfinished_lockfile_is_recovered(), backup_path(), canonical_root(), commit_files_is_all_or_nothing() (+65 more)

### Community 32 - ".activate_target"
Cohesion: 0.10
Nodes (8): keyboard_navigation_esc_and_shortcuts(), overhaul_tool_category_collapse_preserves_identity_and_search_state(), AtlasArticle, AtlasFeed, AtlasHistory, IntelArticle, ProfileTab, ToolCategory

### Community 33 - "TelemetryEvent"
Cohesion: 0.13
Nodes (10): clamp(), coverage_reports_observed_since_and_counts(), db(), events_persist_and_roll_up_once(), id_is_stable_so_replay_never_double_counts(), logical_attempt_and_cache_rows_stay_distinct_under_replay(), record(), safe_payload() (+2 more)

### Community 34 - "InvestigationPart"
Cohesion: 0.10
Nodes (17): event_to_chat_block(), is_thinking_expanded(), InvestigationPart, DirectiveAssessment, EvidencePassage, GateValidation, Handoff, Plan (+9 more)

### Community 35 - "body.rs"
Cohesion: 0.09
Nodes (34): body_fetch_routes(), BodyFetchEvent, Attempt, Failed, InsightsFailed, InsightsRefreshing, InsightsReplaced, Progress (+26 more)

### Community 36 - "provider.rs"
Cohesion: 0.04
Nodes (79): a_blank_osint_user_agent_loads_as_unset(), chat_body(), ChatMessage, complete(), complete_errors_with_finish_reason_when_response_is_empty(), complete_once(), complete_one(), complete_reads_reasoning_only_non_stream_json() (+71 more)

### Community 37 - "brain_detail.rs"
Cohesion: 0.06
Nodes (50): areas(), areas_for(), BrainDetail, DetailAreas, DetailPane, Path, Related, Summary (+42 more)

### Community 38 - "profile_stats.rs"
Cohesion: 0.05
Nodes (63): add_cycle_outcome(), atlas_carries_data(), AtlasStats, BacklogRow, band_for(), CategoryTrigger, classify_tool_cause(), CONFIDENCE_BANDS (+55 more)

### Community 39 - "atlas.rs"
Cohesion: 0.09
Nodes (30): article_card_labels_title_publisher_author_and_classification(), Band, band_sizes(), band_temperature(), category_from_decisions(), CATEGORY_IDS, CLUSTERS, country_label() (+22 more)

### Community 40 - "run_atlas_inner"
Cohesion: 0.09
Nodes (32): article_from_row(), Stats, Status, AtlasJob, charge_newsapi(), charge_quota(), Cursor, cursor_at() (+24 more)

### Community 41 - "whatsmyname.rs"
Cohesion: 0.05
Nodes (46): DatasetStatus, status(), AccountTuple, ACTIVE_SNAPSHOT, ADAPTER_VERSION, apply_strip_bad_char(), benchmark_14_4_parse_index_and_selection(), cache_get() (+38 more)

### Community 42 - "scheduler.rs"
Cohesion: 0.09
Nodes (22): apply_index_change(), apply_index_change_rebuild_reports_outcome(), apply_index_with(), DEFAULT_LEASE_SECS, drain_index_once(), drain_summary_flush(), drain_summary_flush_cached(), drain_summary_flush_completes_cached_tasks() (+14 more)

### Community 43 - "jobs_view.rs"
Cohesion: 0.11
Nodes (21): summary_text(), active_time_is_live_for_open_attempts_and_unavailable_only_without_a_start(), AttemptRow, event_apps(), event_counts(), EventCounts, get_job(), job() (+13 more)

### Community 44 - "provider_attempt.rs"
Cohesion: 0.14
Nodes (25): Acc, attempt(), attempt_with_observer(), AttemptReport, check_final(), Deadlines, fill(), is_subscription() (+17 more)

### Community 45 - "LogicalRole"
Cohesion: 0.13
Nodes (11): LogicalRole, .ALL, ClaimAssessor, Classifier, Controller, EntityResolver, EvidenceCurator, Planner (+3 more)

### Community 46 - "picker.rs"
Cohesion: 0.12
Nodes (26): brain_scrape_options_hide_bare_scrape_and_include_claim_criteria(), CatalogEntry, chat_request(), CONFIDENCE_FLOOR, decisions_request(), directives_phrase(), DONE, eligible_catalog() (+18 more)

### Community 47 - "ReportMode"
Cohesion: 0.12
Nodes (23): HomeDraftState, brief_rating_reuses_the_existing_mean_semantics(), classifiable_modes(), classify_mode_chat(), classify_mode_decisions(), classify_prompt_mode(), classify_prompt_mode_chat(), classify_prompt_mode_decisions() (+15 more)

### Community 48 - "source_sha256"
Cohesion: 0.01
Nodes (222): source_sha256, crates/argos-osint-bin/Cargo.toml, crates/argos-osint-bin/src/cli.rs, crates/argos-osint-bin/src/main.rs, crates/argos-osint-bin/src/tui/app.rs, crates/argos-osint-bin/src/tui/article_image.rs, crates/argos-osint-bin/src/tui/atlas_actions.rs, crates/argos-osint-bin/src/tui/atlas_table.rs (+214 more)

### Community 49 - "atomic"
Cohesion: 0.08
Nodes (15): AttemptOutcome, categorize_http(), execute_with_retries(), other_llm_allows_three(), summarization_stops_at_two_attempts(), TimeoutProfile, .CLASSIFIER, .EXTRACTION (+7 more)

### Community 50 - "tui/jobs.rs"
Cohesion: 0.08
Nodes (23): areas(), BASE_COLUMNS, button_label(), buttons(), detail_lines(), hit(), JobsAreas, JobsView (+15 more)

### Community 51 - "App"
Cohesion: 0.03
Nodes (17): a_click_folds_an_event_and_incoming_entries_do_not_move_a_paused_list(), a_failed_turn_keeps_the_streamed_answer_on_screen(), a_log_or_late_tool_row_does_not_drop_the_live_answer(), add_scroll(), App, atlas_log_level(), auto_run_completion_selects_the_latest_history_row(), LastViewSession (+9 more)

### Community 52 - "events.rs"
Cohesion: 0.08
Nodes (27): clear_events(), DEFAULT_RETENTION_HOURS, event_row(), EventFilter, EventRow, get_event(), job_filter_includes_descendants_and_prune_keeps_jobs(), list_events() (+19 more)

### Community 53 - "exec.rs"
Cohesion: 0.14
Nodes (19): admission_account(), admission_contention_consumes_no_attempt(), AttemptEvent, Finished, Started, AttemptLog, cancelled(), complete_summary_report() (+11 more)

### Community 54 - "contains"
Cohesion: 0.09
Nodes (45): abs_contains(), abs_rect(), AbsRect, contains(), draw_centered_loading_card(), draw_clipped_button(), draw_clipped_intel_loading(), draw_clipped_md_pane() (+37 more)

### Community 55 - "model_exec.rs"
Cohesion: 0.05
Nodes (65): AttemptRecord, cancellable_sleep(), cancellation_stops_the_chain(), cancelled(), ChainReport, DispatchError, execute(), ExecuteOptions (+57 more)

### Community 56 - "ErrorCategory"
Cohesion: 0.05
Nodes (33): operation_kind(), backoff_delay(), can_retry(), ErrorCategory, AuthOrQuota, CancelledOrStale, ConfigurationMissing, ContextLimit (+25 more)

### Community 57 - "JobRow"
Cohesion: 0.12
Nodes (11): job_row(), JobFilter, JobRow, JobStatusFilter, Active, All, Completed, Failed (+3 more)

### Community 58 - "MemoryKind"
Cohesion: 0.09
Nodes (22): AdmissionPolicy, admit_memory(), BrainQuery, pack_context(), AssessmentState, Contradicts, Insufficient, Mentions (+14 more)

### Community 59 - "atlas_table.rs"
Cohesion: 0.17
Nodes (22): border_style(), borders_and_cells_have_matching_display_widths(), bottom_border_line(), char_width(), clip_to_width(), column_widths(), columns_consume_inner_width(), columns_distribute_proportionally_when_not_flex_last() (+14 more)

### Community 60 - "brain_resources.rs"
Cohesion: 0.09
Nodes (26): source_anchor_label(), BRAIN_SCRAPE_PREFIX, CLAIM_CHARS, classify_resource(), file_link_url(), hit(), is_brain_binding(), is_brain_scrape_pick() (+18 more)

### Community 61 - "worker.rs"
Cohesion: 0.15
Nodes (20): IntelReportEvent, InsightsUpdated, JobCreated, JobDone, Section, Stage, IntelReportJobRow, cancel_if_needed() (+12 more)

### Community 62 - "JobHandle"
Cohesion: 0.13
Nodes (10): beat(), beats(), begin_optional_with(), ensure_beat(), JobHandle, live(), now(), ProcessBeat (+2 more)

### Community 63 - "src/brain.rs"
Cohesion: 0.12
Nodes (21): AGREEMENT_WEIGHT, CATEGORIES, category_hint(), format_injection(), hybrid_recall(), hybrid_recall_adds_paraphrases_and_rewards_agreement(), identity_query_prefers_name_memory(), jaccard() (+13 more)

### Community 64 - "intel_recon/jobs.rs"
Cohesion: 0.08
Nodes (33): active_job_for_mode(), cancel_job(), create_job_inserts_all_verify_sections(), create_report_job(), finish_canonical(), intel_recon_links_its_legacy_job_to_one_canonical_registry_job(), pause_job(), register_canonical() (+25 more)

### Community 65 - "cli.rs"
Cohesion: 0.15
Nodes (21): ask(), ask_thread(), atlas_command(), defaults_command(), defaults_json(), defaults_show_includes_the_tool_picker_transport(), DefaultsCommand, Set (+13 more)

### Community 66 - "atlas_insights.rs"
Cohesion: 0.06
Nodes (53): a_country_token_does_not_merge_into_a_longer_name(), a_description_only_span_is_inference_and_an_added_name_is_dropped(), a_fact_requires_both_spans_in_the_title(), accept_claims(), AcceptMode, Context, Lead, aliases_merge_on_shared_tokens_when_the_longer_form_is_a_span() (+45 more)

### Community 67 - "SourceReliability"
Cohesion: 0.09
Nodes (22): article_information_credibility(), AdmiraltyCode, best_credibility(), CredibilityInputs, information_credibility(), InformationCredibility, CannotBeJudged, Confirmed (+14 more)

### Community 68 - ".new"
Cohesion: 0.20
Nodes (28): a_snapshot_never_panics_on_a_partially_migrated_store(), active_jobs(), an_empty_database_returns_empty_sections_not_zeros(), atlas_raw_candidate_occurrences_are_not_added_twice(), count_rows(), distinct_article_body_cohort_deduplicates_tags_and_excludes_empty_cache_rows(), distinct_values(), durable_model_rows_and_supplemental_telemetry_count_once() (+20 more)

### Community 69 - "body_filter.rs"
Cohesion: 0.18
Nodes (22): BATCH_SIZE, BodyChunk, chunk_article_body(), CHUNK_MAX, CHUNK_MIN, CHUNK_TARGET, chunks_preserve_absolute_offsets(), classify_chunk_batch() (+14 more)

### Community 70 - "synthesize.rs"
Cohesion: 0.16
Nodes (20): IntelEvidenceRow, BODY_REFINE_INPUT_CHARS, deterministic_bluf_mentions_title(), deterministic_section(), model_refine_body(), model_synthesize(), parse_json_object(), refine_retrieved_article_body() (+12 more)

### Community 71 - "dataset.rs"
Cohesion: 0.15
Nodes (21): acquire_refresh_lease(), active_manifest_path(), ACTIVE_REFRESHES, dataset_root(), DatasetManifest, get_status(), import_from_file(), load_active_manifest() (+13 more)

### Community 72 - "Loaded"
Cohesion: 0.14
Nodes (18): atlas_origin_rows(), category_triggers(), Dims, dominant_label(), engine_health(), EventRow, evidence_call(), evidence_cited() (+10 more)

### Community 73 - "profile_charts.rs"
Cohesion: 0.06
Nodes (61): absolute_stacks_compare_one_and_one_hundred_on_one_scale(), BLOCKS, cap_eighths(), cells(), coarsen_counts(), column_cell(), connect_cells(), count_coarsening_preserves_totals_boundaries_and_unknown_history() (+53 more)

### Community 74 - "theme.rs"
Cohesion: 0.13
Nodes (24): Block, ACCENT, BG, BORDER, card(), card_accent(), card_dim(), card_text() (+16 more)

### Community 75 - "Overlay"
Cohesion: 0.14
Nodes (14): ChoiceKind, IntelDay, Investigation, Model, Provider, Overlay, AddFallback, Choice (+6 more)

### Community 76 - "pipeline.rs"
Cohesion: 0.11
Nodes (26): apply_recon_directive_coverage(), ClaimRelation, AmbiguousEntity, ChangedQuantity, Equivalent, Negation, PlannedVsCompleted, Unrelated (+18 more)

### Community 77 - "summarization.rs"
Cohesion: 0.10
Nodes (38): cache_get(), cache_key_changes_with_revision_and_focus(), cache_put(), cache_round_trip(), complete_summary(), CoverageMeta, deterministic_article_description(), deterministic_atlas_brief() (+30 more)

### Community 78 - ".new_thread"
Cohesion: 0.18
Nodes (13): answer_evidence_survives_reopen_and_deleted_thread_rejects_late_calls(), claims_deduplicate_and_reject_unsupported_sources(), deleting_an_investigation_removes_its_brain_memories_and_graph_summary(), investigation_memory_ids(), Message, now(), only_a_failed_or_interrupted_latest_run_is_resumable(), persist_claims() (+5 more)

### Community 79 - ".clear"
Cohesion: 0.09
Nodes (12): draft_isolation_and_persistence(), duplicate_submission_prevention(), home_order_renames_and_nine_routes_agree(), overhaul_palette_recency_and_disabled_dispatch(), PaletteItem, pruned_history_closes_the_open_run(), recon_context_hides_on_narrow_screen_without_changing_preference(), session_tabs_open_close_reopen() (+4 more)

### Community 80 - "search_engines.rs"
Cohesion: 0.06
Nodes (55): Candidate, card_snippet(), classify_status_region(), clip_diag(), clip_item_text(), collapse_ws(), collect_candidates(), contains_phrase() (+47 more)

### Community 81 - "logs.rs"
Cohesion: 0.17
Nodes (18): starts(), stamp(), areas(), button_label(), buttons(), count(), draw(), hit() (+10 more)

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
Cohesion: 0.18
Nodes (16): BLOCK_MARKERS, BodyQuality, Complete, Partial, Unavailable, Uncertain, BodyValidation, clean_markdown() (+8 more)

### Community 86 - "Command"
Cohesion: 0.10
Nodes (20): AtlasCommand, Repair, Resume, Verify, Cli, Command, Atlas, Defaults (+12 more)

### Community 87 - "DefaultsRole"
Cohesion: 0.07
Nodes (18): ChoiceItem, codex_models(), DefaultsRole, .ALL, ClaimAssessor, Classifier, EntityResolver, EvidenceCurator (+10 more)

### Community 88 - "graph_explanation.rs"
Cohesion: 0.13
Nodes (13): BASIC_HEADING, Cached, None, Stale, Valid, FAILURE_COOLDOWN, Gate, CoolingDown (+5 more)

### Community 89 - "src/evidence.rs"
Cohesion: 0.09
Nodes (29): AGREEMENT_WEIGHT, AnnMeasurement, AnnPolicy, AnnDeferredUnmeasured, AnnEnabled, ExactSearch, AnnThresholds, chunk_text() (+21 more)

### Community 90 - "ToolRunner"
Cohesion: 0.11
Nodes (13): attribution_is_explicit_never_inferred_from_the_prompt(), failure_reason(), named_engine(), osint_cacheable(), test_tool_runner_cache_hit(), test_tool_runner_concurrency_and_dedup(), ToolAttribution, ToolRunner (+5 more)

### Community 91 - "anyhow"
Cohesion: 0.08
Nodes (38): compile_general_model_prompt(), compile_native(), DecisionAdapterKind, JsonMode, NativeDecisions, OutputTool, StrictJsonSchema, ValidatedText (+30 more)

### Community 92 - "AtlasArticleRow"
Cohesion: 0.11
Nodes (45): accept_one(), apply_admiralty_evaluation(), apply_peer_support(), articles_have_span(), ask_claims(), brief_text(), cap_claims(), catalog_json() (+37 more)

### Community 93 - "BrainResourceSummary"
Cohesion: 0.20
Nodes (10): bindings_use_brain_evidence_ids(), BrainResourceHit, BrainResourceSummary, candidate_json(), clip(), format_counts(), is_http_url(), parse_brain_scrape_index() (+2 more)

### Community 94 - "dork_generator.rs"
Cohesion: 0.10
Nodes (31): ACTIVE_SNAPSHOT, compose_single_query(), compute_query_id(), compute_template_id(), DATASET_NAME, DEFAULT_MAX_QUERIES, DorkCatalog, DorkCategory (+23 more)

### Community 95 - "Section"
Cohesion: 0.18
Nodes (8): every_section_registers_its_reviewed_widget_count(), Section, Atlas, Intel, Models, Recon, Tools, widgets_render_in_the_specs_presentation_priority_order()

### Community 96 - "How"
Cohesion: 0.11
Nodes (18): How, CompanyEmail, Coordinates, DomainAsUrl, EntityPhrase, PackageName, PackageParts, Plain (+10 more)

### Community 97 - "LogsView"
Cohesion: 0.11
Nodes (8): LevelFilter, All, Error, Info, Warn, LogsView, row_lines(), detail_rows()

### Community 98 - "summary_card.rs"
Cohesion: 0.32
Nodes (11): actions(), button_at(), button_cells(), button_row_area(), draw(), height(), is_card_button(), label() (+3 more)

### Community 99 - "explore.rs"
Cohesion: 0.20
Nodes (11): GraphEdgeKind, ContradictionOrUpdate, Evidenced, Inferred, SemanticSuggestion, related_evidence_view(), RelatedEvidenceHit, semantic_edges_are_not_factual() (+3 more)

### Community 100 - "SettingsFile"
Cohesion: 0.09
Nodes (14): commit_profile_config(), ConfigChange, ConfigurationSnapshot, describe_changes(), import_from_path(), ImportPlan, merge_key(), QuotaSettingsFile (+6 more)

### Community 101 - "Region"
Cohesion: 0.09
Nodes (22): atlas_hit(), atlas_row_at(), Region, AtlasFeed, AtlasInsights, AtlasNews, AtlasOrigins, AtlasRuns (+14 more)

### Community 102 - "diversity.rs"
Cohesion: 0.17
Nodes (24): coverage_gaps_summary(), coverage_record_id(), coverage_stats(), CoverageCandidate, CoverageRecord, CoverageStats, covered_record(), credentials_available() (+16 more)

### Community 103 - "Plan"
Cohesion: 0.12
Nodes (18): cut_footer(), cut_short_answer(), decode_plan(), Directive, EntityView, evidence_summary(), Grounding, HypothesisView (+10 more)

### Community 104 - "model_roles.rs"
Cohesion: 0.15
Nodes (13): AccountHealth, AuthRequired, Cooldown, CreditsExhausted, Overloaded, Ready, Restricted, Unverified (+5 more)

### Community 105 - "Frame"
Cohesion: 0.11
Nodes (57): draw(), active_popup_area(), atlas_extracting(), atlas_run_card(), button_areas(), choice_list_room(), configs_area(), cover() (+49 more)

### Community 106 - "search_engines/tests.rs"
Cohesion: 0.10
Nodes (43): build_scrape_body(), build_serp_url(), a_non_serp_path_on_the_engine_host_is_not_a_serp_page(), a_noscript_wrapped_consent_meta_refresh_is_consent(), a_standalone_zero_count_in_a_status_region_is_a_verified_zero(), a_zero_phrase_outside_a_status_region_is_not_a_verified_zero(), an_off_host_final_url_with_cards_keeps_the_items_but_is_not_valid(), an_off_host_final_url_without_cards_is_never_a_zero() (+35 more)

### Community 107 - "profile_config.rs"
Cohesion: 0.13
Nodes (11): a_validation_error_reports_a_pointer_and_never_a_value(), an_oversize_document_is_refused_before_anything_is_applied(), confirmed_export_overwrites_the_same_destination(), direct_buffer_change_invalidates_commit_revision(), draw(), EditorError, locate(), SECRET_WARNING (+3 more)

### Community 108 - "AtlasEvent"
Cohesion: 0.12
Nodes (24): AtlasEvent, Article, Classified, Fault, InsightProgress, MemoriesChanged, MemoryProgress, Note (+16 more)

### Community 109 - "Architecture"
Cohesion: 0.10
Nodes (21): Admiralty source evaluation (WP:RSP), Architecture, Atlas, Binder and executor, Brain recall, Config transfer and the commit, Directives, Intel reports (+13 more)

### Community 110 - "WorkEvent"
Cohesion: 0.08
Nodes (22): work_event(), WorkEvent, AnswerDelta, AnswerNote, AnswerReplacement, AnswerReset, AtlasDone, BrainRelatedExplained (+14 more)

### Community 111 - "store.rs"
Cohesion: 0.07
Nodes (35): article(), atlas_article_from_row(), atlas_articles_for_intel_dedupes_same_article_id_across_runs(), atlas_articles_for_intel_filters_category_day_and_query(), atlas_auto_next_round_trips_through_app_state(), atlas_claims_for_article_returns_linked_claims(), atlas_prune_drops_finished_runs_older_than_the_cutoff(), atlas_recent_articles_lists_retained_rows() (+27 more)

### Community 112 - "ClockSet"
Cohesion: 0.21
Nodes (3): ClockSet, queue_time_does_not_spend_active_but_consumes_wall(), sequential_vs_overlapping_tool_budgets()

### Community 113 - "ProviderAdmission"
Cohesion: 0.18
Nodes (6): shared_rate_limit_cooldown_blocks_admission(), AdmissionGuard, note_shared_rate_limit(), provider_admission_caps_at_two(), ProviderAdmission, rate_limit_cooldown_blocks_acquire()

### Community 114 - "components.rs"
Cohesion: 0.19
Nodes (13): action_rects(), analytics_card(), clip_text(), compact_records_keep_trailing_numeric_values(), detail_records(), detail_table(), editor_height(), measured_lines() (+5 more)

### Community 115 - "ParserInput"
Cohesion: 0.33
Nodes (5): ParserInput, CleanHtml, LinksOnly, Missing, RawHtml

### Community 116 - "release_check.mjs"
Cohesion: 0.12
Nodes (16): root, script, config, effect(), root, rules(), skills, script (+8 more)

### Community 117 - "ServiceResult"
Cohesion: 0.22
Nodes (11): cache_key(), cached_result(), cancel_marks_inconclusive(), check_implemented(), classify_http(), parse_json(), ServiceResult, store_cache() (+3 more)

### Community 118 - "briefing_view.rs"
Cohesion: 0.21
Nodes (18): bucket_extracted(), bucket_extracted_with_explanations(), buckets_split_facts_inferences_context_and_links(), claim(), claim_label(), explain_intel_links_with_model(), explain_relation_link(), ExtractedBuckets (+10 more)

### Community 119 - "subscription.rs"
Cohesion: 0.22
Nodes (8): answer_event(), check_login(), command(), complete(), completion_is_ephemeral_and_does_not_inherit_tools_or_configuration(), exec_args(), is_subscription_status(), login()

### Community 120 - "profile.rs"
Cohesion: 0.14
Nodes (45): amplification_lines(), ATTENTION, attribution_lines(), backlog_lines(), capacity_lines(), cause_lines(), compare_cells(), confidence_lines() (+37 more)

### Community 121 - "TurnEvent"
Cohesion: 0.13
Nodes (16): AnswerContext, compact_page_evidence(), directive_goals_line(), finish_recon_job(), deltas(), page_needs_compact(), Run, snapshot_secret() (+8 more)

### Community 122 - "recon/graph.rs"
Cohesion: 0.06
Nodes (59): draw(), draw_path(), draw_summary(), glyph(), inset(), legend_height(), legend_parts(), legend_rows() (+51 more)

### Community 123 - "tasks.rs"
Cohesion: 0.07
Nodes (70): lease_fencing_rejects_stale_epoch_and_foreign_owner(), add_missing_columns(), adopt_untracked_index_changes(), block_claimed(), claim_complete_and_retry_round_trip(), claim_index_changes(), claim_index_work(), claim_next() (+62 more)

### Community 124 - "ReconCommand"
Cohesion: 0.17
Nodes (12): ReconCommand, Ask, AskNew, Delete, Limits, List, New, Rename (+4 more)

### Community 125 - "config_transfer_tests.rs"
Cohesion: 0.13
Nodes (45): apply_import(), export_document(), export_to_path(), parse_document(), ProfileConfig, ProviderEntry, serialize_document(), a_failed_validation_writes_nothing() (+37 more)

### Community 126 - "run"
Cohesion: 0.06
Nodes (26): atlas_auto_future_tick_waits_and_past_tick_reschedules(), atlas_auto_while_running_only_arms_the_next_slot(), atlas_countdown_visible(), atlas_extracting_visible(), atlas_history_button_counts_down_while_auto_run_is_on(), atlas_poll_wait(), flush_streams(), intel_body_loading_visible() (+18 more)

### Community 127 - "ProviderSecret"
Cohesion: 0.08
Nodes (38): openrouter_ready(), resolve_actor_reviewer_secret(), account_secret(), active_text_secret(), authorize(), bearer_token(), catalog_error(), DeviceGrant (+30 more)

### Community 128 - "ServiceSpec"
Cohesion: 0.20
Nodes (9): by_id(), CATALOG, CATALOG_LEN, UPSTREAM_COMMIT, ServiceSpec, ServiceState, Enabled, Experimental (+1 more)

### Community 129 - "TurnClock"
Cohesion: 0.10
Nodes (6): a_round_keeps_its_time_and_a_tight_ceiling_still_runs_tools(), deadline_seconds(), format_deadline(), format_span(), the_label_updates_when_a_round_adds_calls_and_when_evidence_is_measured(), TurnClock

### Community 130 - "opencode.json"
Cohesion: 0.07
Nodes (31): agents, build, general, plan, reasoningEffort, thinkingBudget, mode, model (+23 more)

### Community 131 - "provider_diag.rs"
Cohesion: 0.23
Nodes (15): text(), bounded(), cause_chain(), classify_status(), endpoint_strips_query_and_userinfo(), find_url(), http_failure(), provider_error() (+7 more)

### Community 132 - "ProfileView"
Cohesion: 0.16
Nodes (9): activate(), capacity_preserves_unknown_disabled_and_overflow_values(), handle_key(), open_run_owner(), open_selected_owner(), ProfileView, remember_row(), select_row() (+1 more)

### Community 133 - "ConfigView"
Cohesion: 0.20
Nodes (4): a_directory_target_is_an_actionable_error_not_a_crash(), ConfigView, editing_the_buffer_disarms_the_import_button(), expand_home()

### Community 134 - "ToolResult"
Cohesion: 0.06
Nodes (57): ToolResult, a_budgeted_call_with_a_dependency_from_an_earlier_batch_runs(), action_call(), advance_stage(), bound(), bounded_reason(), BudgetedOutcome, call_serves() (+49 more)

### Community 135 - "DateTime"
Cohesion: 0.09
Nodes (31): adaptive_bucket_seconds(), ArticleFact, BriefFact, Bucket, bucket_count(), bucket_floor(), bucket_format(), bucket_start() (+23 more)

### Community 136 - "OsintCommand"
Cohesion: 0.14
Nodes (14): DatasetCommand, Import, Refresh, Status, OsintCommand, Attach, Dataset, Describe (+6 more)

### Community 137 - "source_sha256"
Cohesion: 0.01
Nodes (222): source_sha256, crates/argos-osint-bin/Cargo.toml, crates/argos-osint-bin/src/cli.rs, crates/argos-osint-bin/src/main.rs, crates/argos-osint-bin/src/tui/app.rs, crates/argos-osint-bin/src/tui/article_image.rs, crates/argos-osint-bin/src/tui/atlas_actions.rs, crates/argos-osint-bin/src/tui/atlas_table.rs (+214 more)

### Community 138 - "brain_lance_off.rs"
Cohesion: 0.13
Nodes (16): begin_generation(), block_on(), clear_fingerprint(), current_fingerprint(), DUPLICATE_THRESHOLD, fingerprint_matches(), GenerationProgress, LAST_ERROR (+8 more)

### Community 139 - "Argos OSINT — Agent Instructions"
Cohesion: 0.10
Nodes (20): Agent Cargo isolation, Architecture notes, Argos OSINT — Agent Instructions, CI (`.github/workflows/ci.yml`), CLI Entry Points (`argos` binary), Developer Commands, diagrams, Environment Variables (+12 more)

### Community 140 - "modes.rs"
Cohesion: 0.24
Nodes (14): scoped_section_plan(), chat_response_spec(), chat_response_spec_embeds_section_guidelines_and_style_guide(), chat_response_spec_lists_mode_headings(), chat_section_titles(), every_mode_has_bluf_first_and_sources_last(), every_mode_section_has_guideline(), investigation_mode_spec() (+6 more)

### Community 141 - "investigation.rs"
Cohesion: 0.05
Nodes (62): absorb_hit(), account_platform_host(), ACCOUNT_PLATFORMS, ACCOUNTS, accounts_flow(), ADAPTIVE, Alternative, binder_maps_kinds_to_inputs_and_never_hands_hunter_a_social_host() (+54 more)

### Community 142 - "wikipedia_rsp.rs"
Cohesion: 0.09
Nodes (40): admiralty_scales_claim_confidence_from_rsp_and_peers(), raw_initial_confidence_is_captured_before_source_scaling(), API, APP_STATE_KEY, cache(), CACHE_TTL, cached_index(), CachedIndex (+32 more)

### Community 143 - "intel-brief-image-column-2026-10-11/manifest.json"
Cohesion: 0.10
Nodes (20): ARGOS_EMBED, artifact_count, capture_status, CARGO_TARGET_DIR, command, created_utc, fixtures, font (+12 more)

### Community 144 - "serde_json"
Cohesion: 0.12
Nodes (9): DecisionState, EvidencePassageState, SubjectStateSummary, TaskStateSummary, ADAPTER_VERSION, HOST, ID, positive_negative_and_source_field() (+1 more)

### Community 145 - "Category"
Cohesion: 0.05
Nodes (34): Category, Auth, Cancelled, Configuration, Empty, InvalidModel, InvalidResult, MalformedPayload (+26 more)

### Community 146 - "tracked.rs"
Cohesion: 0.23
Nodes (6): begin(), begin_cancellable(), db(), db_path(), finish(), E

### Community 147 - "Rect"
Cohesion: 0.10
Nodes (47): add_fallback_layout(), add_fallback_popup_area(), article_image_area(), brain_form(), brain_hit(), brain_list(), BrainForm, BrainList (+39 more)

### Community 148 - "QueryExecutionStatus"
Cohesion: 0.17
Nodes (11): QueryExecutionStatus, Cancelled, Completed, Failed, Generated, NoResults, Queued, Running (+3 more)

### Community 149 - "results.rs"
Cohesion: 0.07
Nodes (31): classify_output_quality(), extract_observation_items(), extract_search_results(), extracts_from_array_shaped_observations(), extracts_from_object_with_results_array(), normalize_tool_result(), NormalizedToolResult, OutputQuality (+23 more)

### Community 150 - "gates.rs"
Cohesion: 0.10
Nodes (18): DecisionService, DecisionPolicy, EnforcementOutcome, Pass, Reject, Unavailable, Uncertain, ThresholdProfile (+10 more)

### Community 151 - "ToolOutcome"
Cohesion: 0.11
Nodes (15): serp_tool_outcome(), cache_ttl_for(), is_named_serp(), SerpCacheClass, Failure, Valid, VerifiedZero, ToolOutcome (+7 more)

### Community 152 - "WidgetSpec"
Cohesion: 0.25
Nodes (22): attention_lines(), bucket_end(), bucket_start(), build_report_detail_lines(), categorical_plot(), count_buckets(), dashboard_lines(), draw_report() (+14 more)

### Community 153 - "App"
Cohesion: 0.24
Nodes (22): content_layout(), dashboard_layout(), draw_actions(), draw_filter_strip(), draw_picker(), draw_profile(), draw_section_navigator(), draw_status_strip() (+14 more)

### Community 154 - "tui_review.py"
Cohesion: 0.12
Nodes (13): cargo_command(), cargo_env(), main(), box(), color(), render(), capture(), digest() (+5 more)

### Community 155 - ".new"
Cohesion: 0.28
Nodes (5): aging_the_tool_clock_blocks_new_calls_and_cache_hits_do_not_extend_it(), clock_set_for_turn(), foreground_expiry_continues_in_background(), sequential_raise_calls_budgets_higher(), TurnCheckpoint

### Community 156 - "investigation/evidence.rs"
Cohesion: 0.12
Nodes (24): accepted_evidence_events_are_stable_and_keep_provenance(), assess_claim_against_passages(), ClaimAssessmentOutcome, ClaimStance, Disputed, Insufficient, Mention, Supported (+16 more)

### Community 157 - "serde"
Cohesion: 0.07
Nodes (17): InvestigationPattern, ArticleVerification, Bitcoin, BreakingNews, DomainIp, EmailAttribution, FollowUp, GeneralSubject (+9 more)

### Community 165 - ".new"
Cohesion: 0.11
Nodes (12): NoDuplicates, Supplied<T>, D, CompiledSite, DatasetSnapshot, deserialize_flexible_code(), deserialize_headers(), deserialize_optional_string_or_empty() (+4 more)

### Community 166 - "SiteOutcomeStatus"
Cohesion: 0.18
Nodes (10): deserialize_flexible_bool(), SiteOutcomeStatus, Ambiguous, Blocked, Error, Found, NotFound, RateLimited (+2 more)

### Community 167 - ".new"
Cohesion: 0.08
Nodes (67): Account, account_hits(), account_platforms_attach_to_the_subject_and_are_never_entities(), action_order(), actions_are_grounded_capped_and_not_a_sweep(), adaptive_expansion_spends_one_scarce_lookup_then_reranks(), adaptive_step(), additional_tools() (+59 more)

### Community 168 - "CancelRequest"
Cohesion: 0.20
Nodes (8): CancelRequest, Missing, NotCancellable, NotRunning, Requested, job_for_run(), open(), request_cancel()

### Community 169 - "paths.rs"
Cohesion: 0.23
Nodes (13): write_cache(), auth_path(), config_journal_path(), config_lock_path(), config_path(), dataset_dir(), datasets_dir(), db_path() (+5 more)

### Community 170 - "ModuleId"
Cohesion: 0.14
Nodes (17): ModuleId, .ALL, Atlas, Brain, Intel, Jobs, Logs, Osint (+9 more)

### Community 171 - "atlas_work.rs"
Cohesion: 0.06
Nodes (35): articles_rev(), AttemptRecord, check_dependency_coverage(), CycleOutcome, Blocked, Cancelled, Completed, CompletedWithWarnings (+27 more)

### Community 172 - "rusqlite"
Cohesion: 0.13
Nodes (11): record_detail_lines(), ExplanationRecord, GraphSummaryEntry, migrate(), put_record(), SaveOutcome, MemoryChanged, MemoryGone (+3 more)

### Community 173 - "ModelAssignment"
Cohesion: 0.24
Nodes (7): apply_role(), role_assignment(), RoleAssignment, RoleDocument, ModelAssignment, ModelRoute, role_fallbacks_add_delete_reorder_and_reject_duplicates()

### Community 175 - "Finish"
Cohesion: 0.22
Nodes (8): Finish, Blocked, Cancelled, Completed, Failed, Partial, Paused, Waiting

### Community 176 - "atlas_answer_id"
Cohesion: 0.25
Nodes (4): ArticleInsightCommit, atlas_answer_id(), AtlasStoredClaim, insight_fingerprint()

### Community 177 - "ui.rs"
Cohesion: 0.05
Nodes (66): ACTION_H, ApiKeySlot, atlas_auto_label(), atlas_countdown(), atlas_history_live_label(), atlas_run_label(), atlas_source_anchors_are_not_labeled_deleted_origin(), center_line() (+58 more)

### Community 178 - "RecoveryAction"
Cohesion: 0.12
Nodes (12): classify_recovery(), RecoveryAction, AccessRestricted, CircuitCooldown, ContentRefused, CreditsExhausted, FallbackModel, ReduceContext (+4 more)

### Community 179 - "grok_oauth.rs"
Cohesion: 0.15
Nodes (20): auth_path(), bearer(), bearer_from_path(), check_login(), Entry, entry_from(), failed_login_keeps_the_cli_reason_and_recovery_action(), login() (+12 more)

### Community 180 - "LaunchState"
Cohesion: 0.40
Nodes (5): LaunchState, Accepted, Accepting, Editable, RecoverableFailure

### Community 181 - "Conventions"
Cohesion: 0.20
Nodes (10): Agent scratch, Checks, Commits and versions, Conventions, Diagrams, Graphify, Planning files, Schema (+2 more)

### Community 182 - "Argos UI Interaction Audit"
Cohesion: 0.40
Nodes (5): Argos UI Interaction Audit, Audit Matrix, Field inventory (`field_placeholder`), Remaining limits, Shared contracts

### Community 183 - "ScheduledCall"
Cohesion: 0.38
Nodes (8): courtlistener_spacing_and_firecrawl_polling_are_counted(), live(), more_calls_mean_more_tool_time_and_cache_hits_add_nothing(), scheduled(), ScheduledCall, tool_allowance_for_deps(), tool_allowance_for_deps_sequential_sums(), tool_allowance_seconds()

### Community 184 - "config_transfer.rs"
Cohesion: 0.09
Nodes (62): apply_provider(), apply_tool_slot(), array_value(), check_known_fields(), check_limit(), check_required(), check_role_compatibility(), check_route_reference() (+54 more)

### Community 185 - "OpenCode V2 workflow"
Cohesion: 0.20
Nodes (10): Builds, Commit and release tasks, ECC commands, Graph first, OpenCode V2 workflow, Phase edits, Primary tools, TUI verification setup (+2 more)

### Community 186 - "Call"
Cohesion: 0.07
Nodes (49): TranscriptBlock, build_blocks(), call_stamp(), chat_spots(), chat_view(), ChatBlock, ChatRow, clip_pieces() (+41 more)

### Community 187 - "Concepts"
Cohesion: 0.25
Nodes (8): Apps and internal IDs, Bindings, Concepts, Graph, Intel report jobs, Model roles, Persistence, Primary OSINT providers

### Community 201 - "IntelPage"
Cohesion: 0.67
Nodes (3): IntelPage, Briefing, Bulletin

### Community 202 - "TUI components"
Cohesion: 0.17
Nodes (12): Acceptance, ActionBar and Overlay, AnalyticsCard and Dashboard preset, Application presets, DetailTable and Report preset, Measured text, MeasuredEditor and TranscriptBlock, RankedBars, ComparisonBars and Meter (+4 more)

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
Cohesion: 0.20
Nodes (10): Atlas, CLI, Intel, Intel article image quality, Other apps, Profile, Recon, State (+2 more)

### Community 209 - "Profile dashboard and search"
Cohesion: 0.17
Nodes (12): Analytics controls, Import semantics, Metric dictionary, Named search engines, Profile dashboard and search, Provider orchestration companion, Retention and coverage, The 20 primary views (+4 more)

### Community 210 - "RunStats"
Cohesion: 0.13
Nodes (13): a_resumed_cycle_continues_candidate_numbering(), cycle_event_id_is_the_run_key_so_replay_never_double_counts(), CycleTelemetry, db(), each_candidate_occurrence_gets_exactly_one_disposition(), OpenStage, origin_snapshots_are_one_row_per_origin_per_cycle(), parse_stats() (+5 more)

### Community 213 - ".field"
Cohesion: 0.18
Nodes (3): is_picker_field(), multiline_paste_stays_in_composer_and_does_not_run_shortcuts(), provider_label()

### Community 214 - ".order"
Cohesion: 0.23
Nodes (10): checked_serves(), context_additions(), context_tools(), fallback_record(), OrderContext, Ordered, Picker<'a>, serves_for() (+2 more)

### Community 215 - "super"
Cohesion: 0.07
Nodes (18): rows(), intel_recon_publishes_durably_with_provenance_and_no_atlas_receipt(), ReconInsightUpdate, update(), upsert_adds_and_updates_claims(), upsert_recon_insights(), CODES, NAMES (+10 more)

### Community 216 - "Unified Investigation Harness"
Cohesion: 0.33
Nodes (6): Catalog projection, Model roles, Persistence, Reasoning channels, Unified Investigation Harness, Validation gates

### Community 217 - "RendererKind"
Cohesion: 0.17
Nodes (12): applicable_filters(), detail_columns(), renderer_kind(), RendererKind, Comparison, Meter, Ranked, Table (+4 more)

### Community 219 - "TurnContinuation"
Cohesion: 0.40
Nodes (4): TurnContinuation, Active, Background, Exhausted

### Community 220 - "Argos OSINT"
Cohesion: 0.33
Nodes (6): Applications, Argos OSINT, Documentation, Figures, Limits, Quick start

### Community 222 - "EventKind"
Cohesion: 0.08
Nodes (24): applicable_filter_dimensions(), AttentionRow, bounded_label(), dims_from_key(), Reader<'a>, EventKind, .ALL, AtlasCandidate (+16 more)

### Community 223 - "Argos documentation"
Cohesion: 0.40
Nodes (5): Agent and workflow, Argos documentation, Checklists (historical), Figures, Product

### Community 228 - "OriginStat"
Cohesion: 0.16
Nodes (18): a_decisions_model_does_not_extract_claims(), article(), brief_rating_mean(), extract(), insight_packet(), kept(), lead_claims_scale_with_the_gate_and_context_is_not_capped_at_five(), origin() (+10 more)

### Community 229 - "Json"
Cohesion: 0.18
Nodes (17): json_array_len(), json_at(), json_bool(), json_str(), complete_stream_and_json_answers_succeed(), delta(), http_errors_carry_status_code_and_redacted_message(), ok_json() (+9 more)

### Community 231 - "IndexOutcome"
Cohesion: 0.20
Nodes (6): IndexOutcome, Disabled, Pending, PermanentFailure, Ready, RetryableFailure

### Community 232 - "model_facts"
Cohesion: 0.09
Nodes (28): amplification_is_a_raw_multiplier_with_an_eligible_denominator(), AmplificationBucket, AttemptFact, AttemptSummary, Cell, classify_model_failure(), FailureBucket, FallbackRow (+20 more)

### Community 233 - "profile_components.rs"
Cohesion: 0.23
Nodes (7): clipped(), Kpi, panel(), table_row(), count_kpi(), kpis(), rate_kpi()

### Community 235 - "manifest.json"
Cohesion: 0.09
Nodes (21): ARGOS_EMBED, artifact_count, capture_status, CARGO_TARGET_DIR, command, created_utc, fixtures, font (+13 more)

### Community 237 - "parse_serp_response"
Cohesion: 0.16
Nodes (21): cache_fragment(), engine_for_tool(), parse_serp_response(), SerpRequest, a_redirected_final_url_is_not_used_to_rebuild_the_query(), a_single_retry_is_allowed_only_for_a_parser_mismatch_with_a_dom(), an_empty_dom_without_any_payload_is_a_parser_mismatch(), api_http_404_is_an_upstream_failure() (+13 more)

### Community 239 - "fetch_once"
Cohesion: 0.50
Nodes (3): fetch_once(), public_host(), read_limited()

### Community 240 - "provider_metrics.rs"
Cohesion: 0.15
Nodes (15): cache_capacity_rows(), cached_rows_become_an_available_snapshot(), capacity_snapshot(), CapacityRow, CapacitySnapshot, memory(), missing_companion_snapshot_is_unavailable_not_zero(), QueueRow (+7 more)

### Community 242 - "Trigger"
Cohesion: 0.17
Nodes (11): TELEMETRY_TRIGGER, is_forbidden_key(), payload_number(), Trigger, AtlasCycle, IntelBrief, ManualTool, ReconPrompt (+3 more)

### Community 244 - ".recon_outcomes"
Cohesion: 0.20
Nodes (9): normalize_run_outcome(), ReconOutcomeBucket, RunOutcome, Cancelled, CompletedWithEvidence, CompletedZeroEvidence, Failed, Partial (+1 more)

### Community 245 - "BrainListMode"
Cohesion: 0.50
Nodes (4): BrainListMode, Create, Graph, List

### Community 247 - "replace_insights.rs"
Cohesion: 0.35
Nodes (11): commit_refined_body(), article(), claim(), commit_replaces_atomically_and_keeps_shared_support(), failed_validation_leaves_prior_insights(), replace_article_insights_from_body(), ReplaceOutcome, snapshot_counts_are_stable_before_commit() (+3 more)

### Community 248 - "search_engines_fixtures.rs"
Cohesion: 0.37
Nodes (12): every_fixture_matches_its_recorded_outcome(), field(), fixture_cases(), fixtures_dir(), no_fixture_claimed_a_verified_zero_without_a_supported_phrase(), opt_str(), opt_usize(), read_json() (+4 more)

### Community 249 - "PickReply"
Cohesion: 0.43
Nodes (3): parse_chat_pick(), PickReply, PickRequestAdapter<'r, 'b>

### Community 251 - "graph_explanation/tests.rs"
Cohesion: 0.13
Nodes (24): explain(), ExplainOutcome, Failed, Saved, Superseded, ExplainReport, ExplainRequest, Faults (+16 more)

### Community 252 - "SystemTab"
Cohesion: 0.33
Nodes (4): SystemTab, Configs, Overview, System

### Community 253 - ".intel_reports"
Cohesion: 0.17
Nodes (11): mean(), normalize_report_outcome(), report_key(), ReportModeRow, ReportOutcome, Blocked, Cancelled, Completed (+3 more)

### Community 254 - "§9 implementation order"
Cohesion: 0.15
Nodes (12): §10 acceptance checks, §9 implementation order, Atlas memory completion and System apps — implementation checklist, Phase 1 — what landed, Phase 2 — what landed, Phase 3 — what landed, Phase 4 — what landed, Phase 5a — what landed (+4 more)

### Community 255 - "InvestigationSurface"
Cohesion: 0.09
Nodes (16): CallProposal, HandoffRecord, InvestigationSurface, HomeComposer, IntelBrief, JobsResume, ReconChat, validate_tool_preflight() (+8 more)

### Community 257 - "Item"
Cohesion: 0.26
Nodes (6): Item, DecisionQuestionType, Choice, Noul, Score, QuestionSpec

### Community 258 - "actor_review.rs"
Cohesion: 0.29
Nodes (11): ActorReviewItem, ActorReviewResult, apply_reviewed_actors(), deterministic_review_actors(), is_meaningful_actor(), JUNK_ACTORS, model_review_actors(), parse_actor_review_response() (+3 more)

### Community 259 - "run_live"
Cohesion: 0.29
Nodes (6): LiveRun, Fresh, Latest, Run, run_live(), RunInput

### Community 260 - "tool_runner.rs"
Cohesion: 0.11
Nodes (17): ToolDefinition, a_synthesized_timeout_is_a_failure_with_a_tool_timeout_reason(), cancelled_and_cache_modes_are_not_remote_requests(), definition(), every_other_tool_keeps_its_catalog_cache_lifetime(), failure_outcome(), InvocationFact, named_serp_cache_policy_is_semantic() (+9 more)

### Community 261 - "result_cache"
Cohesion: 0.38
Nodes (6): cache_stores_only_definitive(), CacheEntry, clear_caches(), host_cooldown_is_rate_limited(), host_cooldowns(), result_cache()

### Community 262 - "Target"
Cohesion: 0.04
Nodes (46): brain_article_source_opens_intel_brief(), FocusEntry, hit(), IntelReconFocus, Section, Start, Tab, LayoutRegistry (+38 more)

### Community 263 - ".open_job_source"
Cohesion: 0.40
Nodes (4): AtlasRun, JobSource, AtlasRun, ReconThread

### Community 267 - "Profile analytics dashboard"
Cohesion: 0.33
Nodes (6): Dashboard preset, Primary inventory, Profile analytics dashboard, State and interaction, Verification and delivery, Visual and metric rules

### Community 268 - "CheckSignal"
Cohesion: 0.20
Nodes (8): CheckSignal, Blocked, Error, Inconclusive, NotRegistered, RateLimited, Registered, Unsupported

### Community 270 - "Profile TUI implementation session — 2026-10-10"
Cohesion: 0.22
Nodes (9): Artifact retention, Defects found and corrected, Fixture and screenshot construction, Gate results and environment handling, Ownership and implementation, Profile TUI implementation session — 2026-10-10, Scope and starting evidence, Screenshot evidence (+1 more)

### Community 276 - "Diagram conventions"
Cohesion: 0.50
Nodes (4): Agent rule, Catalog, Diagram conventions, When to draw

### Community 278 - "AtlasPage"
Cohesion: 0.67
Nodes (3): AtlasPage, Live, Runs

### Community 279 - "TUI implementation and verification"
Cohesion: 0.29
Nodes (7): Build fixtures and assertions, Capture actual cells, Finish and retain evidence, Plan the contract, Review, fix and recapture, Run the check gate, TUI implementation and verification

### Community 281 - "Unified reliability / summarization / semantic pipelines — completion checklist"
Cohesion: 0.40
Nodes (4): Honest remaining gaps, Phase status (§18), This follow-up, Unified reliability / summarization / semantic pipelines — completion checklist

### Community 285 - "poll_device"
Cohesion: 0.47
Nodes (6): Poll, Denied, poll_device(), Pending, SlowDown, Token

### Community 287 - "Semantic-versioning setup acceptance"
Cohesion: 0.67
Nodes (3): Checks and results, Limits and remaining setup, Semantic-versioning setup acceptance

### Community 289 - "TUI overhaul verification — 10 October 2026"
Cohesion: 0.33
Nodes (6): Defects found and corrected, Interaction and metric evidence, Limits, Reproduction, TUI overhaul verification — 10 October 2026, Visual evidence

### Community 292 - "Versioning, commits and releases"
Cohesion: 0.40
Nodes (5): Agent commit tasks, Commit and PR convention, GitHub setup and first release, Release automation, Versioning, commits and releases

### Community 294 - "pull_request_template.md"
Cohesion: 0.50
Nodes (3): Change, Compatibility, Validation

### Community 295 - "release-please-config.json"
Cohesion: 0.50
Nodes (3): bootstrap-sha, packages, $schema

### Community 303 - "intel-brief-image-column-2026-10-11/README.md"
Cohesion: 0.40
Nodes (3): Intel brief image column — 2026-10-11, Intel image quality review — 2026-10-10, iTerm2 protocol follow-up

## Knowledge Gaps
- **2269 isolated node(s):** `$schema`, `default_agent`, `subagent_depth`, `timeout`, `chunkTimeout` (+2264 more)
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 2944 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **40 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `App` connect `App` to `ProfileView`, `app.rs`, `ConfigView`, `Target`, `.open_job_source`, `ToolResult`, `article_image.rs`, `Store`, `AtlasPage`, `hardware.rs`, `.activate_target`, `brain_detail.rs`, `ModuleId`, `ReportMode`, `tui/jobs.rs`, `LaunchState`, `Call`, `worker.rs`, `src/brain.rs`, `IntelPage`, `Overlay`, `.new_thread`, `.clear`, `RunStats`, `FeedArticle`, `.field`, `DefaultsRole`, `AtlasArticleRow`, `LogsView`, `summary_card.rs`, `SettingsFile`, `WorkEvent`, `store.rs`, `BrainListMode`, `briefing_view.rs`, `recon/graph.rs`, `ProviderSecret`, `run`, `InvestigationSurface`?**
  _High betweenness centrality (0.072) - this node is a cross-community bridge._
- **What connects `$schema`, `default_agent`, `subagent_depth` to the rest of the system?**
  _2269 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `land.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.0076045627376425855 - nodes in this community are weakly interconnected._
- **Why does `Value` connect `Value` to `orchestrate.rs`, `telemetry.rs`, `atlas_memory.rs`, `provider_diag.rs`, `tool_io.rs`, `ToolResult`, `recon.rs`, `Store`, `IntelligenceCategory`, `wikipedia_rsp.rs`, `directives.rs`, `serde_json`, `Category`, `Binding`, `atlas_news.rs`, `execute_steps`, `results.rs`, `TaskStatus`, `whoxy.rs`, `WidgetSpec`, `App`, `news_legal.rs`, `osint.rs`, `holehe/mod.rs`, `TelemetryEvent`, `body.rs`, `provider.rs`, `.new`, `.new`, `ModelAssignment`, `ReconLimits`, `ReportMode`, `picker.rs`, `ui.rs`, `grok_oauth.rs`, `events.rs`, `model_exec.rs`, `config_transfer.rs`, `Call`, `cli.rs`, `atlas_insights.rs`, `body_filter.rs`, `synthesize.rs`, `summarization.rs`, `.new_thread`, `search_engines.rs`, `.new`, `RunStats`, `src/evidence.rs`, `ToolRunner`, `anyhow`, `AtlasArticleRow`, `BrainResourceSummary`, `dork_generator.rs`, `search_engines_fixtures.rs`, `PickReply`, `Plan`, `search_engines/tests.rs`, `AtlasEvent`, `parse_serp_response`, `components.rs`, `ServiceResult`, `subscription.rs`, `profile.rs`, `TurnEvent`, `config_transfer_tests.rs`, `InvestigationSurface`?**
  _High betweenness centrality (0.049) - this node is a cross-community bridge._
- **Should `orchestrate.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.04614825581395349 - nodes in this community are weakly interconnected._
- **Why does `TUI design contract` connect `provider_metrics.rs` to `docs/README.md`, `Frame`?**
  _High betweenness centrality (0.039) - this node is a cross-community bridge._
- **Should `telemetry.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.11693548387096774 - nodes in this community are weakly interconnected._