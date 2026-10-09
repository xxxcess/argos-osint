# Graph Report - argos-osint  (2026-10-09)

## Corpus Check
- 183 files · ~413,968 words
- Verdict: corpus is large enough that graph structure adds value.
- Unclassified: 9 file(s) not represented in the graph (top: (none) 3, .toml 2, .diff 2)

## Summary
- 6531 nodes · 16095 edges · 237 communities (211 shown, 26 thin omitted)
- Extraction: 98% EXTRACTED · 2% INFERRED · 0% AMBIGUOUS · INFERRED: 287 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `ae9d47f1`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- land.rs
- orchestrate.rs
- Binding
- atlas_memory.rs
- tool_io.rs
- .select
- brain_lance.rs
- FieldId
- map.rs
- recon.rs
- ButtonId
- ToolResult
- IntelligenceCategory
- job_registry.rs
- app.rs
- directives.rs
- .handle_key
- absorb_hit
- providers.rs
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
- .field
- provider.rs
- Memory
- InvestigationPart
- body.rs
- ProviderSecret
- brain_detail.rs
- tui/graph.rs
- atlas.rs
- run_atlas_inner
- whatsmyname.rs
- grok_oauth.rs
- jobs_view.rs
- provider_attempt.rs
- LogicalRole
- picker.rs
- ReportMode
- provider_chain.rs
- reliability_faults.rs
- .run_configured
- App
- events.rs
- exec.rs
- draw_intel_briefing
- model_exec.rs
- ErrorCategory
- ChatMessage
- MemoryKind
- Frame
- brain_resources.rs
- worker.rs
- Target
- src/brain.rs
- intel_recon/jobs.rs
- cli.rs
- atlas_insights.rs
- InformationCredibility
- provider_diag.rs
- body_filter.rs
- synthesize.rs
- dataset.rs
- Call
- Category
- atlas_table.rs
- ProviderPage
- pipeline.rs
- summarization.rs
- InvestigationSurface
- LogsView
- search_engines.rs
- tui/jobs.rs
- .new
- FeedArticle
- budget.rs
- Request
- Command
- DefaultsRole
- theme.rs
- src/evidence.rs
- atomic
- anyhow
- AtlasArticleRow
- dispatch
- dork_generator.rs
- graph_explanation/tests.rs
- How
- H3 Task Packet: State and Persistence Foundation
- summary_card.rs
- actor_review.rs
- ModuleId
- Region
- graph_explanation.rs
- validate.rs
- KeptClaim
- logs.rs
- rows_for
- secrets.rs
- Service
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
- InsightStats
- IndexOutcome
- recon/graph.rs
- tasks.rs
- ReconCommand
- JobsView
- .new
- modes.rs
- ServiceSpec
- TurnClock
- agent
- Functional Requirements
- JobStatusFilter
- spotify.rs
- ui.rs
- TaskState
- OsintCommand
- RouteInput
- Severity
- Argos OSINT — Agent Instructions
- AtlasInsightClaim
- TaskStatus
- wikipedia_rsp.rs
- rusqlite
- serde_json
- JobRow
- poll_device
- .is_empty
- QueryExecutionStatus
- results.rs
- Store
- store_cache
- QuestionSpec
- Rect
- §9 implementation order
- .new
- investigation/evidence.rs
- InvestigationPattern
- Part
- argos-osint-bin
- What Was Learned
- AnnPolicy
- SiteOutcomeStatus
- investigation.rs
- Milestone 1 — Core Onboarding (Current)
- paths.rs
- .index_now
- atlas_work.rs
- H2 Contracts: Frozen Interface Decisions
- accept_one
- Codebase Map — argos-osint
- PROJECT.md — argos-osint
- Current Phase State
- diversity.rs
- RecoveryAction
- PlanCall
- decide
- Conventions
- Docs Ingest — argos-osint
- ScheduledCall
- RawClaim
- OpenCode V2 workflow
- Unified reliability / summarization / semantic pipelines — completion checklist
- Concepts
- package.json
- explore.rs
- CheckSignal
- LevelFilter
- Providers
- Decision roles and output contracts
- H0-H2 Completion Summary
- Terminal UI
- registry
- F
- youtube_pair
- CycleOutcome
- render_tui_cells.py
- RecordKind
- super
- Unified Investigation Harness
- ClaimRelation
- HypothesisRecord
- TurnContinuation
- Argos OSINT
- SettingsFile
- Delivery: Recon diversity, reliability, Whoxy, and Holehe
- Argos documentation
- Diagram conventions
- extract_for_article_body
- Argos UI Interaction Audit
- ChainReport<T>
- fetch_once
- RspStatus
- RspEntry
- RspIndex
- SourceReliability
- atlas_actions.rs
- .job_source

## God Nodes (most connected - your core abstractions)
1. `App` - 307 edges
2. `ProviderSecret` - 139 edges
3. `ButtonId` - 130 edges
4. `FieldId` - 74 edges
5. `ToolResult` - 70 edges
6. `Store` - 70 edges
7. `Target` - 65 edges
8. `Store` - 64 edges
9. `AtlasArticleRow` - 55 edges
10. `Binding` - 48 edges

## Surprising Connections (you probably didn't know these)
- `every_tool_request_sends_a_non_empty_user_agent()` --references--> `agent`  [INFERRED]
  crates/argos-osint-core/src/osint.rs → .opencode/opencode.json
- `success_is_keyed_and_any_input_change_invalidates_it()` --references--> `edit`  [INFERRED]
  crates/argos-osint-core/src/graph_explanation/tests.rs → .opencode/opencode.json
- `worker_killed_after_the_lance_write_is_recovered_by_revision()` --references--> `edit`  [INFERRED]
  crates/argos-osint-core/src/store/publication.rs → .opencode/opencode.json
- `table_lines()` --calls--> `format_duration()`  [INFERRED]
  crates/argos-osint-bin/src/tui/jobs.rs → crates/argos-osint-core/src/jobs_view.rs
- `all_four_modes_create_distinct_section_plans()` --calls--> `section_plan()`  [INFERRED]
  crates/argos-osint-core/src/intel_recon/tests_acceptance.rs → crates/argos-osint-core/src/intel_recon/modes.rs

## Import Cycles
- 2-file cycle: `crates/argos-osint-core/src/osint.rs -> crates/argos-osint-core/src/osint/results.rs -> crates/argos-osint-core/src/osint.rs`
- 2-file cycle: `crates/argos-osint-bin/src/cli.rs -> crates/argos-osint-core/src/hardware.rs -> crates/argos-osint-bin/src/cli.rs`

## Communities (237 total, 26 thin omitted)

### Community 0 - "land.rs"
Cohesion: 0.01
Nodes (262): CODES, LABELS, R0, R1, R10, R100, R101, R102 (+254 more)

### Community 1 - "orchestrate.rs"
Cohesion: 0.04
Nodes (154): a_429_stops_picker_calls_and_the_fallback_finishes_the_list(), a_claimed_email_removes_its_bindings(), a_dispatch_error_fails_the_step_and_the_loop_goes_on(), a_dropped_provider_stream_keeps_the_text_already_received(), a_failed_email_finder_gets_one_fallback_and_no_third_picker_call(), a_fallback_with_no_bindings_still_runs_the_satisfied_steps_and_reaches_synthesis(), a_follow_up_keeps_names_from_the_previous_synthesis(), a_gap_filler_domain_feeds_hunter_once_firecrawl_observes_it() (+146 more)

### Community 2 - "Binding"
Cohesion: 0.15
Nodes (28): Binding, canonical(), dependency_order(), depends_on(), question_bindings_and_dependency_fix(), allowed_producer(), best_handle(), bind_step() (+20 more)

### Community 3 - "atlas_memory.rs"
Cohesion: 0.06
Nodes (89): atlas_memory_count(), background_indexing_then_refresh_upgrades_a_partial_run(), Barrier, Checkpoint, CheckpointInput, claim(), claims(), clear() (+81 more)

### Community 4 - "tool_io.rs"
Cohesion: 0.04
Nodes (95): normalize_platform(), question_bindings(), a_gap_filler_only_domain_waits_for_a_primary_observation_before_hunter(), accept_bindings(), batch_scrape_takes_ranked_urls_and_gates_order_before_hunter(), bind_arguments(), BINDING_KINDS, bitcoins_in() (+87 more)

### Community 5 - ".select"
Cohesion: 0.10
Nodes (52): a_click_folds_an_event_and_incoming_entries_do_not_move_a_paused_list(), all_provider_actions_render_with_hit_areas_at_80x24(), atlas_auto_toggle_persists_the_next_trigger(), atlas_history_resume_is_offered_only_for_resumable_cycles_and_repair_starts_once(), atlas_live_feed_opens_intel_brief_and_past_runs_delete(), atlas_map_names_every_highlighted_country(), atlas_news_feed_tags_the_country_code(), atlas_world_map_follows_the_selected_run_until_zoomed_in() (+44 more)

### Community 6 - "brain_lance.rs"
Cohesion: 0.06
Nodes (38): activate_generation(), batch(), begin_generation(), begin_generation_then_activate(), block_on(), BrainIndex, clear_fingerprint(), current_fingerprint() (+30 more)

### Community 7 - "FieldId"
Cohesion: 0.03
Nodes (58): FieldId, BrainApp, BrainConversation, BrainInsight, BrainQuery, ClaimAssessorModel, ClaimAssessorProvider, ClassifierModel (+50 more)

### Community 8 - "map.rs"
Cohesion: 0.05
Nodes (77): BRAILLE_MAP, canonical(), claim_label(), contains(), country_at(), country_fit(), country_pixel(), default_world_scale_is_enlarged_and_fits_small_terminals() (+69 more)

### Community 9 - "recon.rs"
Cohesion: 0.04
Nodes (96): ac6_citation_groups_split_validate_each_id_and_normalize(), answer_evidence_survives_reopen_and_deleted_thread_rejects_late_calls(), answer_step_rate_limit_is_explicit_and_keeps_results(), attribute_word(), auxiliary(), await_completion(), BRIEF_SYNTHESIS, broad_question_collapses_to_grounded_lookups() (+88 more)

### Community 10 - "ButtonId"
Cohesion: 0.02
Nodes (106): ButtonId, Add, AddFallback, AtlasAuto, AtlasDelete, AtlasLive, AtlasNews, AtlasNewsFeed (+98 more)

### Community 11 - "ToolResult"
Cohesion: 0.05
Nodes (29): ToolResult, AnswerContext, cacheable(), cut_footer(), cut_short_answer(), evidence_summary(), finish_recon_job(), BudgetedOutcome (+21 more)

### Community 12 - "IntelligenceCategory"
Cohesion: 0.08
Nodes (23): all_catalog_tools_mapped_to_categories(), argument_contract(), ArgumentBuilderContract, compact_capability_catalog(), CompactToolCapability, IntelligenceCategory, Bitcoin, DomainNetwork (+15 more)

### Community 13 - "job_registry.rs"
Cohesion: 0.05
Nodes (48): begin(), begin_cancellable(), db(), db_path(), finish(), beat(), BEAT_INTERVAL, beats() (+40 more)

### Community 14 - "app.rs"
Cohesion: 0.04
Nodes (58): atlas_auto_future_tick_waits_and_past_tick_reschedules(), ATLAS_AUTO_SECS, atlas_auto_while_running_only_arms_the_next_slot(), atlas_claim(), atlas_countdown_visible(), atlas_extracting_visible(), atlas_headlines_scroll_and_request_failures_reach_the_system_log(), atlas_history_button_counts_down_while_auto_run_is_on() (+50 more)

### Community 15 - "directives.rs"
Cohesion: 0.06
Nodes (63): apply_context_targets(), asks(), asks_about_judge(), asks_for_headlines(), clip_words(), context_entity(), CONTEXT_FILLER, context_gate() (+55 more)

### Community 16 - ".handle_key"
Cohesion: 0.06
Nodes (20): a_finished_tool_call_is_logged_once_and_the_transcript_keeps_a_summary(), add_scroll(), backspace_after_a_sent_question_deletes_one_character(), ctrl_arrows_cycle_app_tabs_forward_and_back(), ctrl_tab_cycles_app_tabs_forward_and_back(), draft_isolation_and_persistence(), home_order_renames_and_nine_routes_agree(), intel_opens_bulletin_filters_and_opens_briefing() (+12 more)

### Community 17 - "absorb_hit"
Cohesion: 0.13
Nodes (20): absorb_hit(), account_platform_host(), Candidate, content_tokens(), domain_label(), EntityIdentifier, first_domain(), Focus (+12 more)

### Community 18 - "providers.rs"
Cohesion: 0.06
Nodes (50): clip_page(), assert_host_locked(), BATCH_SCRAPE_DEFAULT_URLS, BATCH_SCRAPE_MAX_URLS, batch_urls(), company_card(), CRAWL_MAX_PAGES, every_firecrawl_tool_builds_a_host_locked_post() (+42 more)

### Community 19 - "atlas_news.rs"
Cohesion: 0.08
Nodes (44): api_error(), ATLAS_TOOLS, authors(), clip(), country_arg(), country_code(), CURRENTS_COUNTRIES, CURRENTS_HOST (+36 more)

### Community 20 - "Store"
Cohesion: 0.06
Nodes (19): coverage_complete(), article_body_from_row(), ArticleBodyRow, element_from_row(), IntelAssessmentRow, IntelElementRow, IntelEvidenceRow, IntelInvestigationRow (+11 more)

### Community 21 - "publication.rs"
Cohesion: 0.08
Nodes (43): active_index_rows(), bump_memories_changed(), change_watcher_sees_only_committed_changes_and_coalesces(), claim(), clear(), clear_index_state(), CoverageReport, delete_run_memory_state() (+35 more)

### Community 22 - "gates.rs"
Cohesion: 0.09
Nodes (13): execute_harness_step(), InvestigationRuntime, GateOutcome, Passed, Rejected, validate_evidence_admission(), validate_publication(), validate_task_admission() (+5 more)

### Community 23 - "whoxy.rs"
Cohesion: 0.11
Nodes (35): adjacent_changes(), AdjacentChange, balance_request_url(), bounded_model_view(), check_balance(), contact(), ContactCard, date_and_limit_validation() (+27 more)

### Community 24 - "news_legal.rs"
Cohesion: 0.07
Nodes (45): a_rate_limit_switches_to_the_fallback_key_and_a_rejection_does_not(), ac1_each_request_has_its_url_params_and_auth_header_and_no_key_in_the_url(), an_echoed_key_is_redacted_from_the_stored_body_and_error(), article(), articles(), CATEGORIES, choice_arg(), context_kind() (+37 more)

### Community 25 - "hardware.rs"
Cohesion: 0.14
Nodes (20): System, CACHE_TTL_SECS, classify_arch(), disk_totals(), HardwareProfile, metal_vram_gb(), now_secs(), parse_apple_gpu_cores() (+12 more)

### Community 26 - "App"
Cohesion: 0.05
Nodes (82): active_popup_area(), atlas_auto_label(), atlas_cycle_stats_room(), atlas_cycle_stats_scroll_max(), atlas_extracting(), atlas_feed_room(), atlas_feed_room_for(), atlas_news_room() (+74 more)

### Community 27 - "Store"
Cohesion: 0.05
Nodes (15): UnitManifest, atlas_answer_id(), atlas_article_from_row(), atlas_brief_id(), atlas_claims_for_article_returns_linked_claims(), AtlasRunRow, AtlasStoredClaim, delete_article_insights_orphans_brain_and_keeps_shared() (+7 more)

### Community 28 - "embed.rs"
Cohesion: 0.08
Nodes (31): active(), DIM, disable(), disabled(), DisableGuard, download_file(), DOWNLOAD_TIMEOUT_SECS, embed_batch() (+23 more)

### Community 29 - "osint.rs"
Cohesion: 0.07
Nodes (46): bind_request(), CACHE_DAY_SECONDS, cache_identity(), CACHE_MONTH_SECONDS, cache_seconds(), CACHE_WEEK_SECONDS, canonical_tool_id(), credential_key() (+38 more)

### Community 30 - "holehe/mod.rs"
Cohesion: 0.10
Nodes (21): CACHE_TTL, cap_reports_omitted(), DEFAULT_MAX_SITES, default_selection_uses_implemented_adapters(), email_hash(), email_preserves_local_part(), GLOBAL_CONCURRENCY, lookup() (+13 more)

### Community 32 - "provider.rs"
Cohesion: 0.06
Nodes (43): a_blank_osint_user_agent_loads_as_unset(), concrete_free_models(), DECISIONS_MODELS, decisions_url(), default_credit_reset(), default_firecrawl_credits(), default_grok_model(), default_hunter_credits() (+35 more)

### Community 34 - "InvestigationPart"
Cohesion: 0.12
Nodes (15): InvestigationPart, DirectiveAssessment, EvidencePassage, GateValidation, Handoff, Plan, PlanDiagnostics, RoleDecision (+7 more)

### Community 35 - "body.rs"
Cohesion: 0.09
Nodes (35): body_fetch_routes(), BodyFetchEvent, Attempt, Failed, InsightsFailed, InsightsRefreshing, InsightsReplaced, Progress (+27 more)

### Community 36 - "ProviderSecret"
Cohesion: 0.16
Nodes (26): resolve_actor_reviewer_secret(), account_secret(), active_text_secret(), authorize(), bearer_token(), catalog_error(), effective_kind(), http() (+18 more)

### Community 37 - "brain_detail.rs"
Cohesion: 0.05
Nodes (49): areas(), BrainDetail, DetailAreas, DetailPane, Path, Related, Summary, DetailSnapshot (+41 more)

### Community 38 - "tui/graph.rs"
Cohesion: 0.28
Nodes (15): draw(), draw_path(), draw_summary(), inset(), legend_height(), legend_parts(), legend_rows(), path_content() (+7 more)

### Community 39 - "atlas.rs"
Cohesion: 0.08
Nodes (32): article_card_labels_title_publisher_author_and_classification(), Band, band_sizes(), band_temperature(), category_from_decisions(), CATEGORY_IDS, CLUSTERS, country_label() (+24 more)

### Community 40 - "run_atlas_inner"
Cohesion: 0.09
Nodes (31): AtlasEvent, Classified, InsightProgress, MemoriesChanged, MemoryProgress, Note, Replaced, Stats (+23 more)

### Community 41 - "whatsmyname.rs"
Cohesion: 0.06
Nodes (39): AccountTuple, ACTIVE_SNAPSHOT, ADAPTER_VERSION, apply_strip_bad_char(), benchmark_14_4_parse_index_and_selection(), cache_get(), cache_set(), CACHE_TTL (+31 more)

### Community 42 - "grok_oauth.rs"
Cohesion: 0.06
Nodes (42): auth_path(), bearer(), bearer_from_path(), check_login(), Entry, entry_from(), failed_login_keeps_the_cli_reason_and_recovery_action(), login() (+34 more)

### Community 43 - "jobs_view.rs"
Cohesion: 0.11
Nodes (22): summary_text(), active_time_is_live_for_open_attempts_and_unavailable_only_without_a_start(), AttemptRow, event_apps(), event_counts(), EventCounts, format_duration(), get_job() (+14 more)

### Community 44 - "provider_attempt.rs"
Cohesion: 0.11
Nodes (38): Acc, attempt(), attempt_with_observer(), AttemptReport, check_final(), complete_stream_and_json_answers_succeed(), Deadlines, delta() (+30 more)

### Community 45 - "LogicalRole"
Cohesion: 0.13
Nodes (11): LogicalRole, .ALL, ClaimAssessor, Classifier, Controller, EntityResolver, EvidenceCurator, Planner (+3 more)

### Community 46 - "picker.rs"
Cohesion: 0.08
Nodes (39): brain_scrape_options_hide_bare_scrape_and_include_claim_criteria(), CatalogEntry, chat_request(), checked_serves(), CONFIDENCE_FLOOR, context_additions(), context_tools(), decisions_request() (+31 more)

### Community 47 - "ReportMode"
Cohesion: 0.12
Nodes (22): HomeDraftState, classifiable_modes(), classify_mode_chat(), classify_mode_decisions(), classify_prompt_mode(), classify_prompt_mode_chat(), classify_prompt_mode_decisions(), classify_recon_mode() (+14 more)

### Community 48 - "provider_chain.rs"
Cohesion: 0.10
Nodes (32): AttemptRecord, cancellable_sleep(), cancellation_stops_the_chain(), cancelled(), ChainReport, DispatchError, execute(), ExecuteOptions (+24 more)

### Community 49 - "reliability_faults.rs"
Cohesion: 0.08
Nodes (15): AttemptOutcome, categorize_http(), execute_with_retries(), other_llm_allows_three(), summarization_stops_at_two_attempts(), TimeoutProfile, .CLASSIFIER, .EXTRACTION (+7 more)

### Community 50 - ".run_configured"
Cohesion: 0.16
Nodes (17): a_claimed_email_keeps_no_person_data(), bitcoin(), claimed_email(), custom_user_agent(), effective_user_agent(), error_summary(), Executor, get() (+9 more)

### Community 51 - "App"
Cohesion: 0.05
Nodes (9): a_failed_turn_keeps_the_streamed_answer_on_screen(), a_log_or_late_tool_row_does_not_drop_the_live_answer(), App, atlas_log_level(), open_external_url(), ResumeSession, pruned_history_closes_the_open_run(), synthesis_deltas_fill_the_live_bubble_and_the_saved_answer_replaces_them() (+1 more)

### Community 52 - "events.rs"
Cohesion: 0.14
Nodes (18): clear_events(), DEFAULT_RETENTION_HOURS, get_event(), job_filter_includes_descendants_and_prune_keeps_jobs(), list_events(), MAX_DETAIL_CHARS, MAX_MESSAGE_CHARS, mem() (+10 more)

### Community 53 - "exec.rs"
Cohesion: 0.14
Nodes (19): admission_account(), admission_contention_consumes_no_attempt(), AttemptEvent, Finished, Started, AttemptLog, cancelled(), complete_summary_report() (+11 more)

### Community 54 - "draw_intel_briefing"
Cohesion: 0.10
Nodes (38): abs_rect(), AbsRect, draw_centered_loading_card(), draw_clipped_button(), draw_clipped_intel_loading(), draw_clipped_md_pane(), draw_intel_body_loading(), draw_intel_briefing() (+30 more)

### Community 55 - "model_exec.rs"
Cohesion: 0.11
Nodes (21): DecisionsAdapter, ensure_operation(), execute_chat(), execute_decisions_or_chat(), ModelExecEvent, AttemptFinish, AttemptReset, AttemptStart (+13 more)

### Community 56 - "ErrorCategory"
Cohesion: 0.08
Nodes (24): operation_kind(), backoff_delay(), can_retry(), ErrorCategory, AuthOrQuota, CancelledOrStale, ConfigurationMissing, ContextLimit (+16 more)

### Community 57 - "ChatMessage"
Cohesion: 0.13
Nodes (24): chat_body(), ChatMessage, complete(), complete_errors_with_finish_reason_when_response_is_empty(), complete_once(), complete_one(), complete_reads_reasoning_only_non_stream_json(), complete_stream_of_reasoning_deltas_yields_text() (+16 more)

### Community 58 - "MemoryKind"
Cohesion: 0.09
Nodes (22): AdmissionPolicy, admit_memory(), BrainQuery, pack_context(), AssessmentState, Contradicts, Insufficient, Mentions (+14 more)

### Community 59 - "Frame"
Cohesion: 0.15
Nodes (46): intel_category_short(), Block, draw(), button_areas(), cover(), draw(), draw_add_fallback(), draw_atlas() (+38 more)

### Community 60 - "brain_resources.rs"
Cohesion: 0.08
Nodes (32): source_anchor_label(), bindings_use_brain_evidence_ids(), BRAIN_SCRAPE_PREFIX, BrainResourceHit, BrainResourceSummary, candidate_json(), CLAIM_CHARS, classify_resource() (+24 more)

### Community 61 - "worker.rs"
Cohesion: 0.15
Nodes (20): IntelReportEvent, InsightsUpdated, JobCreated, JobDone, Section, Stage, IntelReportJobRow, cancel_if_needed() (+12 more)

### Community 62 - "Target"
Cohesion: 0.06
Nodes (34): brain_article_source_opens_intel_brief(), FocusEntry, hit(), IntelReconFocus, Section, Start, Tab, LayoutRegistry (+26 more)

### Community 63 - "src/brain.rs"
Cohesion: 0.12
Nodes (21): AGREEMENT_WEIGHT, CATEGORIES, category_hint(), format_injection(), hybrid_recall(), hybrid_recall_adds_paraphrases_and_rewards_agreement(), identity_query_prefers_name_memory(), jaccard() (+13 more)

### Community 64 - "intel_recon/jobs.rs"
Cohesion: 0.08
Nodes (31): active_job_for_mode(), cancel_job(), create_job_inserts_all_verify_sections(), create_report_job(), finish_canonical(), intel_recon_links_its_legacy_job_to_one_canonical_registry_job(), pause_job(), register_canonical() (+23 more)

### Community 65 - "cli.rs"
Cohesion: 0.17
Nodes (18): ask(), ask_thread(), atlas_command(), defaults_command(), defaults_json(), defaults_show_includes_the_tool_picker_transport(), dispatch(), login() (+10 more)

### Community 66 - "atlas_insights.rs"
Cohesion: 0.09
Nodes (31): a_country_token_does_not_merge_into_a_longer_name(), a_decisions_model_does_not_extract_claims(), aliases_merge_on_shared_tokens_when_the_longer_form_is_a_span(), articles_have_span(), BODY_CLAIM_LIMIT, BODY_SPAN_CHARS, COL_CLASS, COL_ENTITY (+23 more)

### Community 67 - "InformationCredibility"
Cohesion: 0.13
Nodes (17): AdmiraltyCode, best_credibility(), CredibilityInputs, information_credibility(), InformationCredibility, CannotBeJudged, Confirmed, DoubtfullyTrue (+9 more)

### Community 68 - "provider_diag.rs"
Cohesion: 0.09
Nodes (26): bounded(), cause_chain(), classify_status(), classify_text(), endpoint_strips_query_and_userinfo(), find_url(), http_failure(), Leaf (+18 more)

### Community 69 - "body_filter.rs"
Cohesion: 0.17
Nodes (22): BATCH_SIZE, BodyChunk, chunk_article_body(), CHUNK_MAX, CHUNK_MIN, CHUNK_TARGET, chunks_preserve_absolute_offsets(), classify_chunk_batch() (+14 more)

### Community 70 - "synthesize.rs"
Cohesion: 0.17
Nodes (19): BODY_REFINE_INPUT_CHARS, deterministic_bluf_mentions_title(), deterministic_section(), model_refine_body(), model_synthesize(), parse_json_object(), refine_retrieved_article_body(), refine_without_secret_keeps_validated_scrape() (+11 more)

### Community 71 - "dataset.rs"
Cohesion: 0.17
Nodes (19): acquire_refresh_lease(), active_manifest_path(), ACTIVE_REFRESHES, dataset_root(), DatasetManifest, DatasetStatus, get_status(), import_from_file() (+11 more)

### Community 72 - "Call"
Cohesion: 0.15
Nodes (23): build_blocks(), ChatBlock, clip_chars(), coverage_only_counts_explicit_assessments(), coverage_summary(), decision_row_shows_directives_picker_order_bindings_and_fallbacks(), extract_log(), input_brief() (+15 more)

### Community 73 - "Category"
Cohesion: 0.08
Nodes (21): Category, Auth, Cancelled, Configuration, Empty, InvalidModel, InvalidResult, MalformedPayload (+13 more)

### Community 74 - "atlas_table.rs"
Cohesion: 0.17
Nodes (22): border_style(), borders_and_cells_have_matching_display_widths(), bottom_border_line(), char_width(), clip_to_width(), column_widths(), columns_consume_inner_width(), columns_distribute_proportionally_when_not_flex_last() (+14 more)

### Community 75 - "ProviderPage"
Cohesion: 0.08
Nodes (20): ChoiceKind, IntelDay, Investigation, Model, Provider, LastViewSession, Overlay, AddFallback (+12 more)

### Community 76 - "pipeline.rs"
Cohesion: 0.18
Nodes (19): apply_recon_directive_coverage(), compare_claims(), coverage_requires_cited_evidence_not_similarity(), directive_coverage(), DirectiveCoverage, event_grouping_keeps_separate_days_apart(), EventGroup, group_atlas_events() (+11 more)

### Community 77 - "summarization.rs"
Cohesion: 0.10
Nodes (39): cache_get(), cache_key_changes_with_revision_and_focus(), cache_put(), cache_round_trip(), complete_summary(), CoverageMeta, deterministic_article_description(), deterministic_atlas_brief() (+31 more)

### Community 78 - "InvestigationSurface"
Cohesion: 0.09
Nodes (14): event_to_chat_block(), is_thinking_expanded(), InvestigationSurface, HomeComposer, IntelBrief, JobsResume, ReconChat, atlas_only_tools_blocked_on_all_surfaces() (+6 more)

### Community 79 - "LogsView"
Cohesion: 0.13
Nodes (8): button_label(), buttons(), LogsView, row_lines(), summary_text(), detail_rows(), event_row(), EventRow

### Community 80 - "search_engines.rs"
Cohesion: 0.16
Nodes (19): build_scrape_body(), build_serp_url(), EngineSearchResult, FIRECRAWL_GOOGLE_SEARCH, FIRECRAWL_MOJEEK_SEARCH, FIRECRAWL_YANDEX_SEARCH, normalize_destination_url(), parse_google_serp() (+11 more)

### Community 81 - "tui/jobs.rs"
Cohesion: 0.11
Nodes (21): areas(), BASE_COLUMNS, button_label(), detail_lines(), draw(), hit(), JobsAreas, LOGS_COLUMN (+13 more)

### Community 82 - ".new"
Cohesion: 0.25
Nodes (19): a_spent_primary_quota_uses_the_fallback_key(), classification_request(), country_labels_show_the_name_and_the_code(), daily_cap(), format_run_card(), HttpCall, HttpReply, insights_do_not_call_a_decisions_model() (+11 more)

### Community 83 - "FeedArticle"
Cohesion: 0.11
Nodes (26): apply_hits(), article_from(), article_from_row(), article_row(), Article, canonical_url(), dedup_drops_a_url_an_exact_title_and_keeps_the_higher_domain(), FeedArticle (+18 more)

### Community 84 - "budget.rs"
Cohesion: 0.12
Nodes (14): CUT_NOTE, CUT_SHORT, PHASE_WINDOW_SECONDS, RECON_DEADLINE, RECON_ROUND_SECONDS, STREAM_LOST, synthesis_allowance_capped(), synthesis_allowance_seconds() (+6 more)

### Community 85 - "Request"
Cohesion: 0.17
Nodes (22): annotate(), bounded(), domain(), email_address(), ip(), linkedin_handle(), number_arg(), one_of() (+14 more)

### Community 86 - "Command"
Cohesion: 0.09
Nodes (23): AtlasCommand, Repair, Resume, Verify, Cli, Command, Atlas, Defaults (+15 more)

### Community 87 - "DefaultsRole"
Cohesion: 0.07
Nodes (19): ChoiceItem, codex_models(), DefaultsRole, .ALL, ClaimAssessor, Classifier, EntityResolver, EvidenceCurator (+11 more)

### Community 88 - "theme.rs"
Cohesion: 0.18
Nodes (20): ACCENT, BG, BORDER, card(), card_accent(), card_dim(), card_text(), CODE_BG (+12 more)

### Community 89 - "src/evidence.rs"
Cohesion: 0.22
Nodes (15): AGREEMENT_WEIGHT, chunk_text(), chunks_cover_end_of_long_source(), content_hash(), ensure_identifier_coverage(), EvidencePassage, hybrid_bounds_and_prefers_agreement(), hybrid_passage_candidates() (+7 more)

### Community 90 - "atomic"
Cohesion: 0.16
Nodes (7): osint_cacheable(), test_tool_runner_cache_hit(), test_tool_runner_concurrency_and_dedup(), ToolRunner, .MAX_CONCURRENCY, .REQUEST_TIMEOUT, wait_cancellation()

### Community 91 - "anyhow"
Cohesion: 0.06
Nodes (50): compile_general_model_prompt(), compile_native(), DecisionAdapterKind, JsonMode, NativeDecisions, OutputTool, StrictJsonSchema, ValidatedText (+42 more)

### Community 92 - "AtlasArticleRow"
Cohesion: 0.20
Nodes (19): category_tag(), admiralty_scales_claim_confidence_from_rsp_and_peers(), apply_peer_support(), article(), catalog_json(), classifier_peers_are_preferred_over_token_overlap(), classify_peers_chat(), classify_peers_decisions() (+11 more)

### Community 93 - "dispatch"
Cohesion: 0.19
Nodes (15): Fault, CallSpec, dispatch(), fetch_with_spare_key(), json_message(), one_line(), parse_fault(), phase1_call() (+7 more)

### Community 94 - "dork_generator.rs"
Cohesion: 0.09
Nodes (36): ACTIVE_SNAPSHOT, compose_single_query(), compute_query_id(), compute_template_id(), DATASET_NAME, DEFAULT_MAX_QUERIES, DorkCatalog, DorkCategory (+28 more)

### Community 95 - "graph_explanation/tests.rs"
Cohesion: 0.27
Nodes (16): auth_failure_consumes_primary_budget_gives_guidance_and_redacts(), conn(), count(), failure_is_durable_correlated_bounded_and_harmless(), fixture(), Fx, GOOD, late_completion_for_a_changed_or_deleted_memory_is_not_published() (+8 more)

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
Cohesion: 0.16
Nodes (15): ModuleId, .ALL, Atlas, Brain, Intel, Jobs, Logs, Osint (+7 more)

### Community 101 - "Region"
Cohesion: 0.10
Nodes (20): Region, AtlasFeed, AtlasInsights, AtlasNews, AtlasOrigins, AtlasRuns, Chat, Detail (+12 more)

### Community 102 - "graph_explanation.rs"
Cohesion: 0.12
Nodes (14): BASIC_HEADING, event(), explain(), ExplainOutcome, Failed, Saved, Superseded, ExplainReport (+6 more)

### Community 103 - "validate.rs"
Cohesion: 0.18
Nodes (16): BLOCK_MARKERS, BodyQuality, Complete, Partial, Unavailable, Uncertain, BodyValidation, clean_markdown() (+8 more)

### Community 104 - "KeptClaim"
Cohesion: 0.20
Nodes (18): apply_admiralty_evaluation(), article_information_credibility(), brief_text(), cap_claims(), countries_differ(), dedupe_claims(), entity_path(), fingerprint() (+10 more)

### Community 105 - "logs.rs"
Cohesion: 0.16
Nodes (13): stamp(), areas(), count(), hit(), in_list(), infer_app(), list_geometry(), local_time() (+5 more)

### Community 106 - "rows_for"
Cohesion: 0.17
Nodes (15): ChatRow, clip_pieces(), disclosure_pieces(), draw_transcript(), expanded(), face_background(), memory_list_note(), paint_pieces() (+7 more)

### Community 107 - "secrets.rs"
Cohesion: 0.16
Nodes (5): accounts_persist_with_owner_only_permissions(), legacy_connection_migrates_without_overwriting_other_accounts(), loading_old_auth_removes_gmail_setup_credentials(), owner_only(), write_private()

### Community 108 - "Service"
Cohesion: 0.11
Nodes (39): a_budgeted_call_with_a_dependency_from_an_earlier_batch_runs(), call_cached(), cancelled(), classify_turn_mode(), continue_turn(), credit_map(), derive_directives(), derived_note() (+31 more)

### Community 109 - "Architecture"
Cohesion: 0.12
Nodes (17): Admiralty source evaluation (WP:RSP), Architecture, Atlas, Binder and executor, Brain recall, Directives, Intel reports, Investigation flow (+9 more)

### Community 110 - "WorkEvent"
Cohesion: 0.08
Nodes (22): WorkEvent, AnswerDelta, AnswerNote, AnswerReplacement, AnswerReset, AtlasDone, BrainRelatedExplained, CatalogDone (+14 more)

### Community 111 - "store.rs"
Cohesion: 0.09
Nodes (28): article(), atlas_articles_for_intel_dedupes_same_article_id_across_runs(), atlas_articles_for_intel_filters_category_day_and_query(), atlas_auto_next_round_trips_through_app_state(), atlas_prune_drops_finished_runs_older_than_the_cutoff(), atlas_recent_articles_lists_retained_rows(), atlas_run_days_lists_distinct_days_newest_first(), AUTO_REBUILD_HINT (+20 more)

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
Cohesion: 0.12
Nodes (7): core, home, hooks, names, payload(), run(), setup()

### Community 117 - "ServiceResult"
Cohesion: 0.23
Nodes (9): cached_result(), cancel_marks_inconclusive(), check_implemented(), classify_http(), parse_json(), ServiceResult, parse_body(), parse_body() (+1 more)

### Community 118 - "briefing_view.rs"
Cohesion: 0.23
Nodes (17): bucket_extracted(), bucket_extracted_with_explanations(), buckets_split_facts_inferences_context_and_links(), claim(), claim_label(), explain_intel_links_with_model(), explain_relation_link(), ExtractedBuckets (+9 more)

### Community 119 - "subscription.rs"
Cohesion: 0.25
Nodes (8): answer_event(), check_login(), command(), complete(), completion_is_ephemeral_and_does_not_inherit_tools_or_configuration(), exec_args(), is_subscription_status(), login()

### Community 120 - "InsightStats"
Cohesion: 0.17
Nodes (16): insight_header(), insight_packet(), insight_row(), insight_row_line(), insight_stats_line(), insight_table_lines(), InsightRow, InsightStats (+8 more)

### Community 121 - "IndexOutcome"
Cohesion: 0.12
Nodes (7): IndexOutcome, Disabled, Pending, PermanentFailure, Ready, RetryableFailure, IndexWork

### Community 122 - "recon/graph.rs"
Cohesion: 0.07
Nodes (45): glyph(), basic_explanation(), build_claim_graph(), build_memory_graph(), call_for(), choose_directive(), claim_tokens(), directive_ids() (+37 more)

### Community 123 - "tasks.rs"
Cohesion: 0.07
Nodes (69): lease_fencing_rejects_stale_epoch_and_foreign_owner(), add_missing_columns(), adopt_untracked_index_changes(), block_claimed(), claim_complete_and_retry_round_trip(), claim_index_changes(), claim_index_work(), claim_next() (+61 more)

### Community 124 - "ReconCommand"
Cohesion: 0.17
Nodes (12): ReconCommand, Ask, AskNew, Delete, Limits, List, New, Rename (+4 more)

### Community 126 - ".new"
Cohesion: 0.23
Nodes (10): D, CompiledSite, DatasetSnapshot, deserialize_flexible_code(), deserialize_headers(), deserialize_optional_string_or_empty(), deserialize_string_or_list(), slugify() (+2 more)

### Community 127 - "modes.rs"
Cohesion: 0.27
Nodes (13): scoped_section_plan(), chat_response_spec(), chat_response_spec_embeds_section_guidelines_and_style_guide(), chat_response_spec_lists_mode_headings(), chat_section_titles(), every_mode_has_bluf_first_and_sources_last(), every_mode_section_has_guideline(), investigation_mode_spec() (+5 more)

### Community 128 - "ServiceSpec"
Cohesion: 0.20
Nodes (9): by_id(), CATALOG, CATALOG_LEN, UPSTREAM_COMMIT, ServiceSpec, ServiceState, Enabled, Experimental (+1 more)

### Community 129 - "TurnClock"
Cohesion: 0.10
Nodes (6): a_round_keeps_its_time_and_a_tight_ceiling_still_runs_tools(), deadline_seconds(), format_deadline(), format_span(), the_label_updates_when_a_round_adds_calls_and_when_evidence_is_measured(), TurnClock

### Community 130 - "agent"
Cohesion: 0.14
Nodes (13): agent, compaction, explore, mode, model, variant, default_agent, mode (+5 more)

### Community 131 - "Functional Requirements"
Cohesion: 0.12
Nodes (15): Atlas (News Pipeline), Brain / Memory, Configuration Requirements, Context Provider Keys, Functional Requirements, Non-Functional Requirements, Non-Goals, OSINT Manual Runs (+7 more)

### Community 132 - "JobStatusFilter"
Cohesion: 0.18
Nodes (8): JobFilter, JobStatusFilter, Active, All, Completed, Failed, Retrying, list_jobs()

### Community 133 - "spotify.rs"
Cohesion: 0.17
Nodes (10): ADAPTER_VERSION, HOST, ID, positive_negative_and_unknown(), request_url(), ADAPTER_VERSION, HOST, ID (+2 more)

### Community 134 - "ui.rs"
Cohesion: 0.05
Nodes (62): ACTION_H, ApiKeySlot, atlas_countdown(), atlas_history_live_label(), atlas_source_anchors_are_not_labeled_deleted_origin(), call_stamp(), center_line(), center_row() (+54 more)

### Community 135 - "TaskState"
Cohesion: 0.18
Nodes (9): TaskState, Cancelled, Completed, Failed, Paused, Queued, RetryScheduled, Running (+1 more)

### Community 136 - "OsintCommand"
Cohesion: 0.14
Nodes (14): DatasetCommand, Import, Refresh, Status, OsintCommand, Attach, Dataset, Describe (+6 more)

### Community 137 - "RouteInput"
Cohesion: 0.17
Nodes (11): RouteInput, FacebookUrl, Handle, HandleOrUserId, Hashtag, LinkedinCompanyUrl, LinkedinProfileUrl, Query (+3 more)

### Community 138 - "Severity"
Cohesion: 0.20
Nodes (6): EventFilter, Severity, Debug, Error, Info, Warn

### Community 139 - "Argos OSINT — Agent Instructions"
Cohesion: 0.13
Nodes (15): Architecture notes, Argos OSINT — Agent Instructions, CI (`.github/workflows/ci.yml`), CLI Entry Points (`argos` binary), Developer Commands, diagrams, Environment Variables, graphify (+7 more)

### Community 140 - "AtlasInsightClaim"
Cohesion: 0.30
Nodes (12): recover_legacy_packet(), article(), claim(), commit_replaces_atomically_and_keeps_shared_support(), failed_validation_leaves_prior_insights(), replace_article_insights_from_body(), ReplaceOutcome, snapshot_counts_are_stable_before_commit() (+4 more)

### Community 141 - "TaskStatus"
Cohesion: 0.07
Nodes (17): CallProposal, HandoffRecord, TaskRecord, TaskStatus, Cancelled, Completed, Deferred, Failed (+9 more)

### Community 142 - "wikipedia_rsp.rs"
Cohesion: 0.17
Nodes (14): API, APP_STATE_KEY, CACHE_TTL, index_to_json(), parse_last_year(), parse_rsp_wikitext(), parse_source_name(), parse_summary() (+6 more)

### Community 143 - "rusqlite"
Cohesion: 0.09
Nodes (19): Cached, None, Stale, Valid, Gate, CoolingDown, Ready, Running (+11 more)

### Community 144 - "serde_json"
Cohesion: 0.13
Nodes (9): DecisionState, EvidencePassageState, SubjectStateSummary, TaskStateSummary, ADAPTER_VERSION, HOST, ID, positive_negative_and_source_field() (+1 more)

### Community 145 - "JobRow"
Cohesion: 0.29
Nodes (3): job_row(), JobRow, parse()

### Community 146 - "poll_device"
Cohesion: 0.28
Nodes (9): DeviceGrant, Poll, Denied, poll_device(), Pending, SlowDown, Token, start_device() (+1 more)

### Community 147 - ".is_empty"
Cohesion: 0.17
Nodes (6): ModelAssignment, ModelRoute, resolve_model_choice(), role_fallbacks_add_delete_reorder_and_reject_duplicates(), role_name(), RoleDefaults

### Community 148 - "QueryExecutionStatus"
Cohesion: 0.18
Nodes (11): QueryExecutionStatus, Cancelled, Completed, Failed, Generated, NoResults, Queued, Running (+3 more)

### Community 149 - "results.rs"
Cohesion: 0.16
Nodes (16): classify_output_quality(), extract_observation_items(), extract_search_results(), extracts_from_array_shaped_observations(), extracts_from_object_with_results_array(), normalize_tool_result(), NormalizedToolResult, OutputQuality (+8 more)

### Community 150 - "Store"
Cohesion: 0.53
Nodes (5): charge_newsapi(), charge_quota(), open_key(), quota_open(), utc_day()

### Community 151 - "store_cache"
Cohesion: 0.27
Nodes (8): cache_key(), cache_stores_only_definitive(), CacheEntry, clear_caches(), host_cooldown_is_rate_limited(), host_cooldowns(), result_cache(), store_cache()

### Community 152 - "QuestionSpec"
Cohesion: 0.25
Nodes (5): DecisionQuestionType, Choice, Noul, Score, QuestionSpec

### Community 153 - "Rect"
Cohesion: 0.11
Nodes (52): abs_contains(), add_fallback_layout(), add_fallback_popup_area(), api_key_slot(), atlas_hit(), atlas_live_areas(), atlas_news_areas(), atlas_row_at() (+44 more)

### Community 154 - "§9 implementation order"
Cohesion: 0.15
Nodes (12): §10 acceptance checks, §9 implementation order, Atlas memory completion and System apps — implementation checklist, Phase 1 — what landed, Phase 2 — what landed, Phase 3 — what landed, Phase 4 — what landed, Phase 5a — what landed (+4 more)

### Community 155 - ".new"
Cohesion: 0.28
Nodes (5): aging_the_tool_clock_blocks_new_calls_and_cache_hits_do_not_extend_it(), clock_set_for_turn(), foreground_expiry_continues_in_background(), sequential_raise_calls_budgets_higher(), TurnCheckpoint

### Community 156 - "investigation/evidence.rs"
Cohesion: 0.15
Nodes (16): assess_claim_against_passages(), ClaimAssessmentOutcome, ClaimStance, Disputed, Insufficient, Mention, Supported, curate_passages_from_result() (+8 more)

### Community 157 - "InvestigationPattern"
Cohesion: 0.10
Nodes (17): InvestigationPattern, ArticleVerification, Bitcoin, BreakingNews, DomainIp, EmailAttribution, FollowUp, GeneralSubject (+9 more)

### Community 164 - "What Was Learned"
Cohesion: 0.15
Nodes (12): CLI Entry Points, Codebase Structure, Investigation Flow, Model Roles (configured independently in Providers → Defaults), Next Commands, Onboarding Summary — argos-osint, Planning Artifacts Created, Primary Providers (+4 more)

### Community 165 - "AnnPolicy"
Cohesion: 0.16
Nodes (9): AnnMeasurement, AnnPolicy, AnnDeferredUnmeasured, AnnEnabled, ExactSearch, AnnThresholds, decide_ann_policy(), measure_ann_recall() (+1 more)

### Community 166 - "SiteOutcomeStatus"
Cohesion: 0.18
Nodes (10): deserialize_flexible_bool(), SiteOutcomeStatus, Ambiguous, Blocked, Error, Found, NotFound, RateLimited (+2 more)

### Community 167 - "investigation.rs"
Cohesion: 0.05
Nodes (105): Account, account_hits(), ACCOUNT_PLATFORMS, account_platforms_attach_to_the_subject_and_are_never_entities(), ACCOUNTS, accounts_flow(), accounts_search_query(), action_order() (+97 more)

### Community 168 - "Milestone 1 — Core Onboarding (Current)"
Cohesion: 0.18
Nodes (10): Deliverables, Dependencies, Future Milestones (Planned), Milestone 1 — Core Onboarding (Current), Milestone 2 — CLI Verification, Milestone 3 — Test Coverage, Milestone 4 — Integration & Ship, Objectives (+2 more)

### Community 169 - "paths.rs"
Cohesion: 0.36
Nodes (8): auth_path(), config_path(), dataset_dir(), datasets_dir(), db_path(), ensure_home(), home_dir(), lancedb_dir()

### Community 170 - ".index_now"
Cohesion: 0.23
Nodes (3): ArticleInsightCommit, insight_fingerprint(), IndexEnqueue

### Community 171 - "atlas_work.rs"
Cohesion: 0.13
Nodes (16): articles_rev(), AttemptRecord, check_dependency_coverage(), DependencyCoverage, DISPOSITION_EMPTY, DISPOSITION_FAILED, DISPOSITION_INCOMPLETE, DISPOSITION_REJECTED (+8 more)

### Community 172 - "H2 Contracts: Frozen Interface Decisions"
Cohesion: 0.18
Nodes (10): 1. Home Draft Key, 2. Launch State Machine, 3. Persistence Boundary, 4. Tab Identity, 5. Navigation Intent, 6. State Restoration (Per-Tab), 7. Input Precedence Hierarchy, 8. Layout Measurements (Reference Points) (+2 more)

### Community 173 - "accept_one"
Cohesion: 0.25
Nodes (9): accept_one(), AcceptMode, Context, Lead, contains_span(), context_candidates(), countries_equal(), is_significant() (+1 more)

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
Cohesion: 0.15
Nodes (14): ReconLimits, coverage_gaps_summary(), CoverageCandidate, CoverageRecord, credentials_available(), EngineQueryState, load_coverage_records(), plan_category_diversity() (+6 more)

### Community 178 - "RecoveryAction"
Cohesion: 0.12
Nodes (12): classify_recovery(), RecoveryAction, AccessRestricted, CircuitCooldown, ContentRefused, CreditsExhausted, FallbackModel, ReduceContext (+4 more)

### Community 179 - "PlanCall"
Cohesion: 0.28
Nodes (9): action_call(), bound(), isolation_lines(), isolation_lines_show_what_ran_what_was_held_and_why(), search_call(), served(), step(), WaveOutcome (+1 more)

### Community 180 - "decide"
Cohesion: 0.22
Nodes (6): decide(), DecisionAnswer, decisions_client_posts_to_alpha_decisions_and_parses_answers(), DecisionsResponse, live_decisions_smoke(), parse_decisions()

### Community 181 - "Conventions"
Cohesion: 0.22
Nodes (9): Agent scratch, Checks, Conventions, Diagrams, Graphify, Planning files, Schema, Tests (+1 more)

### Community 182 - "Docs Ingest — argos-osint"
Cohesion: 0.33
Nodes (5): `docs/architecture.md`, Docs Ingest — argos-osint, `docs/providers.md`, Ingest Status, Ingested Documentation

### Community 183 - "ScheduledCall"
Cohesion: 0.38
Nodes (8): courtlistener_spacing_and_firecrawl_polling_are_counted(), live(), more_calls_mean_more_tool_time_and_cache_hits_add_nothing(), scheduled(), ScheduledCall, tool_allowance_for_deps(), tool_allowance_for_deps_sequential_sums(), tool_allowance_seconds()

### Community 184 - "RawClaim"
Cohesion: 0.28
Nodes (8): ask_claims(), AskedClaims, claim_json_tolerates_empty_and_alternate_shapes(), parse_claims(), parse_json_value(), raw_claim(), raw_from_kept(), RawClaim

### Community 185 - "OpenCode V2 workflow"
Cohesion: 0.40
Nodes (4): ECC commands, Graph first, OpenCode V2 workflow, Verify setup

### Community 186 - "Unified reliability / summarization / semantic pipelines — completion checklist"
Cohesion: 0.40
Nodes (4): Honest remaining gaps, Phase status (§18), This follow-up, Unified reliability / summarization / semantic pipelines — completion checklist

### Community 187 - "Concepts"
Cohesion: 0.25
Nodes (8): Apps and internal IDs, Bindings, Concepts, Graph, Intel report jobs, Model roles, Persistence, Primary OSINT providers

### Community 201 - "explore.rs"
Cohesion: 0.20
Nodes (11): GraphEdgeKind, ContradictionOrUpdate, Evidenced, Inferred, SemanticSuggestion, related_evidence_view(), RelatedEvidenceHit, semantic_edges_are_not_factual() (+3 more)

### Community 202 - "CheckSignal"
Cohesion: 0.22
Nodes (8): CheckSignal, Blocked, Error, Inconclusive, NotRegistered, RateLimited, Registered, Unsupported

### Community 203 - "LevelFilter"
Cohesion: 0.29
Nodes (5): LevelFilter, All, Error, Info, Warn

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
Cohesion: 0.25
Nodes (8): Atlas, CLI, Intel, Other apps, Recon, State, Terminal UI, Usage

### Community 209 - "registry"
Cohesion: 0.24
Nodes (10): cache_follows_the_provider_plan_interval(), definition(), job_poll_seconds(), key_schema(), ac1_catalog_has_55_tools_and_the_news_and_legal_entries(), optional_keys(), registry(), registry_and_validation() (+2 more)

### Community 210 - "F"
Cohesion: 0.43
Nodes (5): F, refresh(), refresh_from_upstream(), refresh_with_progress(), refresh_with_progress_and_client()

### Community 211 - "youtube_pair"
Cohesion: 0.33
Nodes (6): https_on_host(), profile_path_token(), facebook_url(), linkedin_url(), youtube_pair(), social_token()

### Community 212 - "CycleOutcome"
Cohesion: 0.18
Nodes (11): CycleOutcome, Blocked, Cancelled, Completed, CompletedWithWarnings, Failed, Partial, Pending (+3 more)

### Community 213 - "render_tui_cells.py"
Cohesion: 0.31
Nodes (3): box(), color(), render()

### Community 214 - "RecordKind"
Cohesion: 0.29
Nodes (6): RecordKind, Claim, DerivedSummary, Memory, Passage, ToolObservation

### Community 215 - "super"
Cohesion: 0.11
Nodes (7): intel_recon_publishes_durably_with_provenance_and_no_atlas_receipt(), ReconInsightUpdate, update(), upsert_adds_and_updates_claims(), upsert_recon_insights(), CODES, NAMES

### Community 216 - "Unified Investigation Harness"
Cohesion: 0.33
Nodes (6): Catalog projection, Model roles, Persistence, Reasoning channels, Unified Investigation Harness, Validation gates

### Community 217 - "ClaimRelation"
Cohesion: 0.29
Nodes (7): ClaimRelation, AmbiguousEntity, ChangedQuantity, Equivalent, Negation, PlannedVsCompleted, Unrelated

### Community 218 - "HypothesisRecord"
Cohesion: 0.38
Nodes (7): Alternative, classify_hypothesis(), contradiction(), draft_hypotheses(), hypothesis_absence_stays_unresolved(), hypothesis_status(), HypothesisRecord

### Community 219 - "TurnContinuation"
Cohesion: 0.40
Nodes (4): TurnContinuation, Active, Background, Exhausted

### Community 220 - "Argos OSINT"
Cohesion: 0.33
Nodes (6): Applications, Argos OSINT, Documentation, Figures, Limits, Quick start

### Community 221 - "SettingsFile"
Cohesion: 0.22
Nodes (3): SettingsFile, RoleRuntime, cost_map()

### Community 222 - "Delivery: Recon diversity, reliability, Whoxy, and Holehe"
Cohesion: 0.29
Nodes (6): Changed files (high level), Delivery: Recon diversity, reliability, Whoxy, and Holehe, Remaining live-validation gaps, Root cause and fix, Supported Holehe catalog, Tests

### Community 223 - "Argos documentation"
Cohesion: 0.40
Nodes (5): Agent and workflow, Argos documentation, Checklists (historical), Figures, Product

### Community 224 - "Diagram conventions"
Cohesion: 0.50
Nodes (4): Agent rule, Catalog, Diagram conventions, When to draw

### Community 225 - "extract_for_article_body"
Cohesion: 0.24
Nodes (12): a_description_only_span_is_inference_and_an_added_name_is_dropped(), a_fact_requires_both_spans_in_the_title(), accept_claims(), article_with_body_spans(), body_lead_prompt(), body_spans_accept_entity_and_object_from_full_article(), clip_chars(), context_claims_require_the_entity_in_the_title_and_link_context_for() (+4 more)

### Community 226 - "Argos UI Interaction Audit"
Cohesion: 0.40
Nodes (5): Argos UI Interaction Audit, Audit Matrix, Field inventory (`field_placeholder`), Remaining limits, Shared contracts

### Community 228 - "fetch_once"
Cohesion: 0.40
Nodes (4): fetch_once(), public_host(), read_limited(), Response

### Community 230 - "RspStatus"
Cohesion: 0.21
Nodes (8): clip_summary(), parse_status(), RspStatus, Blacklisted, Deprecated, GenerallyReliable, GenerallyUnreliable, NoConsensus

### Community 232 - "RspEntry"
Cohesion: 0.27
Nodes (7): normalize_host(), normalize_name(), observation_for(), observation_marks_unlisted(), parses_status_domains_and_maps_reliability(), RspEntry, SourceReliabilityObservation

### Community 235 - "RspIndex"
Cohesion: 0.40
Nodes (9): cache(), cached_index(), CachedIndex, ensure_index(), fetch_index(), index_from_json(), install_index(), RspIndex (+1 more)

### Community 238 - "SourceReliability"
Cohesion: 0.25
Nodes (4): SourceReliability, A, B, C

### Community 239 - "atlas_actions.rs"
Cohesion: 0.33
Nodes (3): record_start(), start_repair(), starts()

### Community 245 - ".job_source"
Cohesion: 0.50
Nodes (4): AtlasRun, JobSource, AtlasRun, ReconThread

## Knowledge Gaps
- **1600 isolated node(s):** `$schema`, `default_agent`, `mode`, `model`, `bash` (+1595 more)
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 2166 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **26 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `App` connect `App` to `.select`, `ui.rs`, `recon.rs`, `ToolResult`, `app.rs`, `.handle_key`, `Store`, `hardware.rs`, `Store`, `.field`, `Memory`, `ProviderSecret`, `brain_detail.rs`, `run_atlas_inner`, `ReportMode`, `worker.rs`, `Target`, `src/brain.rs`, `Call`, `ProviderPage`, `LogsView`, `FeedArticle`, `DefaultsRole`, `AtlasArticleRow`, `SettingsFile`, `summary_card.rs`, `ModuleId`, `WorkEvent`, `.job_source`, `briefing_view.rs`, `recon/graph.rs`, `JobsView`?**
  _High betweenness centrality (0.089) - this node is a cross-community bridge._
- **What connects `$schema`, `default_agent`, `mode` to the rest of the system?**
  _1600 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `land.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.0076045627376425855 - nodes in this community are weakly interconnected._
- **Why does `ProviderSecret` connect `ProviderSecret` to `orchestrate.rs`, `atlas_memory.rs`, `recon.rs`, `ToolResult`, `AtlasInsightClaim`, `poll_device`, `gates.rs`, `body.rs`, `brain_detail.rs`, `atlas.rs`, `grok_oauth.rs`, `provider_attempt.rs`, `picker.rs`, `ReportMode`, `provider_chain.rs`, `App`, `decide`, `exec.rs`, `model_exec.rs`, `RawClaim`, `ChatMessage`, `worker.rs`, `intel_recon/jobs.rs`, `atlas_insights.rs`, `body_filter.rs`, `synthesize.rs`, `ProviderPage`, `summarization.rs`, `FeedArticle`, `anyhow`, `AtlasArticleRow`, `SettingsFile`, `graph_explanation/tests.rs`, `extract_for_article_body`, `actor_review.rs`, `graph_explanation.rs`, `secrets.rs`, `Service`, `briefing_view.rs`, `subscription.rs`?**
  _High betweenness centrality (0.029) - this node is a cross-community bridge._
- **Should `orchestrate.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.038813938388283675 - nodes in this community are weakly interconnected._
- **Why does `Store` connect `Store` to `brain_detail.rs`, `brain_lance.rs`, `.index_now`, `store.rs`, `IndexOutcome`?**
  _High betweenness centrality (0.028) - this node is a cross-community bridge._
- **Should `atlas_memory.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.05624438454627134 - nodes in this community are weakly interconnected._