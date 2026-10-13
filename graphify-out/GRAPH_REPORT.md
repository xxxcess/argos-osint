# Graph Report - argos-osint  (2026-10-12)

## Corpus Check
- 291 files · ~657,739 words
- Verdict: corpus is large enough that graph structure adds value.
- Unclassified: 5 file(s) not represented in the graph (top: (none) 3, .toml 2)

## Summary
- 9487 nodes · 23387 edges · 297 communities (244 shown, 53 thin omitted)
- Extraction: 98% EXTRACTED · 2% INFERRED · 0% AMBIGUOUS · INFERRED: 445 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `79d9dd2a`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- land.rs
- orchestrate.rs
- Store
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
- compromise-dates.min.js
- Value
- atlas_news.rs
- Store
- publication.rs
- TaskStatus
- whoxy.rs
- news_legal.rs
- hardware.rs
- App
- insights.rs
- embed.rs
- osint.rs
- holehe/mod.rs
- config_commit.rs
- Call
- telemetry.rs
- InvestigationPart
- body.rs
- provider.rs
- brain_detail.rs
- profile_stats.rs
- atlas.rs
- .default
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
- draw_intel_briefing
- model_exec.rs
- ErrorCategory
- Binding
- src/brain.rs
- atlas_table.rs
- brain_resources.rs
- SettingsFile
- anyhow
- AtlasEvent
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
- atlas_metadata.rs
- spans.rs
- search_engines.rs
- provider_chain.rs
- .new
- apply_hits
- ProfileView
- SerpOutcome
- Command
- DefaultsRole
- graph_explanation/tests.rs
- src/evidence.rs
- ToolRunner
- atlas_classify.rs
- F
- subscription.rs
- dork_generator.rs
- Section
- How
- insight_persist.rs
- Embedder
- explore.rs
- ledger.rs
- Region
- diversity.rs
- DecisionAnswer
- job_cleanup.rs
- Frame
- search_engines/tests.rs
- profile_config.rs
- summary_card.rs
- Architecture
- WorkEvent
- store.rs
- ClockSet
- ProviderAdmission
- components.rs
- normalize_destination
- release_check.mjs
- logs.rs
- briefing_view.rs
- src/related_articles.rs
- profile.rs
- TurnEvent
- recon/graph.rs
- tasks.rs
- ReconCommand
- config_transfer_tests.rs
- Insight extraction reliability acceptance
- MemoryPhase
- block_on
- TurnClock
- opencode.json
- Category
- model_roles.rs
- home_rows
- run_turn
- DateTime
- OsintCommand
- source_sha256
- brain_lance_off.rs
- Argos OSINT — Agent Instructions
- run_atlas_inner
- .new
- wikipedia_rsp.rs
- intel-brief-image-column-2026-10-11/manifest.json
- validate.rs
- ConfigView
- Verbs
- Rect
- QueryExecutionStatus
- results.rs
- DecisionPolicy
- ToolOutcome
- WidgetSpec
- App
- tui_review.py
- TelemetryEvent
- investigation/evidence.rs
- serde
- Part
- argos-osint-bin
- ToolResult
- SiteOutcomeStatus
- investigation.rs
- PipelineInput
- paths.rs
- ModuleId
- atlas_work.rs
- ImportBuffer
- BrainResourceSummary
- QuestionSpec
- select
- ui.rs
- RecoveryAction
- grok_oauth.rs
- .clear
- Conventions
- Field
- config_transfer.rs
- nlp/mod.rs
- Concepts
- package.json
- TUI components
- gates.rs
- Providers
- Decision roles and output contracts
- LayoutResult
- insight_execution.rs
- docs/README.md
- Profile dashboard and search
- CycleTelemetry
- BrainIndex
- tool_runner.rs
- budget.rs
- modes.rs
- Unified Investigation Harness
- RendererKind
- Numbers
- TaskState
- Argos OSINT
- .order
- EventKind
- nlp/engine.rs
- j
- ChainReport<T>
- graph_explanation.rs
- model_facts
- profile_components.rs
- atlas_recall_tests.rs
- manifest.json
- parse_serp_response
- insight_reliability_tests.rs
- provider_metrics.rs
- RunStats
- compromise.js
- .recon_outcomes
- search_engines_fixtures.rs
- Dates
- AtlasInsightClaim
- Supplied<T>
- Day
- Trigger
- RepairStatus
- .intel_reports
- §9 implementation order
- actor_review.rs
- InvestigationSurface
- Nouns
- Target
- cache.rs
- .json
- OperationScope<'a>
- DecisionState
- AuthFile
- Profile TUI implementation session — 2026-10-10
- Atlas classification and Related articles
- CheckSignal
- Diagram conventions
- Fractions
- TUI implementation and verification
- Unified reliability / summarization / semantic pipelines — completion checklist
- super
- .split
- rusqlite
- Semantic-versioning setup acceptance
- Chunks
- TUI overhaul verification — 10 October 2026
- People
- .new
- View
- pull_request_template.md
- release-please-config.json
- Local NLP preprocessing
- Adjectives
- LevelFilter
- RouteInput
- intel-brief-image-column-2026-10-11/README.md
- nlp/README.md
- PickRequest
- .set_focus
- Argos documentation
- TurnContinuation
- poll_device
- StatFilters
- BrainListMode
- Argos UI Interaction Audit
- ModelOperationError
- Surface

## God Nodes (most connected - your core abstractions)
1. `App` - 332 edges
2. `Value` - 315 edges
3. `source_sha256` - 222 edges
4. `source_sha256` - 222 edges
5. `ProviderSecret` - 149 edges
6. `ButtonId` - 140 edges
7. `Target` - 94 edges
8. `AtlasArticleRow` - 90 edges
9. `ToolResult` - 79 edges
10. `FieldId` - 76 edges

## Surprising Connections (you probably didn't know these)
- `Module layout` --references--> `widget_lines()`  [EXTRACTED]
  docs/architecture.md → crates/argos-osint-bin/src/tui/profile.rs
- `State and interaction` --references--> `v()`  [INFERRED]
  docs/profile-analytics-dashboard.md → crates/argos-osint-core/assets/nlp/compromise-dates.min.js
- `Analytics controls` --references--> `v()`  [INFERRED]
  docs/profile-dashboard-and-search.md → crates/argos-osint-core/assets/nlp/compromise-dates.min.js
- `DetailTable and Report preset` --references--> `v()`  [INFERRED]
  docs/tui-components.md → crates/argos-osint-core/assets/nlp/compromise-dates.min.js
- `Theme, data and interaction` --references--> `v()`  [INFERRED]
  docs/tui-design-spec.md → crates/argos-osint-core/assets/nlp/compromise-dates.min.js

## Import Cycles
- 1-file cycle: `crates/argos-osint-core/src/config_transfer.rs -> crates/argos-osint-core/src/config_transfer.rs`
- 2-file cycle: `crates/argos-osint-core/src/osint.rs -> crates/argos-osint-core/src/osint/results.rs -> crates/argos-osint-core/src/osint.rs`
- 2-file cycle: `crates/argos-osint-bin/src/cli.rs -> crates/argos-osint-core/src/hardware.rs -> crates/argos-osint-bin/src/cli.rs`
- 4-file cycle: `crates/argos-osint-core/src/config_commit.rs -> crates/argos-osint-core/src/provider_attempt.rs -> crates/argos-osint-core/src/provider_diag.rs -> crates/argos-osint-core/src/config_transfer.rs -> crates/argos-osint-core/src/config_commit.rs`

## Communities (297 total, 53 thin omitted)

### Community 0 - "land.rs"
Cohesion: 0.01
Nodes (262): CODES, LABELS, R0, R1, R10, R100, R101, R102 (+254 more)

### Community 1 - "orchestrate.rs"
Cohesion: 0.04
Nodes (130): a_429_stops_picker_calls_and_the_fallback_finishes_the_list(), a_claimed_email_removes_its_bindings(), a_dispatch_error_fails_the_step_and_the_loop_goes_on(), a_dropped_provider_stream_keeps_the_text_already_received(), a_failed_email_finder_gets_one_fallback_and_no_third_picker_call(), a_fallback_with_no_bindings_still_runs_the_satisfied_steps_and_reaches_synthesis(), a_follow_up_keeps_names_from_the_previous_synthesis(), a_gap_filler_domain_feeds_hunter_once_firecrawl_observes_it() (+122 more)

### Community 2 - "Store"
Cohesion: 0.05
Nodes (17): stats_from_stored(), atlas_answer_id(), atlas_brief_id(), atlas_claims_for_article_returns_linked_claims(), AtlasStoredClaim, delete_article_insights_orphans_brain_and_keeps_shared(), has_table(), intel_link_explanations_round_trip_and_cleanup() (+9 more)

### Community 3 - "atlas_memory.rs"
Cohesion: 0.11
Nodes (45): InsightStats, Barrier, Checkpoint, CheckpointInput, claims(), disk_store(), ensure_historical_contract(), extraction() (+37 more)

### Community 4 - "tool_io.rs"
Cohesion: 0.03
Nodes (97): leftovers(), names(), PROMPT_TARGETS, normalize_platform(), question_bindings(), a_gap_filler_only_domain_waits_for_a_primary_observation_before_hunter(), accept_bindings(), batch_scrape_takes_ranked_urls_and_gates_order_before_hunter() (+89 more)

### Community 5 - "app.rs"
Cohesion: 0.06
Nodes (108): a_click_folds_an_event_and_incoming_entries_do_not_move_a_paused_list(), a_finished_tool_call_is_logged_once_and_the_transcript_keeps_a_summary(), all_provider_actions_render_with_hit_areas_at_80x24(), analytics_fixture(), analytics_viewports_and_data_states(), ATLAS_AUTO_SECS, atlas_auto_toggle_persists_the_next_trigger(), atlas_claim() (+100 more)

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
Cohesion: 0.04
Nodes (82): ac6_citation_groups_split_validate_each_id_and_normalize(), answer_step_rate_limit_is_explicit_and_keeps_results(), attribute_word(), auxiliary(), BRIEF_SYNTHESIS, broad_question_collapses_to_grounded_lookups(), char_ceil(), char_floor() (+74 more)

### Community 10 - "ButtonId"
Cohesion: 0.02
Nodes (115): ButtonId, Add, AddFallback, AtlasAuto, AtlasDelete, AtlasLive, AtlasNews, AtlasNewsFeed (+107 more)

### Community 11 - "Store"
Cohesion: 0.06
Nodes (19): ReconLimits, answer_evidence_survives_reopen_and_deleted_thread_rejects_late_calls(), claims_deduplicate_and_reject_unsupported_sources(), CreditHold, deleting_an_investigation_removes_its_brain_memories_and_graph_summary(), investigation_memory_ids(), now(), only_a_failed_or_interrupted_latest_run_is_resumable() (+11 more)

### Community 12 - "IntelligenceCategory"
Cohesion: 0.08
Nodes (26): test_all_catalog_tools_mapped_to_categories(), cache_follows_the_provider_plan_interval(), all_catalog_tools_mapped_to_categories(), argument_contract(), ArgumentBuilderContract, compact_capability_catalog(), CompactToolCapability, IntelligenceCategory (+18 more)

### Community 13 - "job_registry.rs"
Cohesion: 0.05
Nodes (49): begin(), begin_cancellable(), db(), db_path(), finish(), beat(), BEAT_INTERVAL, beats() (+41 more)

### Community 14 - "execute_steps"
Cohesion: 0.13
Nodes (33): after_step(), binding_ground(), cancelled(), context_block(), context_dispatched(), directive_for(), execute_steps(), expand_dork_children() (+25 more)

### Community 15 - "directives.rs"
Cohesion: 0.06
Nodes (65): accounts_search_query(), apply_context_targets(), asks(), asks_about_judge(), asks_for_headlines(), clip_words(), context_entity(), CONTEXT_FILLER (+57 more)

### Community 16 - "article_image.rs"
Cohesion: 0.09
Nodes (22): absent_invalid_failed_images_collapse(), ArticleImages, centered_image_rect(), Completion, Decoded, Encoded, decode(), decode_preserves_source_resolution_and_pixels() (+14 more)

### Community 17 - "compromise-dates.min.js"
Cohesion: 0.04
Nodes (19): AnyMonth, AnyQuarter, D(), Durations, Hour, j(), Minute, Moment (+11 more)

### Community 18 - "Value"
Cohesion: 0.04
Nodes (91): Value, CallProposal, HandoffRecord, annotate(), country_arg(), optional_text(), query_arg(), request() (+83 more)

### Community 19 - "atlas_news.rs"
Cohesion: 0.07
Nodes (40): api_error(), ATLAS_TOOLS, authors(), clip(), country_code(), CURRENTS_COUNTRIES, CURRENTS_HOST, CURRENTS_LATEST (+32 more)

### Community 20 - "Store"
Cohesion: 0.06
Nodes (26): article_body_from_row(), ArticleBodyRow, cited_evidence_keeps_provenance_and_revision_identity(), element_from_row(), full_assessment_parents_are_flagged_for_child_report_totals(), IntelAssessmentRow, IntelEvidenceRow, IntelInvestigationRow (+18 more)

### Community 21 - "publication.rs"
Cohesion: 0.07
Nodes (47): MemorySource, active_index_rows(), bump_memories_changed(), change_watcher_sees_only_committed_changes_and_coalesces(), claim(), clear(), clear_index_state(), CoverageReport (+39 more)

### Community 22 - "TaskStatus"
Cohesion: 0.07
Nodes (14): TaskRecord, TaskStatus, Cancelled, Completed, Deferred, Failed, Partial, Planned (+6 more)

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
Cohesion: 0.06
Nodes (80): active_popup_area(), api_key_slot(), ApiKeySlot, atlas_auto_label(), atlas_cycle_stats_room(), atlas_cycle_stats_scroll_max(), atlas_extracting(), atlas_feed_room() (+72 more)

### Community 27 - "insights.rs"
Cohesion: 0.06
Nodes (62): admit_brain_evidence(), ARTICLE_CLAIM_LIMIT, assess_fidelity(), AssessedLink, Assessment, atlas_shared_pipeline_is_claims_only_and_identical_replay_makes_no_requests(), body_readiness_requires_both_complete_stages_and_original_acquisition(), CLAIM_PROMPT (+54 more)

### Community 28 - "embed.rs"
Cohesion: 0.10
Nodes (25): active(), DIM, disable(), disabled(), DisableGuard, DOWNLOAD_TIMEOUT_SECS, embed_batch(), embed_one() (+17 more)

### Community 29 - "osint.rs"
Cohesion: 0.04
Nodes (90): a_claimed_email_keeps_no_person_data(), bind_request(), bitcoin(), body_cap(), bounded(), bounded_recovery_is_offered_once_for_an_unrecognised_page_only(), CACHE_DAY_SECONDS, cache_identity() (+82 more)

### Community 30 - "holehe/mod.rs"
Cohesion: 0.06
Nodes (46): by_id(), cache_key(), cache_stores_only_definitive(), CACHE_TTL, cached_result(), CacheEntry, cancel_marks_inconclusive(), cap_reports_omitted() (+38 more)

### Community 31 - "config_commit.rs"
Cohesion: 0.05
Nodes (73): a_batch_with_a_duplicate_or_empty_slot_is_refused_before_any_write(), a_committed_journal_needs_no_recovery(), a_second_lock_is_refused_while_one_is_held(), a_stale_lock_is_taken_over(), an_abandoned_unfinished_lockfile_is_recovered(), backup_path(), canonical_root(), commit_files_is_all_or_nothing() (+65 more)

### Community 32 - "Call"
Cohesion: 0.10
Nodes (33): TranscriptBlock, build_blocks(), call_stamp(), ChatBlock, clip_pieces(), coverage_only_counts_explicit_assessments(), coverage_summary(), decision_row_shows_directives_picker_order_bindings_and_fallbacks() (+25 more)

### Community 33 - "telemetry.rs"
Cohesion: 0.12
Nodes (21): bin_index(), Coverage, duration_bins_are_bounded_and_mergeable(), DURATION_BINS_MS, ensure_observed_since(), event_bins(), get_meta(), ID_SEQ (+13 more)

### Community 34 - "InvestigationPart"
Cohesion: 0.10
Nodes (17): event_to_chat_block(), is_thinking_expanded(), InvestigationPart, DirectiveAssessment, EvidencePassage, GateValidation, Handoff, Plan (+9 more)

### Community 35 - "body.rs"
Cohesion: 0.10
Nodes (36): body_fetch_routes(), BodyFetchEvent, Attempt, Failed, InsightsFailed, InsightsRefreshing, InsightsReplaced, Progress (+28 more)

### Community 36 - "provider.rs"
Cohesion: 0.05
Nodes (78): account_secret(), active_text_secret(), authorize(), bearer_token(), catalog_error(), chat_body(), complete(), complete_errors_with_finish_reason_when_response_is_empty() (+70 more)

### Community 37 - "brain_detail.rs"
Cohesion: 0.06
Nodes (52): areas(), areas_for(), BrainDetail, DetailAreas, DetailPane, Path, Related, Summary (+44 more)

### Community 38 - "profile_stats.rs"
Cohesion: 0.04
Nodes (80): adaptive_bucket_seconds(), add_cycle_outcome(), atlas_carries_data(), AtlasStats, AttemptSummary, AttentionRow, BacklogRow, band_for() (+72 more)

### Community 39 - "atlas.rs"
Cohesion: 0.09
Nodes (28): article_card_labels_title_publisher_author_and_classification(), Band, band_sizes(), band_temperature(), category_from_decisions(), CATEGORY_IDS, CLUSTERS, country_label() (+20 more)

### Community 40 - ".default"
Cohesion: 0.08
Nodes (29): a_blank_osint_user_agent_loads_as_unset(), default_credit_reset(), default_firecrawl_credits(), default_hunter_credits(), default_max_calls(), default_max_rounds(), default_one_cost(), default_opening_cap() (+21 more)

### Community 41 - "whatsmyname.rs"
Cohesion: 0.05
Nodes (54): unique_answers(), D, AccountTuple, ACTIVE_SNAPSHOT, ADAPTER_VERSION, apply_strip_bad_char(), benchmark_14_4_parse_index_and_selection(), cache_get() (+46 more)

### Community 42 - "scheduler.rs"
Cohesion: 0.09
Nodes (22): apply_index_change(), apply_index_change_rebuild_reports_outcome(), apply_index_with(), DEFAULT_LEASE_SECS, drain_index_once(), drain_summary_flush(), drain_summary_flush_cached(), drain_summary_flush_completes_cached_tasks() (+14 more)

### Community 43 - "jobs_view.rs"
Cohesion: 0.06
Nodes (31): active_time_is_live_for_open_attempts_and_unavailable_only_without_a_start(), AttemptRow, event_apps(), event_counts(), EventCounts, get_job(), job(), job_apps() (+23 more)

### Community 44 - "provider_attempt.rs"
Cohesion: 0.10
Nodes (41): message_bytes(), Acc, attempt(), attempt_insight(), attempt_options(), attempt_with_observer(), AttemptReport, check_final() (+33 more)

### Community 45 - "LogicalRole"
Cohesion: 0.10
Nodes (11): LogicalRole, .ALL, ClaimAssessor, Classifier, Controller, EntityResolver, EvidenceCurator, Planner (+3 more)

### Community 46 - "picker.rs"
Cohesion: 0.13
Nodes (22): brain_scrape_options_hide_bare_scrape_and_include_claim_criteria(), CatalogEntry, CONFIDENCE_FLOOR, decisions_request(), directives_phrase(), DONE, eligible_catalog(), empty_brain_resources_omitted_from_state() (+14 more)

### Community 47 - "ReportMode"
Cohesion: 0.12
Nodes (23): HomeDraftState, brief_rating_reuses_the_existing_mean_semantics(), classifiable_modes(), classify_mode_chat(), classify_mode_decisions(), classify_prompt_mode(), classify_prompt_mode_chat(), classify_prompt_mode_decisions() (+15 more)

### Community 48 - "source_sha256"
Cohesion: 0.01
Nodes (222): source_sha256, crates/argos-osint-bin/Cargo.toml, crates/argos-osint-bin/src/cli.rs, crates/argos-osint-bin/src/main.rs, crates/argos-osint-bin/src/tui/app.rs, crates/argos-osint-bin/src/tui/article_image.rs, crates/argos-osint-bin/src/tui/atlas_actions.rs, crates/argos-osint-bin/src/tui/atlas_table.rs (+214 more)

### Community 49 - "atomic"
Cohesion: 0.08
Nodes (14): AttemptOutcome, categorize_http(), execute_with_retries(), other_llm_allows_three(), summarization_stops_at_two_attempts(), TimeoutProfile, .CLASSIFIER, .EXTRACTION (+6 more)

### Community 50 - "tui/jobs.rs"
Cohesion: 0.09
Nodes (26): action_geometry(), areas(), BASE_COLUMNS, button_label(), buttons(), detail_lines(), draw(), hit() (+18 more)

### Community 51 - "App"
Cohesion: 0.03
Nodes (34): a_failed_turn_keeps_the_streamed_answer_on_screen(), a_log_or_late_tool_row_does_not_drop_the_live_answer(), App, atlas_countdown_visible(), atlas_extracting_visible(), atlas_log_level(), AtlasPage, Live (+26 more)

### Community 52 - "events.rs"
Cohesion: 0.09
Nodes (27): clear_events(), DEFAULT_RETENTION_HOURS, event_row(), EventFilter, EventRow, get_event(), job_filter_includes_descendants_and_prune_keeps_jobs(), list_events() (+19 more)

### Community 53 - "exec.rs"
Cohesion: 0.13
Nodes (21): admission_account(), admission_contention_consumes_no_attempt(), AttemptEvent, Finished, Started, AttemptLog, cancelled(), complete_summary_report() (+13 more)

### Community 54 - "draw_intel_briefing"
Cohesion: 0.28
Nodes (17): abs_contains(), abs_rect(), AbsRect, draw_centered_loading_card(), draw_clipped_button(), draw_clipped_intel_loading(), draw_clipped_md_pane(), draw_intel_body_loading() (+9 more)

### Community 55 - "model_exec.rs"
Cohesion: 0.09
Nodes (35): adapter_reported_first_response_is_kept_when_no_chunk_arrived(), attempt_outcome_keeps_the_failure_category_for_the_reason_dimension(), AttemptFacts, AttemptOutcome, AttemptTimings, canonical_attempt_id(), canonical_attempt_id_is_stable_across_replay(), chain_error() (+27 more)

### Community 56 - "ErrorCategory"
Cohesion: 0.08
Nodes (24): operation_kind(), backoff_delay(), can_retry(), ErrorCategory, AuthOrQuota, CancelledOrStale, ConfigurationMissing, ContextLimit (+16 more)

### Community 57 - "Binding"
Cohesion: 0.14
Nodes (29): Binding, canonical(), dependency_order(), depends_on(), question_bindings_and_dependency_fix(), allowed_producer(), best_handle(), bind_step() (+21 more)

### Community 58 - "src/brain.rs"
Cohesion: 0.05
Nodes (49): AGREEMENT_WEIGHT, CATEGORIES, category_hint(), format_injection(), hybrid_recall(), hybrid_recall_adds_paraphrases_and_rewards_agreement(), identity_query_prefers_name_memory(), jaccard() (+41 more)

### Community 59 - "atlas_table.rs"
Cohesion: 0.17
Nodes (20): border_style(), borders_and_cells_have_matching_display_widths(), bottom_border_line(), char_width(), clip_to_width(), column_widths(), columns_consume_inner_width(), columns_distribute_proportionally_when_not_flex_last() (+12 more)

### Community 60 - "brain_resources.rs"
Cohesion: 0.09
Nodes (26): source_anchor_label(), BRAIN_SCRAPE_PREFIX, CLAIM_CHARS, classify_resource(), file_link_url(), hit(), is_brain_binding(), is_brain_scrape_pick() (+18 more)

### Community 61 - "SettingsFile"
Cohesion: 0.12
Nodes (24): IntelReportEvent, InsightsUpdated, JobCreated, JobDone, Section, Stage, IntelReportJobRow, cancel_if_needed() (+16 more)

### Community 62 - "anyhow"
Cohesion: 0.06
Nodes (42): compile_general_model_prompt(), compile_native(), DecisionAdapterKind, JsonMode, NativeDecisions, OutputTool, StrictJsonSchema, ValidatedText (+34 more)

### Community 63 - "AtlasEvent"
Cohesion: 0.14
Nodes (15): AtlasEvent, Classified, Fault, InsightProgress, MemoriesChanged, MemoryProgress, Note, Replaced (+7 more)

### Community 64 - "intel_recon/jobs.rs"
Cohesion: 0.14
Nodes (23): active_job_for_mode(), cancel_job(), create_job_inserts_all_verify_sections(), create_report_job(), finish_canonical(), intel_recon_links_its_legacy_job_to_one_canonical_registry_job(), pause_job(), register_canonical() (+15 more)

### Community 65 - "cli.rs"
Cohesion: 0.15
Nodes (23): ask(), ask_thread(), atlas_command(), atlas_memory_maintenance_accepts_historical_and_rejects_collection_contracts(), defaults_command(), defaults_json(), defaults_show_includes_the_tool_picker_transport(), DefaultsCommand (+15 more)

### Community 66 - "atlas_insights.rs"
Cohesion: 0.05
Nodes (109): a_country_token_does_not_merge_into_a_longer_name(), accept_claims(), accept_one(), AcceptMode, Context, Lead, admiralty_scales_claim_confidence_from_rsp_and_peers(), aliases_merge_on_shared_tokens_when_the_longer_form_is_a_span() (+101 more)

### Community 67 - "SourceReliability"
Cohesion: 0.09
Nodes (23): article_information_credibility(), initial_rating(), AdmiraltyCode, best_credibility(), CredibilityInputs, information_credibility(), InformationCredibility, CannotBeJudged (+15 more)

### Community 68 - ".new"
Cohesion: 0.20
Nodes (28): a_snapshot_never_panics_on_a_partially_migrated_store(), active_jobs(), an_empty_database_returns_empty_sections_not_zeros(), atlas_raw_candidate_occurrences_are_not_added_twice(), collection_backlog_uses_durable_article_stages_without_legacy_units(), count_rows(), distinct_article_body_cohort_deduplicates_tags_and_excludes_empty_cache_rows(), distinct_values() (+20 more)

### Community 69 - "body_filter.rs"
Cohesion: 0.16
Nodes (26): BATCH_SIZE, BLOCK_SYSTEM, BodyChunk, chunk_article_body(), CHUNK_MAX, CHUNK_MIN, CHUNK_TARGET, chunks_preserve_absolute_offsets() (+18 more)

### Community 70 - "synthesize.rs"
Cohesion: 0.14
Nodes (21): BODY_REFINE_INPUT_CHARS, deterministic_bluf_mentions_title(), deterministic_section(), model_refine_body(), model_synthesize(), parse_json_object(), refine_retrieved_article_body(), refine_retrieved_article_body_with_runtime() (+13 more)

### Community 71 - "dataset.rs"
Cohesion: 0.17
Nodes (19): acquire_refresh_lease(), active_manifest_path(), ACTIVE_REFRESHES, dataset_root(), DatasetManifest, DatasetStatus, get_status(), import_from_file() (+11 more)

### Community 72 - "Loaded"
Cohesion: 0.11
Nodes (20): atlas_origin_rows(), category_triggers(), Dims, dims_from_key(), dominant_label(), engine_health(), EventRow, evidence_call() (+12 more)

### Community 73 - "profile_charts.rs"
Cohesion: 0.06
Nodes (61): absolute_stacks_compare_one_and_one_hundred_on_one_scale(), BLOCKS, cap_eighths(), cells(), coarsen_counts(), column_cell(), connect_cells(), count_coarsening_preserves_totals_boundaries_and_unknown_history() (+53 more)

### Community 74 - "theme.rs"
Cohesion: 0.13
Nodes (24): Block, ACCENT, BG, BORDER, card(), card_accent(), card_dim(), card_text() (+16 more)

### Community 75 - "Overlay"
Cohesion: 0.08
Nodes (23): ChoiceKind, IntelDay, Investigation, Model, Provider, LastViewSession, Overlay, AddFallback (+15 more)

### Community 76 - "pipeline.rs"
Cohesion: 0.11
Nodes (26): apply_recon_directive_coverage(), ClaimRelation, AmbiguousEntity, ChangedQuantity, Equivalent, Negation, PlannedVsCompleted, Unrelated (+18 more)

### Community 77 - "summarization.rs"
Cohesion: 0.10
Nodes (41): all_source_hashes_routes_fields_and_upgrade_guard(), cache_get(), cache_get_exact(), cache_key_changes_with_revision_and_focus(), cache_put(), cache_round_trip(), complete_summary(), CoverageMeta (+33 more)

### Community 78 - "atlas_metadata.rs"
Cohesion: 0.10
Nodes (38): actor_projection_validates_utf8_repeated_mentions_types_and_staleness(), article(), article_actors(), ArticleActor, ArticleMention, ArticleMetadata, classification_receipts_preserve_native_metadata_and_reject_stale_revision(), collect() (+30 more)

### Community 80 - "search_engines.rs"
Cohesion: 0.06
Nodes (45): build_scrape_body(), Candidate, card_snippet(), classify_status_region(), clip_diag(), clip_item_text(), collapse_ws(), collect_candidates() (+37 more)

### Community 81 - "provider_chain.rs"
Cohesion: 0.10
Nodes (32): AttemptRecord, cancellable_sleep(), cancellation_stops_the_chain(), cancelled(), ChainReport, DispatchError, execute(), ExecuteOptions (+24 more)

### Community 82 - ".new"
Cohesion: 0.25
Nodes (20): a_spent_primary_quota_uses_the_fallback_key(), classification_request(), collection_resume_retries_failed_provider_and_reuses_success_receipt(), country_labels_show_the_name_and_the_code(), daily_cap(), format_run_card(), HttpCall, HttpReply (+12 more)

### Community 83 - "apply_hits"
Cohesion: 0.15
Nodes (21): apply_hits(), article_from(), article_from_row(), article_row(), Article, canonical_url(), category_tag(), dedup_drops_a_url_an_exact_title_and_keeps_the_higher_domain() (+13 more)

### Community 84 - "ProfileView"
Cohesion: 0.11
Nodes (17): activate(), capacity_preserves_unknown_disabled_and_overflow_values(), compare_cells(), handle_key(), open_run_owner(), open_selected_owner(), ordered_rows(), ProfileView (+9 more)

### Community 85 - "SerpOutcome"
Cohesion: 0.14
Nodes (12): retry_allowed(), SerpOutcome, Challenge, Consent, ParserMismatch, RateLimited, ResponseTooLarge, UpstreamFailure (+4 more)

### Community 86 - "Command"
Cohesion: 0.10
Nodes (20): AtlasCommand, Repair, Resume, Verify, Cli, Command, Atlas, Defaults (+12 more)

### Community 87 - "DefaultsRole"
Cohesion: 0.07
Nodes (17): ChoiceItem, codex_models(), DefaultsRole, .ALL, ClaimAssessor, Classifier, EntityResolver, EvidenceCurator (+9 more)

### Community 88 - "graph_explanation/tests.rs"
Cohesion: 0.15
Nodes (25): explain(), ExplainOutcome, Failed, Saved, Superseded, ExplainReport, ExplainRequest, Faults (+17 more)

### Community 89 - "src/evidence.rs"
Cohesion: 0.09
Nodes (29): AGREEMENT_WEIGHT, AnnMeasurement, AnnPolicy, AnnDeferredUnmeasured, AnnEnabled, ExactSearch, AnnThresholds, chunk_text() (+21 more)

### Community 90 - "ToolRunner"
Cohesion: 0.12
Nodes (10): attribution_is_explicit_never_inferred_from_the_prompt(), test_tool_runner_cache_hit(), test_tool_runner_concurrency_and_dedup(), ToolAttribution, ToolRunner, .MAX_CONCURRENCY, .REQUEST_TIMEOUT, .SERP_VALID_CACHE_SECONDS (+2 more)

### Community 91 - "atlas_classify.rs"
Cohesion: 0.09
Nodes (23): article(), batch_count_is_bounded_and_empty_never_dispatches(), BATCH_SIZE, BatchAdapter, batches(), batches_for_model(), duplicate_and_unknown_ids_never_become_successful_unk(), execute() (+15 more)

### Community 92 - "F"
Cohesion: 0.23
Nodes (13): CallSpec, dispatch(), fetch_with_spare_key(), json_message(), phase1_call(), phase2_call(), pretty_body(), provider_fault() (+5 more)

### Community 93 - "subscription.rs"
Cohesion: 0.24
Nodes (8): answer_event(), check_login(), command(), complete(), completion_is_ephemeral_and_does_not_inherit_tools_or_configuration(), exec_args(), is_subscription_status(), login()

### Community 94 - "dork_generator.rs"
Cohesion: 0.09
Nodes (36): ACTIVE_SNAPSHOT, compose_single_query(), compute_query_id(), compute_template_id(), DATASET_NAME, DEFAULT_MAX_QUERIES, DorkCatalog, DorkCategory (+28 more)

### Community 95 - "Section"
Cohesion: 0.20
Nodes (8): every_section_registers_its_reviewed_widget_count(), Section, Atlas, Intel, Models, Recon, Tools, widgets_render_in_the_specs_presentation_priority_order()

### Community 96 - "How"
Cohesion: 0.11
Nodes (18): How, CompanyEmail, Coordinates, DomainAsUrl, EntityPhrase, PackageName, PackageParts, Plain (+10 more)

### Community 97 - "insight_persist.rs"
Cohesion: 0.12
Nodes (37): active_output(), advance_generation(), article_claim_rows(), article_details_eligible(), article_input(), article_readiness_requires_acquisition_coverage_even_when_display_is_complete(), article_refresh_status(), atlas_claim_rows() (+29 more)

### Community 98 - "Embedder"
Cohesion: 0.22
Nodes (6): download_file(), Embedder, ensure_file(), hf_url(), load(), model_dir()

### Community 99 - "explore.rs"
Cohesion: 0.20
Nodes (11): GraphEdgeKind, ContradictionOrUpdate, Evidenced, Inferred, SemanticSuggestion, related_evidence_view(), RelatedEvidenceHit, semantic_edges_are_not_factual() (+3 more)

### Community 100 - "ledger.rs"
Cohesion: 0.14
Nodes (13): body_assertion_candidates(), coverage_complete(), ElementStatus, Assessed, Excluded, InProgress, Pending, Superseded (+5 more)

### Community 101 - "Region"
Cohesion: 0.10
Nodes (20): Region, AtlasFeed, AtlasInsights, AtlasNews, AtlasOrigins, AtlasRuns, Chat, Detail (+12 more)

### Community 102 - "diversity.rs"
Cohesion: 0.17
Nodes (24): coverage_gaps_summary(), coverage_record_id(), coverage_stats(), CoverageCandidate, CoverageRecord, CoverageStats, covered_record(), credentials_available() (+16 more)

### Community 103 - "DecisionAnswer"
Cohesion: 0.29
Nodes (3): DecisionAnswer, parse_decisions(), unique_decision_answers()

### Community 104 - "job_cleanup.rs"
Cohesion: 0.16
Nodes (17): backfill_owners(), BATCH, cleanup_batch(), cleanup_on(), CleanupCounts, crate::store::Store, ensure_admissible(), global_cleanup_reaches_every_batch_and_keeps_younger_ownerless_jobs() (+9 more)

### Community 105 - "Frame"
Cohesion: 0.15
Nodes (47): intel_category_short(), button_areas(), cover(), draw(), draw_add_fallback(), draw_article_image(), draw_atlas(), draw_atlas_coverage() (+39 more)

### Community 106 - "search_engines/tests.rs"
Cohesion: 0.10
Nodes (42): build_serp_url(), a_non_serp_path_on_the_engine_host_is_not_a_serp_page(), a_noscript_wrapped_consent_meta_refresh_is_consent(), a_standalone_zero_count_in_a_status_region_is_a_verified_zero(), a_zero_phrase_outside_a_status_region_is_not_a_verified_zero(), an_off_host_final_url_with_cards_keeps_the_items_but_is_not_valid(), an_off_host_final_url_without_cards_is_never_a_zero(), consent_iframe_is_consent() (+34 more)

### Community 107 - "profile_config.rs"
Cohesion: 0.13
Nodes (11): a_validation_error_reports_a_pointer_and_never_a_value(), an_oversize_document_is_refused_before_anything_is_applied(), confirmed_export_overwrites_the_same_destination(), direct_buffer_change_invalidates_commit_revision(), draw(), EditorError, locate(), SECRET_WARNING (+3 more)

### Community 108 - "summary_card.rs"
Cohesion: 0.35
Nodes (11): actions(), button_at(), button_cells(), button_row_area(), draw(), height(), is_card_button(), label() (+3 more)

### Community 109 - "Architecture"
Cohesion: 0.09
Nodes (23): Admiralty source evaluation (WP:RSP), Architecture, Atlas, Binder and executor, Brain recall, Config transfer and the commit, Directives, Intel reports (+15 more)

### Community 110 - "WorkEvent"
Cohesion: 0.07
Nodes (27): atlas_auto_future_tick_waits_and_past_tick_reschedules(), atlas_auto_while_running_only_arms_the_next_slot(), atlas_history_button_counts_down_while_auto_run_is_on(), atlas_poll_wait(), manual_run_moves_the_next_auto_trigger_out_by_90_minutes(), unix_now(), work_event(), WorkEvent (+19 more)

### Community 111 - "store.rs"
Cohesion: 0.05
Nodes (39): article(), ArticleInsightCommit, atlas_article_from_row(), atlas_articles_for_intel_dedupes_same_article_id_across_runs(), atlas_articles_for_intel_filters_category_day_and_query(), atlas_auto_next_round_trips_through_app_state(), atlas_claim_identity(), atlas_prune_drops_finished_runs_older_than_the_cutoff() (+31 more)

### Community 112 - "ClockSet"
Cohesion: 0.21
Nodes (3): ClockSet, queue_time_does_not_spend_active_but_consumes_wall(), sequential_vs_overlapping_tool_budgets()

### Community 113 - "ProviderAdmission"
Cohesion: 0.18
Nodes (6): shared_rate_limit_cooldown_blocks_admission(), AdmissionGuard, note_shared_rate_limit(), provider_admission_caps_at_two(), ProviderAdmission, rate_limit_cooldown_blocks_acquire()

### Community 114 - "components.rs"
Cohesion: 0.18
Nodes (14): action_rects(), analytics_card(), clip_text(), compact_records_keep_trailing_numeric_values(), detail_records(), detail_table(), editor_height(), measured_lines() (+6 more)

### Community 115 - "normalize_destination"
Cohesion: 0.14
Nodes (17): EngineSearchResult, extract_items(), host_in_any(), host_is(), is_ad_host(), is_private_host(), normalize_destination(), ParserInput (+9 more)

### Community 116 - "release_check.mjs"
Cohesion: 0.12
Nodes (16): root, script, config, effect(), root, rules(), skills, script (+8 more)

### Community 117 - "logs.rs"
Cohesion: 0.09
Nodes (20): stamp(), areas(), button_label(), buttons(), count(), draw(), hit(), in_list() (+12 more)

### Community 118 - "briefing_view.rs"
Cohesion: 0.22
Nodes (18): bucket_extracted(), bucket_extracted_with_explanations(), buckets_split_facts_inferences_context_and_links(), claim(), claim_label(), explain_intel_links_with_model(), explain_relation_link(), ExtractedBuckets (+10 more)

### Community 119 - "src/related_articles.rs"
Cohesion: 0.13
Nodes (27): ArticleRelatedView, rows(), article(), ArticleOrigin, bounded_pages_have_true_total_and_named_matches_rank_first(), build(), canonical_url(), exact_lineage_keeps_all_cycle_memberships() (+19 more)

### Community 120 - "profile.rs"
Cohesion: 0.16
Nodes (42): amplification_lines(), attention_lines(), attribution_lines(), backlog_lines(), capacity_lines(), cause_lines(), confidence_lines(), cycle_lines() (+34 more)

### Community 121 - "TurnEvent"
Cohesion: 0.12
Nodes (18): AnswerContext, await_completion(), cut_footer(), cut_short_answer(), finish_recon_job(), deltas(), RecallInsight, Run (+10 more)

### Community 122 - "recon/graph.rs"
Cohesion: 0.06
Nodes (60): draw(), draw_path(), draw_summary(), glyph(), inset(), legend_height(), legend_parts(), legend_rows() (+52 more)

### Community 123 - "tasks.rs"
Cohesion: 0.07
Nodes (73): cache_invalidates_when_source_revision_changes(), lease_fencing_rejects_stale_epoch_and_foreign_owner(), add_missing_columns(), adopt_untracked_index_changes(), atlas_retirement_fences_refresh_and_preserves_investigation_and_index_work(), block_claimed(), claim_complete_and_retry_round_trip(), claim_index_changes() (+65 more)

### Community 124 - "ReconCommand"
Cohesion: 0.17
Nodes (12): ReconCommand, Ask, AskNew, Delete, Limits, List, New, Rename (+4 more)

### Community 125 - "config_transfer_tests.rs"
Cohesion: 0.08
Nodes (57): apply_import(), commit_profile_config(), ConfigChange, ConfigurationSnapshot, describe_changes(), export_document(), export_to_path(), import_from_path() (+49 more)

### Community 126 - "Insight extraction reliability acceptance"
Cohesion: 0.33
Nodes (5): Compatibility, Contracts exercised, Insight extraction reliability acceptance, Limits, Reproduction

### Community 127 - "MemoryPhase"
Cohesion: 0.11
Nodes (19): CycleOutcome, Blocked, Completed, Failed, Partial, Waiting, memory_cycle_payload(), memory_step() (+11 more)

### Community 129 - "TurnClock"
Cohesion: 0.10
Nodes (6): a_round_keeps_its_time_and_a_tight_ceiling_still_runs_tools(), deadline_seconds(), format_deadline(), format_span(), the_label_updates_when_a_round_adds_calls_and_when_evidence_is_measured(), TurnClock

### Community 130 - "opencode.json"
Cohesion: 0.07
Nodes (31): agents, build, general, plan, reasoningEffort, thinkingBudget, mode, model (+23 more)

### Community 131 - "Category"
Cohesion: 0.05
Nodes (48): text(), bounded(), Category, Auth, Cancelled, Configuration, Empty, InvalidModel (+40 more)

### Community 132 - "model_roles.rs"
Cohesion: 0.14
Nodes (13): AccountHealth, AuthRequired, Cooldown, CreditsExhausted, Overloaded, Ready, Restricted, Unverified (+5 more)

### Community 133 - "home_rows"
Cohesion: 0.13
Nodes (24): center_row(), composer_prompt_text(), cursor_blink_visible(), draw_composer(), draw_home(), gap_row(), home_composer_and_all_content_centered_vertically_and_horizontally(), home_composer_areas() (+16 more)

### Community 134 - "run_turn"
Cohesion: 0.09
Nodes (41): a_budgeted_call_with_a_dependency_from_an_earlier_batch_runs(), ac3_no_key_reaches_inputs_cache_keys_plan_json_raw_bodies_or_logs(), ac6_a_courtlistener_429_still_lets_the_turn_synthesize(), ac8_status_errors_and_401_403_fail_readably_and_are_not_cached(), advance_stage(), call_cached(), cancel_mid_dispatch_releases_credit_holds(), citing_synthesis() (+33 more)

### Community 135 - "DateTime"
Cohesion: 0.11
Nodes (27): ArticleFact, BriefFact, Bucket, bucket_count(), bucket_floor(), bucket_format(), bucket_start(), CoverageFact (+19 more)

### Community 136 - "OsintCommand"
Cohesion: 0.14
Nodes (14): DatasetCommand, Import, Refresh, Status, OsintCommand, Attach, Dataset, Describe (+6 more)

### Community 137 - "source_sha256"
Cohesion: 0.01
Nodes (222): source_sha256, crates/argos-osint-bin/Cargo.toml, crates/argos-osint-bin/src/cli.rs, crates/argos-osint-bin/src/main.rs, crates/argos-osint-bin/src/tui/app.rs, crates/argos-osint-bin/src/tui/article_image.rs, crates/argos-osint-bin/src/tui/atlas_actions.rs, crates/argos-osint-bin/src/tui/atlas_table.rs (+214 more)

### Community 138 - "brain_lance_off.rs"
Cohesion: 0.16
Nodes (14): begin_generation(), clear_fingerprint(), current_fingerprint(), DUPLICATE_THRESHOLD, fingerprint_matches(), GenerationProgress, LAST_ERROR, LAYOUT (+6 more)

### Community 139 - "Argos OSINT — Agent Instructions"
Cohesion: 0.06
Nodes (32): Agent Cargo isolation, Architecture notes, Argos OSINT — Agent Instructions, CI (`.github/workflows/ci.yml`), CLI Entry Points (`argos` binary), Developer Commands, diagrams, Environment Variables (+24 more)

### Community 140 - "run_atlas_inner"
Cohesion: 0.10
Nodes (19): AtlasJob, cursor_at(), cycle_paused(), Job, job_is_before(), leg_order(), lookback_from(), new_run_id() (+11 more)

### Community 141 - ".new"
Cohesion: 0.08
Nodes (65): Account, account_hits(), account_platforms_attach_to_the_subject_and_are_never_entities(), action_order(), actions_are_grounded_capped_and_not_a_sweep(), adaptive_expansion_spends_one_scarce_lookup_then_reranks(), adaptive_step(), additional_tools() (+57 more)

### Community 142 - "wikipedia_rsp.rs"
Cohesion: 0.08
Nodes (44): API, APP_STATE_KEY, cache(), CACHE_TTL, cached_index(), CachedIndex, clip_summary(), durable_ttl_and_host_spoofing_are_checked() (+36 more)

### Community 143 - "intel-brief-image-column-2026-10-11/manifest.json"
Cohesion: 0.10
Nodes (20): ARGOS_EMBED, artifact_count, capture_status, CARGO_TARGET_DIR, command, created_utc, fixtures, font (+12 more)

### Community 144 - "validate.rs"
Cohesion: 0.17
Nodes (17): paywall_and_snippet_fail_validation(), BLOCK_MARKERS, BodyQuality, Complete, Partial, Unavailable, Uncertain, BodyValidation (+9 more)

### Community 145 - "ConfigView"
Cohesion: 0.20
Nodes (4): a_directory_target_is_an_actionable_error_not_a_crash(), ConfigView, editing_the_buffer_disarms_the_import_button(), expand_home()

### Community 146 - "Verbs"
Cohesion: 0.07
Nodes (3): Contractions, Sentences, Verbs

### Community 147 - "Rect"
Cohesion: 0.08
Nodes (59): add_fallback_layout(), add_fallback_popup_area(), article_image_area(), atlas_hit(), atlas_row_at(), brain_form(), brain_hit(), brain_list() (+51 more)

### Community 148 - "QueryExecutionStatus"
Cohesion: 0.18
Nodes (11): QueryExecutionStatus, Cancelled, Completed, Failed, Generated, NoResults, Queued, Running (+3 more)

### Community 149 - "results.rs"
Cohesion: 0.13
Nodes (20): classify_output_quality(), extract_observation_items(), extract_search_results(), extracts_from_array_shaped_observations(), extracts_from_object_with_results_array(), normalize_tool_result(), NormalizedToolResult, OutputQuality (+12 more)

### Community 150 - "DecisionPolicy"
Cohesion: 0.13
Nodes (13): DecisionService, DecisionPolicy, EnforcementOutcome, Pass, Reject, Unavailable, Uncertain, ThresholdProfile (+5 more)

### Community 151 - "ToolOutcome"
Cohesion: 0.12
Nodes (14): cache_ttl_for(), is_named_serp(), SerpCacheClass, Failure, Valid, VerifiedZero, ToolOutcome, Blocked (+6 more)

### Community 152 - "WidgetSpec"
Cohesion: 0.25
Nodes (19): ATTENTION, bucket_end(), bucket_start(), build_report_detail_lines(), categorical_plot(), count_buckets(), dashboard_lines(), draw_report() (+11 more)

### Community 153 - "App"
Cohesion: 0.23
Nodes (23): content_layout(), dashboard_layout(), draw_actions(), draw_filter_strip(), draw_picker(), draw_profile(), draw_section_body(), draw_section_navigator() (+15 more)

### Community 154 - "tui_review.py"
Cohesion: 0.12
Nodes (13): cargo_command(), cargo_env(), main(), box(), color(), render(), capture(), digest() (+5 more)

### Community 155 - "TelemetryEvent"
Cohesion: 0.13
Nodes (10): clamp(), coverage_reports_observed_since_and_counts(), db(), events_persist_and_roll_up_once(), id_is_stable_so_replay_never_double_counts(), logical_attempt_and_cache_rows_stay_distinct_under_replay(), record(), safe_payload() (+2 more)

### Community 156 - "investigation/evidence.rs"
Cohesion: 0.12
Nodes (24): accepted_evidence_events_are_stable_and_keep_provenance(), assess_claim_against_passages(), ClaimAssessmentOutcome, ClaimStance, Disputed, Insufficient, Mention, Supported (+16 more)

### Community 157 - "serde"
Cohesion: 0.08
Nodes (17): InvestigationPattern, ArticleVerification, Bitcoin, BreakingNews, DomainIp, EmailAttribution, FollowUp, GeneralSubject (+9 more)

### Community 164 - "ToolResult"
Cohesion: 0.09
Nodes (24): ToolResult, cacheable(), evidence_summary(), bounded_reason(), BudgetedOutcome, call_serves(), directive_assessments(), DirectiveAssessment (+16 more)

### Community 166 - "SiteOutcomeStatus"
Cohesion: 0.18
Nodes (10): deserialize_flexible_bool(), SiteOutcomeStatus, Ambiguous, Blocked, Error, Found, NotFound, RateLimited (+2 more)

### Community 167 - "investigation.rs"
Cohesion: 0.05
Nodes (65): absorb_hit(), account_platform_host(), ACCOUNT_PLATFORMS, ACCOUNTS, accounts_flow(), ADAPTIVE, Alternative, binder_maps_kinds_to_inputs_and_never_hands_hunter_a_social_host() (+57 more)

### Community 168 - "PipelineInput"
Cohesion: 0.25
Nodes (14): assess(), BATCH_LIMIT, compile(), dictionary(), fits(), Item, operation_scope(), partition_item() (+6 more)

### Community 169 - "paths.rs"
Cohesion: 0.25
Nodes (11): auth_path(), config_journal_path(), config_lock_path(), config_path(), dataset_dir(), datasets_dir(), db_path(), ensure_home() (+3 more)

### Community 170 - "ModuleId"
Cohesion: 0.15
Nodes (16): ModuleId, .ALL, Atlas, Brain, Intel, Jobs, Logs, Osint (+8 more)

### Community 171 - "atlas_work.rs"
Cohesion: 0.05
Nodes (41): dropped_from_receipt(), articles_rev(), AttemptRecord, check_dependency_coverage(), COLLECTION_CONTRACT, CycleOutcome, Blocked, Cancelled (+33 more)

### Community 174 - "BrainResourceSummary"
Cohesion: 0.20
Nodes (10): bindings_use_brain_evidence_ids(), BrainResourceHit, BrainResourceSummary, candidate_json(), clip(), format_counts(), is_http_url(), parse_brain_scrape_index() (+2 more)

### Community 175 - "QuestionSpec"
Cohesion: 0.26
Nodes (5): DecisionQuestionType, Choice, Noul, Score, QuestionSpec

### Community 176 - "select"
Cohesion: 0.11
Nodes (17): marker(), POLICY, select(), Selection, SelectionError, BudgetExceeded, Incomplete, InvalidSource (+9 more)

### Community 177 - "ui.rs"
Cohesion: 0.04
Nodes (69): ACTION_H, atlas_countdown(), atlas_history_live_label(), atlas_source_anchors_are_not_labeled_deleted_origin(), center_line(), center_text(), ChatRow, clip_chars() (+61 more)

### Community 178 - "RecoveryAction"
Cohesion: 0.12
Nodes (12): classify_recovery(), RecoveryAction, AccessRestricted, CircuitCooldown, ContentRefused, CreditsExhausted, FallbackModel, ReduceContext (+4 more)

### Community 179 - "grok_oauth.rs"
Cohesion: 0.16
Nodes (20): auth_path(), bearer(), bearer_from_path(), check_login(), Entry, entry_from(), failed_login_keeps_the_cli_reason_and_recovery_action(), login() (+12 more)

### Community 180 - ".clear"
Cohesion: 0.11
Nodes (8): draft_isolation_and_persistence(), duplicate_submission_prevention(), home_order_renames_and_nine_routes_agree(), PaletteItem, session_tabs_open_close_reopen(), single_action_launch_from_home(), tab_strip_render_and_hit_test(), valid_browser_url()

### Community 181 - "Conventions"
Cohesion: 0.18
Nodes (11): Agent scratch, Architecture and concepts, Checks, Commits and versions, Conventions, Diagrams, Graphify, Planning files (+3 more)

### Community 182 - "Field"
Cohesion: 0.10
Nodes (18): backspace_after_a_sent_question_deletes_one_character(), DefaultRole, defaults_pick_provider_and_model_from_account_access(), defaults_tool_picker_saves_only_its_role(), intel_day_button_label(), intel_opens_bulletin_filters_and_opens_briefing(), is_picker_field(), multiline_paste_stays_in_composer_and_does_not_run_shortcuts() (+10 more)

### Community 184 - "config_transfer.rs"
Cohesion: 0.08
Nodes (65): apply_provider(), apply_role(), apply_tool_slot(), array_value(), check_known_fields(), check_limit(), check_required(), check_role_compatibility() (+57 more)

### Community 185 - "nlp/mod.rs"
Cohesion: 0.07
Nodes (52): MemoryCache, annotate_with_policy(), Annotation, BytePermit, capture_page(), configured_mode(), CoverageRequirement, Complete (+44 more)

### Community 187 - "Concepts"
Cohesion: 0.18
Nodes (11): Apps and internal IDs, Bindings, Concepts, Graph, Intel report jobs, Model roles, NLP artifacts and selections, Persistence (+3 more)

### Community 202 - "TUI components"
Cohesion: 0.08
Nodes (25): v(), Dashboard preset, Primary inventory, Profile analytics dashboard, State and interaction, Verification and delivery, Visual and metric rules, Acceptance (+17 more)

### Community 203 - "gates.rs"
Cohesion: 0.09
Nodes (11): execute_harness_step(), InvestigationRuntime, GateOutcome, Passed, Rejected, validate_claim_assessment(), validate_evidence_admission(), validate_publication() (+3 more)

### Community 204 - "Providers"
Cohesion: 0.22
Nodes (9): Budgeted executor, Graph explanation jobs, Investigative assessment routes, News and legal keys, OSINT data providers, Providers, Tool picker transport, Typed provider diagnostics (+1 more)

### Community 205 - "Decision roles and output contracts"
Cohesion: 0.25
Nodes (7): 12-template registry, Adapters, Decision roles and output contracts, Label and threshold are separate, State isolation, Thresholds, TUI

### Community 206 - "LayoutResult"
Cohesion: 0.19
Nodes (10): DashboardPage, Atlas, Other, Recon, Summary, LayoutResult, PanelRect, reference_geometry_and_responsive_panels() (+2 more)

### Community 207 - "insight_execution.rs"
Cohesion: 0.18
Nodes (10): operation_id(), owner_progress(), Progress, record_reuse(), retry_extraction(), slice(), SliceYield, stage_event() (+2 more)

### Community 208 - "docs/README.md"
Cohesion: 0.22
Nodes (4): Changelog, Initial baseline, Planning artifacts in Git, Reviewed TUI buffers

### Community 209 - "Profile dashboard and search"
Cohesion: 0.09
Nodes (23): k(), x(), Analytics controls, Import semantics, Metric dictionary, Named search engines, Profile dashboard and search, Provider orchestration companion (+15 more)

### Community 210 - "CycleTelemetry"
Cohesion: 0.18
Nodes (10): a_resumed_cycle_continues_candidate_numbering(), cycle_event_id_is_the_run_key_so_replay_never_double_counts(), CycleTelemetry, db(), each_candidate_occurrence_gets_exactly_one_disposition(), OpenStage, origin_snapshots_are_one_row_per_origin_per_cycle(), rfc_ms() (+2 more)

### Community 212 - "tool_runner.rs"
Cohesion: 0.10
Nodes (20): ToolDefinition, a_synthesized_timeout_is_a_failure_with_a_tool_timeout_reason(), cancelled_and_cache_modes_are_not_remote_requests(), definition(), every_other_tool_keeps_its_catalog_cache_lifetime(), failure_outcome(), failure_reason(), InvocationFact (+12 more)

### Community 213 - "budget.rs"
Cohesion: 0.11
Nodes (22): courtlistener_spacing_and_firecrawl_polling_are_counted(), CUT_NOTE, CUT_SHORT, live(), more_calls_mean_more_tool_time_and_cache_hits_add_nothing(), PHASE_WINDOW_SECONDS, RECON_DEADLINE, RECON_ROUND_SECONDS (+14 more)

### Community 215 - "modes.rs"
Cohesion: 0.27
Nodes (12): chat_response_spec(), chat_response_spec_embeds_section_guidelines_and_style_guide(), chat_response_spec_lists_mode_headings(), chat_section_titles(), every_mode_has_bluf_first_and_sources_last(), every_mode_section_has_guideline(), investigation_mode_spec(), investigation_mode_spec_guides_directives_and_picker() (+4 more)

### Community 216 - "Unified Investigation Harness"
Cohesion: 0.29
Nodes (7): Catalog projection, Model roles, Persistence, Reasoning channels, Shared claims and derivations, Unified Investigation Harness, Validation gates

### Community 217 - "RendererKind"
Cohesion: 0.17
Nodes (12): applicable_filters(), detail_columns(), renderer_kind(), RendererKind, Comparison, Meter, Ranked, Table (+4 more)

### Community 219 - "TaskState"
Cohesion: 0.18
Nodes (9): TaskState, Cancelled, Completed, Failed, Paused, Queued, RetryScheduled, Running (+1 more)

### Community 220 - "Argos OSINT"
Cohesion: 0.33
Nodes (6): Applications, Argos OSINT, Documentation, Figures, Limits, Quick start

### Community 221 - ".order"
Cohesion: 0.27
Nodes (8): checked_serves(), context_additions(), context_tools(), fallback_record(), OrderContext, Picker<'a>, serves_for(), serving()

### Community 222 - "EventKind"
Cohesion: 0.09
Nodes (22): applicable_filter_dimensions(), bounded_label(), Reader<'a>, EventKind, .ALL, AtlasCandidate, AtlasCycle, AtlasOriginSnapshot (+14 more)

### Community 225 - "nlp/engine.rs"
Cohesion: 0.17
Nodes (11): ambiguous_date(), chunk_limits_and_interrupt(), chunks(), CORE, DATES, Engine, Guard, invalid_calendar() (+3 more)

### Community 228 - "graph_explanation.rs"
Cohesion: 0.07
Nodes (24): BASIC_HEADING, Cached, None, Stale, Valid, FAILURE_COOLDOWN, Gate, CoolingDown (+16 more)

### Community 232 - "model_facts"
Cohesion: 0.11
Nodes (22): amplification_is_a_raw_multiplier_with_an_eligible_denominator(), AmplificationBucket, AttemptFact, Cell, classify_model_failure(), FailureBucket, mean(), model_blocked() (+14 more)

### Community 233 - "profile_components.rs"
Cohesion: 0.23
Nodes (7): clipped(), Kpi, panel(), table_row(), count_kpi(), kpis(), rate_kpi()

### Community 234 - "atlas_recall_tests.rs"
Cohesion: 0.33
Nodes (9): ambiguous_legacy_unk_is_preserved_and_flagged_without_reclassification(), assert_no_cycle_reasoning(), empty_cycle_finishes_without_any_model_configuration(), historical_maintenance_rejects_new_contract_and_startup_does_not_repair(), legacy_phase4_and_phase5_resume_without_synthesis_or_brain_publication(), missing_classifier_blocks_required_rows_without_faking_unk_success(), resume(), RUN (+1 more)

### Community 235 - "manifest.json"
Cohesion: 0.09
Nodes (21): ARGOS_EMBED, artifact_count, capture_status, CARGO_TARGET_DIR, command, created_utc, fixtures, font (+13 more)

### Community 237 - "parse_serp_response"
Cohesion: 0.20
Nodes (18): cache_fragment(), parse_serp_response(), a_redirected_final_url_is_not_used_to_rebuild_the_query(), a_single_retry_is_allowed_only_for_a_parser_mismatch_with_a_dom(), an_empty_dom_without_any_payload_is_a_parser_mismatch(), api_http_404_is_an_upstream_failure(), api_http_404_with_a_success_envelope_is_an_upstream_failure(), cache_fragment_carries_both_versions_and_changes_with_query_and_limit() (+10 more)

### Community 238 - "insight_reliability_tests.rs"
Cohesion: 0.29
Nodes (14): answers(), captured_routes_and_contract_admit_replacement_without_revisit_retry_loops(), claims(), compact_evidence_rejects_unknown_ids_bad_utf8_and_out_of_window_spans(), entity_resolver_routes_are_relevant_to_article_and_recon_but_not_report_only_work(), finish(), invalid_output_repairs_once_then_uses_declared_fallback_and_legacy_validation_stays_terminal(), overlapping_windows_dedup_before_assessment_preserve_late_denial_and_bound_old_oversized_merge() (+6 more)

### Community 240 - "provider_metrics.rs"
Cohesion: 0.22
Nodes (10): cache_capacity_rows(), cached_rows_become_an_available_snapshot(), capacity_snapshot(), CapacityRow, CapacitySnapshot, memory(), missing_companion_snapshot_is_unavailable_not_zero(), QueueRow (+2 more)

### Community 242 - "RunStats"
Cohesion: 0.18
Nodes (15): Stats, Status, charge_newsapi(), charge_quota(), CoverageStats, Cursor, open_key(), park() (+7 more)

### Community 243 - "compromise.js"
Cohesion: 0.15
Nodes (4): i(), Parentheses, Pronouns, Quotations

### Community 244 - ".recon_outcomes"
Cohesion: 0.20
Nodes (9): normalize_run_outcome(), ReconOutcomeBucket, RunOutcome, Cancelled, CompletedWithEvidence, CompletedZeroEvidence, Failed, Partial (+1 more)

### Community 245 - "search_engines_fixtures.rs"
Cohesion: 0.37
Nodes (12): every_fixture_matches_its_recorded_outcome(), field(), fixture_cases(), fixtures_dir(), no_fixture_claimed_a_verified_zero_without_a_supported_phrase(), opt_str(), opt_usize(), read_json() (+4 more)

### Community 247 - "AtlasInsightClaim"
Cohesion: 0.28
Nodes (14): claim(), article(), claim(), commit_replaces_atomically_and_keeps_shared_support(), failed_validation_leaves_prior_insights(), replace_article_insights_from_body(), replace_article_insights_from_body_with_cancel(), replace_article_insights_with_runtime() (+6 more)

### Community 250 - "Day"
Cohesion: 0.13
Nodes (4): CalendarDate, Day, Holiday, WeekDay

### Community 251 - "Trigger"
Cohesion: 0.17
Nodes (11): TELEMETRY_TRIGGER, is_forbidden_key(), payload_number(), Trigger, AtlasCycle, IntelBrief, ManualTool, ReconPrompt (+3 more)

### Community 252 - "RepairStatus"
Cohesion: 0.22
Nodes (9): RepairStatus, Active, IndexingDisabled, IndexQueued, MissingPayload, NoInsights, Partial, Repaired (+1 more)

### Community 253 - ".intel_reports"
Cohesion: 0.18
Nodes (10): normalize_report_outcome(), report_key(), ReportModeRow, ReportOutcome, Blocked, Cancelled, Completed, Failed (+2 more)

### Community 254 - "§9 implementation order"
Cohesion: 0.17
Nodes (11): §10 acceptance checks, §9 implementation order, Atlas memory completion and System apps — implementation checklist, Phase 1 — what landed, Phase 2 — what landed, Phase 3 — what landed, Phase 4 — what landed, Phase 5a — what landed (+3 more)

### Community 258 - "actor_review.rs"
Cohesion: 0.26
Nodes (11): ActorReviewItem, ActorReviewResult, apply_reviewed_actors(), deterministic_review_actors(), is_meaningful_actor(), JUNK_ACTORS, model_review_actors(), parse_actor_review_response() (+3 more)

### Community 259 - "InvestigationSurface"
Cohesion: 0.09
Nodes (12): InvestigationSurface, HomeComposer, IntelBrief, JobsResume, ReconChat, atlas_only_tools_blocked_on_all_surfaces(), check_need_gate(), sociavault_blocked_on_intel_surface() (+4 more)

### Community 262 - "Target"
Cohesion: 0.04
Nodes (51): FocusEntry, hit(), IntelReconFocus, Section, Start, Tab, LayoutRegistry, RelatedArticleBack (+43 more)

### Community 263 - "cache.rs"
Cohesion: 0.30
Nodes (10): artifact(), canonical(), feature_key(), gc(), get(), key(), migrate(), put() (+2 more)

### Community 268 - "DecisionState"
Cohesion: 0.18
Nodes (4): DecisionState, EvidencePassageState, SubjectStateSummary, TaskStateSummary

### Community 269 - "AuthFile"
Cohesion: 0.12
Nodes (11): resolve_actor_reviewer_secret(), RoleRuntime, snapshot_secret(), accounts_persist_with_owner_only_permissions(), assert_no_staged_files(), auth_save_commits_only_the_auth_slot(), AuthFile, legacy_connection_migrates_without_overwriting_other_accounts() (+3 more)

### Community 270 - "Profile TUI implementation session — 2026-10-10"
Cohesion: 0.22
Nodes (9): Artifact retention, Defects found and corrected, Fixture and screenshot construction, Gate results and environment handling, Ownership and implementation, Profile TUI implementation session — 2026-10-10, Scope and starting evidence, Screenshot evidence (+1 more)

### Community 271 - "Atlas classification and Related articles"
Cohesion: 0.40
Nodes (5): Atlas classification and Related articles, Classification and native metadata, Completion, recovery and compatibility, Related leads and storage, Verification

### Community 274 - "CheckSignal"
Cohesion: 0.22
Nodes (8): CheckSignal, Blocked, Error, Inconclusive, NotRegistered, RateLimited, Registered, Unsupported

### Community 276 - "Diagram conventions"
Cohesion: 0.50
Nodes (4): Agent rule, Catalog, Diagram conventions, When to draw

### Community 279 - "TUI implementation and verification"
Cohesion: 0.29
Nodes (7): Build fixtures and assertions, Capture actual cells, Finish and retain evidence, Plan the contract, Review, fix and recapture, Run the check gate, TUI implementation and verification

### Community 281 - "Unified reliability / summarization / semantic pipelines — completion checklist"
Cohesion: 0.40
Nodes (4): Honest remaining gaps, Phase status (§18), This follow-up, Unified reliability / summarization / semantic pipelines — completion checklist

### Community 282 - "super"
Cohesion: 0.06
Nodes (26): rows(), intel_recon_publishes_durably_with_provenance_and_no_atlas_receipt(), ReconInsightUpdate, update(), upsert_adds_and_updates_claims(), upsert_recon_insights(), CODES, NAMES (+18 more)

### Community 286 - "rusqlite"
Cohesion: 0.23
Nodes (11): article_confidence(), claim(), EvidenceConfidence, persist_article_rating(), rating_gold_cases_round_only_the_final_mean(), RATING_POLICY, reference(), reports_require_embedded_verified_roots_and_legacy_is_unassessed() (+3 more)

### Community 287 - "Semantic-versioning setup acceptance"
Cohesion: 0.67
Nodes (3): Checks and results, Limits and remaining setup, Semantic-versioning setup acceptance

### Community 289 - "TUI overhaul verification — 10 October 2026"
Cohesion: 0.33
Nodes (6): Defects found and corrected, Interaction and metric evidence, Limits, Reproduction, TUI overhaul verification — 10 October 2026, Visual evidence

### Community 291 - ".new"
Cohesion: 0.28
Nodes (5): aging_the_tool_clock_blocks_new_calls_and_cache_hits_do_not_extend_it(), clock_set_for_turn(), foreground_expiry_continues_in_background(), sequential_raise_calls_budgets_higher(), TurnCheckpoint

### Community 292 - "View"
Cohesion: 0.12
Nodes (6): View, Agent commit tasks, Commit and PR convention, GitHub setup and first release, Release automation, Versioning, commits and releases

### Community 294 - "pull_request_template.md"
Cohesion: 0.50
Nodes (3): Change, Compatibility, Validation

### Community 295 - "release-please-config.json"
Cohesion: 0.50
Nodes (3): bootstrap-sha, packages, $schema

### Community 296 - "Local NLP preprocessing"
Cohesion: 0.18
Nodes (11): Configuration and build, Earlier implementation fixture measurements (2026-10-11), Evidence and ownership, Implementation measurements (2026-10-11), Integration inventory, Investigative coverage and publication, Local NLP preprocessing, Passage and model contracts (+3 more)

### Community 301 - "LevelFilter"
Cohesion: 0.29
Nodes (5): LevelFilter, All, Error, Info, Warn

### Community 302 - "RouteInput"
Cohesion: 0.17
Nodes (11): RouteInput, FacebookUrl, Handle, HandleOrUserId, Hashtag, LinkedinCompanyUrl, LinkedinProfileUrl, Query (+3 more)

### Community 303 - "intel-brief-image-column-2026-10-11/README.md"
Cohesion: 0.40
Nodes (3): Intel brief image column — 2026-10-11, Intel image quality review — 2026-10-10, iTerm2 protocol follow-up

### Community 314 - "PickRequest"
Cohesion: 0.23
Nodes (8): chat_request(), Ordered, parse_chat_pick(), PickReply, PickRequest, PickRequestAdapter, PickRequestAdapter<'r, 'b>, state()

### Community 315 - ".set_focus"
Cohesion: 0.06
Nodes (17): add_scroll(), brain_anchors_follow_memory_focus_and_scroll_stops_at_ends(), AtlasRun, JobSource, AtlasRun, ReconThread, keyboard_navigation_esc_and_shortcuts(), overhaul_tool_category_collapse_preserves_identity_and_search_state() (+9 more)

### Community 316 - "Argos documentation"
Cohesion: 0.40
Nodes (5): Agent and workflow, Argos documentation, Checklists (historical), Figures, Product

### Community 317 - "TurnContinuation"
Cohesion: 0.40
Nodes (4): TurnContinuation, Active, Background, Exhausted

### Community 318 - "poll_device"
Cohesion: 0.47
Nodes (6): Poll, Denied, poll_device(), Pending, SlowDown, Token

### Community 322 - "BrainListMode"
Cohesion: 0.50
Nodes (4): BrainListMode, Create, Graph, List

### Community 325 - "Argos UI Interaction Audit"
Cohesion: 0.40
Nodes (5): Argos UI Interaction Audit, Audit Matrix, Field inventory (`field_placeholder`), Remaining limits, Shared contracts

### Community 327 - "Surface"
Cohesion: 0.40
Nodes (5): Surface, Article, Atlas, Recon, Report

## Knowledge Gaps
- **2362 isolated node(s):** `$schema`, `default_agent`, `subagent_depth`, `timeout`, `chunkTimeout` (+2357 more)
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 3271 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **53 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `TUI design contract` connect `TUI components` to `docs/README.md`, `Frame`, `provider_metrics.rs`?**
  _High betweenness centrality (0.097) - this node is a cross-community bridge._
- **What connects `$schema`, `default_agent`, `subagent_depth` to the rest of the system?**
  _2362 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `land.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.0076045627376425855 - nodes in this community are weakly interconnected._
- **Why does `App` connect `App` to `InvestigationSurface`, `app.rs`, `Target`, `recon.rs`, `AuthFile`, `article_image.rs`, `ConfigView`, `Store`, `hardware.rs`, `rusqlite`, `Call`, `ToolResult`, `brain_detail.rs`, `ModuleId`, `ReportMode`, `tui/jobs.rs`, `.clear`, `Field`, `src/brain.rs`, `.set_focus`, `SettingsFile`, `BrainListMode`, `atlas_insights.rs`, `Overlay`, `atlas_metadata.rs`, `apply_hits`, `ProfileView`, `DefaultsRole`, `summary_card.rs`, `WorkEvent`, `store.rs`, `RunStats`, `logs.rs`, `briefing_view.rs`, `src/related_articles.rs`, `recon/graph.rs`?**
  _High betweenness centrality (0.085) - this node is a cross-community bridge._
- **Should `orchestrate.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.04316090130117423 - nodes in this community are weakly interconnected._
- **Why does `Other application presets` connect `Frame` to `App`, `App`, `home_rows`, `TUI components`?**
  _High betweenness centrality (0.081) - this node is a cross-community bridge._
- **Should `Store` be split into smaller, more focused modules?**
  _Cohesion score 0.046620046620046623 - nodes in this community are weakly interconnected._