# Graph Report - argos-osint  (2026-10-07)

## Corpus Check
- 154 files · ~354,444 words
- Verdict: corpus is large enough that graph structure adds value.
- Unclassified: 5 file(s) not represented in the graph (top: (none) 3, .toml 2)

## Summary
- 5710 nodes · 14162 edges · 222 communities (200 shown, 22 thin omitted)
- Extraction: 98% EXTRACTED · 2% INFERRED · 0% AMBIGUOUS · INFERRED: 270 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `d61a4b1a`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- land.rs
- orchestrate.rs
- App
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
- ui.rs
- directives.rs
- .set_focus
- absorb_hit
- providers.rs
- atlas_news.rs
- Store
- publication.rs
- gates.rs
- Message
- news_legal.rs
- hardware.rs
- App
- Store
- embed.rs
- osint.rs
- execute_steps
- mem
- provider.rs
- run_turn
- InvestigationPart
- body.rs
- LogicalRole
- brain_detail.rs
- recon/graph.rs
- atlas.rs
- run_atlas_inner
- .on_work_event
- scheduler.rs
- jobs_view.rs
- provider_attempt.rs
- SettingsFile
- picker.rs
- ReportMode
- run_primary
- reliability_faults.rs
- tests.rs
- apply_peer_support
- events.rs
- exec.rs
- WorkEvent
- .is_empty
- ErrorCategory
- grok_oauth.rs
- rule_bindings
- Frame
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
- markdown.rs
- secrets.rs
- Gate
- theme.rs
- .run_configured
- pipeline.rs
- summarization.rs
- tui/graph.rs
- LogsView
- graph_explanation.rs
- tui/jobs.rs
- .new
- FeedArticle
- budget.rs
- store.rs
- Command
- DefaultsRole
- logs.rs
- src/evidence.rs
- ProviderSecret
- DecisionContract
- .handle_key
- F
- wikipedia_rsp.rs
- TurnClock
- How
- H3 Task Packet: State and Persistence Foundation
- summary_card.rs
- contains
- ModuleId
- Region
- ledger.rs
- validate.rs
- replace_insights.rs
- .default
- .order
- Service
- provider_diag.rs
- Architecture
- KeptClaim
- .evaluate
- ClockSet
- ProviderAdmission
- Phase Plan: Home Recon Composer and Investigation Tabs
- Value
- gsd-v2.js
- inset
- briefing_view.rs
- subscription.rs
- Overlay
- measure_ann_recall
- synthesize
- Connection
- ReconCommand
- JobRow
- RouteInput
- poll_device
- context_turn
- home_rows
- agent
- Functional Requirements
- Store
- InvestigationSurface
- accept_claims
- DecisionAdapterKind
- OsintCommand
- .new
- the_label_updates_when_a_round_adds_calls_and_when_evidence_is_measured
- Argos OSINT — Agent Instructions
- model_roles.rs
- Store
- rusqlite
- tasks.rs
- DecisionState
- ToolResult
- RecordKind
- IndexOutcome
- ClaimRelation
- results.rs
- Store
- .memory
- Tone
- Rect
- §9 implementation order
- TurnContinuation
- investigation/evidence.rs
- InvestigationPattern
- Part
- argos-osint-bin
- What Was Learned
- anyhow
- BrainResourceSummary
- investigation.rs
- Milestone 1 — Core Onboarding (Current)
- paths.rs
- HypothesisRecord
- TaskState
- H2 Contracts: Frozen Interface Decisions
- youtube_pair
- Codebase Map — argos-osint
- PROJECT.md — argos-osint
- Current Phase State
- ReconLimits
- RecoveryAction
- InvestigationEvent
- Item
- ScheduledCall
- Docs Ingest — argos-osint
- decide
- draw
- OpenCode V2 workflow
- Unified reliability / summarization / semantic pipelines — completion checklist
- TaskStatus
- package.json
- super
- unix_now
- article
- modes.rs
- Argos Adaptive Decision Roles and Strict Output Contracts
- H0-H2 Completion Summary
- run
- Argos OSINT — Unified Investigation Harness
- PlanCall
- render_tui_cells.py
- AtlasArticleRow
- classify_relevant_articles
- intel_recon/brain.rs
- TimeoutProfile
- AnnPolicy
- .begin_title
- select_strategy
- subject_of
- AtlasPage
- IntelReconFocus
- resolve_adapter

## God Nodes (most connected - your core abstractions)
1. `App` - 280 edges
2. `ProviderSecret` - 122 edges
3. `ButtonId` - 104 edges
4. `FieldId` - 63 edges
5. `Store` - 63 edges
6. `ToolResult` - 62 edges
7. `Store` - 61 edges
8. `Target` - 58 edges
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

## Communities (222 total, 22 thin omitted)

### Community 0 - "land.rs"
Cohesion: 0.01
Nodes (262): CODES, LABELS, R0, R1, R10, R100, R101, R102 (+254 more)

### Community 1 - "orchestrate.rs"
Cohesion: 0.05
Nodes (48): a_follow_up_keeps_names_from_the_previous_synthesis(), a_handle_named_in_a_derived_question_is_an_unverified_binding(), a_recon_model_429_trips_the_gate_and_later_calls_skip_the_provider(), ac3_no_key_reaches_inputs_cache_keys_plan_json_raw_bodies_or_logs(), ac6_a_courtlistener_429_still_lets_the_turn_synthesize(), ac8_status_errors_and_401_403_fail_readably_and_are_not_cached(), ACCOUNT_TOOLS, ACME (+40 more)

### Community 2 - "App"
Cohesion: 0.06
Nodes (4): App, intel_day_button_label(), IntelBody, IntelReport

### Community 3 - "atlas_memory.rs"
Cohesion: 0.06
Nodes (88): Extraction, atlas_memory_count(), background_indexing_then_refresh_upgrades_a_partial_run(), Barrier, Checkpoint, CheckpointInput, claims(), clear() (+80 more)

### Community 4 - "tool_io.rs"
Cohesion: 0.04
Nodes (90): Binding, canonical(), dependency_order(), depends_on(), question_bindings_and_dependency_fix(), accept_bindings(), allowed_producer(), best_handle() (+82 more)

### Community 5 - "app.rs"
Cohesion: 0.06
Nodes (71): a_click_folds_an_event_and_incoming_entries_do_not_move_a_paused_list(), a_finished_tool_call_is_logged_once_and_the_transcript_keeps_a_summary(), all_provider_actions_render_with_hit_areas_at_80x24(), ATLAS_AUTO_SECS, atlas_auto_toggle_persists_the_next_trigger(), atlas_claim(), atlas_headlines_scroll_and_request_failures_reach_the_system_log(), atlas_history_resume_is_offered_only_for_resumable_cycles_and_repair_starts_once() (+63 more)

### Community 6 - "brain_lance.rs"
Cohesion: 0.06
Nodes (38): activate_generation(), batch(), begin_generation(), begin_generation_then_activate(), block_on(), BrainIndex, clear_fingerprint(), current_fingerprint() (+30 more)

### Community 7 - "FieldId"
Cohesion: 0.04
Nodes (48): FieldId, BrainApp, BrainConversation, BrainInsight, BrainQuery, ClaimAssessorModel, ClaimAssessorProvider, ClassifierModel (+40 more)

### Community 8 - "map.rs"
Cohesion: 0.11
Nodes (33): BRAILLE_MAP, canonical(), claim_label(), contains(), country_at(), country_fit(), country_pixel(), draw_country_mini_map() (+25 more)

### Community 9 - "recon.rs"
Cohesion: 0.05
Nodes (67): ac6_citation_groups_split_validate_each_id_and_normalize(), answer_step_rate_limit_is_explicit_and_keeps_results(), attribute_word(), auxiliary(), BRIEF_SYNTHESIS, broad_question_collapses_to_grounded_lookups(), char_ceil(), char_floor() (+59 more)

### Community 10 - "ButtonId"
Cohesion: 0.02
Nodes (83): ButtonId, Add, AtlasAuto, AtlasDelete, AtlasLive, AtlasNews, AtlasNewsFeed, AtlasRepair (+75 more)

### Community 11 - "Store"
Cohesion: 0.07
Nodes (15): answer_evidence_survives_reopen_and_deleted_thread_rejects_late_calls(), claims_deduplicate_and_reject_unsupported_sources(), CreditHold, deleting_an_investigation_removes_its_brain_memories_and_graph_summary(), investigation_memory_ids(), now(), settle_holds(), persistence_and_plan() (+7 more)

### Community 12 - "IntelligenceCategory"
Cohesion: 0.09
Nodes (22): all_59_tools_mapped_to_categories(), argument_contract(), ArgumentBuilderContract, compact_capability_catalog(), CompactToolCapability, IntelligenceCategory, Bitcoin, DomainNetwork (+14 more)

### Community 13 - "job_registry.rs"
Cohesion: 0.05
Nodes (45): begin(), begin_cancellable(), db(), db_path(), finish(), beat(), BEAT_INTERVAL, beats() (+37 more)

### Community 14 - "ui.rs"
Cohesion: 0.05
Nodes (79): ACTION_H, atlas_countdown(), atlas_history_live_label(), atlas_source_anchors_are_not_labeled_deleted_origin(), build_blocks(), call_stamp(), chat_blocks(), chat_max() (+71 more)

### Community 15 - "directives.rs"
Cohesion: 0.06
Nodes (65): paint_logo(), apply_context_targets(), asks(), asks_about_judge(), asks_for_headlines(), clip_words(), context_entity(), CONTEXT_FILLER (+57 more)

### Community 16 - ".set_focus"
Cohesion: 0.07
Nodes (17): add_scroll(), backspace_after_a_sent_question_deletes_one_character(), draft_isolation_and_persistence(), intel_opens_bulletin_filters_and_opens_briefing(), Section, keyboard_navigation_esc_and_shortcuts(), multiline_paste_stays_in_composer_and_does_not_run_shortcuts(), recon_chat_folds_decisions_and_opens_synthesis_memory() (+9 more)

### Community 17 - "absorb_hit"
Cohesion: 0.12
Nodes (22): absorb_hit(), account_platform_host(), Candidate, content_tokens(), distinct_queries(), domain_label(), EntityIdentifier, first_domain() (+14 more)

### Community 18 - "providers.rs"
Cohesion: 0.05
Nodes (67): clip_page(), domain(), email_address(), number_arg(), assert_host_locked(), BATCH_SCRAPE_DEFAULT_URLS, BATCH_SCRAPE_MAX_URLS, batch_urls() (+59 more)

### Community 19 - "atlas_news.rs"
Cohesion: 0.08
Nodes (44): api_error(), ATLAS_TOOLS, authors(), clip(), country_arg(), country_code(), CURRENTS_COUNTRIES, CURRENTS_HOST (+36 more)

### Community 20 - "Store"
Cohesion: 0.07
Nodes (19): article_body_from_row(), ArticleBodyRow, element_from_row(), IntelAssessmentRow, IntelElementRow, IntelEvidenceRow, IntelInvestigationRow, IntelReportJobRow (+11 more)

### Community 21 - "publication.rs"
Cohesion: 0.09
Nodes (39): bump_memories_changed(), change_watcher_sees_only_committed_changes_and_coalesces(), claim(), clear(), clear_index_state(), CoverageReport, delete_run_memory_state(), DELETED_REVISION (+31 more)

### Community 22 - "gates.rs"
Cohesion: 0.08
Nodes (13): execute_harness_step(), InvestigationRuntime, GateOutcome, Passed, Rejected, validate_evidence_admission(), validate_publication(), validate_task_admission() (+5 more)

### Community 23 - "Message"
Cohesion: 0.32
Nodes (4): chat(), Message, persist_claims(), snapshot_secret()

### Community 24 - "news_legal.rs"
Cohesion: 0.07
Nodes (45): a_rate_limit_switches_to_the_fallback_key_and_a_rejection_does_not(), ac1_each_request_has_its_url_params_and_auth_header_and_no_key_in_the_url(), an_echoed_key_is_redacted_from_the_stored_body_and_error(), article(), articles(), CATEGORIES, choice_arg(), context_kind() (+37 more)

### Community 25 - "hardware.rs"
Cohesion: 0.15
Nodes (18): CACHE_TTL_SECS, classify_arch(), disk_totals(), HardwareProfile, metal_vram_gb(), now_secs(), parse_apple_gpu_cores(), partial_gpu() (+10 more)

### Community 26 - "App"
Cohesion: 0.12
Nodes (42): atlas_cycle_stats_room(), atlas_cycle_stats_scroll_max(), atlas_feed_room(), atlas_feed_room_for(), atlas_hit(), atlas_live_areas(), atlas_news_areas(), atlas_news_room() (+34 more)

### Community 27 - "Store"
Cohesion: 0.07
Nodes (9): atlas_answer_id(), atlas_brief_id(), AtlasRunRow, AtlasStoredClaim, has_table(), ReindexReport, repair_embed_tables(), Store (+1 more)

### Community 28 - "embed.rs"
Cohesion: 0.07
Nodes (33): record_start(), start_repair(), active(), DIM, disable(), disabled(), DisableGuard, download_file() (+25 more)

### Community 29 - "osint.rs"
Cohesion: 0.06
Nodes (50): bounded(), CACHE_DAY_SECONDS, cache_follows_the_provider_plan_interval(), CACHE_MONTH_SECONDS, cache_seconds(), CACHE_WEEK_SECONDS, canonical_tool_id(), credential_key() (+42 more)

### Community 30 - "execute_steps"
Cohesion: 0.13
Nodes (32): action(), after_step(), binding_extraction_drops_values_missing_from_the_observation(), binding_ground(), context_block(), context_dispatched(), directive_for(), execute_steps() (+24 more)

### Community 31 - "mem"
Cohesion: 0.17
Nodes (22): cache_invalidates_when_source_revision_changes(), lease_fencing_rejects_stale_epoch_and_foreign_owner(), claim_complete_and_retry_round_trip(), claim_index_changes(), claim_index_work(), claim_next(), claim_next_in(), complete_index_change() (+14 more)

### Community 32 - "provider.rs"
Cohesion: 0.06
Nodes (32): complete_errors_with_finish_reason_when_response_is_empty(), complete_reads_reasoning_only_non_stream_json(), complete_stream_of_reasoning_deltas_yields_text(), DECISIONS_MODELS, decisions_url(), default_credit_reset(), default_grok_model(), default_hunter_credits() (+24 more)

### Community 33 - "run_turn"
Cohesion: 0.10
Nodes (34): a_budgeted_call_with_a_dependency_from_an_earlier_batch_runs(), ac7_a_pronoun_follow_up_takes_the_thread_subject(), BudgetedOutcome, call_cached(), cancel_mid_dispatch_releases_credit_holds(), classify_turn_mode(), continue_turn(), enabled_tools() (+26 more)

### Community 34 - "InvestigationPart"
Cohesion: 0.12
Nodes (15): InvestigationPart, DirectiveAssessment, EvidencePassage, GateValidation, Handoff, Plan, PlanDiagnostics, RoleDecision (+7 more)

### Community 35 - "body.rs"
Cohesion: 0.09
Nodes (35): body_fetch_routes(), BodyFetchEvent, Attempt, Failed, InsightsFailed, InsightsRefreshing, InsightsReplaced, Progress (+27 more)

### Community 36 - "LogicalRole"
Cohesion: 0.13
Nodes (11): LogicalRole, .ALL, ClaimAssessor, Classifier, Controller, EntityResolver, EvidenceCurator, Planner (+3 more)

### Community 37 - "brain_detail.rs"
Cohesion: 0.06
Nodes (45): areas(), BrainDetail, DetailAreas, DetailPane, Path, Related, Summary, DetailSnapshot (+37 more)

### Community 38 - "recon/graph.rs"
Cohesion: 0.08
Nodes (45): glyph(), basic_explanation(), build_claim_graph(), build_memory_graph(), call_for(), choose_directive(), claim_tokens(), directive_ids() (+37 more)

### Community 39 - "atlas.rs"
Cohesion: 0.08
Nodes (32): article_card_labels_title_publisher_author_and_classification(), Band, band_sizes(), band_temperature(), category_from_decisions(), CATEGORY_IDS, CLUSTERS, country_label() (+24 more)

### Community 40 - "run_atlas_inner"
Cohesion: 0.09
Nodes (31): AtlasEvent, Classified, InsightProgress, MemoriesChanged, MemoryProgress, Note, Replaced, Stats (+23 more)

### Community 41 - ".on_work_event"
Cohesion: 0.09
Nodes (8): a_failed_turn_keeps_the_streamed_answer_on_screen(), a_log_or_late_tool_row_does_not_drop_the_live_answer(), atlas_log_level(), flush_streams(), pump(), subscription_progress_and_result_update_the_correct_page(), summary_system(), synthesis_deltas_fill_the_live_bubble_and_the_saved_answer_replaces_them()

### Community 42 - "scheduler.rs"
Cohesion: 0.09
Nodes (22): apply_index_change(), apply_index_change_rebuild_reports_outcome(), apply_index_with(), DEFAULT_LEASE_SECS, drain_index_once(), drain_summary_flush(), drain_summary_flush_cached(), drain_summary_flush_completes_cached_tasks() (+14 more)

### Community 43 - "jobs_view.rs"
Cohesion: 0.11
Nodes (21): summary_text(), active_time_is_live_for_open_attempts_and_unavailable_only_without_a_start(), AttemptRow, event_apps(), event_counts(), EventCounts, get_job(), job() (+13 more)

### Community 44 - "provider_attempt.rs"
Cohesion: 0.11
Nodes (36): Acc, attempt(), AttemptReport, check_final(), complete_stream_and_json_answers_succeed(), Deadlines, delta(), fill() (+28 more)

### Community 45 - "SettingsFile"
Cohesion: 0.11
Nodes (14): a_blank_osint_user_agent_loads_as_unset(), default_firecrawl_credits(), empty_config_seeds_tool_picker_and_keeps_synthesis(), fallback_keys_come_from_settings_then_env(), missing_max_turn_seconds_loads_as_900_and_the_range_is_120_to_1800(), ModelAssignment, news_and_legal_keys_come_from_settings_then_env(), old_firecrawl_credit_default_migrates_to_1000() (+6 more)

### Community 46 - "picker.rs"
Cohesion: 0.11
Nodes (27): brain_scrape_options_hide_bare_scrape_and_include_claim_criteria(), CatalogEntry, chat_request(), CONFIDENCE_FLOOR, decisions_request(), directives_phrase(), DONE, eligible_catalog() (+19 more)

### Community 47 - "ReportMode"
Cohesion: 0.12
Nodes (22): HomeDraftState, classifiable_modes(), classify_mode_chat(), classify_mode_decisions(), classify_prompt_mode(), classify_prompt_mode_chat(), classify_prompt_mode_decisions(), classify_recon_mode() (+14 more)

### Community 48 - "run_primary"
Cohesion: 0.17
Nodes (16): a_claimed_email_removes_its_bindings(), a_gap_filler_domain_feeds_hunter_once_firecrawl_observes_it(), a_weak_firecrawl_search_adds_one_google_search_with_the_same_query(), a_zero_email_count_skips_the_paid_domain_search(), ac3_seo_results_that_never_mention_the_subject_add_no_domain_org_email_or_url(), ac4_google_search_always_sends_the_replaced_firecrawl_query(), ac5_every_executed_input_is_grounded_and_an_ungrounded_step_is_skipped(), ac8_the_subject_fills_name_and_query_inputs_before_found_values() (+8 more)

### Community 49 - "reliability_faults.rs"
Cohesion: 0.11
Nodes (8): AttemptOutcome, categorize_http(), execute_with_retries(), other_llm_allows_three(), summarization_stops_at_two_attempts(), mock_provider_other_llm_allows_three_attempts(), mock_provider_recovers_after_transient_error(), mock_provider_summarization_retries_then_gives_up()

### Community 50 - "tests.rs"
Cohesion: 0.20
Nodes (19): auth_failure_sends_one_request_gives_guidance_and_redacts(), conn(), count(), failure_is_durable_correlated_bounded_and_harmless(), fixture(), Fx, GOOD, late_completion_for_a_changed_or_deleted_memory_is_not_published() (+11 more)

### Community 51 - "apply_peer_support"
Cohesion: 0.21
Nodes (12): accept_one(), AcceptMode, Context, Lead, apply_peer_support(), classifier_peers_are_preferred_over_token_overlap(), contains_span(), peer_articles_raise_a_claim_to_fact_and_boost_confidence() (+4 more)

### Community 52 - "events.rs"
Cohesion: 0.09
Nodes (27): starts(), infer_app(), session_event(), clear_events(), DEFAULT_RETENTION_HOURS, EventFilter, get_event(), job_filter_includes_descendants_and_prune_keeps_jobs() (+19 more)

### Community 53 - "exec.rs"
Cohesion: 0.15
Nodes (20): ChatMessage, admission_account(), admission_contention_consumes_no_attempt(), AttemptEvent, Finished, Started, AttemptLog, cancelled() (+12 more)

### Community 54 - "WorkEvent"
Cohesion: 0.09
Nodes (17): duplicate_submission_prevention(), single_action_launch_from_home(), work_event(), WorkEvent, Access, AnswerDelta, AnswerNote, AtlasDone (+9 more)

### Community 55 - ".is_empty"
Cohesion: 0.15
Nodes (17): concrete_free_models(), delta_text(), is_concrete_free(), is_free_router(), json_number(), message_json(), message_reasoning(), message_text() (+9 more)

### Community 56 - "ErrorCategory"
Cohesion: 0.08
Nodes (24): operation_kind(), backoff_delay(), can_retry(), ErrorCategory, AuthOrQuota, CancelledOrStale, ConfigurationMissing, ContextLimit (+16 more)

### Community 57 - "grok_oauth.rs"
Cohesion: 0.14
Nodes (20): auth_path(), bearer(), bearer_from_path(), check_login(), Entry, entry_from(), failed_login_keeps_the_cli_reason_and_recovery_action(), login() (+12 more)

### Community 58 - "rule_bindings"
Cohesion: 0.10
Nodes (33): binder_maps_kinds_to_inputs_and_never_hands_hunter_a_social_host(), binding(), normalize_platform(), question_bindings(), a_gap_filler_only_domain_waits_for_a_primary_observation_before_hunter(), batch_scrape_takes_ranked_urls_and_gates_order_before_hunter(), bind_arguments(), bitcoins_in() (+25 more)

### Community 59 - "Frame"
Cohesion: 0.10
Nodes (42): intel_category_short(), abs_contains(), AbsRect, atlas_auto_label(), atlas_extracting(), atlas_run_label(), center_line(), center_text() (+34 more)

### Community 60 - "brain_resources.rs"
Cohesion: 0.10
Nodes (22): source_anchor_label(), BRAIN_SCRAPE_PREFIX, CLAIM_CHARS, classify_resource(), file_link_url(), hit(), is_brain_binding(), is_brain_scrape_pick() (+14 more)

### Community 61 - "worker.rs"
Cohesion: 0.15
Nodes (19): IntelReportEvent, InsightsUpdated, JobCreated, JobDone, Section, Stage, cancel_if_needed(), claim_relevant_evidence() (+11 more)

### Community 62 - "Target"
Cohesion: 0.06
Nodes (31): brain_article_source_opens_intel_brief(), hit(), ProviderEvent, Finished, Progress, ProviderPage, .ALL, Defaults (+23 more)

### Community 63 - "src/brain.rs"
Cohesion: 0.12
Nodes (21): AGREEMENT_WEIGHT, CATEGORIES, category_hint(), format_injection(), hybrid_recall(), hybrid_recall_adds_paraphrases_and_rewards_agreement(), identity_query_prefers_name_memory(), jaccard() (+13 more)

### Community 64 - "intel_recon/jobs.rs"
Cohesion: 0.15
Nodes (18): active_job_for_mode(), cancel_job(), create_job_inserts_all_verify_sections(), create_report_job(), finish_canonical(), intel_recon_links_its_legacy_job_to_one_canonical_registry_job(), pause_job(), register_canonical() (+10 more)

### Community 65 - "cli.rs"
Cohesion: 0.17
Nodes (18): ask(), ask_thread(), atlas_command(), defaults_command(), defaults_json(), defaults_show_includes_the_tool_picker_transport(), dispatch(), login() (+10 more)

### Community 66 - "atlas_insights.rs"
Cohesion: 0.11
Nodes (26): BODY_CLAIM_LIMIT, BODY_SPAN_CHARS, COL_CLASS, COL_ENTITY, COL_OBJECT, COL_PREDICATE, COL_TOPIC, countries_differ() (+18 more)

### Community 67 - "InformationCredibility"
Cohesion: 0.13
Nodes (18): article_information_credibility(), AdmiraltyCode, best_credibility(), CredibilityInputs, information_credibility(), InformationCredibility, CannotBeJudged, Confirmed (+10 more)

### Community 68 - "Category"
Cohesion: 0.08
Nodes (21): Category, Auth, Cancelled, Configuration, Empty, InvalidModel, InvalidResult, MalformedPayload (+13 more)

### Community 69 - "body_filter.rs"
Cohesion: 0.17
Nodes (22): BATCH_SIZE, BodyChunk, chunk_article_body(), CHUNK_MAX, CHUNK_MIN, CHUNK_TARGET, chunks_preserve_absolute_offsets(), classify_chunk_batch() (+14 more)

### Community 70 - "synthesize.rs"
Cohesion: 0.17
Nodes (19): BODY_REFINE_INPUT_CHARS, deterministic_bluf_mentions_title(), deterministic_section(), model_refine_body(), model_synthesize(), parse_json_object(), refine_retrieved_article_body(), refine_without_secret_keeps_validated_scrape() (+11 more)

### Community 71 - "markdown.rs"
Cohesion: 0.18
Nodes (28): clip_cell(), fenced_code_is_marked_and_plus_lists_use_a_bullet(), flat(), hard_rows(), heading_marks(), inline(), is_rule(), is_table_separator_cells() (+20 more)

### Community 72 - "secrets.rs"
Cohesion: 0.15
Nodes (10): account_secret(), role_secret(), summarization_inherits_synthesis_when_unset(), writer_secret(), accounts_persist_with_owner_only_permissions(), AuthFile, legacy_connection_migrates_without_overwriting_other_accounts(), loading_old_auth_removes_gmail_setup_credentials() (+2 more)

### Community 73 - "Gate"
Cohesion: 0.13
Nodes (13): Cached, None, Stale, Valid, Gate, CoolingDown, Ready, Running (+5 more)

### Community 74 - "theme.rs"
Cohesion: 0.19
Nodes (19): ACCENT, BG, BORDER, card_accent(), card_dim(), card_text(), CODE_BG, DIM (+11 more)

### Community 75 - ".run_configured"
Cohesion: 0.13
Nodes (23): a_claimed_email_keeps_no_person_data(), annotate(), bind_request(), bitcoin(), claimed_email(), custom_user_agent(), effective_user_agent(), error_summary() (+15 more)

### Community 76 - "pipeline.rs"
Cohesion: 0.18
Nodes (19): apply_recon_directive_coverage(), compare_claims(), coverage_requires_cited_evidence_not_similarity(), directive_coverage(), DirectiveCoverage, event_grouping_keeps_separate_days_apart(), EventGroup, group_atlas_events() (+11 more)

### Community 77 - "summarization.rs"
Cohesion: 0.09
Nodes (38): cache_get(), cache_key_changes_with_revision_and_focus(), cache_put(), cache_round_trip(), complete_summary(), CoverageMeta, deterministic_article_description(), deterministic_atlas_brief() (+30 more)

### Community 78 - "tui/graph.rs"
Cohesion: 0.27
Nodes (15): draw(), draw_path(), draw_summary(), inset(), legend_height(), legend_parts(), legend_rows(), path_content() (+7 more)

### Community 79 - "LogsView"
Cohesion: 0.10
Nodes (10): LevelFilter, All, Error, Info, Warn, LogsView, row_lines(), detail_rows() (+2 more)

### Community 80 - "graph_explanation.rs"
Cohesion: 0.12
Nodes (14): BASIC_HEADING, event(), explain(), ExplainOutcome, Failed, Saved, Superseded, ExplainReport (+6 more)

### Community 81 - "tui/jobs.rs"
Cohesion: 0.09
Nodes (23): areas(), BASE_COLUMNS, button_label(), buttons(), detail_lines(), hit(), JobsAreas, JobsView (+15 more)

### Community 82 - ".new"
Cohesion: 0.25
Nodes (19): a_spent_primary_quota_uses_the_fallback_key(), classification_request(), country_labels_show_the_name_and_the_code(), daily_cap(), format_run_card(), HttpCall, HttpReply, insights_do_not_call_a_decisions_model() (+11 more)

### Community 83 - "FeedArticle"
Cohesion: 0.10
Nodes (27): apply_hits(), article_from(), article_from_row(), article_row(), Article, canonical_url(), category_tag(), dedup_drops_a_url_an_exact_title_and_keeps_the_higher_domain() (+19 more)

### Community 84 - "budget.rs"
Cohesion: 0.11
Nodes (15): CUT_NOTE, CUT_SHORT, deadline_seconds(), PHASE_WINDOW_SECONDS, RECON_DEADLINE, RECON_ROUND_SECONDS, STREAM_LOST, synthesis_allowance_capped() (+7 more)

### Community 85 - "store.rs"
Cohesion: 0.11
Nodes (21): atlas_article_from_row(), AUTO_REBUILD_HINT, boost_with_passage_hybrid(), deleting_a_memory_removes_its_graph_summary(), embedding_disabled_degrades_to_jaccard(), embedding_failure_mid_session_degrades_to_jaccard(), every_memory_has_provenance_and_category(), fingerprint_mismatch_rebuilds_and_drift_is_reconciled() (+13 more)

### Community 86 - "Command"
Cohesion: 0.09
Nodes (23): AtlasCommand, Repair, Resume, Verify, Cli, Command, Atlas, Defaults (+15 more)

### Community 87 - "DefaultsRole"
Cohesion: 0.09
Nodes (17): ChoiceItem, codex_models(), DefaultsRole, .ALL, ClaimAssessor, Classifier, EntityResolver, EvidenceCurator (+9 more)

### Community 88 - "logs.rs"
Cohesion: 0.19
Nodes (15): stamp(), areas(), button_label(), buttons(), count(), draw(), hit(), in_list() (+7 more)

### Community 89 - "src/evidence.rs"
Cohesion: 0.20
Nodes (14): AGREEMENT_WEIGHT, chunk_text(), chunks_cover_end_of_long_source(), content_hash(), ensure_identifier_coverage(), EvidencePassage, hybrid_bounds_and_prefers_agreement(), hybrid_passage_candidates() (+6 more)

### Community 90 - "ProviderSecret"
Cohesion: 0.18
Nodes (24): active_text_secret(), authorize(), bearer_token(), catalog_error(), chat_body(), complete(), complete_once(), effective_kind() (+16 more)

### Community 91 - "DecisionContract"
Cohesion: 0.18
Nodes (18): compile_general_model_prompt(), compile_native(), DecisionContract, template_claim_relation(), template_directive_alignment(), template_entity_binding(), template_evidence_relevance(), template_extraction_fidelity() (+10 more)

### Community 92 - ".handle_key"
Cohesion: 0.10
Nodes (7): ctrl_arrows_cycle_app_tabs_forward_and_back(), ctrl_tab_cycles_app_tabs_forward_and_back(), home_order_renames_and_nine_routes_agree(), is_picker_field(), PaletteItem, provider_label(), unavailable_palette_action_stays_visible_and_does_not_execute()

### Community 93 - "F"
Cohesion: 0.18
Nodes (16): Fault, CallSpec, dispatch(), fetch_with_spare_key(), json_message(), one_line(), parse_fault(), phase1_call() (+8 more)

### Community 94 - "wikipedia_rsp.rs"
Cohesion: 0.07
Nodes (43): SourceReliability, A, B, C, D, API, APP_STATE_KEY, cache() (+35 more)

### Community 96 - "How"
Cohesion: 0.11
Nodes (18): How, CompanyEmail, Coordinates, DomainAsUrl, EntityPhrase, PackageName, PackageParts, Plain (+10 more)

### Community 97 - "H3 Task Packet: State and Persistence Foundation"
Cohesion: 0.07
Nodes (26): Acceptance Checks and Exact Existing Test Targets:, Current Diff / Prior Changes to Preserve:, Effective Agent / Model / Variant:, Exact Existing Test Targets to Reuse/Extend:, Existing APIs / Data Models to Reuse:, Expected Changed Files:, Expected Commands:, Files This Worker Owns: (+18 more)

### Community 98 - "summary_card.rs"
Cohesion: 0.32
Nodes (11): actions(), button_at(), button_cells(), button_row_area(), draw(), height(), is_card_button(), label() (+3 more)

### Community 99 - "contains"
Cohesion: 0.14
Nodes (22): abs_rect(), atlas_row_at(), contains(), intel_body_loading(), intel_body_progress_lines(), intel_brief_full_lines(), intel_brief_preview_lines(), intel_brief_reports_lines() (+14 more)

### Community 100 - "ModuleId"
Cohesion: 0.14
Nodes (17): ModuleId, .ALL, Atlas, Brain, Intel, Jobs, Logs, Osint (+9 more)

### Community 101 - "Region"
Cohesion: 0.12
Nodes (17): Region, AtlasInsights, AtlasNews, AtlasOrigins, AtlasRuns, Chat, Detail, IntelBrief (+9 more)

### Community 102 - "ledger.rs"
Cohesion: 0.15
Nodes (12): body_assertion_candidates(), coverage_complete(), ElementStatus, Assessed, Excluded, InProgress, Pending, Superseded (+4 more)

### Community 103 - "validate.rs"
Cohesion: 0.19
Nodes (15): BLOCK_MARKERS, BodyQuality, Complete, Partial, Unavailable, Uncertain, BodyValidation, clean_markdown() (+7 more)

### Community 104 - "replace_insights.rs"
Cohesion: 0.24
Nodes (14): claim(), article(), claim(), commit_replaces_atomically_and_keeps_shared_support(), failed_validation_leaves_prior_insights(), replace_article_insights_from_body(), ReplaceOutcome, snapshot_counts_are_stable_before_commit() (+6 more)

### Community 105 - ".default"
Cohesion: 0.22
Nodes (30): a_429_stops_picker_calls_and_the_fallback_finishes_the_list(), a_dispatch_error_fails_the_step_and_the_loop_goes_on(), a_failed_email_finder_gets_one_fallback_and_no_third_picker_call(), a_fallback_with_no_bindings_still_runs_the_satisfied_steps_and_reaches_synthesis(), a_goal_naming_a_tool_or_provider_is_repaired_then_falls_back(), a_goal_naming_a_tool_outside_the_prompt_entity_is_still_rejected(), a_prompt_entity_named_like_a_provider_keeps_the_models_directives(), a_question_handle_fills_the_handle_steps_without_a_fallback() (+22 more)

### Community 106 - ".order"
Cohesion: 0.22
Nodes (10): checked_serves(), context_additions(), context_tools(), fallback_record(), OrderContext, Ordered, Picker<'a>, serves_for() (+2 more)

### Community 107 - "Service"
Cohesion: 0.17
Nodes (16): AnswerContext, cut_footer(), cut_short_answer(), finish_recon_job(), credit_map(), deltas(), note_cache(), RecallInsight (+8 more)

### Community 108 - "provider_diag.rs"
Cohesion: 0.09
Nodes (25): bounded(), cause_chain(), classify_status(), classify_text(), endpoint_strips_query_and_userinfo(), find_url(), http_failure(), Leaf (+17 more)

### Community 109 - "Architecture"
Cohesion: 0.07
Nodes (24): Admiralty source evaluation (WP:RSP), Architecture, Atlas, Investigation flow, Model and UI boundaries, News and Legal context tools (#29), Persistence, Primary providers (+16 more)

### Community 110 - "KeptClaim"
Cohesion: 0.29
Nodes (14): apply_admiralty_evaluation(), body_lead_prompt(), brief_text(), cap_claims(), dedupe_claims(), entity_path(), extract_for_article_body(), fingerprint() (+6 more)

### Community 111 - ".evaluate"
Cohesion: 0.17
Nodes (12): DecisionService, DecisionPolicy, EnforcementOutcome, Pass, Reject, Unavailable, Uncertain, ThresholdProfile (+4 more)

### Community 112 - "ClockSet"
Cohesion: 0.23
Nodes (3): ClockSet, queue_time_does_not_spend_active_but_consumes_wall(), sequential_vs_overlapping_tool_budgets()

### Community 113 - "ProviderAdmission"
Cohesion: 0.18
Nodes (6): shared_rate_limit_cooldown_blocks_admission(), AdmissionGuard, note_shared_rate_limit(), provider_admission_caps_at_two(), ProviderAdmission, rate_limit_cooldown_blocks_acquire()

### Community 114 - "Phase Plan: Home Recon Composer and Investigation Tabs"
Cohesion: 0.11
Nodes (17): 1. Outcome, 2. Home Layout (Section 4), 3. Home Composer (Section 5), 4. Submission & Transition (Section 6), 5. Persistent Investigation Tabs (Section 7), 6. Keyboard & Mouse Contract (Section 8), 7. Recovery & QoL (Section 9), Acceptance Criteria (Spec Section 12) (+9 more)

### Community 115 - "Value"
Cohesion: 0.21
Nodes (14): await_completion(), clip_chars_ellipsis(), clip_long_strings(), compact_page(), compact_page_evidence(), compact_page_system(), long_page_evidence_compacts_to_a_summary_for_synthesis(), packet_observation() (+6 more)

### Community 116 - "gsd-v2.js"
Cohesion: 0.12
Nodes (7): core, home, hooks, names, payload(), run(), setup()

### Community 117 - "inset"
Cohesion: 0.26
Nodes (17): atlas_run_card(), choice_hits(), choice_list_room(), cover(), draw_choice(), draw_intel_recon_popup(), draw_overlay(), draw_palette() (+9 more)

### Community 118 - "briefing_view.rs"
Cohesion: 0.24
Nodes (13): bucket_extracted(), buckets_split_facts_inferences_context_and_links(), claim(), claim_label(), ExtractedBuckets, ExtractedLine, MAX_ACTORS, MAX_CONTEXT (+5 more)

### Community 119 - "subscription.rs"
Cohesion: 0.32
Nodes (8): answer_event(), check_login(), command(), complete(), completion_is_ephemeral_and_does_not_inherit_tools_or_configuration(), exec_args(), is_subscription_status(), login()

### Community 120 - "Overlay"
Cohesion: 0.14
Nodes (14): ChoiceKind, IntelDay, Investigation, Model, Provider, Overlay, Block, Choice (+6 more)

### Community 121 - "measure_ann_recall"
Cohesion: 0.33
Nodes (5): AnnMeasurement, AnnThresholds, decide_ann_policy(), measure_ann_recall(), measure_ann_recall_justifies_only_when_thresholds_met()

### Community 122 - "synthesize"
Cohesion: 0.23
Nodes (20): a_dropped_provider_stream_keeps_the_text_already_received(), a_provider_that_rejects_streaming_still_returns_the_answer(), a_streamed_answer_with_a_bad_citation_ends_on_the_repaired_answer(), an_idle_stall_or_the_ceiling_keeps_the_partial_answer(), answer_text(), cancel_during_synthesis_marks_the_run_cancelled_and_keeps_partial_text(), chat_server(), ChatReply (+12 more)

### Community 123 - "Connection"
Cohesion: 0.29
Nodes (15): block_claimed(), claim_task(), claim_with(), ClaimedTask, complete_claimed(), fail_claimed(), finish_attempt(), finish_index_work() (+7 more)

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

### Community 128 - "context_turn"
Cohesion: 0.23
Nodes (14): a_keyword_inside_the_subjects_name_runs_no_news_or_legal_tool(), a_keyword_outside_the_subjects_name_still_runs_news_or_legal(), ac2_without_a_key_news_and_legal_tools_are_never_picked_and_with_a_key_they_are_eligible(), ac4_a_news_prompt_runs_newsapi_search_with_the_entity_and_who_is_runs_none(), ac5_a_sued_prompt_runs_courtlistener_case_or_docket_search_with_the_entity(), ac6_caps_hold_and_a_courtlistener_429_skips_the_rest(), ac7_the_relevance_gate_drops_an_off_topic_article(), context_keys() (+6 more)

### Community 129 - "home_rows"
Cohesion: 0.21
Nodes (17): center_row(), draw_home(), gap_row(), home_composer_areas(), home_group(), home_line(), home_line_text(), home_pads_titles_and_application_order() (+9 more)

### Community 130 - "agent"
Cohesion: 0.09
Nodes (25): agent, build, compaction, explore, plan, mode, model, permission (+17 more)

### Community 131 - "Functional Requirements"
Cohesion: 0.12
Nodes (15): Atlas (News Pipeline), Brain / Memory, Configuration Requirements, Context Provider Keys, Functional Requirements, Non-Functional Requirements, Non-Goals, OSINT Manual Runs (+7 more)

### Community 132 - "Store"
Cohesion: 0.18
Nodes (5): ArticleInsightCommit, insight_fingerprint(), active_index_rows(), Store, IndexEnqueue

### Community 133 - "InvestigationSurface"
Cohesion: 0.14
Nodes (9): InvestigationSurface, HomeComposer, IntelBrief, JobsResume, ReconChat, atlas_only_tools_blocked_on_all_surfaces(), check_need_gate(), sociavault_blocked_on_intel_surface() (+1 more)

### Community 134 - "accept_claims"
Cohesion: 0.18
Nodes (15): a_description_only_span_is_inference_and_an_added_name_is_dropped(), a_fact_requires_both_spans_in_the_title(), accept_claims(), article_with_body_spans(), ask_claims(), body_spans_accept_entity_and_object_from_full_article(), claim_json_tolerates_empty_and_alternate_shapes(), clip_chars() (+7 more)

### Community 135 - "DecisionAdapterKind"
Cohesion: 0.17
Nodes (13): DecisionAdapterKind, JsonMode, NativeDecisions, OutputTool, StrictJsonSchema, ValidatedText, parse_general_model_response(), DecisionValidationStatus (+5 more)

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
Cohesion: 0.15
Nodes (12): Architecture Notes (non-obvious), Argos OSINT — Agent Instructions, CI (`.github/workflows/ci.yml`), CLI Entry Points (`argos` binary), Developer Commands, Environment Variables, graphify, Key Files to Read for Context (+4 more)

### Community 140 - "model_roles.rs"
Cohesion: 0.12
Nodes (13): AccountHealth, AuthRequired, Cooldown, CreditsExhausted, Overloaded, Ready, Restricted, Unverified (+5 more)

### Community 141 - "Store"
Cohesion: 0.11
Nodes (6): CallProposal, HandoffRecord, TaskRecord, ClaimAssessment, validate_claim_assessment(), Store

### Community 142 - "rusqlite"
Cohesion: 0.19
Nodes (6): record_detail_lines(), ExplanationRecord, GraphSummaryEntry, migrate(), put_record(), Store

### Community 143 - "tasks.rs"
Cohesion: 0.08
Nodes (34): add_missing_columns(), adopt_untracked_index_changes(), complete_task(), DEFAULT_LLM_ATTEMPTS, DEFAULT_PROVIDER_CONCURRENCY, EMBEDDINGS_DISABLED, enqueue_index_work(), enqueue_job() (+26 more)

### Community 144 - "DecisionState"
Cohesion: 0.22
Nodes (4): DecisionState, EvidencePassageState, SubjectStateSummary, TaskStateSummary

### Community 145 - "ToolResult"
Cohesion: 0.18
Nodes (5): ToolResult, cacheable(), evidence_summary(), evidence_notes(), signatures()

### Community 146 - "RecordKind"
Cohesion: 0.29
Nodes (6): RecordKind, Claim, DerivedSummary, Memory, Passage, ToolObservation

### Community 147 - "IndexOutcome"
Cohesion: 0.17
Nodes (6): IndexOutcome, Disabled, Pending, PermanentFailure, Ready, RetryableFailure

### Community 148 - "ClaimRelation"
Cohesion: 0.29
Nodes (7): ClaimRelation, AmbiguousEntity, ChangedQuantity, Equivalent, Negation, PlannedVsCompleted, Unrelated

### Community 149 - "results.rs"
Cohesion: 0.16
Nodes (16): classify_output_quality(), extract_observation_items(), extract_search_results(), extracts_from_array_shaped_observations(), extracts_from_object_with_results_array(), normalize_tool_result(), NormalizedToolResult, OutputQuality (+8 more)

### Community 150 - "Store"
Cohesion: 0.53
Nodes (5): charge_newsapi(), charge_quota(), open_key(), quota_open(), utc_day()

### Community 151 - ".memory"
Cohesion: 0.18
Nodes (12): article(), atlas_articles_for_intel_dedupes_same_article_id_across_runs(), atlas_articles_for_intel_filters_category_day_and_query(), atlas_auto_next_round_trips_through_app_state(), atlas_claims_for_article_returns_linked_claims(), atlas_prune_drops_finished_runs_older_than_the_cutoff(), atlas_recent_articles_lists_retained_rows(), atlas_run_days_lists_distinct_days_newest_first() (+4 more)

### Community 152 - "Tone"
Cohesion: 0.14
Nodes (14): style(), Tone, Accent, Body, Bold, Code, Dim, Error (+6 more)

### Community 153 - "Rect"
Cohesion: 0.10
Nodes (56): draw(), api_key_slot(), ApiKeySlot, auth_areas(), brain_form(), brain_hit(), brain_list(), BrainForm (+48 more)

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

### Community 165 - "anyhow"
Cohesion: 0.22
Nodes (4): parse_native_response(), test_abstention_and_uncertainty_policy(), test_invalid_probability_distribution_rejected(), test_tool_selection_template_compilation()

### Community 166 - "BrainResourceSummary"
Cohesion: 0.19
Nodes (10): bindings_use_brain_evidence_ids(), BrainResourceHit, BrainResourceSummary, candidate_json(), clip(), format_counts(), is_http_url(), parse_brain_scrape_index() (+2 more)

### Community 167 - "investigation.rs"
Cohesion: 0.07
Nodes (74): Account, account_hits(), ACCOUNT_PLATFORMS, account_platforms_attach_to_the_subject_and_are_never_entities(), ACCOUNTS, action_order(), actions_are_grounded_capped_and_not_a_sweep(), ADAPTIVE (+66 more)

### Community 168 - "Milestone 1 — Core Onboarding (Current)"
Cohesion: 0.18
Nodes (10): Deliverables, Dependencies, Future Milestones (Planned), Milestone 1 — Core Onboarding (Current), Milestone 2 — CLI Verification, Milestone 3 — Test Coverage, Milestone 4 — Integration & Ship, Objectives (+2 more)

### Community 169 - "paths.rs"
Cohesion: 0.33
Nodes (7): auth_path(), config_path(), db_path(), ensure_home(), hardware_cache_path(), home_dir(), lancedb_dir()

### Community 170 - "HypothesisRecord"
Cohesion: 0.18
Nodes (13): Alternative, classify_hypothesis(), contradiction(), draft_hypotheses(), hypothesis_absence_stays_unresolved(), hypothesis_status(), HypothesisRecord, leading_name() (+5 more)

### Community 171 - "TaskState"
Cohesion: 0.18
Nodes (9): TaskState, Cancelled, Completed, Failed, Paused, Queued, RetryScheduled, Running (+1 more)

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

### Community 179 - "InvestigationEvent"
Cohesion: 0.22
Nodes (5): event_to_chat_block(), is_thinking_expanded(), InvestigationEvent, redact_secrets(), redacts_sensitive_keys()

### Community 180 - "Item"
Cohesion: 0.26
Nodes (6): Item, DecisionQuestionType, Choice, Noul, Score, QuestionSpec

### Community 181 - "ScheduledCall"
Cohesion: 0.29
Nodes (9): clock_set_for_turn(), courtlistener_spacing_and_firecrawl_polling_are_counted(), live(), more_calls_mean_more_tool_time_and_cache_hits_add_nothing(), scheduled(), ScheduledCall, tool_allowance_for_deps(), tool_allowance_for_deps_sequential_sums() (+1 more)

### Community 182 - "Docs Ingest — argos-osint"
Cohesion: 0.33
Nodes (5): `docs/architecture.md`, Docs Ingest — argos-osint, `docs/providers.md`, Ingest Status, Ingested Documentation

### Community 183 - "decide"
Cohesion: 0.24
Nodes (6): decide(), DecisionAnswer, decisions_client_posts_to_alpha_decisions_and_parses_answers(), DecisionsResponse, live_decisions_smoke(), parse_decisions()

### Community 184 - "draw"
Cohesion: 0.24
Nodes (11): Chrome, composer_height(), draw(), draw_header(), draw_slash_hint(), draw_tab_strip(), footer_line(), header_detail() (+3 more)

### Community 185 - "OpenCode V2 workflow"
Cohesion: 0.40
Nodes (4): Focused ECC commands, OpenCode V2 workflow, Start with the graph, Verify setup

### Community 186 - "Unified reliability / summarization / semantic pipelines — completion checklist"
Cohesion: 0.40
Nodes (4): Honest remaining gaps, Phase status (§18), This follow-up, Unified reliability / summarization / semantic pipelines — completion checklist

### Community 187 - "TaskStatus"
Cohesion: 0.14
Nodes (11): TaskStatus, Cancelled, Completed, Deferred, Failed, Partial, Planned, Ready (+3 more)

### Community 201 - "super"
Cohesion: 0.08
Nodes (13): GraphEdgeKind, ContradictionOrUpdate, Evidenced, Inferred, SemanticSuggestion, related_evidence_view(), RelatedEvidenceHit, semantic_edges_are_not_factual() (+5 more)

### Community 202 - "unix_now"
Cohesion: 0.22
Nodes (7): atlas_auto_future_tick_waits_and_past_tick_reschedules(), atlas_auto_while_running_only_arms_the_next_slot(), atlas_history_button_counts_down_while_auto_run_is_on(), atlas_poll_wait(), manual_run_moves_the_next_auto_trigger_out_by_90_minutes(), unix_now(), Atlas

### Community 203 - "article"
Cohesion: 0.29
Nodes (8): admiralty_scales_claim_confidence_from_rsp_and_peers(), article(), kept(), lead_claims_scale_with_the_gate_and_context_is_not_capped_at_five(), peer_decisions_offer_candidate_ids_or_none(), peer_decisions_request(), the_highest_confidence_claims_survive_a_budget(), the_packet_keeps_four_and_reports_the_full_gate()

### Community 204 - "modes.rs"
Cohesion: 0.28
Nodes (11): scoped_section_plan(), chat_response_spec(), chat_response_spec_lists_mode_headings(), chat_section_titles(), every_mode_has_bluf_first_and_sources_last(), investigation_mode_spec(), investigation_mode_spec_guides_directives_and_picker(), normalize_chat_mode() (+3 more)

### Community 205 - "Argos Adaptive Decision Roles and Strict Output Contracts"
Cohesion: 0.25
Nodes (7): Argos Adaptive Decision Roles and Strict Output Contracts, Authoritative 12-Template Registry, Execution Adapters, Overview, State Isolation & Prompt Hardening, Threshold Policy & Uncertainty, TUI Display & Trace

### Community 206 - "H0-H2 Completion Summary"
Cohesion: 0.33
Nodes (5): H0-H2 Completion Summary, H0: Resolve Harness and Current Phase ✓, H1: Map Feature Seams ✓, H2: Freeze Contracts and Packets ✓, Next Step: H3 - State and Persistence Foundation (Build Agent)

### Community 207 - "run"
Cohesion: 0.07
Nodes (23): atlas_countdown_visible(), atlas_extracting_visible(), BrainListMode, Create, Graph, List, AtlasRun, intel_body_loading_visible() (+15 more)

### Community 208 - "Argos OSINT — Unified Investigation Harness"
Cohesion: 0.18
Nodes (10): 3-Level Catalog Projection, 3-Level Tool Catalog & 14 Intelligence Categories, 9 Logical Model Roles & Inheritance, Architecture, Argos OSINT — Unified Investigation Harness, Chronological Trace & Persistence, Inheritance Rules, Overview (+2 more)

### Community 209 - "PlanCall"
Cohesion: 0.60
Nodes (5): bound(), search_call(), served(), step(), PlanCall

### Community 210 - "render_tui_cells.py"
Cohesion: 0.31
Nodes (3): box(), color(), render()

### Community 211 - "AtlasArticleRow"
Cohesion: 0.16
Nodes (20): a_country_token_does_not_merge_into_a_longer_name(), a_decisions_model_does_not_extract_claims(), aliases_merge_on_shared_tokens_when_the_longer_form_is_a_span(), articles_have_span(), context_candidates(), context_prompt(), countries_equal(), extract() (+12 more)

### Community 212 - "classify_relevant_articles"
Cohesion: 0.43
Nodes (7): catalog_json(), classify_peers_chat(), classify_peers_decisions(), classify_relevant_articles(), parse_peer_matches(), peer_match_json_keeps_valid_cycle_ids(), PeerMatch

### Community 213 - "intel_recon/brain.rs"
Cohesion: 0.57
Nodes (5): intel_recon_publishes_durably_with_provenance_and_no_atlas_receipt(), ReconInsightUpdate, update(), upsert_adds_and_updates_claims(), upsert_recon_insights()

### Community 214 - "TimeoutProfile"
Cohesion: 0.29
Nodes (6): TimeoutProfile, .CLASSIFIER, .EXTRACTION, .PLANNING, .SUMMARIZATION, .SYNTHESIS

### Community 215 - "AnnPolicy"
Cohesion: 0.33
Nodes (4): AnnPolicy, AnnDeferredUnmeasured, AnnEnabled, ExactSearch

### Community 217 - "select_strategy"
Cohesion: 0.40
Nodes (6): has_concrete_identifier(), hypothesis_question(), select_strategy(), strategy_change_reason(), strategy_follows_the_question_and_can_change_without_erasing_work(), StrategyChoice

### Community 218 - "subject_of"
Cohesion: 0.12
Nodes (17): accounts_flow(), accounts_search_query(), clip_query(), complementary_queries(), derived_question_handles(), DiscoveryQuery, investigation_frame(), InvestigationFrame (+9 more)

### Community 219 - "AtlasPage"
Cohesion: 0.67
Nodes (3): AtlasPage, Live, Runs

### Community 220 - "IntelReconFocus"
Cohesion: 0.67
Nodes (3): IntelReconFocus, Start, Tab

### Community 221 - "resolve_adapter"
Cohesion: 0.67
Nodes (3): resolve_adapter(), is_decisions_model(), picker_transport()

## Knowledge Gaps
- **1385 isolated node(s):** `$schema`, `model`, `small_model`, `default_agent`, `mode` (+1380 more)
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 1884 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **22 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `App` connect `App` to `app.rs`, `ui.rs`, `.set_focus`, `ToolResult`, `Store`, `Message`, `hardware.rs`, `Store`, `brain_detail.rs`, `recon/graph.rs`, `run_atlas_inner`, `.on_work_event`, `SettingsFile`, `ReportMode`, `WorkEvent`, `Target`, `src/brain.rs`, `secrets.rs`, `unix_now`, `run`, `LogsView`, `tui/jobs.rs`, `FeedArticle`, `AtlasArticleRow`, `DefaultsRole`, `AtlasPage`, `IntelReconFocus`, `.handle_key`, `summary_card.rs`, `ModuleId`, `briefing_view.rs`, `Overlay`?**
  _High betweenness centrality (0.076) - this node is a cross-community bridge._
- **What connects `$schema`, `model`, `small_model` to the rest of the system?**
  _1385 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `land.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.0076045627376425855 - nodes in this community are weakly interconnected._
- **Why does `ButtonId` connect `ButtonId` to `App`, `summary_card.rs`, `app.rs`, `run`, `tui/jobs.rs`, `DefaultsRole`, `logs.rs`, `Rect`, `Frame`, `Target`?**
  _High betweenness centrality (0.044) - this node is a cross-community bridge._
- **Should `orchestrate.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.05454545454545454 - nodes in this community are weakly interconnected._
- **Why does `ProviderSecret` connect `ProviderSecret` to `orchestrate.rs`, `App`, `atlas_memory.rs`, `accept_claims`, `recon.rs`, `gates.rs`, `Message`, `execute_steps`, `run_turn`, `body.rs`, `anyhow`, `atlas.rs`, `scheduler.rs`, `provider_attempt.rs`, `ReportMode`, `tests.rs`, `exec.rs`, `decide`, `worker.rs`, `intel_recon/jobs.rs`, `atlas_insights.rs`, `body_filter.rs`, `synthesize.rs`, `secrets.rs`, `summarization.rs`, `graph_explanation.rs`, `AtlasArticleRow`, `classify_relevant_articles`, `FeedArticle`, `.begin_title`, `replace_insights.rs`, `.default`, `.order`, `Service`, `KeptClaim`, `.evaluate`, `Value`, `subscription.rs`, `synthesize`, `poll_device`?**
  _High betweenness centrality (0.042) - this node is a cross-community bridge._
- **Should `App` be split into smaller, more focused modules?**
  _Cohesion score 0.0593607305936073 - nodes in this community are weakly interconnected._