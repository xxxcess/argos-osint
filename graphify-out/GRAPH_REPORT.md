# Graph Report - argos-osint  (2026-10-07)

## Corpus Check
- 128 files · ~336,665 words
- Verdict: corpus is large enough that graph structure adds value.
- Unclassified: 5 file(s) not represented in the graph (top: (none) 3, .toml 2)

## Summary
- 5305 nodes · 13446 edges · 195 communities (170 shown, 25 thin omitted)
- Extraction: 98% EXTRACTED · 2% INFERRED · 0% AMBIGUOUS · INFERRED: 228 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `20bad275`
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
- absorb_hit
- job_registry.rs
- ui.rs
- directives.rs
- .set_focus
- investigation.rs
- providers.rs
- atlas_news.rs
- Store
- publication.rs
- Call
- TurnEvent
- news_legal.rs
- hardware.rs
- App
- Store
- embed.rs
- osint.rs
- execute_steps
- tasks.rs
- provider.rs
- run_turn
- InvestigationPart
- body.rs
- .run_configured
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
- .default
- reliability_faults.rs
- tests.rs
- AtlasArticleRow
- events.rs
- exec.rs
- WorkEvent
- .is_empty
- ErrorCategory
- grok_oauth.rs
- now
- Rect
- brain_resources.rs
- worker.rs
- ProviderPage
- src/brain.rs
- intel_recon/jobs.rs
- cli.rs
- atlas_insights.rs
- InformationCredibility
- Category
- body_filter.rs
- synthesize.rs
- Binding
- anyhow
- rusqlite
- theme.rs
- Request
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
- evidence.rs
- ProviderSecret
- .new
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
- result
- validate.rs
- replace_insights.rs
- all_tools
- .order
- ToolResult
- provider_diag.rs
- Architecture
- KeptClaim
- serde
- ClockSet
- ProviderAdmission
- Phase Plan: Home Recon Composer and Investigation Tabs
- Value
- gsd-v2.js
- extract
- briefing_view.rs
- subscription.rs
- Overlay
- AnnPolicy
- chat_model
- RspEntry
- ReconCommand
- JobRow
- RouteInput
- poll_device
- context_turn
- RspIndex
- agent
- Functional Requirements
- Store
- IndexOutcome
- accept_claims
- SourceReliability
- OsintCommand
- .new
- the_label_updates_when_a_round_adds_calls_and_when_evidence_is_measured
- Argos OSINT — Agent Instructions
- home_rows
- ScheduledCall
- RspStatus
- Connection
- atlas_actions.rs
- run_live
- RecordKind
- AtlasJob
- ClaimRelation
- PublicationVerification
- Store
- split_vertical
- §9 implementation order
- TurnContinuation
- Part
- argos-osint-bin
- What Was Learned
- .new
- Milestone 1 — Core Onboarding (Current)
- paths.rs
- article
- TaskState
- H2 Contracts: Frozen Interface Decisions
- youtube_pair
- Codebase Map — argos-osint
- PROJECT.md — argos-osint
- Current Phase State
- ReconLimits
- GraphNodeKind
- Docs Ingest — argos-osint
- GraphEdgeKind
- OpenCode V2 workflow
- Unified reliability / summarization / semantic pipelines — completion checklist
- package.json
- super
- H0-H2 Completion Summary
- LaunchState
- Value
- BrainListMode

## God Nodes (most connected - your core abstractions)
1. `App` - 280 edges
2. `ProviderSecret` - 115 edges
3. `ButtonId` - 100 edges
4. `Store` - 63 edges
5. `Store` - 60 edges
6. `Target` - 58 edges
7. `ToolResult` - 56 edges
8. `FieldId` - 55 edges
9. `AtlasArticleRow` - 51 edges
10. `run_atlas_inner()` - 46 edges

## Surprising Connections (you probably didn't know these)
- `every_tool_request_sends_a_non_empty_user_agent()` --references--> `agent`  [INFERRED]
  crates/argos-osint-core/src/osint.rs → .opencode/opencode.json
- `success_is_keyed_and_any_input_change_invalidates_it()` --references--> `edit`  [INFERRED]
  crates/argos-osint-core/src/graph_explanation/tests.rs → .opencode/opencode.json
- `worker_killed_after_the_lance_write_is_recovered_by_revision()` --references--> `edit`  [INFERRED]
  crates/argos-osint-core/src/store/publication.rs → .opencode/opencode.json
- `draw_intel_bulletin()` --calls--> `intel_category_short()`  [INFERRED]
  crates/argos-osint-bin/src/tui/ui.rs → crates/argos-osint-bin/src/tui/app.rs
- `create_report_job()` --calls--> `body_assertion_candidates()`  [INFERRED]
  crates/argos-osint-core/src/intel_recon/jobs.rs → crates/argos-osint-core/src/intel_recon/ledger.rs

## Import Cycles
- 2-file cycle: `crates/argos-osint-bin/src/cli.rs -> crates/argos-osint-core/src/hardware.rs -> crates/argos-osint-bin/src/cli.rs`

## Communities (195 total, 25 thin omitted)

### Community 0 - "land.rs"
Cohesion: 0.01
Nodes (262): CODES, LABELS, R0, R1, R10, R100, R101, R102 (+254 more)

### Community 1 - "orchestrate.rs"
Cohesion: 0.06
Nodes (45): a_follow_up_keeps_names_from_the_previous_synthesis(), a_recon_model_429_trips_the_gate_and_later_calls_skip_the_provider(), ac6_a_courtlistener_429_still_lets_the_turn_synthesize(), ac7_a_pronoun_follow_up_takes_the_thread_subject(), ACCOUNT_TOOLS, ACME, action_call(), binding_ground() (+37 more)

### Community 2 - "App"
Cohesion: 0.05
Nodes (18): App, atlas_countdown_visible(), atlas_extracting_visible(), AtlasPage, Live, Runs, intel_body_loading_visible(), intel_insights_loading_visible() (+10 more)

### Community 3 - "atlas_memory.rs"
Cohesion: 0.06
Nodes (86): Extraction, atlas_memory_count(), background_indexing_then_refresh_upgrades_a_partial_run(), Barrier, Checkpoint, CheckpointInput, claims(), clear() (+78 more)

### Community 4 - "tool_io.rs"
Cohesion: 0.04
Nodes (94): display_name(), normalize_platform(), question_bindings(), a_gap_filler_only_domain_waits_for_a_primary_observation_before_hunter(), accept_bindings(), batch_scrape_takes_ranked_urls_and_gates_order_before_hunter(), bind_arguments(), BINDING_KINDS (+86 more)

### Community 5 - "app.rs"
Cohesion: 0.06
Nodes (70): a_click_folds_an_event_and_incoming_entries_do_not_move_a_paused_list(), a_finished_tool_call_is_logged_once_and_the_transcript_keeps_a_summary(), all_provider_actions_render_with_hit_areas_at_80x24(), ATLAS_AUTO_SECS, atlas_auto_toggle_persists_the_next_trigger(), atlas_claim(), atlas_headlines_scroll_and_request_failures_reach_the_system_log(), atlas_history_resume_is_offered_only_for_resumable_cycles_and_repair_starts_once() (+62 more)

### Community 6 - "brain_lance.rs"
Cohesion: 0.06
Nodes (38): activate_generation(), batch(), begin_generation(), begin_generation_then_activate(), block_on(), BrainIndex, clear_fingerprint(), current_fingerprint() (+30 more)

### Community 7 - "FieldId"
Cohesion: 0.05
Nodes (40): FieldId, BrainApp, BrainConversation, BrainInsight, BrainQuery, ClassifierModel, ClassifierProvider, Composer (+32 more)

### Community 8 - "map.rs"
Cohesion: 0.05
Nodes (75): BRAILLE_MAP, canonical(), claim_label(), contains(), country_at(), country_fit(), country_pixel(), draw_country_mini_map() (+67 more)

### Community 9 - "recon.rs"
Cohesion: 0.05
Nodes (59): answer_step_rate_limit_is_explicit_and_keeps_results(), attribute_word(), auxiliary(), BRIEF_SYNTHESIS, broad_question_collapses_to_grounded_lookups(), cacheable(), char_ceil(), char_floor() (+51 more)

### Community 10 - "ButtonId"
Cohesion: 0.03
Nodes (79): ButtonId, Add, AtlasAuto, AtlasDelete, AtlasLive, AtlasNews, AtlasNewsFeed, AtlasRepair (+71 more)

### Community 11 - "Store"
Cohesion: 0.08
Nodes (4): CreditHold, Store, strategy_and_provider_credits_survive_reopen(), Thread

### Community 12 - "absorb_hit"
Cohesion: 0.14
Nodes (18): absorb_hit(), account_platform_host(), Candidate, content_tokens(), distinct_queries(), domain_label(), EntityIdentifier, first_domain() (+10 more)

### Community 13 - "job_registry.rs"
Cohesion: 0.05
Nodes (46): begin(), begin_cancellable(), db(), db_path(), finish(), register_canonical(), beat(), BEAT_INTERVAL (+38 more)

### Community 14 - "ui.rs"
Cohesion: 0.05
Nodes (73): ACTION_H, atlas_countdown(), atlas_history_live_label(), atlas_source_anchors_are_not_labeled_deleted_origin(), call_stamp(), center_line(), center_text(), chat_blocks() (+65 more)

### Community 15 - "directives.rs"
Cohesion: 0.06
Nodes (63): apply_context_targets(), asks(), asks_about_judge(), asks_for_headlines(), clip_words(), context_entity(), CONTEXT_FILLER, context_gate() (+55 more)

### Community 16 - ".set_focus"
Cohesion: 0.05
Nodes (38): add_scroll(), brain_article_source_opens_intel_brief(), AtlasRun, hit(), IntelReconFocus, Section, Start, Tab (+30 more)

### Community 17 - "investigation.rs"
Cohesion: 0.05
Nodes (73): ACCOUNT_PLATFORMS, ACCOUNTS, accounts_flow(), accounts_search_query(), ADAPTIVE, Alternative, binder_maps_kinds_to_inputs_and_never_hands_hunter_a_social_host(), binding() (+65 more)

### Community 18 - "providers.rs"
Cohesion: 0.05
Nodes (63): clip_page(), number_arg(), assert_host_locked(), BATCH_SCRAPE_DEFAULT_URLS, BATCH_SCRAPE_MAX_URLS, batch_urls(), company_card(), crawl_limit() (+55 more)

### Community 19 - "atlas_news.rs"
Cohesion: 0.06
Nodes (48): api_error(), ATLAS_TOOLS, authors(), clip(), country_arg(), country_code(), CURRENTS_COUNTRIES, CURRENTS_HOST (+40 more)

### Community 20 - "Store"
Cohesion: 0.07
Nodes (18): article_body_from_row(), ArticleBodyRow, element_from_row(), IntelAssessmentRow, IntelElementRow, IntelEvidenceRow, IntelInvestigationRow, IntelReportSectionRow (+10 more)

### Community 21 - "publication.rs"
Cohesion: 0.09
Nodes (41): claim(), AtlasInsightClaim, bump_memories_changed(), change_watcher_sees_only_committed_changes_and_coalesces(), claim(), clear(), clear_index_state(), delete_run_memory_state() (+33 more)

### Community 22 - "Call"
Cohesion: 0.14
Nodes (24): build_blocks(), clip_chars(), coverage_only_counts_explicit_assessments(), coverage_summary(), decision_row_shows_directives_picker_order_bindings_and_fallbacks(), extract_log(), footer_line(), input_brief() (+16 more)

### Community 23 - "TurnEvent"
Cohesion: 0.14
Nodes (15): await_completion(), compact_page(), compact_page_evidence(), compact_page_system(), finish_recon_job(), deltas(), Run, snapshot_secret() (+7 more)

### Community 24 - "news_legal.rs"
Cohesion: 0.07
Nodes (47): a_rate_limit_switches_to_the_fallback_key_and_a_rejection_does_not(), ac1_catalog_has_55_tools_and_the_news_and_legal_entries(), ac1_each_request_has_its_url_params_and_auth_header_and_no_key_in_the_url(), an_echoed_key_is_redacted_from_the_stored_body_and_error(), article(), articles(), CATEGORIES, choice_arg() (+39 more)

### Community 25 - "hardware.rs"
Cohesion: 0.16
Nodes (17): CACHE_TTL_SECS, classify_arch(), disk_totals(), HardwareProfile, metal_vram_gb(), now_secs(), parse_apple_gpu_cores(), partial_gpu() (+9 more)

### Community 26 - "App"
Cohesion: 0.08
Nodes (69): atlas_auto_label(), atlas_cycle_stats_room(), atlas_cycle_stats_scroll_max(), atlas_extracting(), atlas_feed_room(), atlas_feed_room_for(), atlas_hit(), atlas_live_areas() (+61 more)

### Community 27 - "Store"
Cohesion: 0.06
Nodes (10): atlas_answer_id(), atlas_brief_id(), AtlasRunRow, AtlasStoredClaim, GraphSummary, has_table(), ReindexReport, repair_embed_tables() (+2 more)

### Community 28 - "embed.rs"
Cohesion: 0.08
Nodes (31): active(), DIM, disable(), disabled(), DisableGuard, download_file(), DOWNLOAD_TIMEOUT_SECS, embed_batch() (+23 more)

### Community 29 - "osint.rs"
Cohesion: 0.07
Nodes (42): CACHE_DAY_SECONDS, cache_follows_the_provider_plan_interval(), CACHE_MONTH_SECONDS, cache_seconds(), CACHE_WEEK_SECONDS, canonical_tool_id(), default_enabled(), DEFAULT_USER_AGENT (+34 more)

### Community 30 - "execute_steps"
Cohesion: 0.19
Nodes (25): after_step(), apply_order(), context_block(), context_dispatched(), directive_for(), execute_steps(), expand_per_platform(), extract_bindings() (+17 more)

### Community 31 - "tasks.rs"
Cohesion: 0.09
Nodes (37): adopt_untracked_index_changes(), claim_index_changes(), claim_index_work(), complete_index_change(), complete_task(), DEFAULT_LLM_ATTEMPTS, DEFAULT_PROVIDER_CONCURRENCY, disabled_outcome_blocks_without_spending_attempts_and_resumes() (+29 more)

### Community 32 - "provider.rs"
Cohesion: 0.06
Nodes (32): concrete_free_models(), decide(), DecisionAnswer, decisions_client_posts_to_alpha_decisions_and_parses_answers(), DECISIONS_MODELS, decisions_url(), DecisionsResponse, default_grok_model() (+24 more)

### Community 33 - "run_turn"
Cohesion: 0.15
Nodes (25): call_cached(), cancelled(), classify_turn_mode(), continue_turn(), derive_directives(), derived_note(), enabled_tools(), execute_ordered() (+17 more)

### Community 34 - "InvestigationPart"
Cohesion: 0.18
Nodes (10): InvestigationPart, DirectiveAssessment, Plan, PlanDiagnostics, Status, StreamingSynthesis, Synthesis, ToolActivity (+2 more)

### Community 35 - "body.rs"
Cohesion: 0.09
Nodes (34): body_fetch_routes(), BodyFetchEvent, Attempt, Failed, InsightsFailed, InsightsRefreshing, InsightsReplaced, Progress (+26 more)

### Community 36 - ".run_configured"
Cohesion: 0.12
Nodes (25): a_claimed_email_keeps_no_person_data(), annotate(), bind_request(), claimed_email(), credential_key(), custom_user_agent(), effective_user_agent(), Executor (+17 more)

### Community 37 - "brain_detail.rs"
Cohesion: 0.06
Nodes (45): areas(), BrainDetail, DetailAreas, DetailPane, Path, Related, Summary, DetailSnapshot (+37 more)

### Community 38 - "recon/graph.rs"
Cohesion: 0.14
Nodes (29): basic_explanation(), build_claim_graph(), build_memory_graph(), call_for(), choose_directive(), claim_tokens(), directive_ids(), directive_tokens() (+21 more)

### Community 39 - "atlas.rs"
Cohesion: 0.09
Nodes (29): article_card_labels_title_publisher_author_and_classification(), Band, band_sizes(), band_temperature(), category_from_decisions(), CATEGORY_IDS, CLUSTERS, country_label() (+21 more)

### Community 40 - "run_atlas_inner"
Cohesion: 0.09
Nodes (36): article_from_row(), AtlasEvent, Article, Classified, InsightProgress, MemoriesChanged, MemoryProgress, Note (+28 more)

### Community 41 - ".on_work_event"
Cohesion: 0.08
Nodes (8): a_failed_turn_keeps_the_streamed_answer_on_screen(), a_log_or_late_tool_row_does_not_drop_the_live_answer(), atlas_log_level(), logs_record_errors_durably_and_scroll(), recon_chat_folds_decisions_and_opens_synthesis_memory(), summary_system(), synthesis_deltas_fill_the_live_bubble_and_the_saved_answer_replaces_them(), RelatedRow

### Community 42 - "scheduler.rs"
Cohesion: 0.10
Nodes (22): apply_index_change(), apply_index_change_rebuild_reports_outcome(), apply_index_with(), DEFAULT_LEASE_SECS, drain_index_once(), drain_summary_flush(), drain_summary_flush_cached(), drain_summary_flush_completes_cached_tasks() (+14 more)

### Community 43 - "jobs_view.rs"
Cohesion: 0.11
Nodes (21): summary_text(), active_time_is_live_for_open_attempts_and_unavailable_only_without_a_start(), AttemptRow, event_apps(), event_counts(), EventCounts, get_job(), job() (+13 more)

### Community 44 - "provider_attempt.rs"
Cohesion: 0.10
Nodes (36): Acc, attempt(), AttemptReport, check_final(), complete_stream_and_json_answers_succeed(), Deadlines, delta(), fill() (+28 more)

### Community 45 - "SettingsFile"
Cohesion: 0.08
Nodes (26): a_blank_osint_user_agent_loads_as_unset(), default_credit_reset(), default_firecrawl_credits(), default_hunter_credits(), default_max_calls(), default_max_rounds(), default_one_cost(), default_opening_cap() (+18 more)

### Community 46 - "picker.rs"
Cohesion: 0.10
Nodes (27): brain_scrape_options_hide_bare_scrape_and_include_claim_criteria(), CatalogEntry, chat_request(), CONFIDENCE_FLOOR, context_additions(), context_tools(), decisions_request(), directives_phrase() (+19 more)

### Community 47 - "ReportMode"
Cohesion: 0.12
Nodes (22): HomeDraftState, classifiable_modes(), classify_mode_chat(), classify_mode_decisions(), classify_prompt_mode(), classify_prompt_mode_chat(), classify_prompt_mode_decisions(), classify_recon_mode() (+14 more)

### Community 48 - ".default"
Cohesion: 0.10
Nodes (30): a_budgeted_call_with_a_dependency_from_an_earlier_batch_runs(), a_claimed_email_removes_its_bindings(), a_gap_filler_domain_feeds_hunter_once_firecrawl_observes_it(), a_goal_naming_a_tool_or_provider_is_repaired_then_falls_back(), a_goal_naming_a_tool_outside_the_prompt_entity_is_still_rejected(), a_handle_named_in_a_derived_question_is_an_unverified_binding(), a_prompt_entity_named_like_a_provider_keeps_the_models_directives(), a_weak_firecrawl_search_adds_one_google_search_with_the_same_query() (+22 more)

### Community 49 - "reliability_faults.rs"
Cohesion: 0.08
Nodes (14): AttemptOutcome, categorize_http(), execute_with_retries(), other_llm_allows_three(), summarization_stops_at_two_attempts(), TimeoutProfile, .CLASSIFIER, .EXTRACTION (+6 more)

### Community 50 - "tests.rs"
Cohesion: 0.27
Nodes (16): auth_failure_sends_one_request_gives_guidance_and_redacts(), conn(), count(), failure_is_durable_correlated_bounded_and_harmless(), fixture(), Fx, GOOD, late_completion_for_a_changed_or_deleted_memory_is_not_published() (+8 more)

### Community 51 - "AtlasArticleRow"
Cohesion: 0.18
Nodes (20): category_tag(), accept_one(), AcceptMode, Context, Lead, apply_peer_support(), catalog_json(), classifier_peers_are_preferred_over_token_overlap() (+12 more)

### Community 52 - "events.rs"
Cohesion: 0.10
Nodes (24): clear_events(), DEFAULT_RETENTION_HOURS, EventFilter, get_event(), job_filter_includes_descendants_and_prune_keeps_jobs(), list_events(), MAX_DETAIL_CHARS, MAX_MESSAGE_CHARS (+16 more)

### Community 53 - "exec.rs"
Cohesion: 0.14
Nodes (19): admission_account(), admission_contention_consumes_no_attempt(), AttemptEvent, Finished, Started, AttemptLog, cancelled(), complete_summary_report() (+11 more)

### Community 54 - "WorkEvent"
Cohesion: 0.08
Nodes (22): atlas_auto_future_tick_waits_and_past_tick_reschedules(), atlas_auto_while_running_only_arms_the_next_slot(), atlas_history_button_counts_down_while_auto_run_is_on(), atlas_poll_wait(), manual_run_moves_the_next_auto_trigger_out_by_90_minutes(), unix_now(), work_event(), WorkEvent (+14 more)

### Community 55 - ".is_empty"
Cohesion: 0.14
Nodes (22): chat_body(), ChatMessage, complete(), complete_errors_with_finish_reason_when_response_is_empty(), complete_once(), complete_reads_reasoning_only_non_stream_json(), complete_stream_of_reasoning_deltas_yields_text(), Completion (+14 more)

### Community 56 - "ErrorCategory"
Cohesion: 0.08
Nodes (24): operation_kind(), backoff_delay(), can_retry(), ErrorCategory, AuthOrQuota, CancelledOrStale, ConfigurationMissing, ContextLimit (+16 more)

### Community 57 - "grok_oauth.rs"
Cohesion: 0.12
Nodes (20): auth_path(), bearer(), bearer_from_path(), check_login(), Entry, entry_from(), failed_login_keeps_the_cli_reason_and_recovery_action(), login() (+12 more)

### Community 58 - "now"
Cohesion: 0.20
Nodes (13): answer_evidence_survives_reopen_and_deleted_thread_rejects_late_calls(), claims_deduplicate_and_reject_unsupported_sources(), deleting_an_investigation_removes_its_brain_memories_and_graph_summary(), investigation_memory_ids(), Message, now(), persist_claims(), persistence_and_plan() (+5 more)

### Community 59 - "Rect"
Cohesion: 0.16
Nodes (45): AbsRect, button_areas(), cursor_at(), draw(), draw_atlas(), draw_atlas_cycle_stats(), draw_atlas_live(), draw_atlas_news() (+37 more)

### Community 60 - "brain_resources.rs"
Cohesion: 0.08
Nodes (32): source_anchor_label(), bindings_use_brain_evidence_ids(), BRAIN_SCRAPE_PREFIX, BrainResourceHit, BrainResourceSummary, candidate_json(), CLAIM_CHARS, classify_resource() (+24 more)

### Community 61 - "worker.rs"
Cohesion: 0.17
Nodes (19): IntelReportEvent, InsightsUpdated, JobCreated, JobDone, Section, Stage, IntelReportJobRow, cancel_if_needed() (+11 more)

### Community 62 - "ProviderPage"
Cohesion: 0.15
Nodes (10): ProviderEvent, Finished, Progress, ProviderPage, .ALL, Defaults, Grok, OpenAI (+2 more)

### Community 63 - "src/brain.rs"
Cohesion: 0.12
Nodes (21): AGREEMENT_WEIGHT, CATEGORIES, category_hint(), format_injection(), hybrid_recall(), hybrid_recall_adds_paraphrases_and_rewards_agreement(), identity_query_prefers_name_memory(), jaccard() (+13 more)

### Community 64 - "intel_recon/jobs.rs"
Cohesion: 0.10
Nodes (28): active_job_for_mode(), cancel_job(), create_job_inserts_all_verify_sections(), create_report_job(), finish_canonical(), intel_recon_links_its_legacy_job_to_one_canonical_registry_job(), pause_job(), ReportScope (+20 more)

### Community 65 - "cli.rs"
Cohesion: 0.18
Nodes (18): ask(), ask_thread(), atlas_command(), defaults_command(), defaults_json(), defaults_show_includes_the_tool_picker_transport(), dispatch(), login() (+10 more)

### Community 66 - "atlas_insights.rs"
Cohesion: 0.09
Nodes (34): a_country_token_does_not_merge_into_a_longer_name(), aliases_merge_on_shared_tokens_when_the_longer_form_is_a_span(), article_with_body_spans(), articles_have_span(), BODY_CLAIM_LIMIT, BODY_SPAN_CHARS, clip_chars(), COL_CLASS (+26 more)

### Community 67 - "InformationCredibility"
Cohesion: 0.13
Nodes (17): AdmiraltyCode, best_credibility(), CredibilityInputs, information_credibility(), InformationCredibility, CannotBeJudged, Confirmed, DoubtfullyTrue (+9 more)

### Community 68 - "Category"
Cohesion: 0.05
Nodes (34): Category, Auth, Cancelled, Configuration, Empty, InvalidModel, InvalidResult, MalformedPayload (+26 more)

### Community 69 - "body_filter.rs"
Cohesion: 0.17
Nodes (22): BATCH_SIZE, BodyChunk, chunk_article_body(), CHUNK_MAX, CHUNK_MIN, CHUNK_TARGET, chunks_preserve_absolute_offsets(), classify_chunk_batch() (+14 more)

### Community 70 - "synthesize.rs"
Cohesion: 0.17
Nodes (19): BODY_REFINE_INPUT_CHARS, deterministic_bluf_mentions_title(), deterministic_section(), model_refine_body(), model_synthesize(), parse_json_object(), refine_retrieved_article_body(), refine_without_secret_keeps_validated_scrape() (+11 more)

### Community 71 - "Binding"
Cohesion: 0.14
Nodes (29): Binding, canonical(), dependency_order(), depends_on(), question_bindings_and_dependency_fix(), allowed_producer(), best_handle(), bind_step() (+21 more)

### Community 72 - "anyhow"
Cohesion: 0.16
Nodes (7): openrouter_ready(), accounts_persist_with_owner_only_permissions(), AuthFile, legacy_connection_migrates_without_overwriting_other_accounts(), loading_old_auth_removes_gmail_setup_credentials(), owner_only(), write_private()

### Community 73 - "rusqlite"
Cohesion: 0.14
Nodes (10): record_detail_lines(), ExplanationRecord, GraphSummaryEntry, migrate(), put_record(), SaveOutcome, MemoryChanged, MemoryGone (+2 more)

### Community 74 - "theme.rs"
Cohesion: 0.18
Nodes (19): ACCENT, BG, BORDER, card_accent(), card_dim(), card_text(), CODE_BG, DIM (+11 more)

### Community 75 - "Request"
Cohesion: 0.28
Nodes (9): bitcoin(), bounded(), domain(), email_address(), ip(), linkedin_handle(), one_of(), hunter_request() (+1 more)

### Community 76 - "pipeline.rs"
Cohesion: 0.16
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
Cohesion: 0.08
Nodes (23): BASIC_HEADING, Cached, None, Stale, Valid, event(), explain(), ExplainOutcome (+15 more)

### Community 81 - "tui/jobs.rs"
Cohesion: 0.09
Nodes (24): areas(), BASE_COLUMNS, button_label(), buttons(), detail_lines(), draw(), hit(), JobsAreas (+16 more)

### Community 82 - ".new"
Cohesion: 0.26
Nodes (18): a_spent_primary_quota_uses_the_fallback_key(), country_labels_show_the_name_and_the_code(), daily_cap(), format_run_card(), HttpCall, HttpReply, insights_do_not_call_a_decisions_model(), keys() (+10 more)

### Community 83 - "FeedArticle"
Cohesion: 0.19
Nodes (17): apply_hits(), article_from(), article_row(), canonical_url(), dedup_drops_a_url_an_exact_title_and_keeps_the_higher_domain(), FeedArticle, host_of(), judge() (+9 more)

### Community 84 - "budget.rs"
Cohesion: 0.12
Nodes (14): CUT_NOTE, CUT_SHORT, PHASE_WINDOW_SECONDS, RECON_DEADLINE, RECON_ROUND_SECONDS, STREAM_LOST, synthesis_allowance_capped(), synthesis_allowance_seconds() (+6 more)

### Community 85 - "store.rs"
Cohesion: 0.08
Nodes (32): article(), atlas_article_from_row(), atlas_articles_for_intel_dedupes_same_article_id_across_runs(), atlas_articles_for_intel_filters_category_day_and_query(), atlas_auto_next_round_trips_through_app_state(), atlas_claims_for_article_returns_linked_claims(), atlas_prune_drops_finished_runs_older_than_the_cutoff(), atlas_recent_articles_lists_retained_rows() (+24 more)

### Community 86 - "Command"
Cohesion: 0.09
Nodes (23): AtlasCommand, Repair, Resume, Verify, Cli, Command, Atlas, Defaults (+15 more)

### Community 87 - "DefaultsRole"
Cohesion: 0.09
Nodes (13): ChoiceItem, codex_models(), DefaultsRole, .ALL, Classifier, Recon, Summarization, Synthesis (+5 more)

### Community 88 - "logs.rs"
Cohesion: 0.17
Nodes (18): starts(), stamp(), areas(), button_label(), buttons(), count(), draw(), hit() (+10 more)

### Community 89 - "evidence.rs"
Cohesion: 0.23
Nodes (14): AGREEMENT_WEIGHT, chunk_text(), chunks_cover_end_of_long_source(), content_hash(), ensure_identifier_coverage(), EvidencePassage, hybrid_bounds_and_prefers_agreement(), hybrid_passage_candidates() (+6 more)

### Community 90 - "ProviderSecret"
Cohesion: 0.21
Nodes (21): account_secret(), active_text_secret(), authorize(), bearer_token(), catalog_error(), effective_kind(), http(), list_catalog() (+13 more)

### Community 91 - ".new"
Cohesion: 0.20
Nodes (16): cache_invalidates_when_source_revision_changes(), lease_fencing_rejects_stale_epoch_and_foreign_owner(), claim_complete_and_retry_round_trip(), claim_next(), claim_next_in(), enqueue_job(), enqueue_job_with(), job() (+8 more)

### Community 92 - ".handle_key"
Cohesion: 0.07
Nodes (15): backspace_after_a_sent_question_deletes_one_character(), ctrl_arrows_cycle_app_tabs_forward_and_back(), ctrl_tab_cycles_app_tabs_forward_and_back(), draft_isolation_and_persistence(), duplicate_submission_prevention(), home_order_renames_and_nine_routes_agree(), is_picker_field(), multiline_paste_stays_in_composer_and_does_not_run_shortcuts() (+7 more)

### Community 93 - "F"
Cohesion: 0.17
Nodes (17): Fault, CallSpec, classification_request(), dispatch(), fetch_with_spare_key(), json_message(), one_line(), parse_fault() (+9 more)

### Community 94 - "wikipedia_rsp.rs"
Cohesion: 0.17
Nodes (15): API, APP_STATE_KEY, CACHE_TTL, index_to_json(), normalize_host(), parse_last_year(), parse_rsp_wikitext(), parse_source_name() (+7 more)

### Community 96 - "How"
Cohesion: 0.11
Nodes (18): How, CompanyEmail, Coordinates, DomainAsUrl, EntityPhrase, PackageName, PackageParts, Plain (+10 more)

### Community 97 - "H3 Task Packet: State and Persistence Foundation"
Cohesion: 0.07
Nodes (26): Acceptance Checks and Exact Existing Test Targets:, Current Diff / Prior Changes to Preserve:, Effective Agent / Model / Variant:, Exact Existing Test Targets to Reuse/Extend:, Existing APIs / Data Models to Reuse:, Expected Changed Files:, Expected Commands:, Files This Worker Owns: (+18 more)

### Community 98 - "summary_card.rs"
Cohesion: 0.29
Nodes (11): actions(), button_at(), button_cells(), button_row_area(), draw(), height(), is_card_button(), label() (+3 more)

### Community 99 - "contains"
Cohesion: 0.10
Nodes (29): abs_contains(), abs_rect(), contains(), home_line(), home_line_text(), intel_body_loading(), intel_body_progress_lines(), intel_brief_full_lines() (+21 more)

### Community 100 - "ModuleId"
Cohesion: 0.15
Nodes (16): ModuleId, .ALL, Atlas, Brain, Intel, Jobs, Logs, Osint (+8 more)

### Community 101 - "Region"
Cohesion: 0.12
Nodes (17): Region, AtlasInsights, AtlasNews, AtlasOrigins, AtlasRuns, Chat, Detail, IntelBrief (+9 more)

### Community 102 - "result"
Cohesion: 0.12
Nodes (17): intel_recon_publishes_durably_with_provenance_and_no_atlas_receipt(), ReconInsightUpdate, update(), upsert_adds_and_updates_claims(), upsert_recon_insights(), body_assertion_candidates(), coverage_complete(), ElementStatus (+9 more)

### Community 103 - "validate.rs"
Cohesion: 0.19
Nodes (15): BLOCK_MARKERS, BodyQuality, Complete, Partial, Unavailable, Uncertain, BodyValidation, clean_markdown() (+7 more)

### Community 104 - "replace_insights.rs"
Cohesion: 0.31
Nodes (11): commit_refined_body(), article(), claim(), commit_replaces_atomically_and_keeps_shared_support(), failed_validation_leaves_prior_insights(), replace_article_insights_from_body(), ReplaceOutcome, snapshot_counts_are_stable_before_commit() (+3 more)

### Community 105 - "all_tools"
Cohesion: 0.16
Nodes (28): a_429_stops_picker_calls_and_the_fallback_finishes_the_list(), a_dispatch_error_fails_the_step_and_the_loop_goes_on(), a_failed_email_finder_gets_one_fallback_and_no_third_picker_call(), a_fallback_with_no_bindings_still_runs_the_satisfied_steps_and_reaches_synthesis(), a_question_handle_fills_the_handle_steps_without_a_fallback(), all_tools(), an_attribute_question_names_the_person_and_drops_qa_hosts(), an_imperative_social_prompt_names_the_person_not_the_sentence() (+20 more)

### Community 106 - ".order"
Cohesion: 0.28
Nodes (9): checked_serves(), fallback_record(), OrderContext, Ordered, parse_chat_pick(), Picker<'a>, PickReply, serves_for() (+1 more)

### Community 107 - "ToolResult"
Cohesion: 0.18
Nodes (20): ToolResult, ac6_citation_groups_split_validate_each_id_and_normalize(), AnswerContext, chat(), citation_groups(), citation_ids(), cut_footer(), cut_short_answer() (+12 more)

### Community 108 - "provider_diag.rs"
Cohesion: 0.17
Nodes (12): bounded(), classify_status(), endpoint_strips_query_and_userinfo(), find_url(), http_failure(), Leaf, provider_error(), request_id() (+4 more)

### Community 109 - "Architecture"
Cohesion: 0.07
Nodes (24): Admiralty source evaluation (WP:RSP), Architecture, Atlas, Investigation flow, Model and UI boundaries, News and Legal context tools (#29), Persistence, Primary providers (+16 more)

### Community 110 - "KeptClaim"
Cohesion: 0.26
Nodes (15): apply_admiralty_evaluation(), article_information_credibility(), body_lead_prompt(), brief_text(), cap_claims(), dedupe_claims(), entity_path(), extract_for_article_body() (+7 more)

### Community 111 - "serde"
Cohesion: 0.18
Nodes (11): GraphEdgeKind, ContradictionOrUpdate, Evidenced, Inferred, SemanticSuggestion, related_evidence_view(), RelatedEvidenceHit, semantic_edges_are_not_factual() (+3 more)

### Community 112 - "ClockSet"
Cohesion: 0.21
Nodes (3): ClockSet, queue_time_does_not_spend_active_but_consumes_wall(), sequential_vs_overlapping_tool_budgets()

### Community 113 - "ProviderAdmission"
Cohesion: 0.18
Nodes (6): shared_rate_limit_cooldown_blocks_admission(), AdmissionGuard, note_shared_rate_limit(), provider_admission_caps_at_two(), ProviderAdmission, rate_limit_cooldown_blocks_acquire()

### Community 114 - "Phase Plan: Home Recon Composer and Investigation Tabs"
Cohesion: 0.11
Nodes (17): 1. Outcome, 2. Home Layout (Section 4), 3. Home Composer (Section 5), 4. Submission & Transition (Section 6), 5. Persistent Investigation Tabs (Section 7), 6. Keyboard & Mouse Contract (Section 8), 7. Recovery & QoL (Section 9), Acceptance Criteria (Spec Section 12) (+9 more)

### Community 115 - "Value"
Cohesion: 0.23
Nodes (9): cited(), clip_chars_ellipsis(), clip_long_strings(), long_page_evidence_compacts_to_a_summary_for_synthesis(), page_excerpt(), page_needs_compact(), page_summary_observation(), replay_elon_multi_id_bracket_citation_validates() (+1 more)

### Community 116 - "gsd-v2.js"
Cohesion: 0.12
Nodes (7): core, home, hooks, names, payload(), run(), setup()

### Community 117 - "extract"
Cohesion: 0.19
Nodes (14): a_decisions_model_does_not_extract_claims(), context_candidates(), context_prompt(), countries_equal(), extract(), insight_packet(), is_significant(), lead_prompt() (+6 more)

### Community 118 - "briefing_view.rs"
Cohesion: 0.24
Nodes (13): bucket_extracted(), buckets_split_facts_inferences_context_and_links(), claim(), claim_label(), ExtractedBuckets, ExtractedLine, MAX_ACTORS, MAX_CONTEXT (+5 more)

### Community 119 - "subscription.rs"
Cohesion: 0.32
Nodes (8): answer_event(), check_login(), command(), complete(), completion_is_ephemeral_and_does_not_inherit_tools_or_configuration(), exec_args(), is_subscription_status(), login()

### Community 120 - "Overlay"
Cohesion: 0.14
Nodes (14): ChoiceKind, IntelDay, Investigation, Model, Provider, Overlay, Block, Choice (+6 more)

### Community 121 - "AnnPolicy"
Cohesion: 0.16
Nodes (9): AnnMeasurement, AnnPolicy, AnnDeferredUnmeasured, AnnEnabled, ExactSearch, AnnThresholds, decide_ann_policy(), measure_ann_recall() (+1 more)

### Community 122 - "chat_model"
Cohesion: 0.25
Nodes (21): a_dropped_provider_stream_keeps_the_text_already_received(), a_provider_that_rejects_streaming_still_returns_the_answer(), a_streamed_answer_with_a_bad_citation_ends_on_the_repaired_answer(), an_idle_stall_or_the_ceiling_keeps_the_partial_answer(), answer_text(), cancel_during_synthesis_marks_the_run_cancelled_and_keeps_partial_text(), cancel_during_tools_marks_the_run_cancelled(), chat_model() (+13 more)

### Community 123 - "RspEntry"
Cohesion: 0.22
Nodes (7): clip_summary(), normalize_name(), observation_for(), observation_marks_unlisted(), parses_status_domains_and_maps_reliability(), RspEntry, SourceReliabilityObservation

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
Cohesion: 0.39
Nodes (9): a_keyword_inside_the_subjects_name_runs_no_news_or_legal_tool(), a_keyword_outside_the_subjects_name_still_runs_news_or_legal(), ac2_without_a_key_news_and_legal_tools_are_never_picked_and_with_a_key_they_are_eligible(), ac4_a_news_prompt_runs_newsapi_search_with_the_entity_and_who_is_runs_none(), ac5_a_sued_prompt_runs_courtlistener_case_or_docket_search_with_the_entity(), context_keys(), context_turn(), missing_providers() (+1 more)

### Community 129 - "RspIndex"
Cohesion: 0.36
Nodes (9): cache(), cached_index(), CachedIndex, ensure_index(), fetch_index(), index_from_json(), install_index(), RspIndex (+1 more)

### Community 130 - "agent"
Cohesion: 0.09
Nodes (25): agent, build, compaction, explore, plan, mode, model, permission (+17 more)

### Community 131 - "Functional Requirements"
Cohesion: 0.12
Nodes (15): Atlas (News Pipeline), Brain / Memory, Configuration Requirements, Context Provider Keys, Functional Requirements, Non-Functional Requirements, Non-Goals, OSINT Manual Runs (+7 more)

### Community 132 - "Store"
Cohesion: 0.18
Nodes (5): ArticleInsightCommit, insight_fingerprint(), active_index_rows(), Store, IndexEnqueue

### Community 133 - "IndexOutcome"
Cohesion: 0.17
Nodes (6): IndexOutcome, Disabled, Pending, PermanentFailure, Ready, RetryableFailure

### Community 134 - "accept_claims"
Cohesion: 0.22
Nodes (13): a_description_only_span_is_inference_and_an_added_name_is_dropped(), a_fact_requires_both_spans_in_the_title(), accept_claims(), ask_claims(), body_spans_accept_entity_and_object_from_full_article(), claim_json_tolerates_empty_and_alternate_shapes(), context_claims_require_the_entity_in_the_title_and_link_context_for(), parse_claims() (+5 more)

### Community 135 - "SourceReliability"
Cohesion: 0.22
Nodes (5): SourceReliability, A, B, C, D

### Community 136 - "OsintCommand"
Cohesion: 0.22
Nodes (9): OsintCommand, Attach, Describe, Disable, Enable, History, List, Run (+1 more)

### Community 137 - ".new"
Cohesion: 0.25
Nodes (5): aging_the_tool_clock_blocks_new_calls_and_cache_hits_do_not_extend_it(), clock_set_for_turn(), foreground_expiry_continues_in_background(), sequential_raise_calls_budgets_higher(), TurnCheckpoint

### Community 138 - "the_label_updates_when_a_round_adds_calls_and_when_evidence_is_measured"
Cohesion: 0.24
Nodes (4): a_round_keeps_its_time_and_a_tight_ceiling_still_runs_tools(), format_deadline(), format_span(), the_label_updates_when_a_round_adds_calls_and_when_evidence_is_measured()

### Community 139 - "Argos OSINT — Agent Instructions"
Cohesion: 0.15
Nodes (12): Architecture Notes (non-obvious), Argos OSINT — Agent Instructions, CI (`.github/workflows/ci.yml`), CLI Entry Points (`argos` binary), Developer Commands, Environment Variables, graphify, Key Files to Read for Context (+4 more)

### Community 140 - "home_rows"
Cohesion: 0.21
Nodes (16): center_row(), gap_row(), home_composer_areas(), home_group(), home_pads_titles_and_application_order(), home_rows(), HomeComposerAreas, HomeKind (+8 more)

### Community 141 - "ScheduledCall"
Cohesion: 0.38
Nodes (8): courtlistener_spacing_and_firecrawl_polling_are_counted(), live(), more_calls_mean_more_tool_time_and_cache_hits_add_nothing(), scheduled(), ScheduledCall, tool_allowance_for_deps(), tool_allowance_for_deps_sequential_sums(), tool_allowance_seconds()

### Community 142 - "RspStatus"
Cohesion: 0.29
Nodes (7): parse_status(), RspStatus, Blacklisted, Deprecated, GenerallyReliable, GenerallyUnreliable, NoConsensus

### Community 143 - "Connection"
Cohesion: 0.24
Nodes (18): add_missing_columns(), block_claimed(), claim_task(), claim_with(), ClaimedTask, complete_claimed(), fail_claimed(), finish_attempt() (+10 more)

### Community 145 - "run_live"
Cohesion: 0.29
Nodes (6): LiveRun, Fresh, Latest, Run, run_live(), RunInput

### Community 146 - "RecordKind"
Cohesion: 0.29
Nodes (6): RecordKind, Claim, DerivedSummary, Memory, Passage, ToolObservation

### Community 148 - "ClaimRelation"
Cohesion: 0.29
Nodes (7): ClaimRelation, AmbiguousEntity, ChangedQuantity, Equivalent, Negation, PlannedVsCompleted, Unrelated

### Community 150 - "Store"
Cohesion: 0.53
Nodes (5): charge_newsapi(), charge_quota(), open_key(), quota_open(), utc_day()

### Community 153 - "split_vertical"
Cohesion: 0.10
Nodes (31): api_key_slot(), ApiKeySlot, auth_areas(), brain_form(), brain_hit(), brain_list(), BrainForm, BrainList (+23 more)

### Community 154 - "§9 implementation order"
Cohesion: 0.15
Nodes (12): §10 acceptance checks, §9 implementation order, Atlas memory completion and System apps — implementation checklist, Phase 1 — what landed, Phase 2 — what landed, Phase 3 — what landed, Phase 4 — what landed, Phase 5a — what landed (+4 more)

### Community 155 - "TurnContinuation"
Cohesion: 0.50
Nodes (4): TurnContinuation, Active, Background, Exhausted

### Community 164 - "What Was Learned"
Cohesion: 0.15
Nodes (12): CLI Entry Points, Codebase Structure, Investigation Flow, Model Roles (configured independently in Providers → Defaults), Next Commands, Onboarding Summary — argos-osint, Planning Artifacts Created, Primary Providers (+4 more)

### Community 167 - ".new"
Cohesion: 0.10
Nodes (45): Account, account_hits(), account_platforms_attach_to_the_subject_and_are_never_entities(), action_order(), actions_are_grounded_capped_and_not_a_sweep(), adaptive_expansion_spends_one_scarce_lookup_then_reranks(), adaptive_step(), additional_tools() (+37 more)

### Community 168 - "Milestone 1 — Core Onboarding (Current)"
Cohesion: 0.18
Nodes (10): Deliverables, Dependencies, Future Milestones (Planned), Milestone 1 — Core Onboarding (Current), Milestone 2 — CLI Verification, Milestone 3 — Test Coverage, Milestone 4 — Integration & Ship, Objectives (+2 more)

### Community 169 - "paths.rs"
Cohesion: 0.31
Nodes (8): write_cache(), auth_path(), config_path(), db_path(), ensure_home(), hardware_cache_path(), home_dir(), lancedb_dir()

### Community 170 - "article"
Cohesion: 0.29
Nodes (8): admiralty_scales_claim_confidence_from_rsp_and_peers(), article(), kept(), lead_claims_scale_with_the_gate_and_context_is_not_capped_at_five(), peer_decisions_offer_candidate_ids_or_none(), peer_decisions_request(), the_highest_confidence_claims_survive_a_budget(), the_packet_keeps_four_and_reports_the_full_gate()

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

### Community 180 - "GraphNodeKind"
Cohesion: 0.20
Nodes (9): glyph(), GraphNodeKind, Directive, Entity, Evidence, Finding, Investigation, Source (+1 more)

### Community 182 - "Docs Ingest — argos-osint"
Cohesion: 0.33
Nodes (5): `docs/architecture.md`, Docs Ingest — argos-osint, `docs/providers.md`, Ingest Status, Ingested Documentation

### Community 183 - "GraphEdgeKind"
Cohesion: 0.25
Nodes (7): GraphEdgeKind, Answers, Concerns, DerivedFrom, DiscoveredIn, Investigates, Supports

### Community 185 - "OpenCode V2 workflow"
Cohesion: 0.40
Nodes (4): Focused ECC commands, OpenCode V2 workflow, Start with the graph, Verify setup

### Community 186 - "Unified reliability / summarization / semantic pipelines — completion checklist"
Cohesion: 0.40
Nodes (4): Honest remaining gaps, Phase status (§18), This follow-up, Unified reliability / summarization / semantic pipelines — completion checklist

### Community 206 - "H0-H2 Completion Summary"
Cohesion: 0.33
Nodes (5): H0-H2 Completion Summary, H0: Resolve Harness and Current Phase ✓, H1: Map Feature Seams ✓, H2: Freeze Contracts and Packets ✓, Next Step: H3 - State and Persistence Foundation (Build Agent)

### Community 207 - "LaunchState"
Cohesion: 0.40
Nodes (5): LaunchState, Accepted, Accepting, Editable, RecoverableFailure

### Community 209 - "Value"
Cohesion: 0.18
Nodes (15): ac7_the_relevance_gate_drops_an_off_topic_article(), action(), binding_extraction_drops_values_missing_from_the_observation(), bound(), court_rows(), firecrawl_weak(), generic(), news_rows() (+7 more)

### Community 210 - "BrainListMode"
Cohesion: 0.50
Nodes (4): BrainListMode, Create, Graph, List

## Knowledge Gaps
- **1248 isolated node(s):** `$schema`, `model`, `small_model`, `default_agent`, `mode` (+1243 more)
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 1682 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **25 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `App` connect `App` to `app.rs`, `ui.rs`, `.set_focus`, `Store`, `Call`, `hardware.rs`, `Store`, `provider.rs`, `brain_detail.rs`, `recon/graph.rs`, `run_atlas_inner`, `.on_work_event`, `SettingsFile`, `ReportMode`, `AtlasArticleRow`, `WorkEvent`, `now`, `worker.rs`, `ProviderPage`, `src/brain.rs`, `anyhow`, `LaunchState`, `LogsView`, `tui/jobs.rs`, `BrainListMode`, `FeedArticle`, `DefaultsRole`, `.handle_key`, `summary_card.rs`, `ModuleId`, `ToolResult`, `briefing_view.rs`, `Overlay`?**
  _High betweenness centrality (0.105) - this node is a cross-community bridge._
- **What connects `$schema`, `model`, `small_model` to the rest of the system?**
  _1248 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `land.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.0076045627376425855 - nodes in this community are weakly interconnected._
- **Why does `ProviderSecret` connect `ProviderSecret` to `App`, `atlas_memory.rs`, `accept_claims`, `recon.rs`, `Store`, `run_live`, `TurnEvent`, `execute_steps`, `provider.rs`, `run_turn`, `body.rs`, `atlas.rs`, `run_atlas_inner`, `scheduler.rs`, `provider_attempt.rs`, `SettingsFile`, `picker.rs`, `ReportMode`, `tests.rs`, `AtlasArticleRow`, `exec.rs`, `.is_empty`, `worker.rs`, `intel_recon/jobs.rs`, `atlas_insights.rs`, `body_filter.rs`, `synthesize.rs`, `anyhow`, `summarization.rs`, `graph_explanation.rs`, `replace_insights.rs`, `all_tools`, `ToolResult`, `KeptClaim`, `extract`, `subscription.rs`, `chat_model`, `poll_device`?**
  _High betweenness centrality (0.051) - this node is a cross-community bridge._
- **Should `orchestrate.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.05660377358490566 - nodes in this community are weakly interconnected._
- **Why does `ButtonId` connect `ButtonId` to `App`, `summary_card.rs`, `app.rs`, `.set_focus`, `tui/jobs.rs`, `DefaultsRole`, `logs.rs`, `split_vertical`, `Rect`?**
  _High betweenness centrality (0.044) - this node is a cross-community bridge._
- **Should `App` be split into smaller, more focused modules?**
  _Cohesion score 0.047332185886402756 - nodes in this community are weakly interconnected._