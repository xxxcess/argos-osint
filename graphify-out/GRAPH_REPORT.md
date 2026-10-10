# Graph Report - argos-osint  (2026-10-10)

## Corpus Check
- 247 files · ~539,408 words
- Verdict: corpus is large enough that graph structure adds value.
- Unclassified: 6 file(s) not represented in the graph (top: (none) 3, .toml 2, .orig 1)

## Summary
- 8011 nodes · 20629 edges · 270 communities (232 shown, 38 thin omitted)
- Extraction: 98% EXTRACTED · 2% INFERRED · 0% AMBIGUOUS · INFERRED: 389 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `00b52075`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- land.rs
- orchestrate.rs
- snapshot
- atlas_memory.rs
- tool_io.rs
- app.rs
- BrainIndex
- FieldId
- map.rs
- recon.rs
- ButtonId
- ToolResult
- IntelligenceCategory
- job_registry.rs
- unix_now
- directives.rs
- .clear
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
- tui/jobs.rs
- App
- events.rs
- exec.rs
- Line
- model_exec.rs
- ErrorCategory
- Binding
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
- inset
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
- AtlasEvent
- dork_generator.rs
- ProfileView
- How
- .memory
- summary_card.rs
- actor_review.rs
- ModuleId
- Region
- diversity.rs
- brain_lance.rs
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
- rows_for
- gsd-v2.js
- ServiceResult
- briefing_view.rs
- subscription.rs
- profile.rs
- DecisionAdapterKind
- recon/graph.rs
- tasks.rs
- ReconCommand
- config_transfer_tests.rs
- Severity
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
- accept_claims
- Argos OSINT — Agent Instructions
- serde_json
- TaskStatus
- wikipedia_rsp.rs
- rusqlite
- DecisionState
- graph_explanation/tests.rs
- execute_steps
- ledger.rs
- QueryExecutionStatus
- results.rs
- TelemetryEvent
- SerpOutcome
- ProfileSnapshot
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
- activate_generation
- tool_runner.rs
- Codebase Map — argos-osint
- PROJECT.md — argos-osint
- Current Phase State
- Trigger
- serde
- grok_oauth.rs
- JobRow
- Conventions
- Docs Ingest — argos-osint
- ScheduledCall
- config_transfer.rs
- OpenCode V2 workflow
- TaskState
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
- draw_field
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
- .rebuild_vectors
- draw_intel_confidence
- Argos UI Interaction Audit
- ChainReport<T>
- ImportBuffer
- ExecuteOptions
- InvestigationSurface
- brain_lance_off.rs
- model_facts
- profile_components.rs
- ReconLimits
- DecisionsResponse
- parse_serp_response
- Block
- atlas_actions.rs
- provider_metrics.rs
- Guard
- IndexOutcome
- .recon_outcomes
- .job_source
- PlanCall
- AtlasInsightClaim
- BrainResourceSummary
- HypothesisRecord
- ConfigTab
- LaunchState
- SystemTab
- .intel_reports
- DispatchError
- run_live
- Section
- Target
- OperationScope<'a>
- Profile analytics dashboard
- Profile TUI implementation session — 2026-10-10
- Diagram conventions
- TUI implementation and verification

## God Nodes (most connected - your core abstractions)
1. `App` - 317 edges
2. `Value` - 285 edges
3. `ProviderSecret` - 143 edges
4. `ButtonId` - 130 edges
5. `Target` - 81 edges
6. `ToolResult` - 78 edges
7. `FieldId` - 76 edges
8. `Store` - 74 edges
9. `Store` - 64 edges
10. `AtlasArticleRow` - 57 edges

## Surprising Connections (you probably didn't know these)
- `Module layout` --references--> `widget_lines()`  [EXTRACTED]
  docs/architecture.md → crates/argos-osint-bin/src/tui/profile.rs
- `Test log` --references--> `main()`  [INFERRED]
  docs/atlas-memory-system-apps-checklist.md → scripts/tui_review.py
- `Other application presets` --references--> `draw_system_tab()`  [EXTRACTED]
  docs/tui-design-spec.md → crates/argos-osint-bin/src/tui/profile.rs
- `Profile edit map` --references--> `LayoutResult`  [EXTRACTED]
  docs/tui-design-spec.md → crates/argos-osint-bin/src/tui/profile_layout.rs
- `Other application presets` --references--> `home_layout_metrics()`  [EXTRACTED]
  docs/tui-design-spec.md → crates/argos-osint-bin/src/tui/ui.rs

## Import Cycles
- 1-file cycle: `crates/argos-osint-core/src/config_transfer.rs -> crates/argos-osint-core/src/config_transfer.rs`
- 2-file cycle: `crates/argos-osint-core/src/osint.rs -> crates/argos-osint-core/src/osint/results.rs -> crates/argos-osint-core/src/osint.rs`
- 2-file cycle: `crates/argos-osint-bin/src/cli.rs -> crates/argos-osint-core/src/hardware.rs -> crates/argos-osint-bin/src/cli.rs`
- 4-file cycle: `crates/argos-osint-core/src/config_commit.rs -> crates/argos-osint-core/src/provider_attempt.rs -> crates/argos-osint-core/src/provider_diag.rs -> crates/argos-osint-core/src/config_transfer.rs -> crates/argos-osint-core/src/config_commit.rs`

## Communities (270 total, 38 thin omitted)

### Community 0 - "land.rs"
Cohesion: 0.01
Nodes (262): CODES, LABELS, R0, R1, R10, R100, R101, R102 (+254 more)

### Community 1 - "orchestrate.rs"
Cohesion: 0.04
Nodes (129): a_429_stops_picker_calls_and_the_fallback_finishes_the_list(), a_claimed_email_removes_its_bindings(), a_dispatch_error_fails_the_step_and_the_loop_goes_on(), a_dropped_provider_stream_keeps_the_text_already_received(), a_failed_email_finder_gets_one_fallback_and_no_third_picker_call(), a_fallback_with_no_bindings_still_runs_the_satisfied_steps_and_reaches_synthesis(), a_follow_up_keeps_names_from_the_previous_synthesis(), a_gap_filler_domain_feeds_hunter_once_firecrawl_observes_it() (+121 more)

### Community 2 - "snapshot"
Cohesion: 0.11
Nodes (23): active_jobs(), adaptive_bucket_seconds(), bucket_count(), count_rows(), distinct_values(), filter_options(), global_status(), has_table() (+15 more)

### Community 3 - "atlas_memory.rs"
Cohesion: 0.05
Nodes (94): atlas_memory_count(), background_indexing_then_refresh_upgrades_a_partial_run(), Barrier, Checkpoint, CheckpointInput, claims(), clear(), crash_after_checkpoint_resumes_without_reextracting() (+86 more)

### Community 4 - "tool_io.rs"
Cohesion: 0.03
Nodes (95): leftovers(), names(), PROMPT_TARGETS, normalize_platform(), question_bindings(), a_gap_filler_only_domain_waits_for_a_primary_observation_before_hunter(), accept_bindings(), batch_scrape_takes_ranked_urls_and_gates_order_before_hunter() (+87 more)

### Community 5 - "app.rs"
Cohesion: 0.05
Nodes (86): a_click_folds_an_event_and_incoming_entries_do_not_move_a_paused_list(), a_finished_tool_call_is_logged_once_and_the_transcript_keeps_a_summary(), all_provider_actions_render_with_hit_areas_at_80x24(), analytics_fixture(), analytics_viewports_and_data_states(), ATLAS_AUTO_SECS, atlas_auto_toggle_persists_the_next_trigger(), atlas_claim() (+78 more)

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
Nodes (64): answer_step_rate_limit_is_explicit_and_keeps_results(), attribute_word(), auxiliary(), BRIEF_SYNTHESIS, broad_question_collapses_to_grounded_lookups(), cacheable(), char_ceil(), char_floor() (+56 more)

### Community 10 - "ButtonId"
Cohesion: 0.02
Nodes (106): ButtonId, Add, AddFallback, AtlasAuto, AtlasDelete, AtlasLive, AtlasNews, AtlasNewsFeed (+98 more)

### Community 11 - "ToolResult"
Cohesion: 0.05
Nodes (47): ToolResult, ac6_citation_groups_split_validate_each_id_and_normalize(), answer_evidence_survives_reopen_and_deleted_thread_rejects_late_calls(), AnswerContext, await_completion(), Call, chat(), citation_groups() (+39 more)

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
Cohesion: 0.05
Nodes (68): paint_logo(), accounts_search_query(), binder_maps_kinds_to_inputs_and_never_hands_hunter_a_social_host(), binding(), apply_context_targets(), asks(), asks_about_judge(), asks_for_headlines() (+60 more)

### Community 16 - ".clear"
Cohesion: 0.14
Nodes (8): draft_isolation_and_persistence(), duplicate_submission_prevention(), home_order_renames_and_nine_routes_agree(), PaletteItem, session_tabs_open_close_reopen(), single_action_launch_from_home(), tab_strip_render_and_hit_test(), unavailable_palette_action_stays_visible_and_does_not_execute()

### Community 17 - "absorb_hit"
Cohesion: 0.13
Nodes (20): absorb_hit(), account_platform_host(), Candidate, content_tokens(), distinct_queries(), domain_label(), EntityIdentifier, first_domain() (+12 more)

### Community 18 - "Value"
Cohesion: 0.04
Nodes (97): Value, CallProposal, HandoffRecord, redact_secrets(), redacts_sensitive_keys(), annotate(), bitcoin(), bounded() (+89 more)

### Community 19 - "atlas_news.rs"
Cohesion: 0.07
Nodes (44): api_error(), ATLAS_TOOLS, authors(), clip(), country_arg(), country_code(), CURRENTS_COUNTRIES, CURRENTS_HOST (+36 more)

### Community 20 - "Store"
Cohesion: 0.06
Nodes (26): article_body_from_row(), ArticleBodyRow, cited_evidence_keeps_provenance_and_revision_identity(), element_from_row(), full_assessment_parents_are_flagged_for_child_report_totals(), IntelAssessmentRow, IntelEvidenceRow, IntelInvestigationRow (+18 more)

### Community 21 - "publication.rs"
Cohesion: 0.07
Nodes (46): Phase5Report, active_index_rows(), bump_memories_changed(), change_watcher_sees_only_committed_changes_and_coalesces(), claim(), clear(), clear_index_state(), CoverageReport (+38 more)

### Community 22 - "gates.rs"
Cohesion: 0.09
Nodes (11): execute_harness_step(), InvestigationRuntime, GateOutcome, Passed, Rejected, validate_claim_assessment(), validate_evidence_admission(), validate_publication() (+3 more)

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
Nodes (61): atlas_cycle_stats_room(), atlas_cycle_stats_scroll_max(), atlas_feed_room(), atlas_feed_room_for(), atlas_news_room(), atlas_news_room_for(), atlas_runs_room(), body_rect() (+53 more)

### Community 27 - "Store"
Cohesion: 0.06
Nodes (8): atlas_answer_id(), atlas_brief_id(), AtlasStoredClaim, has_table(), intel_link_explanations_round_trip_and_cleanup(), repair_embed_tables(), Store, version()

### Community 28 - "embed.rs"
Cohesion: 0.09
Nodes (29): active(), DIM, disable(), disabled(), DisableGuard, download_file(), DOWNLOAD_TIMEOUT_SECS, embed_batch() (+21 more)

### Community 29 - "osint.rs"
Cohesion: 0.04
Nodes (99): test_all_catalog_tools_mapped_to_categories(), a_claimed_email_keeps_no_person_data(), bind_request(), body_cap(), bounded_recovery_is_offered_once_for_an_unrecognised_page_only(), CACHE_DAY_SECONDS, cache_follows_the_provider_plan_interval(), cache_identity() (+91 more)

### Community 30 - "holehe/mod.rs"
Cohesion: 0.09
Nodes (24): CACHE_TTL, cancel_marks_inconclusive(), cap_reports_omitted(), DEFAULT_MAX_SITES, default_selection_uses_implemented_adapters(), email_hash(), email_preserves_local_part(), fetch_once() (+16 more)

### Community 31 - "config_commit.rs"
Cohesion: 0.05
Nodes (73): a_batch_with_a_duplicate_or_empty_slot_is_refused_before_any_write(), a_committed_journal_needs_no_recovery(), a_second_lock_is_refused_while_one_is_held(), a_stale_lock_is_taken_over(), an_abandoned_unfinished_lockfile_is_recovered(), backup_path(), canonical_root(), commit_files_is_all_or_nothing() (+65 more)

### Community 32 - ".default"
Cohesion: 0.08
Nodes (29): a_blank_osint_user_agent_loads_as_unset(), default_credit_reset(), default_firecrawl_credits(), default_hunter_credits(), default_max_calls(), default_max_rounds(), default_one_cost(), default_opening_cap() (+21 more)

### Community 33 - "telemetry.rs"
Cohesion: 0.12
Nodes (21): bin_index(), Coverage, duration_bins_are_bounded_and_mergeable(), DURATION_BINS_MS, ensure_observed_since(), event_bins(), get_meta(), ID_SEQ (+13 more)

### Community 34 - "InvestigationPart"
Cohesion: 0.08
Nodes (18): event_to_chat_block(), is_thinking_expanded(), InvestigationPart, DirectiveAssessment, EvidencePassage, GateValidation, Handoff, Plan (+10 more)

### Community 35 - "body.rs"
Cohesion: 0.09
Nodes (34): body_fetch_routes(), BodyFetchEvent, Attempt, Failed, InsightsFailed, InsightsRefreshing, InsightsReplaced, Progress (+26 more)

### Community 36 - "provider.rs"
Cohesion: 0.05
Nodes (82): account_secret(), active_text_secret(), authorize(), bearer_token(), catalog_error(), chat_body(), complete(), complete_errors_with_finish_reason_when_response_is_empty() (+74 more)

### Community 37 - "brain_detail.rs"
Cohesion: 0.06
Nodes (50): areas(), areas_for(), BrainDetail, DetailAreas, DetailPane, Path, Related, Summary (+42 more)

### Community 38 - "profile_stats.rs"
Cohesion: 0.05
Nodes (68): atlas_carries_data(), AtlasStats, AttemptSummary, BacklogRow, band_for(), CategoryTrigger, classify_model_failure(), classify_tool_cause() (+60 more)

### Community 39 - "atlas.rs"
Cohesion: 0.09
Nodes (30): article_card_labels_title_publisher_author_and_classification(), Band, band_sizes(), band_temperature(), category_from_decisions(), CATEGORY_IDS, CLUSTERS, country_label() (+22 more)

### Community 40 - "run_atlas_inner"
Cohesion: 0.09
Nodes (32): article_from_row(), Stats, Status, AtlasJob, charge_newsapi(), charge_quota(), Cursor, cursor_at() (+24 more)

### Community 41 - "whatsmyname.rs"
Cohesion: 0.05
Nodes (56): DatasetStatus, status(), D, AccountTuple, ACTIVE_SNAPSHOT, ADAPTER_VERSION, apply_strip_bad_char(), benchmark_14_4_parse_index_and_selection() (+48 more)

### Community 42 - "scheduler.rs"
Cohesion: 0.09
Nodes (22): apply_index_change(), apply_index_change_rebuild_reports_outcome(), apply_index_with(), DEFAULT_LEASE_SECS, drain_index_once(), drain_summary_flush(), drain_summary_flush_cached(), drain_summary_flush_completes_cached_tasks() (+14 more)

### Community 43 - "jobs_view.rs"
Cohesion: 0.09
Nodes (23): summary_text(), active_time_is_live_for_open_attempts_and_unavailable_only_without_a_start(), AttemptRow, event_apps(), event_counts(), EventCounts, get_job(), job() (+15 more)

### Community 44 - "provider_attempt.rs"
Cohesion: 0.11
Nodes (39): Acc, attempt(), attempt_with_observer(), AttemptReport, check_final(), complete_stream_and_json_answers_succeed(), Deadlines, delta() (+31 more)

### Community 45 - "LogicalRole"
Cohesion: 0.10
Nodes (11): LogicalRole, .ALL, ClaimAssessor, Classifier, Controller, EntityResolver, EvidenceCurator, Planner (+3 more)

### Community 46 - "picker.rs"
Cohesion: 0.09
Nodes (29): brain_scrape_options_hide_bare_scrape_and_include_claim_criteria(), CatalogEntry, chat_request(), CONFIDENCE_FLOOR, decisions_request(), directives_phrase(), DONE, eligible_catalog() (+21 more)

### Community 47 - "ReportMode"
Cohesion: 0.09
Nodes (36): HomeDraftState, brief_rating_reuses_the_existing_mean_semantics(), classifiable_modes(), classify_mode_chat(), classify_mode_decisions(), classify_prompt_mode(), classify_prompt_mode_chat(), classify_prompt_mode_decisions() (+28 more)

### Community 48 - "provider_chain.rs"
Cohesion: 0.16
Nodes (22): AttemptRecord, cancellation_stops_the_chain(), ChainReport, execute(), FALLBACK_ATTEMPTS, fallback_emits_three_attempts_with_10_20_waits(), FALLBACK_WAITS, http_stream_and_parse_failures_all_retry() (+14 more)

### Community 49 - "atomic"
Cohesion: 0.08
Nodes (15): AttemptOutcome, categorize_http(), execute_with_retries(), other_llm_allows_three(), summarization_stops_at_two_attempts(), TimeoutProfile, .CLASSIFIER, .EXTRACTION (+7 more)

### Community 50 - "tui/jobs.rs"
Cohesion: 0.08
Nodes (25): areas(), BASE_COLUMNS, button_label(), buttons(), detail_lines(), hit(), JobsAreas, JobsView (+17 more)

### Community 51 - "App"
Cohesion: 0.03
Nodes (27): a_failed_turn_keeps_the_streamed_answer_on_screen(), a_log_or_late_tool_row_does_not_drop_the_live_answer(), App, atlas_countdown_visible(), atlas_extracting_visible(), atlas_log_level(), auto_run_completion_selects_the_latest_history_row(), brain_anchors_follow_memory_focus_and_scroll_stops_at_ends() (+19 more)

### Community 52 - "events.rs"
Cohesion: 0.12
Nodes (19): clear_events(), DEFAULT_RETENTION_HOURS, event_row(), EventRow, get_event(), job_filter_includes_descendants_and_prune_keeps_jobs(), list_events(), MAX_DETAIL_CHARS (+11 more)

### Community 53 - "exec.rs"
Cohesion: 0.15
Nodes (19): admission_account(), admission_contention_consumes_no_attempt(), AttemptEvent, Finished, Started, AttemptLog, cancelled(), complete_summary_report() (+11 more)

### Community 54 - "Line"
Cohesion: 0.12
Nodes (23): abs_rect(), extracted_actors_markdown(), extracted_claims_markdown(), extracted_context_markdown(), extracted_inferences_markdown(), extracted_links_markdown(), intel_body_loading(), intel_body_progress_lines() (+15 more)

### Community 55 - "model_exec.rs"
Cohesion: 0.10
Nodes (33): adapter_reported_first_response_is_kept_when_no_chunk_arrived(), attempt_outcome_keeps_the_failure_category_for_the_reason_dimension(), AttemptFacts, AttemptOutcome, AttemptTimings, canonical_attempt_id(), canonical_attempt_id_is_stable_across_replay(), DecisionsAdapter (+25 more)

### Community 56 - "ErrorCategory"
Cohesion: 0.08
Nodes (24): operation_kind(), backoff_delay(), can_retry(), ErrorCategory, AuthOrQuota, CancelledOrStale, ConfigurationMissing, ContextLimit (+16 more)

### Community 57 - "Binding"
Cohesion: 0.14
Nodes (29): Binding, canonical(), dependency_order(), depends_on(), question_bindings_and_dependency_fix(), allowed_producer(), best_handle(), bind_step() (+21 more)

### Community 58 - "MemoryKind"
Cohesion: 0.09
Nodes (22): AdmissionPolicy, admit_memory(), BrainQuery, pack_context(), AssessmentState, Contradicts, Insufficient, Mentions (+14 more)

### Community 59 - "Frame"
Cohesion: 0.11
Nodes (47): intel_category_short(), draw(), AbsRect, atlas_auto_label(), atlas_extracting(), atlas_run_label(), button_areas(), draw_atlas() (+39 more)

### Community 60 - "brain_resources.rs"
Cohesion: 0.08
Nodes (31): source_anchor_label(), BRAIN_SCRAPE_PREFIX, BrainResourceHit, candidate_json(), CLAIM_CHARS, classify_resource(), clip(), file_link_url() (+23 more)

### Community 61 - "worker.rs"
Cohesion: 0.15
Nodes (20): IntelReportEvent, InsightsUpdated, JobCreated, JobDone, Section, Stage, IntelReportJobRow, cancel_if_needed() (+12 more)

### Community 62 - ".handle_key"
Cohesion: 0.06
Nodes (26): add_scroll(), backspace_after_a_sent_question_deletes_one_character(), brain_tab_still_edits_and_finds_sourced_memories(), DefaultRole, compact_home_and_recon_pages_keep_controls_reachable(), defaults_pick_provider_and_model_from_account_access(), defaults_tool_picker_saves_only_its_role(), intel_opens_bulletin_filters_and_opens_briefing() (+18 more)

### Community 63 - "src/brain.rs"
Cohesion: 0.14
Nodes (19): AGREEMENT_WEIGHT, CATEGORIES, category_hint(), format_injection(), hybrid_recall(), hybrid_recall_adds_paraphrases_and_rewards_agreement(), identity_query_prefers_name_memory(), jaccard() (+11 more)

### Community 64 - "intel_recon/jobs.rs"
Cohesion: 0.14
Nodes (20): active_job_for_mode(), cancel_job(), create_job_inserts_all_verify_sections(), create_report_job(), finish_canonical(), intel_recon_links_its_legacy_job_to_one_canonical_registry_job(), pause_job(), register_canonical() (+12 more)

### Community 65 - "cli.rs"
Cohesion: 0.16
Nodes (21): ask(), ask_thread(), atlas_command(), defaults_command(), defaults_json(), defaults_show_includes_the_tool_picker_transport(), DefaultsCommand, Set (+13 more)

### Community 66 - "atlas_insights.rs"
Cohesion: 0.06
Nodes (63): a_country_token_does_not_merge_into_a_longer_name(), a_decisions_model_does_not_extract_claims(), aliases_merge_on_shared_tokens_when_the_longer_form_is_a_span(), articles_have_span(), ask_claims(), AskedClaims, BODY_CLAIM_LIMIT, BODY_SPAN_CHARS (+55 more)

### Community 67 - "SourceReliability"
Cohesion: 0.09
Nodes (22): article_information_credibility(), AdmiraltyCode, best_credibility(), CredibilityInputs, information_credibility(), InformationCredibility, CannotBeJudged, Confirmed (+14 more)

### Community 68 - ".new"
Cohesion: 0.31
Nodes (18): a_snapshot_never_panics_on_a_partially_migrated_store(), an_empty_database_returns_empty_sections_not_zeros(), atlas_raw_candidate_occurrences_are_not_added_twice(), dimension_values(), distinct_article_body_cohort_deduplicates_tags_and_excludes_empty_cache_rows(), durable_model_rows_and_supplemental_telemetry_count_once(), evidence_acceptance_counts_logical_calls_and_keeps_zero_contributors(), evidence_missing_provenance_and_rollups_do_not_invent_acceptance() (+10 more)

### Community 69 - "body_filter.rs"
Cohesion: 0.18
Nodes (22): BATCH_SIZE, BodyChunk, chunk_article_body(), CHUNK_MAX, CHUNK_MIN, CHUNK_TARGET, chunks_preserve_absolute_offsets(), classify_chunk_batch() (+14 more)

### Community 70 - "synthesize.rs"
Cohesion: 0.17
Nodes (20): commit_refined_body(), BODY_REFINE_INPUT_CHARS, deterministic_bluf_mentions_title(), deterministic_section(), model_refine_body(), model_synthesize(), parse_json_object(), refine_retrieved_article_body() (+12 more)

### Community 71 - "dataset.rs"
Cohesion: 0.15
Nodes (20): acquire_refresh_lease(), active_manifest_path(), ACTIVE_REFRESHES, dataset_root(), DatasetManifest, get_status(), import_from_file(), load_active_manifest() (+12 more)

### Community 72 - "Loaded"
Cohesion: 0.11
Nodes (23): add_cycle_outcome(), atlas_origin_rows(), category_triggers(), CycleOutcomeBucket, dominant_label(), engine_health(), EventRow, evidence_call() (+15 more)

### Community 73 - "profile_charts.rs"
Cohesion: 0.06
Nodes (61): absolute_stacks_compare_one_and_one_hundred_on_one_scale(), BLOCKS, cap_eighths(), cells(), coarsen_counts(), column_cell(), connect_cells(), count_coarsening_preserves_totals_boundaries_and_unknown_history() (+53 more)

### Community 74 - "atlas_table.rs"
Cohesion: 0.17
Nodes (22): border_style(), borders_and_cells_have_matching_display_widths(), bottom_border_line(), char_width(), clip_to_width(), column_widths(), columns_consume_inner_width(), columns_distribute_proportionally_when_not_flex_last() (+14 more)

### Community 75 - "Overlay"
Cohesion: 0.08
Nodes (23): ChoiceKind, IntelDay, Investigation, Model, Provider, LastViewSession, Overlay, AddFallback (+15 more)

### Community 76 - "pipeline.rs"
Cohesion: 0.11
Nodes (26): apply_recon_directive_coverage(), ClaimRelation, AmbiguousEntity, ChangedQuantity, Equivalent, Negation, PlannedVsCompleted, Unrelated (+18 more)

### Community 77 - "summarization.rs"
Cohesion: 0.10
Nodes (38): cache_get(), cache_key_changes_with_revision_and_focus(), cache_put(), cache_round_trip(), complete_summary(), CoverageMeta, deterministic_article_description(), deterministic_atlas_brief() (+30 more)

### Community 78 - "theme.rs"
Cohesion: 0.15
Nodes (21): ACCENT, BG, BORDER, card_accent(), card_dim(), card_text(), CODE_BG, DIM (+13 more)

### Community 79 - "inset"
Cohesion: 0.14
Nodes (27): active_popup_area(), add_fallback_popup_area(), atlas_run_card(), choice_list_room(), clip_pieces(), configs_area(), cover(), disclosure_pieces() (+19 more)

### Community 80 - "search_engines.rs"
Cohesion: 0.08
Nodes (35): build_scrape_body(), Candidate, card_snippet(), classify_status_region(), clip_item_text(), collapse_ws(), collect_candidates(), contains_phrase() (+27 more)

### Community 81 - "logs.rs"
Cohesion: 0.12
Nodes (16): areas(), button_label(), buttons(), count(), draw(), hit(), in_list(), list_geometry() (+8 more)

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
Nodes (17): ChoiceItem, codex_models(), DefaultsRole, .ALL, ClaimAssessor, Classifier, EntityResolver, EvidenceCurator (+9 more)

### Community 88 - "graph_explanation.rs"
Cohesion: 0.12
Nodes (13): BASIC_HEADING, explain(), ExplainOutcome, Failed, Saved, Superseded, ExplainReport, ExplainRequest (+5 more)

### Community 89 - "src/evidence.rs"
Cohesion: 0.09
Nodes (29): AGREEMENT_WEIGHT, AnnMeasurement, AnnPolicy, AnnDeferredUnmeasured, AnnEnabled, ExactSearch, AnnThresholds, chunk_text() (+21 more)

### Community 90 - "ToolRunner"
Cohesion: 0.09
Nodes (18): ToolDefinition, attribution_is_explicit_never_inferred_from_the_prompt(), failure_reason(), InvocationFact, named_engine(), osint_cacheable(), test_tool_runner_cache_hit(), test_tool_runner_concurrency_and_dedup() (+10 more)

### Community 91 - "anyhow"
Cohesion: 0.11
Nodes (25): compile_general_model_prompt(), compile_native(), parse_general_model_response(), parse_native_response(), DecisionContract, template_claim_relation(), template_directive_alignment(), template_entity_binding() (+17 more)

### Community 92 - "AtlasArticleRow"
Cohesion: 0.12
Nodes (37): admiralty_scales_claim_confidence_from_rsp_and_peers(), apply_admiralty_evaluation(), apply_peer_support(), article(), article_with_body_spans(), body_lead_prompt(), brief_text(), cap_claims() (+29 more)

### Community 93 - "AtlasEvent"
Cohesion: 0.12
Nodes (24): AtlasEvent, Article, Classified, Fault, InsightProgress, MemoriesChanged, MemoryProgress, Note (+16 more)

### Community 94 - "dork_generator.rs"
Cohesion: 0.10
Nodes (32): ACTIVE_SNAPSHOT, compose_single_query(), compute_query_id(), compute_template_id(), DATASET_NAME, DEFAULT_MAX_QUERIES, DorkCatalog, DorkCategory (+24 more)

### Community 95 - "ProfileView"
Cohesion: 0.14
Nodes (12): activate(), capacity_preserves_unknown_disabled_and_overflow_values(), compare_cells(), handle_key(), open_run_owner(), open_selected_owner(), ordered_rows(), ProfileView (+4 more)

### Community 96 - "How"
Cohesion: 0.11
Nodes (18): How, CompanyEmail, Coordinates, DomainAsUrl, EntityPhrase, PackageName, PackageParts, Plain (+10 more)

### Community 97 - ".memory"
Cohesion: 0.12
Nodes (16): article(), atlas_articles_for_intel_dedupes_same_article_id_across_runs(), atlas_articles_for_intel_filters_category_day_and_query(), atlas_auto_next_round_trips_through_app_state(), atlas_claims_for_article_returns_linked_claims(), atlas_prune_drops_finished_runs_older_than_the_cutoff(), atlas_recent_articles_lists_retained_rows(), atlas_run_days_lists_distinct_days_newest_first() (+8 more)

### Community 98 - "summary_card.rs"
Cohesion: 0.32
Nodes (11): actions(), button_at(), button_cells(), button_row_area(), draw(), height(), is_card_button(), label() (+3 more)

### Community 99 - "actor_review.rs"
Cohesion: 0.26
Nodes (11): ActorReviewItem, ActorReviewResult, apply_reviewed_actors(), deterministic_review_actors(), is_meaningful_actor(), JUNK_ACTORS, model_review_actors(), parse_actor_review_response() (+3 more)

### Community 100 - "ModuleId"
Cohesion: 0.14
Nodes (17): ModuleId, .ALL, Atlas, Brain, Intel, Jobs, Logs, Osint (+9 more)

### Community 101 - "Region"
Cohesion: 0.11
Nodes (19): Region, AtlasInsights, AtlasNews, AtlasOrigins, AtlasRuns, Chat, Detail, IntelBrief (+11 more)

### Community 102 - "diversity.rs"
Cohesion: 0.17
Nodes (24): coverage_gaps_summary(), coverage_record_id(), coverage_stats(), CoverageCandidate, CoverageRecord, CoverageStats, covered_record(), credentials_available() (+16 more)

### Community 103 - "brain_lance.rs"
Cohesion: 0.12
Nodes (16): current_fingerprint(), DUPLICATE_THRESHOLD, EMBED_CHUNK, ensure_ann_index_respects_exact_policy(), fingerprint_matches(), GENERATION_BATCH, LAST_ERROR, LAYOUT (+8 more)

### Community 104 - "Category"
Cohesion: 0.05
Nodes (48): text(), bounded(), Category, Auth, Cancelled, Configuration, Empty, InvalidModel (+40 more)

### Community 105 - "Rect"
Cohesion: 0.09
Nodes (57): abs_contains(), add_fallback_layout(), api_key_slot(), ApiKeySlot, atlas_hit(), atlas_live_areas(), atlas_news_areas(), atlas_row_at() (+49 more)

### Community 106 - "search_engines/tests.rs"
Cohesion: 0.10
Nodes (42): build_serp_url(), a_non_serp_path_on_the_engine_host_is_not_a_serp_page(), a_noscript_wrapped_consent_meta_refresh_is_consent(), a_single_retry_is_allowed_only_for_a_parser_mismatch_with_a_dom(), a_standalone_zero_count_in_a_status_region_is_a_verified_zero(), a_zero_phrase_outside_a_status_region_is_not_a_verified_zero(), an_off_host_final_url_with_cards_keeps_the_items_but_is_not_valid(), an_off_host_final_url_without_cards_is_never_a_zero() (+34 more)

### Community 107 - "profile_config.rs"
Cohesion: 0.22
Nodes (10): a_validation_error_reports_a_pointer_and_never_a_value(), credential_summary(), draw(), draw_export(), draw_import(), EditorError, locate(), rows_at() (+2 more)

### Community 108 - "run_turn"
Cohesion: 0.07
Nodes (51): a_budgeted_call_with_a_dependency_from_an_earlier_batch_runs(), ac3_no_key_reaches_inputs_cache_keys_plan_json_raw_bodies_or_logs(), ac6_a_courtlistener_429_still_lets_the_turn_synthesize(), ac8_status_errors_and_401_403_fail_readably_and_are_not_cached(), advance_stage(), bounded_reason(), call_cached(), call_serves() (+43 more)

### Community 109 - "Architecture"
Cohesion: 0.10
Nodes (21): Admiralty source evaluation (WP:RSP), Architecture, Atlas, Binder and executor, Brain recall, Config transfer and the commit, Directives, Intel reports (+13 more)

### Community 110 - "WorkEvent"
Cohesion: 0.08
Nodes (22): work_event(), WorkEvent, AnswerDelta, AnswerNote, AnswerReplacement, AnswerReset, AtlasDone, BrainRelatedExplained (+14 more)

### Community 111 - "store.rs"
Cohesion: 0.08
Nodes (25): MemorySource, ArticleInsightCommit, atlas_article_from_row(), AUTO_REBUILD_HINT, boost_with_passage_hybrid(), embedding_disabled_degrades_to_jaccard(), embedding_failure_mid_session_degrades_to_jaccard(), fingerprint_mismatch_rebuilds_and_drift_is_reconciled() (+17 more)

### Community 112 - "ClockSet"
Cohesion: 0.21
Nodes (3): ClockSet, queue_time_does_not_spend_active_but_consumes_wall(), sequential_vs_overlapping_tool_budgets()

### Community 113 - "ProviderAdmission"
Cohesion: 0.18
Nodes (6): shared_rate_limit_cooldown_blocks_admission(), AdmissionGuard, note_shared_rate_limit(), provider_admission_caps_at_two(), ProviderAdmission, rate_limit_cooldown_blocks_acquire()

### Community 114 - "components.rs"
Cohesion: 0.18
Nodes (11): action_rects(), analytics_card(), clip_text(), compact_records_keep_trailing_numeric_values(), detail_records(), detail_table(), editor_height(), measured_lines() (+3 more)

### Community 115 - "rows_for"
Cohesion: 0.12
Nodes (23): TranscriptBlock, call_stamp(), ChatBlock, ChatRow, expanded(), face_background(), frame_stamp(), FrameCache (+15 more)

### Community 116 - "gsd-v2.js"
Cohesion: 0.10
Nodes (13): core, home, hooks, names, payload(), run(), setup(), config (+5 more)

### Community 117 - "ServiceResult"
Cohesion: 0.10
Nodes (24): cache_key(), cache_stores_only_definitive(), cached_result(), CacheEntry, check_implemented(), CheckSignal, Blocked, Error (+16 more)

### Community 118 - "briefing_view.rs"
Cohesion: 0.21
Nodes (18): bucket_extracted(), bucket_extracted_with_explanations(), buckets_split_facts_inferences_context_and_links(), claim(), claim_label(), explain_intel_links_with_model(), explain_relation_link(), ExtractedBuckets (+10 more)

### Community 119 - "subscription.rs"
Cohesion: 0.35
Nodes (8): answer_event(), check_login(), command(), complete(), completion_is_ephemeral_and_does_not_inherit_tools_or_configuration(), exec_args(), is_subscription_status(), login()

### Community 120 - "profile.rs"
Cohesion: 0.16
Nodes (41): amplification_lines(), attribution_lines(), backlog_lines(), capacity_lines(), cause_lines(), confidence_lines(), cycle_lines(), cycle_time_lines() (+33 more)

### Community 121 - "DecisionAdapterKind"
Cohesion: 0.10
Nodes (19): DecisionAdapterKind, JsonMode, NativeDecisions, OutputTool, StrictJsonSchema, ValidatedText, resolve_adapter(), DecisionQuestionType (+11 more)

### Community 122 - "recon/graph.rs"
Cohesion: 0.06
Nodes (59): draw(), draw_path(), draw_summary(), glyph(), inset(), legend_height(), legend_parts(), legend_rows() (+51 more)

### Community 123 - "tasks.rs"
Cohesion: 0.07
Nodes (71): lease_fencing_rejects_stale_epoch_and_foreign_owner(), add_missing_columns(), adopt_untracked_index_changes(), block_claimed(), claim_complete_and_retry_round_trip(), claim_index_changes(), claim_index_work(), claim_next() (+63 more)

### Community 124 - "ReconCommand"
Cohesion: 0.17
Nodes (12): ReconCommand, Ask, AskNew, Delete, Limits, List, New, Rename (+4 more)

### Community 125 - "config_transfer_tests.rs"
Cohesion: 0.06
Nodes (75): apply_import(), apply_provider(), apply_role(), apply_tool_slot(), commit_profile_config(), ConfigChange, ConfigurationSnapshot, Credential (+67 more)

### Community 126 - "Severity"
Cohesion: 0.10
Nodes (16): starts(), infer_app(), LevelFilter, All, Error, Info, Warn, session_event() (+8 more)

### Community 127 - "SettingsFile"
Cohesion: 0.09
Nodes (13): resolve_actor_reviewer_secret(), SettingsFile, RoleRuntime, cost_map(), snapshot_secret(), accounts_persist_with_owner_only_permissions(), assert_no_staged_files(), auth_save_commits_only_the_auth_slot() (+5 more)

### Community 128 - "ServiceSpec"
Cohesion: 0.20
Nodes (9): by_id(), CATALOG, CATALOG_LEN, UPSTREAM_COMMIT, ServiceSpec, ServiceState, Enabled, Experimental (+1 more)

### Community 129 - "TurnClock"
Cohesion: 0.11
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
Nodes (70): ACTION_H, atlas_countdown(), atlas_history_live_label(), atlas_source_anchors_are_not_labeled_deleted_origin(), build_blocks(), center_row(), Chrome, clip_chars() (+62 more)

### Community 135 - "DateTime"
Cohesion: 0.11
Nodes (24): amplification_is_a_raw_multiplier_with_an_eligible_denominator(), AmplificationBucket, ArticleFact, BriefFact, Bucket, bucket_floor(), bucket_format(), bucket_start() (+16 more)

### Community 136 - "OsintCommand"
Cohesion: 0.14
Nodes (14): DatasetCommand, Import, Refresh, Status, OsintCommand, Attach, Dataset, Describe (+6 more)

### Community 137 - "normalize_destination"
Cohesion: 0.21
Nodes (14): detect_interstitial(), host_in_any(), host_is(), host_is_captcha(), host_is_consent(), is_ad_host(), is_private_host(), meta_refresh_is_consent() (+6 more)

### Community 138 - "accept_claims"
Cohesion: 0.19
Nodes (16): a_description_only_span_is_inference_and_an_added_name_is_dropped(), a_fact_requires_both_spans_in_the_title(), accept_claims(), accept_one(), AcceptMode, Context, Lead, body_spans_accept_entity_and_object_from_full_article() (+8 more)

### Community 139 - "Argos OSINT — Agent Instructions"
Cohesion: 0.11
Nodes (18): Architecture notes, Argos OSINT — Agent Instructions, CI (`.github/workflows/ci.yml`), CLI Entry Points (`argos` binary), Developer Commands, diagrams, Environment Variables, graphify (+10 more)

### Community 140 - "serde_json"
Cohesion: 0.19
Nodes (17): ADAPTER_VERSION, HOST, ID, positive_negative_and_source_field(), request_url(), every_fixture_matches_its_recorded_outcome(), field(), fixture_cases() (+9 more)

### Community 141 - "TaskStatus"
Cohesion: 0.07
Nodes (14): TaskRecord, TaskStatus, Cancelled, Completed, Deferred, Failed, Partial, Planned (+6 more)

### Community 142 - "wikipedia_rsp.rs"
Cohesion: 0.09
Nodes (38): API, APP_STATE_KEY, cache(), CACHE_TTL, cached_index(), CachedIndex, clip_summary(), ensure_index() (+30 more)

### Community 143 - "rusqlite"
Cohesion: 0.09
Nodes (19): Cached, None, Stale, Valid, Gate, CoolingDown, Ready, Running (+11 more)

### Community 144 - "DecisionState"
Cohesion: 0.20
Nodes (4): DecisionState, EvidencePassageState, SubjectStateSummary, TaskStateSummary

### Community 145 - "graph_explanation/tests.rs"
Cohesion: 0.25
Nodes (16): auth_failure_consumes_primary_budget_gives_guidance_and_redacts(), conn(), count(), failure_is_durable_correlated_bounded_and_harmless(), fixture(), Fx, GOOD, late_completion_for_a_changed_or_deleted_memory_is_not_published() (+8 more)

### Community 146 - "execute_steps"
Cohesion: 0.10
Nodes (38): after_step(), binding_ground(), context_block(), context_dispatched(), deltas(), directive_for(), execute_steps(), expand_dork_children() (+30 more)

### Community 147 - "ledger.rs"
Cohesion: 0.13
Nodes (13): body_assertion_candidates(), coverage_complete(), ElementStatus, Assessed, Excluded, InProgress, Pending, Superseded (+5 more)

### Community 148 - "QueryExecutionStatus"
Cohesion: 0.18
Nodes (11): QueryExecutionStatus, Cancelled, Completed, Failed, Generated, NoResults, Queued, Running (+3 more)

### Community 149 - "results.rs"
Cohesion: 0.13
Nodes (20): classify_output_quality(), extract_observation_items(), extract_search_results(), extracts_from_array_shaped_observations(), extracts_from_object_with_results_array(), normalize_tool_result(), NormalizedToolResult, OutputQuality (+12 more)

### Community 150 - "TelemetryEvent"
Cohesion: 0.13
Nodes (10): clamp(), coverage_reports_observed_since_and_counts(), db(), events_persist_and_roll_up_once(), id_is_stable_so_replay_never_double_counts(), logical_attempt_and_cache_rows_stay_distinct_under_replay(), record(), safe_payload() (+2 more)

### Community 151 - "SerpOutcome"
Cohesion: 0.15
Nodes (11): SerpOutcome, Challenge, Consent, ParserMismatch, RateLimited, ResponseTooLarge, UpstreamFailure, Valid (+3 more)

### Community 152 - "ProfileSnapshot"
Cohesion: 0.21
Nodes (25): ATTENTION, attention_lines(), bucket_end(), bucket_start(), build_report_detail_lines(), categorical_plot(), count_buckets(), dashboard_lines() (+17 more)

### Community 153 - "App"
Cohesion: 0.28
Nodes (19): content_layout(), dashboard_layout(), draw_actions(), draw_filter_strip(), draw_picker(), draw_profile(), draw_section_body(), draw_section_navigator() (+11 more)

### Community 154 - "tui_review.py"
Cohesion: 0.07
Nodes (24): §10 acceptance checks, §9 implementation order, Atlas memory completion and System apps — implementation checklist, Phase 1 — what landed, Phase 2 — what landed, Phase 3 — what landed, Phase 4 — what landed, Phase 5a — what landed (+16 more)

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
Nodes (88): Account, account_hits(), ACCOUNT_PLATFORMS, account_platforms_attach_to_the_subject_and_are_never_entities(), ACCOUNTS, accounts_flow(), action_order(), actions_are_grounded_capped_and_not_a_sweep() (+80 more)

### Community 168 - "Milestone 1 — Core Onboarding (Current)"
Cohesion: 0.18
Nodes (10): Deliverables, Dependencies, Future Milestones (Planned), Milestone 1 — Core Onboarding (Current), Milestone 2 — CLI Verification, Milestone 3 — Test Coverage, Milestone 4 — Integration & Ship, Objectives (+2 more)

### Community 169 - "paths.rs"
Cohesion: 0.25
Nodes (11): auth_path(), config_journal_path(), config_lock_path(), config_path(), dataset_dir(), datasets_dir(), db_path(), ensure_home() (+3 more)

### Community 170 - "model_roles.rs"
Cohesion: 0.13
Nodes (13): AccountHealth, AuthRequired, Cooldown, CreditsExhausted, Overloaded, Ready, Restricted, Unverified (+5 more)

### Community 171 - "atlas_work.rs"
Cohesion: 0.06
Nodes (37): dropped_from_receipt(), reusable_kept(), articles_rev(), AttemptRecord, check_dependency_coverage(), CycleOutcome, Blocked, Cancelled (+29 more)

### Community 172 - "activate_generation"
Cohesion: 0.21
Nodes (15): activate_generation(), begin_generation(), begin_generation_then_activate(), clear_fingerprint(), fingerprint_round_trips_through_memory_embed_meta(), generation_table_name(), GenerationProgress, migrate_generations() (+7 more)

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

### Community 178 - "serde"
Cohesion: 0.10
Nodes (12): classify_recovery(), RecoveryAction, AccessRestricted, CircuitCooldown, ContentRefused, CreditsExhausted, FallbackModel, ReduceContext (+4 more)

### Community 179 - "grok_oauth.rs"
Cohesion: 0.13
Nodes (20): auth_path(), bearer(), bearer_from_path(), check_login(), Entry, entry_from(), failed_login_keeps_the_cli_reason_and_recovery_action(), login() (+12 more)

### Community 180 - "JobRow"
Cohesion: 0.29
Nodes (3): job_row(), JobRow, parse()

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
Cohesion: 0.12
Nodes (47): array_value(), check_known_fields(), check_limit(), check_required(), check_role_compatibility(), check_route_reference(), check_scheme(), child() (+39 more)

### Community 185 - "OpenCode V2 workflow"
Cohesion: 0.25
Nodes (8): Builds, ECC commands, Graph first, OpenCode V2 workflow, Phase edits, Primary tools, TUI verification setup, Verify setup

### Community 186 - "TaskState"
Cohesion: 0.18
Nodes (9): TaskState, Cancelled, Completed, Failed, Paused, Queued, RetryScheduled, Running (+1 more)

### Community 187 - "Concepts"
Cohesion: 0.25
Nodes (8): Apps and internal IDs, Bindings, Concepts, Graph, Intel report jobs, Model roles, Persistence, Primary OSINT providers

### Community 201 - "explore.rs"
Cohesion: 0.20
Nodes (11): GraphEdgeKind, ContradictionOrUpdate, Evidenced, Inferred, SemanticSuggestion, related_evidence_view(), RelatedEvidenceHit, semantic_edges_are_not_factual() (+3 more)

### Community 202 - "TUI components"
Cohesion: 0.18
Nodes (11): Acceptance, ActionBar and Overlay, AnalyticsCard and Dashboard preset, Application presets, DetailTable and Report preset, Measured text, MeasuredEditor and TranscriptBlock, RankedBars, ComparisonBars and Meter (+3 more)

### Community 203 - "ConfigView"
Cohesion: 0.15
Nodes (6): a_directory_target_is_an_actionable_error_not_a_crash(), an_oversize_document_is_refused_before_anything_is_applied(), ConfigView, editing_the_buffer_disarms_the_import_button(), expand_home(), the_pasted_secret_buffer_is_cleared_on_close()

### Community 204 - "Providers"
Cohesion: 0.25
Nodes (8): Budgeted executor, Graph explanation jobs, News and legal keys, OSINT data providers, Providers, Tool picker transport, Typed provider diagnostics, Verification and testing

### Community 205 - "Decision roles and output contracts"
Cohesion: 0.29
Nodes (6): 12-template registry, Adapters, Decision roles and output contracts, State isolation, Thresholds, TUI

### Community 206 - "LayoutResult"
Cohesion: 0.20
Nodes (9): DashboardPage, Atlas, Other, Recon, Summary, LayoutResult, PanelRect, reference_geometry_and_responsive_panels() (+1 more)

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

### Community 213 - "draw_field"
Cohesion: 0.31
Nodes (9): center_line(), center_text(), cursor_at(), draw_field(), field_placeholder(), field_value_area(), line_col(), value_area_or_composer() (+1 more)

### Community 214 - ".order"
Cohesion: 0.21
Nodes (10): checked_serves(), context_additions(), context_tools(), fallback_record(), OrderContext, Ordered, Picker<'a>, serves_for() (+2 more)

### Community 215 - "intel_recon/brain.rs"
Cohesion: 0.57
Nodes (5): intel_recon_publishes_durably_with_provenance_and_no_atlas_receipt(), ReconInsightUpdate, update(), upsert_adds_and_updates_claims(), upsert_recon_insights()

### Community 216 - "Unified Investigation Harness"
Cohesion: 0.33
Nodes (6): Catalog projection, Model roles, Persistence, Reasoning channels, Unified Investigation Harness, Validation gates

### Community 217 - "RendererKind"
Cohesion: 0.17
Nodes (12): applicable_filters(), detail_columns(), renderer_kind(), RendererKind, Comparison, Meter, Ranked, Table (+4 more)

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
Nodes (24): applicable_filter_dimensions(), bounded_label(), Dims, dims_from_key(), Reader<'a>, EventKind, .ALL, AtlasCandidate (+16 more)

### Community 223 - "Argos documentation"
Cohesion: 0.40
Nodes (5): Agent and workflow, Argos documentation, Checklists (historical), Figures, Product

### Community 225 - "draw_intel_confidence"
Cohesion: 0.29
Nodes (6): draw_intel_confidence(), grade_ascii_lines(), grade_score_color(), intel_confidence_scores(), intel_source_evaluation(), IntelSourceEval

### Community 226 - "Argos UI Interaction Audit"
Cohesion: 0.40
Nodes (5): Argos UI Interaction Audit, Audit Matrix, Field inventory (`field_placeholder`), Remaining limits, Shared contracts

### Community 229 - "ExecuteOptions"
Cohesion: 0.38
Nodes (5): cancellable_sleep(), cancelled(), ExecuteOptions, sleep_recorded(), wait_cancelled()

### Community 230 - "InvestigationSurface"
Cohesion: 0.15
Nodes (9): InvestigationSurface, HomeComposer, IntelBrief, JobsResume, ReconChat, atlas_only_tools_blocked_on_all_surfaces(), check_need_gate(), sociavault_blocked_on_intel_surface() (+1 more)

### Community 231 - "brain_lance_off.rs"
Cohesion: 0.15
Nodes (16): begin_generation(), block_on(), clear_fingerprint(), current_fingerprint(), DUPLICATE_THRESHOLD, fingerprint_matches(), GenerationProgress, LAST_ERROR (+8 more)

### Community 232 - "model_facts"
Cohesion: 0.15
Nodes (17): AttemptFact, Cell, model_blocked(), model_cancelled(), model_duration_cell(), model_facts(), model_fallback(), model_finished() (+9 more)

### Community 233 - "profile_components.rs"
Cohesion: 0.23
Nodes (7): clipped(), Kpi, panel(), table_row(), count_kpi(), kpis(), rate_kpi()

### Community 236 - "DecisionsResponse"
Cohesion: 0.33
Nodes (3): DecisionAnswer, DecisionsResponse, parse_decisions()

### Community 237 - "parse_serp_response"
Cohesion: 0.15
Nodes (21): clip_diag(), is_serp_path(), num_at(), parse_serp_response(), str_at(), a_redirected_final_url_is_not_used_to_rebuild_the_query(), an_empty_dom_without_any_payload_is_a_parser_mismatch(), api_http_404_is_an_upstream_failure() (+13 more)

### Community 238 - "Block"
Cohesion: 0.67
Nodes (3): Block, card(), panel()

### Community 240 - "provider_metrics.rs"
Cohesion: 0.16
Nodes (14): cache_capacity_rows(), cached_rows_become_an_available_snapshot(), capacity_snapshot(), CapacityRow, CapacitySnapshot, memory(), missing_companion_snapshot_is_unavailable_not_zero(), QueueRow (+6 more)

### Community 243 - "IndexOutcome"
Cohesion: 0.20
Nodes (6): IndexOutcome, Disabled, Pending, PermanentFailure, Ready, RetryableFailure

### Community 244 - ".recon_outcomes"
Cohesion: 0.20
Nodes (9): normalize_run_outcome(), ReconOutcomeBucket, RunOutcome, Cancelled, CompletedWithEvidence, CompletedZeroEvidence, Failed, Partial (+1 more)

### Community 245 - ".job_source"
Cohesion: 0.50
Nodes (4): AtlasRun, JobSource, AtlasRun, ReconThread

### Community 246 - "PlanCall"
Cohesion: 0.15
Nodes (14): CreditHold, action_call(), bound(), BudgetedOutcome, isolation_lines(), isolation_lines_show_what_ran_what_was_held_and_why(), run_wave(), search_call() (+6 more)

### Community 247 - "AtlasInsightClaim"
Cohesion: 0.30
Nodes (12): claim(), article(), claim(), commit_replaces_atomically_and_keeps_shared_support(), failed_validation_leaves_prior_insights(), replace_article_insights_from_body(), ReplaceOutcome, snapshot_counts_are_stable_before_commit() (+4 more)

### Community 248 - "BrainResourceSummary"
Cohesion: 0.27
Nodes (5): bindings_use_brain_evidence_ids(), BrainResourceSummary, is_http_url(), parse_brain_scrape_index(), scrape_picks_cap_article_and_web_links()

### Community 249 - "HypothesisRecord"
Cohesion: 0.12
Nodes (19): Alternative, classify_hypothesis(), contradiction(), draft_hypotheses(), has_concrete_identifier(), hypothesis_absence_stays_unresolved(), hypothesis_question(), hypothesis_status() (+11 more)

### Community 250 - "ConfigTab"
Cohesion: 0.40
Nodes (3): ConfigTab, Export, Import

### Community 251 - "LaunchState"
Cohesion: 0.08
Nodes (18): AtlasPage, Live, Runs, BrainListMode, Create, Graph, List, IntelPage (+10 more)

### Community 252 - "SystemTab"
Cohesion: 0.40
Nodes (3): SystemTab, Overview, System

### Community 253 - ".intel_reports"
Cohesion: 0.18
Nodes (10): normalize_report_outcome(), report_key(), ReportModeRow, ReportOutcome, Blocked, Cancelled, Completed, Failed (+2 more)

### Community 255 - "DispatchError"
Cohesion: 0.25
Nodes (5): DispatchError, RetryDisposition, NextRoute, RetryRoute, Stop

### Community 259 - "run_live"
Cohesion: 0.29
Nodes (6): LiveRun, Fresh, Latest, Run, run_live(), RunInput

### Community 261 - "Section"
Cohesion: 0.20
Nodes (8): every_section_registers_its_reviewed_widget_count(), Section, Atlas, Intel, Models, Recon, Tools, widgets_render_in_the_specs_presentation_priority_order()

### Community 262 - "Target"
Cohesion: 0.05
Nodes (36): brain_article_source_opens_intel_brief(), FocusEntry, hit(), LayoutRegistry, Target, App, AtlasCycleStats, BrainMark (+28 more)

### Community 267 - "Profile analytics dashboard"
Cohesion: 0.33
Nodes (6): Dashboard preset, Primary inventory, Profile analytics dashboard, State and interaction, Verification and delivery, Visual and metric rules

### Community 270 - "Profile TUI implementation session — 2026-10-10"
Cohesion: 0.22
Nodes (9): Artifact retention, Defects found and corrected, Fixture and screenshot construction, Gate results and environment handling, Ownership and implementation, Profile TUI implementation session — 2026-10-10, Scope and starting evidence, Screenshot evidence (+1 more)

### Community 276 - "Diagram conventions"
Cohesion: 0.50
Nodes (4): Agent rule, Catalog, Diagram conventions, When to draw

### Community 279 - "TUI implementation and verification"
Cohesion: 0.29
Nodes (7): Build fixtures and assertions, Capture actual cells, Finish and retain evidence, Plan the contract, Review, fix and recapture, Run the check gate, TUI implementation and verification

## Knowledge Gaps
- **1796 isolated node(s):** `$schema`, `default_agent`, `subagent_depth`, `timeout`, `chunkTimeout` (+1791 more)
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 2465 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **38 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `App` connect `App` to `app.rs`, `Target`, `ToolResult`, `unix_now`, `.clear`, `Store`, `hardware.rs`, `InvestigationPart`, `brain_detail.rs`, `ReportMode`, `tui/jobs.rs`, `worker.rs`, `.handle_key`, `src/brain.rs`, `Overlay`, `ConfigView`, `logs.rs`, `RunStats`, `FeedArticle`, `DefaultsRole`, `AtlasArticleRow`, `ProfileView`, `.memory`, `summary_card.rs`, `ModuleId`, `recon/graph.rs`, `WorkEvent`, `rows_for`, `.job_source`, `briefing_view.rs`, `ConfigTab`, `LaunchState`, `SettingsFile`?**
  _High betweenness centrality (0.082) - this node is a cross-community bridge._
- **What connects `$schema`, `default_agent`, `subagent_depth` to the rest of the system?**
  _1796 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `land.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.0076045627376425855 - nodes in this community are weakly interconnected._
- **Why does `Value` connect `Value` to `orchestrate.rs`, `atlas_memory.rs`, `tool_io.rs`, `ui.rs`, `recon.rs`, `accept_claims`, `ToolResult`, `IntelligenceCategory`, `TaskStatus`, `wikipedia_rsp.rs`, `directives.rs`, `DecisionState`, `serde_json`, `execute_steps`, `atlas_news.rs`, `results.rs`, `TelemetryEvent`, `SerpOutcome`, `ProfileSnapshot`, `news_legal.rs`, `whoxy.rs`, `osint.rs`, `holehe/mod.rs`, `telemetry.rs`, `InvestigationPart`, `body.rs`, `provider.rs`, `Supplied<T>`, `investigation.rs`, `provider_attempt.rs`, `picker.rs`, `ReportMode`, `grok_oauth.rs`, `model_exec.rs`, `config_transfer.rs`, `Binding`, `brain_resources.rs`, `cli.rs`, `atlas_insights.rs`, `body_filter.rs`, `synthesize.rs`, `summarization.rs`, `search_engines.rs`, `.new`, `RunStats`, `src/evidence.rs`, `ToolRunner`, `anyhow`, `AtlasArticleRow`, `AtlasEvent`, `dork_generator.rs`, `ProfileView`, `Category`, `search_engines/tests.rs`, `ReconLimits`, `run_turn`, `parse_serp_response`, `components.rs`, `ServiceResult`, `PlanCall`, `subscription.rs`, `BrainResourceSummary`, `config_transfer_tests.rs`, `Severity`?**
  _High betweenness centrality (0.052) - this node is a cross-community bridge._
- **Should `orchestrate.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.041601501407569595 - nodes in this community are weakly interconnected._
- **Why does `FieldId` connect `FieldId` to `app.rs`, `Target`, `Rect`, `App`, `draw_field`, `DefaultsRole`, `.handle_key`?**
  _High betweenness centrality (0.037) - this node is a cross-community bridge._
- **Should `snapshot` be split into smaller, more focused modules?**
  _Cohesion score 0.10984848484848485 - nodes in this community are weakly interconnected._