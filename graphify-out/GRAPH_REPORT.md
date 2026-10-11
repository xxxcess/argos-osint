# Graph Report - argos-osint  (2026-10-11)

## Corpus Check
- 269 files · ~596,506 words
- Verdict: corpus is large enough that graph structure adds value.
- Unclassified: 5 file(s) not represented in the graph (top: (none) 3, .toml 2)

## Summary
- 8649 nodes · 21469 edges · 300 communities (263 shown, 37 thin omitted)
- Extraction: 98% EXTRACTED · 2% INFERRED · 0% AMBIGUOUS · INFERRED: 387 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `bc57de6e`
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
- serde_json
- job_registry.rs
- Value
- directives.rs
- article_image.rs
- Binding
- providers.rs
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
- .default
- TelemetryEvent
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
- synthesize
- brain_resources.rs
- worker.rs
- ledger.rs
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
- atlas_table.rs
- .clear
- search_engines.rs
- Severity
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
- logs.rs
- summary_card.rs
- explore.rs
- .new_thread
- Region
- diversity.rs
- .default
- Category
- Frame
- search_engines/tests.rs
- profile_config.rs
- F
- Architecture
- WorkEvent
- store.rs
- ClockSet
- ProviderAdmission
- components.rs
- normalize_destination
- release_check.mjs
- ServiceResult
- briefing_view.rs
- subscription.rs
- profile.rs
- Service
- recon/graph.rs
- tasks.rs
- ReconCommand
- config_transfer_tests.rs
- run
- AuthFile
- ServiceSpec
- TurnClock
- opencode.json
- Functional Requirements
- ProfileView
- super
- ProviderSecret
- DateTime
- OsintCommand
- source_sha256
- ToolResult
- Argos OSINT — Agent Instructions
- complete
- absorb_hit
- wikipedia_rsp.rs
- intel-brief-image-column-2026-10-11/manifest.json
- DecisionState
- provider_diag.rs
- execute_steps
- Rect
- QueryExecutionStatus
- results.rs
- .index_now
- SerpOutcome
- WidgetSpec
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
- ModuleId
- atlas_work.rs
- model_roles.rs
- ModelAssignment
- Codebase Map — argos-osint
- PROJECT.md — argos-osint
- Current Phase State
- note_run
- RecoveryAction
- grok_oauth.rs
- tool_runner.rs
- Conventions
- Docs Ingest — argos-osint
- ScheduledCall
- config_transfer.rs
- OpenCode V2 workflow
- ui.rs
- Concepts
- package.json
- .is_empty
- TUI components
- result
- Providers
- Decision roles and output contracts
- LayoutResult
- Terminal UI
- docs/README.md
- Profile dashboard and search
- RunStats
- brain_lance_off.rs
- extract_items
- .handle_key
- .order
- spotify.rs
- Unified Investigation Harness
- RendererKind
- ConfigView
- TurnContinuation
- Argos OSINT
- ToolOutcome
- EventKind
- Argos documentation
- .drop
- accept_claims
- Argos UI Interaction Audit
- ChainReport<T>
- KeptClaim
- Json
- run_primary
- IndexOutcome
- model_facts
- profile_components.rs
- manifest.json
- StatFilters
- parse_serp_response
- modes.rs
- fetch_once
- provider_metrics.rs
- Trigger
- ImportBuffer
- .recon_outcomes
- BrainListMode
- SettingsFile
- AtlasInsightClaim
- search_engines_fixtures.rs
- PickReply
- RouteInput
- graph_explanation/tests.rs
- SystemTab
- .intel_reports
- §9 implementation order
- serde
- QuestionSpec
- actor_review.rs
- run_live
- PlanInterval
- result_cache
- Target
- .job_source
- TaskState
- .reload_memories
- OperationScope<'a>
- Profile analytics dashboard
- CheckSignal
- role_secret
- Profile TUI implementation session — 2026-10-10
- Diagram conventions
- PlanCall
- AtlasPage
- TUI implementation and verification
- JobStatusFilter
- Unified reliability / summarization / semantic pipelines — completion checklist
- select_strategy
- enqueue_job
- poll_device
- LayoutRegistry
- draw_field
- intel_recon/brain.rs
- TUI overhaul verification — 10 October 2026
- Store
- TUI design contract
- Versioning, commits and releases
- CredentialSource
- pull_request_template.md
- release-please-config.json
- a_recon_model_429_trips_the_gate_and_later_calls_skip_the_provider
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
- `Other application presets` --references--> `draw_recon_chat()`  [EXTRACTED]
  docs/tui-design-spec.md → crates/argos-osint-bin/src/tui/ui.rs

## Import Cycles
- 1-file cycle: `crates/argos-osint-core/src/config_transfer.rs -> crates/argos-osint-core/src/config_transfer.rs`
- 2-file cycle: `crates/argos-osint-core/src/osint.rs -> crates/argos-osint-core/src/osint/results.rs -> crates/argos-osint-core/src/osint.rs`
- 2-file cycle: `crates/argos-osint-bin/src/cli.rs -> crates/argos-osint-core/src/hardware.rs -> crates/argos-osint-bin/src/cli.rs`
- 4-file cycle: `crates/argos-osint-core/src/config_commit.rs -> crates/argos-osint-core/src/provider_attempt.rs -> crates/argos-osint-core/src/provider_diag.rs -> crates/argos-osint-core/src/config_transfer.rs -> crates/argos-osint-core/src/config_commit.rs`

## Communities (300 total, 37 thin omitted)

### Community 0 - "land.rs"
Cohesion: 0.01
Nodes (262): CODES, LABELS, R0, R1, R10, R100, R101, R102 (+254 more)

### Community 1 - "orchestrate.rs"
Cohesion: 0.05
Nodes (59): a_follow_up_keeps_names_from_the_previous_synthesis(), a_handle_named_in_a_derived_question_is_an_unverified_binding(), a_keyword_inside_the_subjects_name_runs_no_news_or_legal_tool(), a_keyword_outside_the_subjects_name_still_runs_news_or_legal(), ac2_without_a_key_news_and_legal_tools_are_never_picked_and_with_a_key_they_are_eligible(), ac4_a_news_prompt_runs_newsapi_search_with_the_entity_and_who_is_runs_none(), ac5_a_sued_prompt_runs_courtlistener_case_or_docket_search_with_the_entity(), ac6_caps_hold_and_a_courtlistener_429_skips_the_rest() (+51 more)

### Community 2 - "telemetry.rs"
Cohesion: 0.13
Nodes (21): bin_index(), Coverage, duration_bins_are_bounded_and_mergeable(), DURATION_BINS_MS, ensure_observed_since(), event_bins(), get_meta(), ID_SEQ (+13 more)

### Community 3 - "atlas_memory.rs"
Cohesion: 0.05
Nodes (96): atlas_memory_count(), background_indexing_then_refresh_upgrades_a_partial_run(), Barrier, Checkpoint, CheckpointInput, claim(), claims(), clear() (+88 more)

### Community 4 - "tool_io.rs"
Cohesion: 0.04
Nodes (96): PROMPT_TARGETS, normalize_platform(), question_bindings(), a_gap_filler_only_domain_waits_for_a_primary_observation_before_hunter(), accept_bindings(), batch_scrape_takes_ranked_urls_and_gates_order_before_hunter(), bind_arguments(), BINDING_KINDS (+88 more)

### Community 5 - "app.rs"
Cohesion: 0.06
Nodes (94): a_click_folds_an_event_and_incoming_entries_do_not_move_a_paused_list(), all_provider_actions_render_with_hit_areas_at_80x24(), analytics_fixture(), analytics_viewports_and_data_states(), ATLAS_AUTO_SECS, atlas_auto_toggle_persists_the_next_trigger(), atlas_claim(), atlas_headlines_scroll_and_request_failures_reach_the_system_log() (+86 more)

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
Nodes (67): ac6_citation_groups_split_validate_each_id_and_normalize(), answer_step_rate_limit_is_explicit_and_keeps_results(), attribute_word(), auxiliary(), BRIEF_SYNTHESIS, broad_question_collapses_to_grounded_lookups(), char_ceil(), char_floor() (+59 more)

### Community 10 - "ButtonId"
Cohesion: 0.02
Nodes (108): ButtonId, Add, AddFallback, AtlasAuto, AtlasDelete, AtlasLive, AtlasNews, AtlasNewsFeed (+100 more)

### Community 11 - "Store"
Cohesion: 0.08
Nodes (5): CreditHold, Store, strategy_and_provider_credits_survive_reopen(), Thread, whoxy_prepaid_pool_does_not_reset_monthly()

### Community 12 - "serde_json"
Cohesion: 0.08
Nodes (26): test_all_catalog_tools_mapped_to_categories(), cache_follows_the_provider_plan_interval(), all_catalog_tools_mapped_to_categories(), argument_contract(), ArgumentBuilderContract, compact_capability_catalog(), CompactToolCapability, IntelligenceCategory (+18 more)

### Community 13 - "job_registry.rs"
Cohesion: 0.05
Nodes (49): begin(), begin_cancellable(), db(), db_path(), finish(), register_canonical(), beat(), BEAT_INTERVAL (+41 more)

### Community 14 - "Value"
Cohesion: 0.34
Nodes (25): array_value(), check_known_fields(), check_limit(), check_required(), check_role_compatibility(), check_route_reference(), check_scheme(), child() (+17 more)

### Community 15 - "directives.rs"
Cohesion: 0.06
Nodes (63): accounts_search_query(), binder_maps_kinds_to_inputs_and_never_hands_hunter_a_social_host(), binding(), derived_question_handles(), apply_context_targets(), asks(), asks_about_judge(), asks_for_headlines() (+55 more)

### Community 16 - "article_image.rs"
Cohesion: 0.09
Nodes (22): absent_invalid_failed_images_collapse(), ArticleImages, centered_image_rect(), Completion, Decoded, Encoded, decode(), decode_preserves_source_resolution_and_pixels() (+14 more)

### Community 17 - "Binding"
Cohesion: 0.13
Nodes (31): leftovers(), names(), Binding, canonical(), dependency_order(), depends_on(), question_bindings_and_dependency_fix(), allowed_producer() (+23 more)

### Community 18 - "providers.rs"
Cohesion: 0.04
Nodes (80): annotate(), bitcoin(), bounded(), clip_page(), domain(), email_address(), https_on_host(), ip() (+72 more)

### Community 19 - "atlas_news.rs"
Cohesion: 0.07
Nodes (44): api_error(), ATLAS_TOOLS, authors(), clip(), country_arg(), country_code(), CURRENTS_COUNTRIES, CURRENTS_HOST (+36 more)

### Community 20 - "Store"
Cohesion: 0.06
Nodes (25): article_body_from_row(), ArticleBodyRow, cited_evidence_keeps_provenance_and_revision_identity(), element_from_row(), full_assessment_parents_are_flagged_for_child_report_totals(), IntelAssessmentRow, IntelInvestigationRow, IntelReportSectionRow (+17 more)

### Community 21 - "publication.rs"
Cohesion: 0.09
Nodes (41): active_index_rows(), bump_memories_changed(), change_watcher_sees_only_committed_changes_and_coalesces(), claim(), clear(), clear_index_state(), CoverageReport, delete_run_memory_state() (+33 more)

### Community 22 - "TaskStatus"
Cohesion: 0.07
Nodes (15): TaskRecord, TaskStatus, Cancelled, Completed, Deferred, Failed, Partial, Planned (+7 more)

### Community 23 - "whoxy.rs"
Cohesion: 0.11
Nodes (35): adjacent_changes(), AdjacentChange, balance_request_url(), bounded_model_view(), check_balance(), contact(), ContactCard, date_and_limit_validation() (+27 more)

### Community 24 - "news_legal.rs"
Cohesion: 0.06
Nodes (46): a_rate_limit_switches_to_the_fallback_key_and_a_rejection_does_not(), ac1_catalog_has_55_tools_and_the_news_and_legal_entries(), ac1_each_request_has_its_url_params_and_auth_header_and_no_key_in_the_url(), an_echoed_key_is_redacted_from_the_stored_body_and_error(), article(), articles(), CATEGORIES, choice_arg() (+38 more)

### Community 25 - "hardware.rs"
Cohesion: 0.14
Nodes (18): system_host_lines(), CACHE_TTL_SECS, classify_arch(), disk_totals(), HardwareProfile, metal_vram_gb(), now_secs(), parse_apple_gpu_cores() (+10 more)

### Community 26 - "App"
Cohesion: 0.08
Nodes (66): atlas_cycle_stats_room(), atlas_cycle_stats_scroll_max(), atlas_feed_room(), atlas_feed_room_for(), atlas_live_areas(), atlas_news_room(), atlas_news_room_for(), atlas_runs_room() (+58 more)

### Community 27 - "Store"
Cohesion: 0.05
Nodes (13): atlas_answer_id(), atlas_brief_id(), AtlasRunRow, AtlasStoredClaim, fingerprint_mismatch_rebuilds_and_drift_is_reconciled(), GraphSummary, has_table(), intel_link_explanations_round_trip_and_cleanup() (+5 more)

### Community 28 - "embed.rs"
Cohesion: 0.07
Nodes (33): record_start(), start_repair(), active(), DIM, disable(), disabled(), DisableGuard, download_file() (+25 more)

### Community 29 - "osint.rs"
Cohesion: 0.05
Nodes (87): a_claimed_email_keeps_no_person_data(), bind_request(), body_cap(), bounded_recovery_is_offered_once_for_an_unrecognised_page_only(), CACHE_DAY_SECONDS, cache_identity(), CACHE_MONTH_SECONDS, cache_seconds() (+79 more)

### Community 30 - "holehe/mod.rs"
Cohesion: 0.10
Nodes (21): CACHE_TTL, cap_reports_omitted(), DEFAULT_MAX_SITES, default_selection_uses_implemented_adapters(), email_hash(), email_preserves_local_part(), GLOBAL_CONCURRENCY, lookup() (+13 more)

### Community 31 - "config_commit.rs"
Cohesion: 0.06
Nodes (73): a_batch_with_a_duplicate_or_empty_slot_is_refused_before_any_write(), a_committed_journal_needs_no_recovery(), a_second_lock_is_refused_while_one_is_held(), a_stale_lock_is_taken_over(), an_abandoned_unfinished_lockfile_is_recovered(), backup_path(), canonical_root(), commit_files_is_all_or_nothing() (+65 more)

### Community 32 - ".default"
Cohesion: 0.11
Nodes (20): a_blank_osint_user_agent_loads_as_unset(), default_credit_reset(), default_firecrawl_credits(), default_hunter_credits(), default_max_calls(), default_max_rounds(), default_one_cost(), default_opening_cap() (+12 more)

### Community 33 - "TelemetryEvent"
Cohesion: 0.13
Nodes (10): clamp(), coverage_reports_observed_since_and_counts(), db(), events_persist_and_roll_up_once(), id_is_stable_so_replay_never_double_counts(), logical_attempt_and_cache_rows_stay_distinct_under_replay(), record(), safe_payload() (+2 more)

### Community 34 - "Call"
Cohesion: 0.07
Nodes (35): event_to_chat_block(), is_thinking_expanded(), InvestigationPart, DirectiveAssessment, EvidencePassage, GateValidation, Handoff, Plan (+27 more)

### Community 35 - "body.rs"
Cohesion: 0.09
Nodes (34): body_fetch_routes(), BodyFetchEvent, Attempt, Failed, InsightsFailed, InsightsRefreshing, InsightsReplaced, Progress (+26 more)

### Community 36 - "provider.rs"
Cohesion: 0.06
Nodes (29): concrete_free_models(), decide(), DecisionAnswer, decisions_client_posts_to_alpha_decisions_and_parses_answers(), DECISIONS_MODELS, decisions_url(), DecisionsResponse, default_grok_model() (+21 more)

### Community 37 - "brain_detail.rs"
Cohesion: 0.06
Nodes (50): areas(), areas_for(), BrainDetail, DetailAreas, DetailPane, Path, Related, Summary (+42 more)

### Community 38 - "profile_stats.rs"
Cohesion: 0.06
Nodes (59): atlas_carries_data(), AtlasStats, BacklogRow, band_for(), CategoryTrigger, classify_tool_cause(), CONFIDENCE_BANDS, ConfidenceBand (+51 more)

### Community 39 - "atlas.rs"
Cohesion: 0.09
Nodes (29): article_card_labels_title_publisher_author_and_classification(), Band, band_sizes(), band_temperature(), category_from_decisions(), CATEGORY_IDS, CLUSTERS, country_label() (+21 more)

### Community 40 - "run_atlas_inner"
Cohesion: 0.08
Nodes (38): article_from_row(), AtlasEvent, Article, Classified, Fault, InsightProgress, MemoriesChanged, MemoryProgress (+30 more)

### Community 41 - "whatsmyname.rs"
Cohesion: 0.05
Nodes (54): D, AccountTuple, ACTIVE_SNAPSHOT, ADAPTER_VERSION, apply_strip_bad_char(), benchmark_14_4_parse_index_and_selection(), cache_get(), cache_set() (+46 more)

### Community 42 - "scheduler.rs"
Cohesion: 0.09
Nodes (22): apply_index_change(), apply_index_change_rebuild_reports_outcome(), apply_index_with(), DEFAULT_LEASE_SECS, drain_index_once(), drain_summary_flush(), drain_summary_flush_cached(), drain_summary_flush_completes_cached_tasks() (+14 more)

### Community 43 - "jobs_view.rs"
Cohesion: 0.09
Nodes (23): summary_text(), active_time_is_live_for_open_attempts_and_unavailable_only_without_a_start(), AttemptRow, event_apps(), event_counts(), EventCounts, get_job(), job() (+15 more)

### Community 44 - "provider_attempt.rs"
Cohesion: 0.11
Nodes (35): Acc, attempt(), attempt_with_observer(), AttemptReport, check_final(), complete_stream_and_json_answers_succeed(), Deadlines, delta() (+27 more)

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
Nodes (25): areas(), BASE_COLUMNS, button_label(), buttons(), detail_lines(), hit(), JobsAreas, JobsView (+17 more)

### Community 51 - "App"
Cohesion: 0.03
Nodes (26): a_failed_turn_keeps_the_streamed_answer_on_screen(), a_log_or_late_tool_row_does_not_drop_the_live_answer(), App, atlas_log_level(), flush_streams(), IntelPage, Briefing, Bulletin (+18 more)

### Community 52 - "events.rs"
Cohesion: 0.12
Nodes (19): clear_events(), DEFAULT_RETENTION_HOURS, event_row(), EventRow, get_event(), job_filter_includes_descendants_and_prune_keeps_jobs(), list_events(), MAX_DETAIL_CHARS (+11 more)

### Community 53 - "exec.rs"
Cohesion: 0.14
Nodes (20): admission_account(), admission_contention_consumes_no_attempt(), AttemptEvent, Finished, Started, AttemptLog, cancelled(), complete_summary_report() (+12 more)

### Community 54 - "contains"
Cohesion: 0.07
Nodes (55): abs_contains(), abs_rect(), AbsRect, api_key_slot(), ApiKeySlot, article_image_area(), bulletin_image_preview(), contains() (+47 more)

### Community 55 - "model_exec.rs"
Cohesion: 0.05
Nodes (65): AttemptRecord, cancellable_sleep(), cancellation_stops_the_chain(), cancelled(), ChainReport, DispatchError, execute(), ExecuteOptions (+57 more)

### Community 56 - "ErrorCategory"
Cohesion: 0.08
Nodes (24): operation_kind(), backoff_delay(), can_retry(), ErrorCategory, AuthOrQuota, CancelledOrStale, ConfigurationMissing, ContextLimit (+16 more)

### Community 57 - "JobRow"
Cohesion: 0.29
Nodes (3): job_row(), JobRow, parse()

### Community 58 - "MemoryKind"
Cohesion: 0.09
Nodes (22): AdmissionPolicy, admit_memory(), BrainQuery, pack_context(), AssessmentState, Contradicts, Insufficient, Mentions (+14 more)

### Community 59 - "synthesize"
Cohesion: 0.24
Nodes (17): a_dropped_provider_stream_keeps_the_text_already_received(), a_provider_that_rejects_streaming_fails_without_a_hidden_retry(), a_streamed_answer_with_a_bad_citation_ends_on_the_repaired_answer(), an_idle_stall_or_the_ceiling_keeps_the_partial_answer(), cancel_during_synthesis_marks_the_run_cancelled_and_keeps_partial_text(), chat_server(), ChatReply, DropAfter (+9 more)

### Community 60 - "brain_resources.rs"
Cohesion: 0.09
Nodes (26): source_anchor_label(), BRAIN_SCRAPE_PREFIX, CLAIM_CHARS, classify_resource(), file_link_url(), hit(), is_brain_binding(), is_brain_scrape_pick() (+18 more)

### Community 61 - "worker.rs"
Cohesion: 0.15
Nodes (19): IntelReportEvent, InsightsUpdated, JobCreated, JobDone, Section, Stage, cancel_if_needed(), claim_relevant_evidence() (+11 more)

### Community 62 - "ledger.rs"
Cohesion: 0.13
Nodes (13): body_assertion_candidates(), coverage_complete(), ElementStatus, Assessed, Excluded, InProgress, Pending, Superseded (+5 more)

### Community 63 - "src/brain.rs"
Cohesion: 0.12
Nodes (21): AGREEMENT_WEIGHT, CATEGORIES, category_hint(), format_injection(), hybrid_recall(), hybrid_recall_adds_paraphrases_and_rewards_agreement(), identity_query_prefers_name_memory(), jaccard() (+13 more)

### Community 64 - "intel_recon/jobs.rs"
Cohesion: 0.13
Nodes (20): active_job_for_mode(), cancel_job(), create_job_inserts_all_verify_sections(), create_report_job(), finish_canonical(), intel_recon_links_its_legacy_job_to_one_canonical_registry_job(), pause_job(), ReportScope (+12 more)

### Community 65 - "cli.rs"
Cohesion: 0.15
Nodes (21): ask(), ask_thread(), atlas_command(), defaults_command(), defaults_json(), defaults_show_includes_the_tool_picker_transport(), DefaultsCommand, Set (+13 more)

### Community 66 - "atlas_insights.rs"
Cohesion: 0.07
Nodes (50): a_country_token_does_not_merge_into_a_longer_name(), a_decisions_model_does_not_extract_claims(), aliases_merge_on_shared_tokens_when_the_longer_form_is_a_span(), BODY_CLAIM_LIMIT, BODY_SPAN_CHARS, brief_rating_mean(), COL_CLASS, COL_ENTITY (+42 more)

### Community 67 - "SourceReliability"
Cohesion: 0.09
Nodes (22): article_information_credibility(), AdmiraltyCode, best_credibility(), CredibilityInputs, information_credibility(), InformationCredibility, CannotBeJudged, Confirmed (+14 more)

### Community 68 - ".new"
Cohesion: 0.16
Nodes (30): a_snapshot_never_panics_on_a_partially_migrated_store(), active_jobs(), an_empty_database_returns_empty_sections_not_zeros(), atlas_raw_candidate_occurrences_are_not_added_twice(), count_rows(), distinct_article_body_cohort_deduplicates_tags_and_excludes_empty_cache_rows(), distinct_values(), durable_model_rows_and_supplemental_telemetry_count_once() (+22 more)

### Community 69 - "body_filter.rs"
Cohesion: 0.18
Nodes (22): BATCH_SIZE, BodyChunk, chunk_article_body(), CHUNK_MAX, CHUNK_MIN, CHUNK_TARGET, chunks_preserve_absolute_offsets(), classify_chunk_batch() (+14 more)

### Community 70 - "synthesize.rs"
Cohesion: 0.16
Nodes (20): IntelEvidenceRow, BODY_REFINE_INPUT_CHARS, deterministic_bluf_mentions_title(), deterministic_section(), model_refine_body(), model_synthesize(), parse_json_object(), refine_retrieved_article_body() (+12 more)

### Community 71 - "dataset.rs"
Cohesion: 0.15
Nodes (21): acquire_refresh_lease(), active_manifest_path(), ACTIVE_REFRESHES, dataset_root(), DatasetManifest, DatasetStatus, get_status(), import_from_file() (+13 more)

### Community 72 - "Loaded"
Cohesion: 0.14
Nodes (16): add_cycle_outcome(), atlas_origin_rows(), category_triggers(), CycleOutcomeBucket, Dims, engine_health(), EventRow, evidence_call() (+8 more)

### Community 73 - "profile_charts.rs"
Cohesion: 0.06
Nodes (64): absolute_stacks_compare_one_and_one_hundred_on_one_scale(), BLOCKS, cap_eighths(), cells(), coarsen_counts(), column_cell(), connect_cells(), count_coarsening_preserves_totals_boundaries_and_unknown_history() (+56 more)

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

### Community 78 - "atlas_table.rs"
Cohesion: 0.17
Nodes (22): border_style(), borders_and_cells_have_matching_display_widths(), bottom_border_line(), char_width(), clip_to_width(), column_widths(), columns_consume_inner_width(), columns_distribute_proportionally_when_not_flex_last() (+14 more)

### Community 79 - ".clear"
Cohesion: 0.10
Nodes (10): draft_isolation_and_persistence(), duplicate_submission_prevention(), home_order_renames_and_nine_routes_agree(), PaletteItem, pruned_history_closes_the_open_run(), session_tabs_open_close_reopen(), single_action_launch_from_home(), tab_strip_render_and_hit_test() (+2 more)

### Community 80 - "search_engines.rs"
Cohesion: 0.08
Nodes (35): build_scrape_body(), Candidate, card_snippet(), classify_status_region(), clip_item_text(), collapse_ws(), collect_candidates(), contains_phrase() (+27 more)

### Community 81 - "Severity"
Cohesion: 0.10
Nodes (16): starts(), infer_app(), LevelFilter, All, Error, Info, Warn, session_event() (+8 more)

### Community 82 - ".new"
Cohesion: 0.20
Nodes (22): a_spent_primary_quota_uses_the_fallback_key(), classification_request(), country_labels_show_the_name_and_the_code(), daily_cap(), dedup_drops_a_url_an_exact_title_and_keeps_the_higher_domain(), dedup_ignores_a_better_source_when_the_prior_hit_is_outside_this_run(), format_run_card(), HttpCall (+14 more)

### Community 83 - "FeedArticle"
Cohesion: 0.17
Nodes (19): a_resumed_cycle_continues_candidate_numbering(), apply_hits(), article_from(), article_row(), canonical_url(), category_tag(), FeedArticle, host_of() (+11 more)

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
Cohesion: 0.06
Nodes (19): ChoiceItem, codex_models(), DefaultsRole, .ALL, ClaimAssessor, Classifier, EntityResolver, EvidenceCurator (+11 more)

### Community 88 - "graph_explanation.rs"
Cohesion: 0.06
Nodes (25): basic_explanation(), BASIC_HEADING, Cached, None, Stale, Valid, FAILURE_COOLDOWN, Gate (+17 more)

### Community 89 - "src/evidence.rs"
Cohesion: 0.09
Nodes (29): AGREEMENT_WEIGHT, AnnMeasurement, AnnPolicy, AnnDeferredUnmeasured, AnnEnabled, ExactSearch, AnnThresholds, chunk_text() (+21 more)

### Community 90 - "ToolRunner"
Cohesion: 0.13
Nodes (9): attribution_is_explicit_never_inferred_from_the_prompt(), test_tool_runner_cache_hit(), test_tool_runner_concurrency_and_dedup(), ToolAttribution, ToolRunner, .MAX_CONCURRENCY, .REQUEST_TIMEOUT, .SERP_VALID_CACHE_SECONDS (+1 more)

### Community 91 - "anyhow"
Cohesion: 0.06
Nodes (50): compile_general_model_prompt(), compile_native(), DecisionAdapterKind, JsonMode, NativeDecisions, OutputTool, StrictJsonSchema, ValidatedText (+42 more)

### Community 92 - "AtlasArticleRow"
Cohesion: 0.15
Nodes (26): accept_one(), apply_peer_support(), article_with_body_spans(), articles_have_span(), body_lead_prompt(), catalog_json(), classifier_peers_are_preferred_over_token_overlap(), classify_peers_chat() (+18 more)

### Community 93 - "BrainResourceSummary"
Cohesion: 0.20
Nodes (10): bindings_use_brain_evidence_ids(), BrainResourceHit, BrainResourceSummary, candidate_json(), clip(), format_counts(), is_http_url(), parse_brain_scrape_index() (+2 more)

### Community 94 - "dork_generator.rs"
Cohesion: 0.09
Nodes (33): ACTIVE_SNAPSHOT, compose_single_query(), compute_query_id(), compute_template_id(), DATASET_NAME, DEFAULT_MAX_QUERIES, DorkCatalog, DorkCategory (+25 more)

### Community 95 - "Section"
Cohesion: 0.18
Nodes (8): every_section_registers_its_reviewed_widget_count(), Section, Atlas, Intel, Models, Recon, Tools, widgets_render_in_the_specs_presentation_priority_order()

### Community 96 - "How"
Cohesion: 0.11
Nodes (18): How, CompanyEmail, Coordinates, DomainAsUrl, EntityPhrase, PackageName, PackageParts, Plain (+10 more)

### Community 97 - "logs.rs"
Cohesion: 0.12
Nodes (16): areas(), button_label(), buttons(), count(), draw(), hit(), in_list(), list_geometry() (+8 more)

### Community 98 - "summary_card.rs"
Cohesion: 0.29
Nodes (11): actions(), button_at(), button_cells(), button_row_area(), draw(), height(), is_card_button(), label() (+3 more)

### Community 99 - "explore.rs"
Cohesion: 0.20
Nodes (11): GraphEdgeKind, ContradictionOrUpdate, Evidenced, Inferred, SemanticSuggestion, related_evidence_view(), RelatedEvidenceHit, semantic_edges_are_not_factual() (+3 more)

### Community 100 - ".new_thread"
Cohesion: 0.15
Nodes (14): answer_evidence_survives_reopen_and_deleted_thread_rejects_late_calls(), claims_deduplicate_and_reject_unsupported_sources(), deleting_an_investigation_removes_its_brain_memories_and_graph_summary(), investigation_memory_ids(), Message, now(), only_a_failed_or_interrupted_latest_run_is_resumable(), persist_claims() (+6 more)

### Community 101 - "Region"
Cohesion: 0.11
Nodes (19): Region, AtlasInsights, AtlasNews, AtlasOrigins, AtlasRuns, Chat, Detail, IntelBrief (+11 more)

### Community 102 - "diversity.rs"
Cohesion: 0.13
Nodes (25): ReconLimits, coverage_gaps_summary(), coverage_record_id(), coverage_stats(), CoverageCandidate, CoverageRecord, CoverageStats, covered_record() (+17 more)

### Community 103 - ".default"
Cohesion: 0.19
Nodes (34): a_429_stops_picker_calls_and_the_fallback_finishes_the_list(), a_dispatch_error_fails_the_step_and_the_loop_goes_on(), a_failed_email_finder_gets_one_fallback_and_no_third_picker_call(), a_fallback_with_no_bindings_still_runs_the_satisfied_steps_and_reaches_synthesis(), a_goal_naming_a_tool_or_provider_is_repaired_then_falls_back(), a_goal_naming_a_tool_outside_the_prompt_entity_is_still_rejected(), a_prompt_entity_named_like_a_provider_keeps_the_models_directives(), a_question_handle_fills_the_handle_steps_without_a_fallback() (+26 more)

### Community 104 - "Category"
Cohesion: 0.08
Nodes (21): Category, Auth, Cancelled, Configuration, Empty, InvalidModel, InvalidResult, MalformedPayload (+13 more)

### Community 105 - "Frame"
Cohesion: 0.09
Nodes (58): intel_category_short(), draw(), active_popup_area(), add_fallback_popup_area(), atlas_auto_label(), atlas_extracting(), atlas_run_card(), atlas_run_label() (+50 more)

### Community 106 - "search_engines/tests.rs"
Cohesion: 0.10
Nodes (42): build_serp_url(), a_non_serp_path_on_the_engine_host_is_not_a_serp_page(), a_noscript_wrapped_consent_meta_refresh_is_consent(), a_single_retry_is_allowed_only_for_a_parser_mismatch_with_a_dom(), a_standalone_zero_count_in_a_status_region_is_a_verified_zero(), a_zero_phrase_outside_a_status_region_is_not_a_verified_zero(), an_off_host_final_url_with_cards_keeps_the_items_but_is_not_valid(), an_off_host_final_url_without_cards_is_never_a_zero() (+34 more)

### Community 107 - "profile_config.rs"
Cohesion: 0.13
Nodes (11): a_validation_error_reports_a_pointer_and_never_a_value(), an_oversize_document_is_refused_before_anything_is_applied(), confirmed_export_overwrites_the_same_destination(), direct_buffer_change_invalidates_commit_revision(), draw(), EditorError, locate(), SECRET_WARNING (+3 more)

### Community 108 - "F"
Cohesion: 0.23
Nodes (13): CallSpec, dispatch(), fetch_with_spare_key(), json_message(), phase1_call(), phase2_call(), pretty_body(), provider_fault() (+5 more)

### Community 109 - "Architecture"
Cohesion: 0.10
Nodes (21): Admiralty source evaluation (WP:RSP), Architecture, Atlas, Binder and executor, Brain recall, Config transfer and the commit, Directives, Intel reports (+13 more)

### Community 110 - "WorkEvent"
Cohesion: 0.06
Nodes (29): atlas_auto_future_tick_waits_and_past_tick_reschedules(), atlas_auto_while_running_only_arms_the_next_slot(), atlas_history_button_counts_down_while_auto_run_is_on(), atlas_poll_wait(), manual_run_moves_the_next_auto_trigger_out_by_90_minutes(), unix_now(), work_event(), WorkEvent (+21 more)

### Community 111 - "store.rs"
Cohesion: 0.07
Nodes (33): rows(), article(), atlas_article_from_row(), atlas_articles_for_intel_dedupes_same_article_id_across_runs(), atlas_articles_for_intel_filters_category_day_and_query(), atlas_auto_next_round_trips_through_app_state(), atlas_claims_for_article_returns_linked_claims(), atlas_prune_drops_finished_runs_older_than_the_cutoff() (+25 more)

### Community 112 - "ClockSet"
Cohesion: 0.21
Nodes (3): ClockSet, queue_time_does_not_spend_active_but_consumes_wall(), sequential_vs_overlapping_tool_budgets()

### Community 113 - "ProviderAdmission"
Cohesion: 0.18
Nodes (6): shared_rate_limit_cooldown_blocks_admission(), AdmissionGuard, note_shared_rate_limit(), provider_admission_caps_at_two(), ProviderAdmission, rate_limit_cooldown_blocks_acquire()

### Community 114 - "components.rs"
Cohesion: 0.19
Nodes (13): action_rects(), analytics_card(), clip_text(), compact_records_keep_trailing_numeric_values(), detail_records(), detail_table(), editor_height(), measured_lines() (+5 more)

### Community 115 - "normalize_destination"
Cohesion: 0.21
Nodes (14): detect_interstitial(), host_in_any(), host_is(), host_is_captcha(), host_is_consent(), is_ad_host(), is_private_host(), meta_refresh_is_consent() (+6 more)

### Community 116 - "release_check.mjs"
Cohesion: 0.09
Nodes (23): core, home, hooks, names, payload(), run(), setup(), root (+15 more)

### Community 117 - "ServiceResult"
Cohesion: 0.22
Nodes (11): cache_key(), cached_result(), cancel_marks_inconclusive(), check_implemented(), classify_http(), parse_json(), ServiceResult, store_cache() (+3 more)

### Community 118 - "briefing_view.rs"
Cohesion: 0.21
Nodes (18): bucket_extracted(), bucket_extracted_with_explanations(), buckets_split_facts_inferences_context_and_links(), claim(), claim_label(), explain_intel_links_with_model(), explain_relation_link(), ExtractedBuckets (+10 more)

### Community 119 - "subscription.rs"
Cohesion: 0.24
Nodes (8): answer_event(), check_login(), command(), complete(), completion_is_ephemeral_and_does_not_inherit_tools_or_configuration(), exec_args(), is_subscription_status(), login()

### Community 120 - "profile.rs"
Cohesion: 0.14
Nodes (45): amplification_lines(), ATTENTION, attribution_lines(), backlog_lines(), capacity_lines(), cause_lines(), compare_cells(), confidence_lines() (+37 more)

### Community 121 - "Service"
Cohesion: 0.09
Nodes (21): AnswerContext, await_completion(), compact_page_evidence(), directive_goals_line(), finish_recon_job(), credit_map(), deltas(), note_cache() (+13 more)

### Community 122 - "recon/graph.rs"
Cohesion: 0.06
Nodes (58): draw(), draw_path(), draw_summary(), glyph(), inset(), legend_height(), legend_parts(), legend_rows() (+50 more)

### Community 123 - "tasks.rs"
Cohesion: 0.08
Nodes (63): add_missing_columns(), adopt_untracked_index_changes(), block_claimed(), claim_index_changes(), claim_index_work(), claim_next_in(), claim_task(), claim_with() (+55 more)

### Community 124 - "ReconCommand"
Cohesion: 0.17
Nodes (12): ReconCommand, Ask, AskNew, Delete, Limits, List, New, Rename (+4 more)

### Community 125 - "config_transfer_tests.rs"
Cohesion: 0.14
Nodes (39): apply_import(), parse_document(), a_failed_validation_writes_nothing(), a_subscription_account_keeps_its_device_endpoints_through_an_import(), a_syntax_error_reports_line_and_column(), an_error_message_never_echoes_a_key(), an_unresolved_env_reference_warns_instead_of_importing_an_empty_key(), assert_no_staged_files() (+31 more)

### Community 126 - "run"
Cohesion: 0.11
Nodes (13): atlas_countdown_visible(), atlas_extracting_visible(), intel_body_loading_visible(), intel_insights_loading_visible(), intel_summary_loading_visible(), LaunchState, Accepted, Accepting (+5 more)

### Community 127 - "AuthFile"
Cohesion: 0.15
Nodes (8): accounts_persist_with_owner_only_permissions(), assert_no_staged_files(), auth_save_commits_only_the_auth_slot(), AuthFile, legacy_connection_migrates_without_overwriting_other_accounts(), loading_old_auth_removes_gmail_setup_credentials(), saving_through_the_lock_keeps_the_file_owner_only(), write_private()

### Community 128 - "ServiceSpec"
Cohesion: 0.20
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

### Community 132 - "ProfileView"
Cohesion: 0.16
Nodes (9): activate(), capacity_preserves_unknown_disabled_and_overflow_values(), handle_key(), open_run_owner(), open_selected_owner(), ProfileView, remember_row(), select_row() (+1 more)

### Community 133 - "super"
Cohesion: 0.09
Nodes (7): CODES, NAMES, ADAPTER_VERSION, HOST, ID, positive_negative_and_source_field(), request_url()

### Community 134 - "ProviderSecret"
Cohesion: 0.15
Nodes (29): call_cached(), cancelled(), classify_turn_mode(), continue_turn(), derive_directives(), enabled_tools(), execute_ordered(), extract_bindings() (+21 more)

### Community 135 - "DateTime"
Cohesion: 0.10
Nodes (29): adaptive_bucket_seconds(), ArticleFact, AttemptFact, BriefFact, Bucket, bucket_count(), bucket_floor(), bucket_format() (+21 more)

### Community 136 - "OsintCommand"
Cohesion: 0.14
Nodes (14): DatasetCommand, Import, Refresh, Status, OsintCommand, Attach, Dataset, Describe (+6 more)

### Community 137 - "source_sha256"
Cohesion: 0.01
Nodes (222): source_sha256, crates/argos-osint-bin/Cargo.toml, crates/argos-osint-bin/src/cli.rs, crates/argos-osint-bin/src/main.rs, crates/argos-osint-bin/src/tui/app.rs, crates/argos-osint-bin/src/tui/article_image.rs, crates/argos-osint-bin/src/tui/atlas_actions.rs, crates/argos-osint-bin/src/tui/atlas_table.rs (+214 more)

### Community 138 - "ToolResult"
Cohesion: 0.12
Nodes (18): ToolResult, cacheable(), cut_footer(), cut_short_answer(), decode_plan(), Directive, EntityView, evidence_summary() (+10 more)

### Community 139 - "Argos OSINT — Agent Instructions"
Cohesion: 0.10
Nodes (20): Agent Cargo isolation, Architecture notes, Argos OSINT — Agent Instructions, CI (`.github/workflows/ci.yml`), CLI Entry Points (`argos` binary), Developer Commands, diagrams, Environment Variables (+12 more)

### Community 140 - "complete"
Cohesion: 0.12
Nodes (24): authorize(), bearer_token(), catalog_error(), chat_body(), complete(), complete_errors_with_finish_reason_when_response_is_empty(), complete_once(), complete_one() (+16 more)

### Community 141 - "absorb_hit"
Cohesion: 0.12
Nodes (22): absorb_hit(), account_platform_host(), Candidate, content_tokens(), relevance_gate(), result_mentions(), distinct_queries(), domain_label() (+14 more)

### Community 142 - "wikipedia_rsp.rs"
Cohesion: 0.08
Nodes (44): admiralty_scales_claim_confidence_from_rsp_and_peers(), article(), raw_initial_confidence_is_captured_before_source_scaling(), recover_legacy_packet(), recover_legacy_packet_has_no_initial_confidence(), the_packet_keeps_four_and_reports_the_full_gate(), API, APP_STATE_KEY (+36 more)

### Community 143 - "intel-brief-image-column-2026-10-11/manifest.json"
Cohesion: 0.10
Nodes (20): ARGOS_EMBED, artifact_count, capture_status, CARGO_TARGET_DIR, command, created_utc, fixtures, font (+12 more)

### Community 144 - "DecisionState"
Cohesion: 0.20
Nodes (4): DecisionState, EvidencePassageState, SubjectStateSummary, TaskStateSummary

### Community 145 - "provider_diag.rs"
Cohesion: 0.09
Nodes (27): text(), bounded(), cause_chain(), classify_status(), classify_text(), endpoint_strips_query_and_userinfo(), find_url(), http_failure() (+19 more)

### Community 146 - "execute_steps"
Cohesion: 0.15
Nodes (26): after_step(), binding_ground(), context_block(), context_dispatched(), directive_for(), execute_steps(), expand_dork_children(), expand_per_platform() (+18 more)

### Community 147 - "Rect"
Cohesion: 0.09
Nodes (55): add_fallback_layout(), atlas_hit(), atlas_news_areas(), atlas_row_at(), atlas_runs_areas(), brain_form(), brain_hit(), brain_list() (+47 more)

### Community 148 - "QueryExecutionStatus"
Cohesion: 0.18
Nodes (11): QueryExecutionStatus, Cancelled, Completed, Failed, Generated, NoResults, Queued, Running (+3 more)

### Community 149 - "results.rs"
Cohesion: 0.13
Nodes (20): classify_output_quality(), extract_observation_items(), extract_search_results(), extracts_from_array_shaped_observations(), extracts_from_object_with_results_array(), normalize_tool_result(), NormalizedToolResult, OutputQuality (+12 more)

### Community 150 - ".index_now"
Cohesion: 0.22
Nodes (4): ArticleInsightCommit, insight_fingerprint(), new_id(), IndexEnqueue

### Community 151 - "SerpOutcome"
Cohesion: 0.13
Nodes (13): every_typed_outcome_maps_to_one_status_reason_and_class(), SerpOutcome, Challenge, Consent, ParserMismatch, RateLimited, ResponseTooLarge, UpstreamFailure (+5 more)

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
Cohesion: 0.05
Nodes (102): Account, account_hits(), ACCOUNT_PLATFORMS, account_platforms_attach_to_the_subject_and_are_never_entities(), ACCOUNTS, accounts_flow(), action_order(), actions_are_grounded_capped_and_not_a_sweep() (+94 more)

### Community 168 - "Milestone 1 — Core Onboarding (Current)"
Cohesion: 0.18
Nodes (10): Deliverables, Dependencies, Future Milestones (Planned), Milestone 1 — Core Onboarding (Current), Milestone 2 — CLI Verification, Milestone 3 — Test Coverage, Milestone 4 — Integration & Ship, Objectives (+2 more)

### Community 169 - "paths.rs"
Cohesion: 0.25
Nodes (13): write_cache(), auth_path(), config_journal_path(), config_lock_path(), config_path(), dataset_dir(), datasets_dir(), db_path() (+5 more)

### Community 170 - "ModuleId"
Cohesion: 0.15
Nodes (16): ModuleId, .ALL, Atlas, Brain, Intel, Jobs, Logs, Osint (+8 more)

### Community 171 - "atlas_work.rs"
Cohesion: 0.06
Nodes (35): articles_rev(), AttemptRecord, check_dependency_coverage(), CycleOutcome, Blocked, Cancelled, Completed, CompletedWithWarnings (+27 more)

### Community 172 - "model_roles.rs"
Cohesion: 0.15
Nodes (13): AccountHealth, AuthRequired, Cooldown, CreditsExhausted, Overloaded, Ready, Restricted, Unverified (+5 more)

### Community 173 - "ModelAssignment"
Cohesion: 0.23
Nodes (6): ModelAssignment, ModelRoute, role_defaults_round_trip_through_settings(), role_fallback_secrets(), role_fallbacks_add_delete_reorder_and_reject_duplicates(), route_secret()

### Community 174 - "Codebase Map — argos-osint"
Cohesion: 0.22
Nodes (8): `argos-osint-bin` — CLI and TUI, `argos-osint-core` — Core Library, Codebase Map — argos-osint, Crates, Documentation Ingest, Primary Data Flow, State & Config (all under `~/.argos`), Tool Input & Binding Kinds (from `tool_io.rs`)

### Community 175 - "PROJECT.md — argos-osint"
Cohesion: 0.22
Nodes (8): Applications, CLI Entry Points, Core Components, Crates, Investigation Flow, PROJECT.md — argos-osint, Project Purpose, State Directory (`~/.argos`)

### Community 176 - "Current Phase State"
Cohesion: 0.25
Nodes (7): Artifacts Status, Configuration State, Current Phase State, Next Steps, Pending Items, Phase: Core Onboarding, STATE.md — argos-osint

### Community 177 - "note_run"
Cohesion: 0.24
Nodes (11): bounded_reason(), call_serves(), directive_assessments(), DirectiveAssessment, note_directives(), note_run(), now_stamp(), run_facts() (+3 more)

### Community 178 - "RecoveryAction"
Cohesion: 0.12
Nodes (12): classify_recovery(), RecoveryAction, AccessRestricted, CircuitCooldown, ContentRefused, CreditsExhausted, FallbackModel, ReduceContext (+4 more)

### Community 179 - "grok_oauth.rs"
Cohesion: 0.15
Nodes (20): auth_path(), bearer(), bearer_from_path(), check_login(), Entry, entry_from(), failed_login_keeps_the_cli_reason_and_recovery_action(), login() (+12 more)

### Community 180 - "tool_runner.rs"
Cohesion: 0.10
Nodes (21): ToolDefinition, a_synthesized_timeout_is_a_failure_with_a_tool_timeout_reason(), cancelled_and_cache_modes_are_not_remote_requests(), definition(), every_other_tool_keeps_its_catalog_cache_lifetime(), failure_outcome(), failure_reason(), InvocationFact (+13 more)

### Community 181 - "Conventions"
Cohesion: 0.20
Nodes (10): Agent scratch, Checks, Commits and versions, Conventions, Diagrams, Graphify, Planning files, Schema (+2 more)

### Community 182 - "Docs Ingest — argos-osint"
Cohesion: 0.33
Nodes (5): `docs/architecture.md`, Docs Ingest — argos-osint, `docs/providers.md`, Ingest Status, Ingested Documentation

### Community 183 - "ScheduledCall"
Cohesion: 0.38
Nodes (8): courtlistener_spacing_and_firecrawl_polling_are_counted(), live(), more_calls_mean_more_tool_time_and_cache_hits_add_nothing(), scheduled(), ScheduledCall, tool_allowance_for_deps(), tool_allowance_for_deps_sequential_sums(), tool_allowance_seconds()

### Community 184 - "config_transfer.rs"
Cohesion: 0.06
Nodes (55): apply_provider(), apply_role(), apply_tool_slot(), commit_profile_config(), ConfigChange, ConfigurationSnapshot, Credential, CREDENTIAL_FIELDS (+47 more)

### Community 185 - "OpenCode V2 workflow"
Cohesion: 0.22
Nodes (9): Builds, Commit and release tasks, ECC commands, Graph first, OpenCode V2 workflow, Phase edits, Primary tools, TUI verification setup (+1 more)

### Community 186 - "ui.rs"
Cohesion: 0.04
Nodes (76): TranscriptBlock, ACTION_H, atlas_countdown(), atlas_history_live_label(), atlas_source_anchors_are_not_labeled_deleted_origin(), call_stamp(), center_line(), center_row() (+68 more)

### Community 187 - "Concepts"
Cohesion: 0.25
Nodes (8): Apps and internal IDs, Bindings, Concepts, Graph, Intel report jobs, Model roles, Persistence, Primary OSINT providers

### Community 201 - ".is_empty"
Cohesion: 0.13
Nodes (15): Completion, delta_text(), message_json(), message_reasoning(), message_text(), parse_completion(), parse_completion_errors_clearly_when_truly_empty(), parse_completion_prefers_content_over_reasoning() (+7 more)

### Community 202 - "TUI components"
Cohesion: 0.17
Nodes (12): Acceptance, ActionBar and Overlay, AnalyticsCard and Dashboard preset, Application presets, DetailTable and Report preset, Measured text, MeasuredEditor and TranscriptBlock, RankedBars, ComparisonBars and Meter (+4 more)

### Community 203 - "result"
Cohesion: 0.10
Nodes (10): execute_harness_step(), InvestigationRuntime, GateOutcome, Passed, Rejected, validate_evidence_admission(), validate_publication(), validate_task_admission() (+2 more)

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

### Community 208 - "docs/README.md"
Cohesion: 0.26
Nodes (3): Changelog, Initial baseline, Reviewed TUI buffers

### Community 209 - "Profile dashboard and search"
Cohesion: 0.17
Nodes (12): Analytics controls, Import semantics, Metric dictionary, Named search engines, Profile dashboard and search, Provider orchestration companion, Retention and coverage, The 20 primary views (+4 more)

### Community 210 - "RunStats"
Cohesion: 0.16
Nodes (11): cycle_event_id_is_the_run_key_so_replay_never_double_counts(), CycleTelemetry, db(), each_candidate_occurrence_gets_exactly_one_disposition(), OpenStage, origin_snapshots_are_one_row_per_origin_per_cycle(), rfc_ms(), RunStats (+3 more)

### Community 211 - "brain_lance_off.rs"
Cohesion: 0.07
Nodes (18): begin_generation(), block_on(), BrainIndex, clear_fingerprint(), current_fingerprint(), DUPLICATE_THRESHOLD, fingerprint_matches(), GenerationProgress (+10 more)

### Community 212 - "extract_items"
Cohesion: 0.19
Nodes (10): EngineSearchResult, extract_items(), ParserInput, CleanHtml, LinksOnly, Missing, RawHtml, retry_allowed() (+2 more)

### Community 213 - ".handle_key"
Cohesion: 0.05
Nodes (28): a_finished_tool_call_is_logged_once_and_the_transcript_keeps_a_summary(), add_scroll(), backspace_after_a_sent_question_deletes_one_character(), ctrl_arrows_cycle_app_tabs_forward_and_back(), ctrl_tab_cycles_app_tabs_forward_and_back(), dump_overhaul_screens(), IntelReconFocus, Section (+20 more)

### Community 214 - ".order"
Cohesion: 0.23
Nodes (10): checked_serves(), context_additions(), context_tools(), fallback_record(), OrderContext, Ordered, Picker<'a>, serves_for() (+2 more)

### Community 215 - "spotify.rs"
Cohesion: 0.17
Nodes (10): ADAPTER_VERSION, HOST, ID, positive_negative_and_unknown(), request_url(), ADAPTER_VERSION, HOST, ID (+2 more)

### Community 216 - "Unified Investigation Harness"
Cohesion: 0.33
Nodes (6): Catalog projection, Model roles, Persistence, Reasoning channels, Unified Investigation Harness, Validation gates

### Community 217 - "RendererKind"
Cohesion: 0.17
Nodes (12): applicable_filters(), detail_columns(), renderer_kind(), RendererKind, Comparison, Meter, Ranked, Table (+4 more)

### Community 218 - "ConfigView"
Cohesion: 0.20
Nodes (4): a_directory_target_is_an_actionable_error_not_a_crash(), ConfigView, editing_the_buffer_disarms_the_import_button(), expand_home()

### Community 219 - "TurnContinuation"
Cohesion: 0.40
Nodes (4): TurnContinuation, Active, Background, Exhausted

### Community 220 - "Argos OSINT"
Cohesion: 0.33
Nodes (6): Applications, Argos OSINT, Documentation, Figures, Limits, Quick start

### Community 221 - "ToolOutcome"
Cohesion: 0.12
Nodes (14): cache_ttl_for(), is_named_serp(), SerpCacheClass, Failure, Valid, VerifiedZero, ToolOutcome, Blocked (+6 more)

### Community 222 - "EventKind"
Cohesion: 0.08
Nodes (24): applicable_filter_dimensions(), AttentionRow, bounded_label(), dims_from_key(), Reader<'a>, EventKind, .ALL, AtlasCandidate (+16 more)

### Community 223 - "Argos documentation"
Cohesion: 0.40
Nodes (5): Agent and workflow, Argos documentation, Checklists (historical), Figures, Product

### Community 224 - ".drop"
Cohesion: 0.17
Nodes (13): a_budgeted_call_with_a_dependency_from_an_earlier_batch_runs(), ac3_no_key_reaches_inputs_cache_keys_plan_json_raw_bodies_or_logs(), ac6_a_courtlistener_429_still_lets_the_turn_synthesize(), ac8_status_errors_and_401_403_fail_readably_and_are_not_cached(), advance_stage(), BudgetedOutcome, cancel_mid_dispatch_releases_credit_holds(), citing_synthesis() (+5 more)

### Community 225 - "accept_claims"
Cohesion: 0.15
Nodes (18): a_description_only_span_is_inference_and_an_added_name_is_dropped(), a_fact_requires_both_spans_in_the_title(), accept_claims(), AcceptMode, Context, Lead, ask_claims(), AskedClaims (+10 more)

### Community 226 - "Argos UI Interaction Audit"
Cohesion: 0.40
Nodes (5): Argos UI Interaction Audit, Audit Matrix, Field inventory (`field_placeholder`), Remaining limits, Shared contracts

### Community 228 - "KeptClaim"
Cohesion: 0.20
Nodes (18): apply_admiralty_evaluation(), brief_text(), cap_claims(), countries_differ(), dedupe_claims(), entity_path(), fingerprint(), finish_insights() (+10 more)

### Community 229 - "Json"
Cohesion: 0.47
Nodes (5): json_array_len(), json_at(), json_bool(), json_str(), Json

### Community 230 - "run_primary"
Cohesion: 0.17
Nodes (16): a_claimed_email_removes_its_bindings(), a_gap_filler_domain_feeds_hunter_once_firecrawl_observes_it(), a_weak_firecrawl_search_adds_one_google_search_with_the_same_query(), a_zero_email_count_skips_the_paid_domain_search(), ac3_seo_results_that_never_mention_the_subject_add_no_domain_org_email_or_url(), ac4_google_search_always_sends_the_replaced_firecrawl_query(), ac5_every_executed_input_is_grounded_and_an_ungrounded_step_is_skipped(), ac8_the_subject_fills_name_and_query_inputs_before_found_values() (+8 more)

### Community 231 - "IndexOutcome"
Cohesion: 0.17
Nodes (6): IndexOutcome, Disabled, Pending, PermanentFailure, Ready, RetryableFailure

### Community 232 - "model_facts"
Cohesion: 0.08
Nodes (35): amplification_is_a_raw_multiplier_with_an_eligible_denominator(), AmplificationBucket, AttemptSummary, Cell, classify_model_failure(), dominant_label(), FailureBucket, FallbackRow (+27 more)

### Community 233 - "profile_components.rs"
Cohesion: 0.23
Nodes (7): clipped(), Kpi, panel(), table_row(), count_kpi(), kpis(), rate_kpi()

### Community 235 - "manifest.json"
Cohesion: 0.09
Nodes (21): ARGOS_EMBED, artifact_count, capture_status, CARGO_TARGET_DIR, command, created_utc, fixtures, font (+13 more)

### Community 237 - "parse_serp_response"
Cohesion: 0.15
Nodes (21): clip_diag(), is_serp_path(), num_at(), parse_serp_response(), str_at(), a_redirected_final_url_is_not_used_to_rebuild_the_query(), an_empty_dom_without_any_payload_is_a_parser_mismatch(), api_http_404_is_an_upstream_failure() (+13 more)

### Community 238 - "modes.rs"
Cohesion: 0.27
Nodes (13): scoped_section_plan(), chat_response_spec(), chat_response_spec_embeds_section_guidelines_and_style_guide(), chat_response_spec_lists_mode_headings(), chat_section_titles(), every_mode_has_bluf_first_and_sources_last(), every_mode_section_has_guideline(), investigation_mode_spec() (+5 more)

### Community 239 - "fetch_once"
Cohesion: 0.50
Nodes (3): fetch_once(), public_host(), read_limited()

### Community 240 - "provider_metrics.rs"
Cohesion: 0.24
Nodes (9): cache_capacity_rows(), cached_rows_become_an_available_snapshot(), capacity_snapshot(), CapacityRow, CapacitySnapshot, memory(), missing_companion_snapshot_is_unavailable_not_zero(), QueueRow (+1 more)

### Community 242 - "Trigger"
Cohesion: 0.17
Nodes (11): TELEMETRY_TRIGGER, is_forbidden_key(), payload_number(), Trigger, AtlasCycle, IntelBrief, ManualTool, ReconPrompt (+3 more)

### Community 244 - ".recon_outcomes"
Cohesion: 0.20
Nodes (9): normalize_run_outcome(), ReconOutcomeBucket, RunOutcome, Cancelled, CompletedWithEvidence, CompletedZeroEvidence, Failed, Partial (+1 more)

### Community 245 - "BrainListMode"
Cohesion: 0.50
Nodes (4): BrainListMode, Create, Graph, List

### Community 246 - "SettingsFile"
Cohesion: 0.22
Nodes (4): SettingsFile, RoleRuntime, cost_map(), StrategyPrompt

### Community 247 - "AtlasInsightClaim"
Cohesion: 0.27
Nodes (14): commit_refined_body(), article(), claim(), commit_replaces_atomically_and_keeps_shared_support(), failed_validation_leaves_prior_insights(), replace_article_insights_from_body(), ReplaceOutcome, snapshot_counts_are_stable_before_commit() (+6 more)

### Community 248 - "search_engines_fixtures.rs"
Cohesion: 0.37
Nodes (12): every_fixture_matches_its_recorded_outcome(), field(), fixture_cases(), fixtures_dir(), no_fixture_claimed_a_verified_zero_without_a_supported_phrase(), opt_str(), opt_usize(), read_json() (+4 more)

### Community 249 - "PickReply"
Cohesion: 0.43
Nodes (3): parse_chat_pick(), PickReply, PickRequestAdapter<'r, 'b>

### Community 250 - "RouteInput"
Cohesion: 0.17
Nodes (11): RouteInput, FacebookUrl, Handle, HandleOrUserId, Hashtag, LinkedinCompanyUrl, LinkedinProfileUrl, Query (+3 more)

### Community 251 - "graph_explanation/tests.rs"
Cohesion: 0.13
Nodes (27): explain(), ExplainOutcome, Failed, Saved, Superseded, ExplainReport, ExplainRequest, Faults (+19 more)

### Community 252 - "SystemTab"
Cohesion: 0.33
Nodes (4): SystemTab, Configs, Overview, System

### Community 253 - ".intel_reports"
Cohesion: 0.18
Nodes (10): normalize_report_outcome(), report_key(), ReportModeRow, ReportOutcome, Blocked, Cancelled, Completed, Failed (+2 more)

### Community 254 - "§9 implementation order"
Cohesion: 0.15
Nodes (12): §10 acceptance checks, §9 implementation order, Atlas memory completion and System apps — implementation checklist, Phase 1 — what landed, Phase 2 — what landed, Phase 3 — what landed, Phase 4 — what landed, Phase 5a — what landed (+4 more)

### Community 255 - "serde"
Cohesion: 0.08
Nodes (14): CallProposal, HandoffRecord, InvestigationSurface, HomeComposer, IntelBrief, JobsResume, ReconChat, atlas_only_tools_blocked_on_all_surfaces() (+6 more)

### Community 257 - "QuestionSpec"
Cohesion: 0.25
Nodes (5): DecisionQuestionType, Choice, Noul, Score, QuestionSpec

### Community 258 - "actor_review.rs"
Cohesion: 0.26
Nodes (12): ActorReviewItem, ActorReviewResult, apply_reviewed_actors(), deterministic_review_actors(), is_meaningful_actor(), JUNK_ACTORS, model_review_actors(), parse_actor_review_response() (+4 more)

### Community 259 - "run_live"
Cohesion: 0.29
Nodes (6): LiveRun, Fresh, Latest, Run, run_live(), RunInput

### Community 260 - "PlanInterval"
Cohesion: 0.33
Nodes (5): PlanInterval, Daily, Monthly, Never, Weekly

### Community 261 - "result_cache"
Cohesion: 0.38
Nodes (6): cache_stores_only_definitive(), CacheEntry, clear_caches(), host_cooldown_is_rate_limited(), host_cooldowns(), result_cache()

### Community 262 - "Target"
Cohesion: 0.05
Nodes (41): brain_article_source_opens_intel_brief(), hit(), Target, App, AtlasArticle, AtlasCycleStats, AtlasFeed, BrainMark (+33 more)

### Community 263 - ".job_source"
Cohesion: 0.50
Nodes (4): AtlasRun, JobSource, AtlasRun, ReconThread

### Community 264 - "TaskState"
Cohesion: 0.18
Nodes (9): TaskState, Cancelled, Completed, Failed, Paused, Queued, RetryScheduled, Running (+1 more)

### Community 265 - ".reload_memories"
Cohesion: 0.26
Nodes (7): brain_summary_failure_card_explains_links_and_retries_in_place(), dump_phase7_screens(), graph_jobs(), saved_summary_is_reused_until_its_inputs_change_then_shown_as_earlier(), settle_summary(), summary_fixture(), SummaryFx

### Community 267 - "Profile analytics dashboard"
Cohesion: 0.33
Nodes (6): Dashboard preset, Primary inventory, Profile analytics dashboard, State and interaction, Verification and delivery, Visual and metric rules

### Community 268 - "CheckSignal"
Cohesion: 0.20
Nodes (8): CheckSignal, Blocked, Error, Inconclusive, NotRegistered, RateLimited, Registered, Unsupported

### Community 269 - "role_secret"
Cohesion: 0.26
Nodes (11): account_secret(), active_text_secret(), preset(), presets(), ProviderPreset, role_name(), role_secret(), subscription_connection() (+3 more)

### Community 270 - "Profile TUI implementation session — 2026-10-10"
Cohesion: 0.22
Nodes (9): Artifact retention, Defects found and corrected, Fixture and screenshot construction, Gate results and environment handling, Ownership and implementation, Profile TUI implementation session — 2026-10-10, Scope and starting evidence, Screenshot evidence (+1 more)

### Community 276 - "Diagram conventions"
Cohesion: 0.50
Nodes (4): Agent rule, Catalog, Diagram conventions, When to draw

### Community 277 - "PlanCall"
Cohesion: 0.24
Nodes (11): action_call(), bound(), isolation_lines(), isolation_lines_show_what_ran_what_was_held_and_why(), run_wave(), search_call(), served(), step() (+3 more)

### Community 278 - "AtlasPage"
Cohesion: 0.67
Nodes (3): AtlasPage, Live, Runs

### Community 279 - "TUI implementation and verification"
Cohesion: 0.29
Nodes (7): Build fixtures and assertions, Capture actual cells, Finish and retain evidence, Plan the contract, Review, fix and recapture, Run the check gate, TUI implementation and verification

### Community 280 - "JobStatusFilter"
Cohesion: 0.25
Nodes (6): JobStatusFilter, Active, All, Completed, Failed, Retrying

### Community 281 - "Unified reliability / summarization / semantic pipelines — completion checklist"
Cohesion: 0.40
Nodes (4): Honest remaining gaps, Phase status (§18), This follow-up, Unified reliability / summarization / semantic pipelines — completion checklist

### Community 282 - "select_strategy"
Cohesion: 0.40
Nodes (6): has_concrete_identifier(), hypothesis_question(), select_strategy(), strategy_change_reason(), strategy_follows_the_question_and_can_change_without_erasing_work(), StrategyChoice

### Community 283 - "enqueue_job"
Cohesion: 0.36
Nodes (7): lease_fencing_rejects_stale_epoch_and_foreign_owner(), claim_complete_and_retry_round_trip(), claim_next(), enqueue_job(), enqueue_job_with(), JobMeta, NewJob

### Community 285 - "poll_device"
Cohesion: 0.28
Nodes (9): DeviceGrant, Poll, Denied, poll_device(), Pending, SlowDown, Token, start_device() (+1 more)

### Community 287 - "draw_field"
Cohesion: 0.43
Nodes (7): cursor_at(), draw_field(), field_placeholder(), field_value_area(), line_col(), value_area_or_composer(), viewport()

### Community 288 - "intel_recon/brain.rs"
Cohesion: 0.57
Nodes (5): intel_recon_publishes_durably_with_provenance_and_no_atlas_receipt(), ReconInsightUpdate, update(), upsert_adds_and_updates_claims(), upsert_recon_insights()

### Community 289 - "TUI overhaul verification — 10 October 2026"
Cohesion: 0.33
Nodes (6): Defects found and corrected, Interaction and metric evidence, Limits, Reproduction, TUI overhaul verification — 10 October 2026, Visual evidence

### Community 290 - "Store"
Cohesion: 0.53
Nodes (5): charge_newsapi(), charge_quota(), open_key(), quota_open(), utc_day()

### Community 291 - "TUI design contract"
Cohesion: 0.33
Nodes (6): Agent workflow and acceptance, Profile components and presets, Profile edit map, Shared navigation and portable configuration, Theme, data and interaction, TUI design contract

### Community 292 - "Versioning, commits and releases"
Cohesion: 0.40
Nodes (5): Agent commit tasks, Commit and PR convention, GitHub setup and first release, Release automation, Versioning, commits and releases

### Community 293 - "CredentialSource"
Cohesion: 0.50
Nodes (4): CredentialSource, Env, Inline, None

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
- **2317 isolated node(s):** `$schema`, `default_agent`, `subagent_depth`, `timeout`, `chunkTimeout` (+2312 more)
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 2999 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **37 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `App` connect `App` to `ProfileView`, `app.rs`, `Target`, `.job_source`, `.reload_memories`, `ToolResult`, `article_image.rs`, `Store`, `AtlasPage`, `hardware.rs`, `Store`, `LayoutRegistry`, `Call`, `brain_detail.rs`, `ModuleId`, `ReportMode`, `tui/jobs.rs`, `ui.rs`, `src/brain.rs`, `intel_recon/jobs.rs`, `Overlay`, `.clear`, `RunStats`, `FeedArticle`, `.handle_key`, `DefaultsRole`, `ConfigView`, `AtlasArticleRow`, `logs.rs`, `summary_card.rs`, `.new_thread`, `WorkEvent`, `BrainListMode`, `SettingsFile`, `briefing_view.rs`, `recon/graph.rs`, `AuthFile`, `run`, `serde`?**
  _High betweenness centrality (0.081) - this node is a cross-community bridge._
- **What connects `$schema`, `default_agent`, `subagent_depth` to the rest of the system?**
  _2317 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `land.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.0076045627376425855 - nodes in this community are weakly interconnected._
- **Why does `Value` connect `Value` to `orchestrate.rs`, `telemetry.rs`, `atlas_memory.rs`, `tool_io.rs`, `ProviderSecret`, `recon.rs`, `ToolResult`, `Store`, `serde_json`, `complete`, `wikipedia_rsp.rs`, `directives.rs`, `DecisionState`, `provider_diag.rs`, `providers.rs`, `atlas_news.rs`, `absorb_hit`, `results.rs`, `TaskStatus`, `whoxy.rs`, `WidgetSpec`, `App`, `news_legal.rs`, `PlanCall`, `osint.rs`, `holehe/mod.rs`, `TelemetryEvent`, `Call`, `body.rs`, `provider.rs`, `Supplied<T>`, `investigation.rs`, `run_atlas_inner`, `provider_attempt.rs`, `picker.rs`, `ReportMode`, `grok_oauth.rs`, `model_exec.rs`, `config_transfer.rs`, `ui.rs`, `cli.rs`, `atlas_insights.rs`, `body_filter.rs`, `synthesize.rs`, `.is_empty`, `summarization.rs`, `search_engines.rs`, `Severity`, `.new`, `RunStats`, `Binding`, `src/evidence.rs`, `ToolRunner`, `anyhow`, `AtlasArticleRow`, `BrainResourceSummary`, `dork_generator.rs`, `execute_steps`, `search_engines_fixtures.rs`, `accept_claims`, `.new_thread`, `PickReply`, `diversity.rs`, `run_primary`, `.default`, `search_engines/tests.rs`, `F`, `parse_serp_response`, `components.rs`, `ServiceResult`, `subscription.rs`, `profile.rs`, `Service`, `config_transfer_tests.rs`, `serde`?**
  _High betweenness centrality (0.059) - this node is a cross-community bridge._
- **Should `orchestrate.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.04855769230769231 - nodes in this community are weakly interconnected._
- **Why does `TUI design contract` connect `TUI design contract` to `docs/README.md`, `Frame`?**
  _High betweenness centrality (0.038) - this node is a cross-community bridge._
- **Should `telemetry.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.12873563218390804 - nodes in this community are weakly interconnected._