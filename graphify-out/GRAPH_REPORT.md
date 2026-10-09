# Graph Report - argos-osint  (2026-10-08)

## Corpus Check
- 172 files · ~394,496 words
- Verdict: corpus is large enough that graph structure adds value.
- Unclassified: 9 file(s) not represented in the graph (top: (none) 3, .toml 2, .diff 2)

## Summary
- 6236 nodes · 15439 edges · 247 communities (222 shown, 25 thin omitted)
- Extraction: 98% EXTRACTED · 2% INFERRED · 0% AMBIGUOUS · INFERRED: 283 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `c27cd6cd`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- land.rs
- orchestrate.rs
- Binding
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
- run
- directives.rs
- .set_focus
- absorb_hit
- providers.rs
- atlas_news.rs
- Store
- publication.rs
- gates.rs
- ToolResult
- news_legal.rs
- hardware.rs
- App
- Store
- embed.rs
- osint.rs
- execute_steps
- .handle_key
- provider.rs
- .on_work_event
- InvestigationPart
- body.rs
- ChatMessage
- brain_detail.rs
- recon/graph.rs
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
- reliability_faults.rs
- Request
- .activate_button
- events.rs
- exec.rs
- Line
- ProviderSecret
- ErrorCategory
- grok_oauth.rs
- MemoryKind
- Rect
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
- dataset.rs
- draw
- contains
- theme.rs
- Overlay
- pipeline.rs
- summarization.rs
- InvestigationSurface
- LogsView
- check_single_site
- tui/jobs.rs
- .new
- FeedArticle
- budget.rs
- store.rs
- Command
- DefaultsRole
- logs.rs
- src/evidence.rs
- seeded
- .evaluate
- AtlasArticleRow
- F
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
- all_tools
- .order
- anyhow
- run_turn
- Architecture
- WorkEvent
- .add_memory
- ClockSet
- ProviderAdmission
- Phase Plan: Home Recon Composer and Investigation Tabs
- BrainResourceSummary
- gsd-v2.js
- now
- briefing_view.rs
- subscription.rs
- extract
- IndexOutcome
- chat_model
- tasks.rs
- ReconCommand
- JobStatusFilter
- RouteInput
- modes.rs
- Value
- TurnClock
- agent
- Functional Requirements
- DecisionAdapterKind
- super
- ui.rs
- TaskState
- OsintCommand
- .default
- InvestigationEvent
- Argos OSINT — Agent Instructions
- replace_insights.rs
- Store
- wikipedia_rsp.rs
- rusqlite
- serde_json
- TurnEvent
- secrets.rs
- table_lines
- QueryExecutionStatus
- results.rs
- Store
- ledger.rs
- QuestionSpec
- split_vertical
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
- the_label_updates_when_a_round_adds_calls_and_when_evidence_is_measured
- Codebase Map — argos-osint
- PROJECT.md — argos-osint
- Current Phase State
- ReconLimits
- RecoveryAction
- refresh_run_indexing
- JobsView
- Conventions
- Docs Ingest — argos-osint
- poll_device
- apply_peer_support
- OpenCode V2 workflow
- Unified reliability / summarization / semantic pipelines — completion checklist
- Concepts
- package.json
- explore.rs
- App
- .new
- Providers
- Decision roles and output contracts
- H0-H2 Completion Summary
- Terminal UI
- PlanInterval
- TaskStatus
- Value
- CycleOutcome
- json
- RecordKind
- intel_recon/brain.rs
- Unified Investigation Harness
- ClaimRelation
- draw_overlay
- TurnContinuation
- Argos OSINT
- SettingsFile
- Severity
- Argos documentation
- Diagram conventions
- finish_insights
- Argos UI Interaction Audit
- ChainReport<T>
- run_live
- DecisionContract
- RspStatus
- JobRow
- RspEntry
- AtlasInsightClaim
- RspIndex
- RepairStatus
- StepState
- SourceReliability
- atlas_actions.rs
- OriginStat
- elon_plan
- LevelFilter
- merge_aliases
- QueryCompatibility
- .job_source
- BrainListMode

## God Nodes (most connected - your core abstractions)
1. `App` - 303 edges
2. `ProviderSecret` - 139 edges
3. `ButtonId` - 128 edges
4. `FieldId` - 72 edges
5. `Store` - 70 edges
6. `Target` - 65 edges
7. `ToolResult` - 62 edges
8. `Store` - 61 edges
9. `AtlasArticleRow` - 55 edges
10. `ReportMode` - 47 edges

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
- 2-file cycle: `crates/argos-osint-core/src/osint.rs -> crates/argos-osint-core/src/osint/results.rs -> crates/argos-osint-core/src/osint.rs`
- 2-file cycle: `crates/argos-osint-bin/src/cli.rs -> crates/argos-osint-core/src/hardware.rs -> crates/argos-osint-bin/src/cli.rs`

## Communities (247 total, 25 thin omitted)

### Community 0 - "land.rs"
Cohesion: 0.01
Nodes (262): CODES, LABELS, R0, R1, R10, R100, R101, R102 (+254 more)

### Community 1 - "orchestrate.rs"
Cohesion: 0.06
Nodes (41): a_follow_up_keeps_names_from_the_previous_synthesis(), a_recon_model_429_trips_the_gate_and_later_calls_skip_the_provider(), ac3_no_key_reaches_inputs_cache_keys_plan_json_raw_bodies_or_logs(), ac6_a_courtlistener_429_still_lets_the_turn_synthesize(), ACCOUNT_TOOLS, ACME, action_call(), binding_extraction_drops_values_missing_from_the_observation() (+33 more)

### Community 2 - "Binding"
Cohesion: 0.15
Nodes (28): Binding, canonical(), dependency_order(), depends_on(), question_bindings_and_dependency_fix(), allowed_producer(), best_handle(), bind_step() (+20 more)

### Community 3 - "atlas_memory.rs"
Cohesion: 0.13
Nodes (37): atlas_memory_count(), Barrier, disk_store(), finished_run(), index_and_verify(), INDEXING_DISABLED_LINE, legacy_published(), legacy_receipt() (+29 more)

### Community 4 - "tool_io.rs"
Cohesion: 0.04
Nodes (94): normalize_platform(), question_bindings(), a_gap_filler_only_domain_waits_for_a_primary_observation_before_hunter(), accept_bindings(), batch_scrape_takes_ranked_urls_and_gates_order_before_hunter(), bind_arguments(), BINDING_KINDS, bitcoins_in() (+86 more)

### Community 5 - "app.rs"
Cohesion: 0.06
Nodes (74): a_click_folds_an_event_and_incoming_entries_do_not_move_a_paused_list(), a_finished_tool_call_is_logged_once_and_the_transcript_keeps_a_summary(), all_provider_actions_render_with_hit_areas_at_80x24(), ATLAS_AUTO_SECS, atlas_auto_toggle_persists_the_next_trigger(), atlas_claim(), atlas_headlines_scroll_and_request_failures_reach_the_system_log(), atlas_history_resume_is_offered_only_for_resumable_cycles_and_repair_starts_once() (+66 more)

### Community 6 - "brain_lance.rs"
Cohesion: 0.06
Nodes (38): activate_generation(), batch(), begin_generation(), begin_generation_then_activate(), block_on(), BrainIndex, clear_fingerprint(), current_fingerprint() (+30 more)

### Community 7 - "FieldId"
Cohesion: 0.04
Nodes (56): FieldId, BrainApp, BrainConversation, BrainInsight, BrainQuery, ClaimAssessorModel, ClaimAssessorProvider, ClassifierModel (+48 more)

### Community 8 - "map.rs"
Cohesion: 0.05
Nodes (77): BRAILLE_MAP, canonical(), claim_label(), contains(), country_at(), country_fit(), country_pixel(), default_world_scale_is_enlarged_and_fits_small_terminals() (+69 more)

### Community 9 - "recon.rs"
Cohesion: 0.05
Nodes (66): ac6_citation_groups_split_validate_each_id_and_normalize(), answer_step_rate_limit_is_explicit_and_keeps_results(), attribute_word(), auxiliary(), BRIEF_SYNTHESIS, broad_question_collapses_to_grounded_lookups(), cacheable(), char_ceil() (+58 more)

### Community 10 - "ButtonId"
Cohesion: 0.02
Nodes (104): ButtonId, Add, AddFallback, AtlasAuto, AtlasDelete, AtlasLive, AtlasNews, AtlasNewsFeed (+96 more)

### Community 11 - "Store"
Cohesion: 0.08
Nodes (4): CreditHold, Store, strategy_and_provider_credits_survive_reopen(), Thread

### Community 12 - "IntelligenceCategory"
Cohesion: 0.07
Nodes (32): cache_follows_the_provider_plan_interval(), all_61_tools_mapped_to_categories(), argument_contract(), ArgumentBuilderContract, compact_capability_catalog(), CompactToolCapability, IntelligenceCategory, Bitcoin (+24 more)

### Community 13 - "job_registry.rs"
Cohesion: 0.05
Nodes (49): begin(), begin_cancellable(), db(), db_path(), finish(), register_canonical(), beat(), BEAT_INTERVAL (+41 more)

### Community 14 - "run"
Cohesion: 0.08
Nodes (20): atlas_auto_future_tick_waits_and_past_tick_reschedules(), atlas_auto_while_running_only_arms_the_next_slot(), atlas_countdown_visible(), atlas_extracting_visible(), atlas_history_button_counts_down_while_auto_run_is_on(), atlas_poll_wait(), flush_streams(), intel_body_loading_visible() (+12 more)

### Community 15 - "directives.rs"
Cohesion: 0.06
Nodes (63): apply_context_targets(), asks(), asks_about_judge(), asks_for_headlines(), clip_words(), context_entity(), CONTEXT_FILLER, context_gate() (+55 more)

### Community 16 - ".set_focus"
Cohesion: 0.08
Nodes (14): backspace_after_a_sent_question_deletes_one_character(), draft_isolation_and_persistence(), duplicate_submission_prevention(), intel_opens_bulletin_filters_and_opens_briefing(), keyboard_navigation_esc_and_shortcuts(), multiline_paste_stays_in_composer_and_does_not_run_shortcuts(), pointer_and_chords_do_not_switch_apps_on_their_own(), recon_chat_folds_decisions_and_opens_synthesis_memory() (+6 more)

### Community 17 - "absorb_hit"
Cohesion: 0.12
Nodes (22): absorb_hit(), account_platform_host(), Candidate, content_tokens(), distinct_queries(), domain_label(), EntityIdentifier, first_domain() (+14 more)

### Community 18 - "providers.rs"
Cohesion: 0.05
Nodes (57): clip_page(), https_on_host(), profile_path_token(), assert_host_locked(), BATCH_SCRAPE_DEFAULT_URLS, BATCH_SCRAPE_MAX_URLS, batch_urls(), company_card() (+49 more)

### Community 19 - "atlas_news.rs"
Cohesion: 0.08
Nodes (44): api_error(), ATLAS_TOOLS, authors(), clip(), country_arg(), country_code(), CURRENTS_COUNTRIES, CURRENTS_HOST (+36 more)

### Community 20 - "Store"
Cohesion: 0.07
Nodes (17): article_body_from_row(), ArticleBodyRow, element_from_row(), IntelAssessmentRow, IntelEvidenceRow, IntelInvestigationRow, IntelReportSectionRow, IntelReportTaskRow (+9 more)

### Community 21 - "publication.rs"
Cohesion: 0.09
Nodes (43): active_index_rows(), bump_memories_changed(), change_watcher_sees_only_committed_changes_and_coalesces(), claim(), clear(), clear_index_state(), CoverageReport, delete_run_memory_state() (+35 more)

### Community 22 - "gates.rs"
Cohesion: 0.09
Nodes (12): execute_harness_step(), InvestigationRuntime, GateOutcome, Passed, Rejected, validate_claim_assessment(), validate_evidence_admission(), validate_publication() (+4 more)

### Community 23 - "ToolResult"
Cohesion: 0.17
Nodes (13): ToolResult, cut_footer(), cut_short_answer(), evidence_summary(), Message, credit_map(), evidence_notes(), settle_holds() (+5 more)

### Community 24 - "news_legal.rs"
Cohesion: 0.07
Nodes (45): a_rate_limit_switches_to_the_fallback_key_and_a_rejection_does_not(), ac1_each_request_has_its_url_params_and_auth_header_and_no_key_in_the_url(), an_echoed_key_is_redacted_from_the_stored_body_and_error(), article(), articles(), CATEGORIES, choice_arg(), context_kind() (+37 more)

### Community 25 - "hardware.rs"
Cohesion: 0.15
Nodes (18): System, CACHE_TTL_SECS, classify_arch(), disk_totals(), HardwareProfile, metal_vram_gb(), now_secs(), parse_apple_gpu_cores() (+10 more)

### Community 26 - "App"
Cohesion: 0.08
Nodes (65): active_popup_area(), api_key_slot(), atlas_cycle_stats_room(), atlas_cycle_stats_scroll_max(), atlas_feed_room(), atlas_feed_room_for(), atlas_live_areas(), atlas_news_room() (+57 more)

### Community 27 - "Store"
Cohesion: 0.06
Nodes (8): UnitManifest, atlas_article_from_row(), atlas_brief_id(), AtlasRunRow, GraphSummary, intel_link_explanations_round_trip_and_cleanup(), memory_row(), Store

### Community 28 - "embed.rs"
Cohesion: 0.08
Nodes (31): active(), DIM, disable(), disabled(), DisableGuard, download_file(), DOWNLOAD_TIMEOUT_SECS, embed_batch() (+23 more)

### Community 29 - "osint.rs"
Cohesion: 0.08
Nodes (47): a_claimed_email_keeps_no_person_data(), bind_request(), bitcoin(), bounded(), CACHE_DAY_SECONDS, CACHE_MONTH_SECONDS, cache_seconds(), CACHE_WEEK_SECONDS (+39 more)

### Community 30 - "execute_steps"
Cohesion: 0.14
Nodes (27): after_step(), binding_ground(), context_block(), context_dispatched(), directive_for(), execute_steps(), expand_dork_children(), expand_per_platform() (+19 more)

### Community 31 - ".handle_key"
Cohesion: 0.10
Nodes (13): ctrl_arrows_cycle_app_tabs_forward_and_back(), ctrl_tab_cycles_app_tabs_forward_and_back(), IntelReconFocus, Section, Start, Tab, is_picker_field(), PaletteItem (+5 more)

### Community 32 - "provider.rs"
Cohesion: 0.05
Nodes (58): openrouter_ready(), account_secret(), active_text_secret(), authorize(), bearer_token(), catalog_error(), concrete_free_models(), decide() (+50 more)

### Community 33 - ".on_work_event"
Cohesion: 0.08
Nodes (7): a_failed_turn_keeps_the_streamed_answer_on_screen(), a_log_or_late_tool_row_does_not_drop_the_live_answer(), atlas_log_level(), brain_anchors_follow_memory_focus_and_scroll_stops_at_ends(), summary_system(), synthesis_deltas_fill_the_live_bubble_and_the_saved_answer_replaces_them(), RelatedRow

### Community 34 - "InvestigationPart"
Cohesion: 0.12
Nodes (15): InvestigationPart, DirectiveAssessment, EvidencePassage, GateValidation, Handoff, Plan, PlanDiagnostics, RoleDecision (+7 more)

### Community 35 - "body.rs"
Cohesion: 0.08
Nodes (35): body_fetch_routes(), BodyFetchEvent, Attempt, Failed, InsightsFailed, InsightsRefreshing, InsightsReplaced, Progress (+27 more)

### Community 36 - "ChatMessage"
Cohesion: 0.12
Nodes (25): chat_body(), ChatMessage, complete(), complete_errors_with_finish_reason_when_response_is_empty(), complete_once(), complete_one(), complete_reads_reasoning_only_non_stream_json(), complete_stream_of_reasoning_deltas_yields_text() (+17 more)

### Community 37 - "brain_detail.rs"
Cohesion: 0.06
Nodes (49): areas(), BrainDetail, DetailAreas, DetailPane, Path, Related, Summary, DetailSnapshot (+41 more)

### Community 38 - "recon/graph.rs"
Cohesion: 0.06
Nodes (60): draw(), draw_path(), draw_summary(), glyph(), inset(), legend_height(), legend_parts(), legend_rows() (+52 more)

### Community 39 - "atlas.rs"
Cohesion: 0.08
Nodes (32): article_card_labels_title_publisher_author_and_classification(), Band, band_sizes(), band_temperature(), category_from_decisions(), CATEGORY_IDS, CLUSTERS, country_label() (+24 more)

### Community 40 - "run_atlas_inner"
Cohesion: 0.09
Nodes (33): AtlasEvent, Classified, InsightProgress, MemoriesChanged, MemoryProgress, Note, Replaced, Stats (+25 more)

### Community 41 - "whatsmyname.rs"
Cohesion: 0.06
Nodes (40): D, AccountTuple, ADAPTER_VERSION, benchmark_14_4_parse_index_and_selection(), CACHE_TTL, CompiledSite, CoverageSummary, DATASET_NAME (+32 more)

### Community 42 - "scheduler.rs"
Cohesion: 0.09
Nodes (22): apply_index_change(), apply_index_change_rebuild_reports_outcome(), apply_index_with(), DEFAULT_LEASE_SECS, drain_index_once(), drain_summary_flush(), drain_summary_flush_cached(), drain_summary_flush_completes_cached_tasks() (+14 more)

### Community 43 - "jobs_view.rs"
Cohesion: 0.11
Nodes (21): summary_text(), active_time_is_live_for_open_attempts_and_unavailable_only_without_a_start(), AttemptRow, event_apps(), event_counts(), EventCounts, get_job(), job() (+13 more)

### Community 44 - "provider_attempt.rs"
Cohesion: 0.10
Nodes (37): Acc, attempt(), AttemptReport, check_final(), complete_stream_and_json_answers_succeed(), Deadlines, delta(), fill() (+29 more)

### Community 45 - "LogicalRole"
Cohesion: 0.13
Nodes (11): LogicalRole, .ALL, ClaimAssessor, Classifier, Controller, EntityResolver, EvidenceCurator, Planner (+3 more)

### Community 46 - "picker.rs"
Cohesion: 0.11
Nodes (27): brain_scrape_options_hide_bare_scrape_and_include_claim_criteria(), CatalogEntry, chat_request(), CONFIDENCE_FLOOR, decisions_request(), directives_phrase(), DONE, eligible_catalog() (+19 more)

### Community 47 - "ReportMode"
Cohesion: 0.12
Nodes (22): HomeDraftState, classifiable_modes(), classify_mode_chat(), classify_mode_decisions(), classify_prompt_mode(), classify_prompt_mode_chat(), classify_prompt_mode_decisions(), classify_recon_mode() (+14 more)

### Community 48 - "provider_chain.rs"
Cohesion: 0.13
Nodes (26): AttemptRecord, cancellation_stops_the_chain(), cancelled(), ChainReport, DispatchError, execute(), ExecuteOptions, FALLBACK_ATTEMPTS (+18 more)

### Community 49 - "reliability_faults.rs"
Cohesion: 0.08
Nodes (15): AttemptOutcome, categorize_http(), execute_with_retries(), other_llm_allows_three(), summarization_stops_at_two_attempts(), TimeoutProfile, .CLASSIFIER, .EXTRACTION (+7 more)

### Community 50 - "Request"
Cohesion: 0.17
Nodes (22): annotate(), domain(), email_address(), get(), hunter_observations(), job_status_url(), key_schema(), no_results() (+14 more)

### Community 51 - ".activate_button"
Cohesion: 0.07
Nodes (9): intel_day_button_label(), open_external_url(), ProviderPage, .ALL, Defaults, Google, Nvidia, OpenRouter (+1 more)

### Community 52 - "events.rs"
Cohesion: 0.14
Nodes (17): clear_events(), DEFAULT_RETENTION_HOURS, get_event(), job_filter_includes_descendants_and_prune_keeps_jobs(), list_events(), MAX_DETAIL_CHARS, MAX_MESSAGE_CHARS, mem() (+9 more)

### Community 53 - "exec.rs"
Cohesion: 0.14
Nodes (20): admission_account(), admission_contention_consumes_no_attempt(), AttemptEvent, Finished, Started, AttemptLog, cancelled(), complete_summary_report() (+12 more)

### Community 54 - "Line"
Cohesion: 0.12
Nodes (23): abs_contains(), abs_rect(), extracted_actors_markdown(), extracted_claims_markdown(), extracted_context_markdown(), extracted_inferences_markdown(), extracted_links_markdown(), intel_brief_full_lines() (+15 more)

### Community 55 - "ProviderSecret"
Cohesion: 0.22
Nodes (18): call_cached(), cancelled(), derive_directives(), derived_note(), extract_bindings(), model_bindings(), model_json(), model_queries() (+10 more)

### Community 56 - "ErrorCategory"
Cohesion: 0.08
Nodes (24): operation_kind(), backoff_delay(), can_retry(), ErrorCategory, AuthOrQuota, CancelledOrStale, ConfigurationMissing, ContextLimit (+16 more)

### Community 57 - "grok_oauth.rs"
Cohesion: 0.15
Nodes (20): auth_path(), bearer(), bearer_from_path(), check_login(), Entry, entry_from(), failed_login_keeps_the_cli_reason_and_recovery_action(), login() (+12 more)

### Community 58 - "MemoryKind"
Cohesion: 0.09
Nodes (22): AdmissionPolicy, admit_memory(), BrainQuery, pack_context(), AssessmentState, Contradicts, Insufficient, Mentions (+14 more)

### Community 59 - "Rect"
Cohesion: 0.15
Nodes (49): Block, draw(), AbsRect, button_areas(), cursor_at(), draw_add_fallback(), draw_atlas(), draw_atlas_cycle_stats() (+41 more)

### Community 60 - "brain_resources.rs"
Cohesion: 0.10
Nodes (22): source_anchor_label(), BRAIN_SCRAPE_PREFIX, CLAIM_CHARS, classify_resource(), file_link_url(), hit(), is_brain_binding(), is_brain_scrape_pick() (+14 more)

### Community 61 - "worker.rs"
Cohesion: 0.14
Nodes (20): IntelReportEvent, InsightsUpdated, JobCreated, JobDone, Section, Stage, IntelReportJobRow, cancel_if_needed() (+12 more)

### Community 62 - "Target"
Cohesion: 0.08
Nodes (27): brain_article_source_opens_intel_brief(), FocusEntry, hit(), LayoutRegistry, Target, App, AtlasCycleStats, BrainMark (+19 more)

### Community 63 - "src/brain.rs"
Cohesion: 0.14
Nodes (19): AGREEMENT_WEIGHT, CATEGORIES, category_hint(), format_injection(), hybrid_recall(), hybrid_recall_adds_paraphrases_and_rewards_agreement(), identity_query_prefers_name_memory(), jaccard() (+11 more)

### Community 64 - "intel_recon/jobs.rs"
Cohesion: 0.15
Nodes (18): active_job_for_mode(), cancel_job(), create_job_inserts_all_verify_sections(), create_report_job(), finish_canonical(), intel_recon_links_its_legacy_job_to_one_canonical_registry_job(), pause_job(), ReportScope (+10 more)

### Community 65 - "cli.rs"
Cohesion: 0.18
Nodes (18): ask(), ask_thread(), atlas_command(), defaults_command(), defaults_json(), defaults_show_includes_the_tool_picker_transport(), dispatch(), login() (+10 more)

### Community 66 - "atlas_insights.rs"
Cohesion: 0.09
Nodes (26): ask_claims(), AskedClaims, BODY_CLAIM_LIMIT, BODY_SPAN_CHARS, claim_json_tolerates_empty_and_alternate_shapes(), COL_CLASS, COL_ENTITY, COL_OBJECT (+18 more)

### Community 67 - "InformationCredibility"
Cohesion: 0.13
Nodes (17): AdmiraltyCode, best_credibility(), CredibilityInputs, information_credibility(), InformationCredibility, CannotBeJudged, Confirmed, DoubtfullyTrue (+9 more)

### Community 68 - "Category"
Cohesion: 0.05
Nodes (46): bounded(), Category, Auth, Cancelled, Configuration, Empty, InvalidModel, InvalidResult (+38 more)

### Community 69 - "body_filter.rs"
Cohesion: 0.17
Nodes (22): BATCH_SIZE, BodyChunk, chunk_article_body(), CHUNK_MAX, CHUNK_MIN, CHUNK_TARGET, chunks_preserve_absolute_offsets(), classify_chunk_batch() (+14 more)

### Community 70 - "synthesize.rs"
Cohesion: 0.12
Nodes (25): BODY_REFINE_INPUT_CHARS, deterministic_bluf_mentions_title(), deterministic_section(), model_refine_body(), model_synthesize(), parse_json_object(), refine_retrieved_article_body(), refine_without_secret_keeps_validated_scrape() (+17 more)

### Community 71 - "dataset.rs"
Cohesion: 0.17
Nodes (19): acquire_refresh_lease(), active_manifest_path(), ACTIVE_REFRESHES, dataset_root(), DatasetManifest, DatasetStatus, get_status(), import_from_file() (+11 more)

### Community 72 - "draw"
Cohesion: 0.09
Nodes (36): center_row(), Chrome, composer_height(), composer_parts(), cursor_blink_visible(), draw(), draw_composer(), draw_header() (+28 more)

### Community 73 - "contains"
Cohesion: 0.15
Nodes (22): atlas_hit(), atlas_news_areas(), atlas_row_at(), contains(), draw_tab_strip(), focus_order(), hit_test(), in_pane() (+14 more)

### Community 74 - "theme.rs"
Cohesion: 0.05
Nodes (55): border_style(), borders_and_cells_have_matching_display_widths(), bottom_border_line(), char_width(), clip_to_width(), column_widths(), columns_consume_inner_width(), columns_distribute_proportionally_when_not_flex_last() (+47 more)

### Community 75 - "Overlay"
Cohesion: 0.15
Nodes (13): ChoiceKind, IntelDay, Investigation, Model, Provider, Overlay, AddFallback, Choice (+5 more)

### Community 76 - "pipeline.rs"
Cohesion: 0.18
Nodes (19): apply_recon_directive_coverage(), compare_claims(), coverage_requires_cited_evidence_not_similarity(), directive_coverage(), DirectiveCoverage, event_grouping_keeps_separate_days_apart(), EventGroup, group_atlas_events() (+11 more)

### Community 77 - "summarization.rs"
Cohesion: 0.10
Nodes (37): cache_get(), cache_key_changes_with_revision_and_focus(), cache_put(), cache_round_trip(), complete_summary(), CoverageMeta, deterministic_article_description(), deterministic_atlas_brief() (+29 more)

### Community 78 - "InvestigationSurface"
Cohesion: 0.15
Nodes (9): InvestigationSurface, HomeComposer, IntelBrief, JobsResume, ReconChat, atlas_only_tools_blocked_on_all_surfaces(), check_need_gate(), sociavault_blocked_on_intel_surface() (+1 more)

### Community 79 - "LogsView"
Cohesion: 0.16
Nodes (5): LogsView, row_lines(), detail_rows(), event_row(), EventRow

### Community 80 - "check_single_site"
Cohesion: 0.12
Nodes (13): ACTIVE_SNAPSHOT, apply_strip_bad_char(), cache_get(), cache_set(), check_ip_safe(), check_single_site(), HOST_PACER, HostPacer (+5 more)

### Community 81 - "tui/jobs.rs"
Cohesion: 0.16
Nodes (13): areas(), BASE_COLUMNS, hit(), JobsAreas, LOGS_COLUMN, MIN_TITLE, PAGE, PHASE_COLUMN (+5 more)

### Community 82 - ".new"
Cohesion: 0.26
Nodes (18): a_spent_primary_quota_uses_the_fallback_key(), country_labels_show_the_name_and_the_code(), daily_cap(), format_run_card(), HttpCall, HttpReply, insights_do_not_call_a_decisions_model(), keys() (+10 more)

### Community 83 - "FeedArticle"
Cohesion: 0.16
Nodes (20): apply_hits(), article_from(), article_from_row(), article_row(), Article, canonical_url(), dedup_drops_a_url_an_exact_title_and_keeps_the_higher_domain(), FeedArticle (+12 more)

### Community 84 - "budget.rs"
Cohesion: 0.11
Nodes (24): courtlistener_spacing_and_firecrawl_polling_are_counted(), CUT_NOTE, CUT_SHORT, deadline_seconds(), live(), more_calls_mean_more_tool_time_and_cache_hits_add_nothing(), PHASE_WINDOW_SECONDS, RECON_DEADLINE (+16 more)

### Community 85 - "store.rs"
Cohesion: 0.10
Nodes (20): article(), atlas_articles_for_intel_dedupes_same_article_id_across_runs(), atlas_articles_for_intel_filters_category_day_and_query(), atlas_auto_next_round_trips_through_app_state(), atlas_claims_for_article_returns_linked_claims(), atlas_prune_drops_finished_runs_older_than_the_cutoff(), atlas_recent_articles_lists_retained_rows(), atlas_run_days_lists_distinct_days_newest_first() (+12 more)

### Community 86 - "Command"
Cohesion: 0.09
Nodes (23): AtlasCommand, Repair, Resume, Verify, Cli, Command, Atlas, Defaults (+15 more)

### Community 87 - "DefaultsRole"
Cohesion: 0.07
Nodes (16): ChoiceItem, codex_models(), DefaultsRole, .ALL, ClaimAssessor, Classifier, EntityResolver, EvidenceCurator (+8 more)

### Community 88 - "logs.rs"
Cohesion: 0.22
Nodes (13): areas(), button_label(), buttons(), count(), draw(), hit(), in_list(), list_geometry() (+5 more)

### Community 89 - "src/evidence.rs"
Cohesion: 0.22
Nodes (14): AGREEMENT_WEIGHT, chunk_text(), chunks_cover_end_of_long_source(), content_hash(), ensure_identifier_coverage(), EvidencePassage, hybrid_bounds_and_prefers_agreement(), hybrid_passage_candidates() (+6 more)

### Community 90 - "seeded"
Cohesion: 0.45
Nodes (20): background_indexing_then_refresh_upgrades_a_partial_run(), claims(), clear(), crash_after_checkpoint_resumes_without_reextracting(), cycle_jobs(), disabled_embeddings_save_memories_and_say_so_honestly(), embedding_failure_keeps_memories_visible_reports_partial_and_retry_verifies(), enabling_embeddings_resumes_outstanding_work_and_upgrades_the_run() (+12 more)

### Community 91 - ".evaluate"
Cohesion: 0.14
Nodes (13): DecisionService, DecisionPolicy, EnforcementOutcome, Pass, Reject, Unavailable, Uncertain, ThresholdProfile (+5 more)

### Community 92 - "AtlasArticleRow"
Cohesion: 0.26
Nodes (14): article_with_body_spans(), body_lead_prompt(), catalog_json(), classify_peers_chat(), classify_peers_decisions(), classify_relevant_articles(), clip_chars(), extract_for_article_body() (+6 more)

### Community 93 - "F"
Cohesion: 0.17
Nodes (17): Fault, CallSpec, classification_request(), dispatch(), fetch_with_spare_key(), json_message(), one_line(), parse_fault() (+9 more)

### Community 94 - "dork_generator.rs"
Cohesion: 0.11
Nodes (31): ACTIVE_SNAPSHOT, compose_single_query(), compute_query_id(), compute_template_id(), DATASET_NAME, DEFAULT_MAX_QUERIES, DorkCatalog, DorkCategory (+23 more)

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
Cohesion: 0.35
Nodes (11): actions(), button_at(), button_cells(), button_row_area(), draw(), height(), is_card_button(), label() (+3 more)

### Community 99 - "actor_review.rs"
Cohesion: 0.23
Nodes (12): ActorReviewItem, ActorReviewResult, apply_reviewed_actors(), deterministic_review_actors(), is_meaningful_actor(), JUNK_ACTORS, model_review_actors(), parse_actor_review_response() (+4 more)

### Community 100 - "ModuleId"
Cohesion: 0.16
Nodes (15): ModuleId, .ALL, Atlas, Brain, Intel, Jobs, Logs, Osint (+7 more)

### Community 101 - "Region"
Cohesion: 0.10
Nodes (20): Region, AtlasFeed, AtlasInsights, AtlasNews, AtlasOrigins, AtlasRuns, Chat, Detail (+12 more)

### Community 102 - "graph_explanation.rs"
Cohesion: 0.13
Nodes (14): BASIC_HEADING, event(), explain(), ExplainOutcome, Failed, Saved, Superseded, ExplainReport (+6 more)

### Community 103 - "validate.rs"
Cohesion: 0.30
Nodes (11): paywall_and_snippet_fail_validation(), BLOCK_MARKERS, clean_markdown(), content_hash(), empty_is_unavailable(), looks_like_search_snippet(), paywall_is_unavailable(), short_complete_article_passes() (+3 more)

### Community 104 - "KeptClaim"
Cohesion: 0.17
Nodes (16): admiralty_scales_claim_confidence_from_rsp_and_peers(), apply_admiralty_evaluation(), article_information_credibility(), brief_text(), countries_differ(), dedupe_claims(), entity_path(), fingerprint() (+8 more)

### Community 105 - "all_tools"
Cohesion: 0.17
Nodes (28): a_429_stops_picker_calls_and_the_fallback_finishes_the_list(), a_dispatch_error_fails_the_step_and_the_loop_goes_on(), a_failed_email_finder_gets_one_fallback_and_no_third_picker_call(), a_fallback_with_no_bindings_still_runs_the_satisfied_steps_and_reaches_synthesis(), a_question_handle_fills_the_handle_steps_without_a_fallback(), all_tools(), an_attribute_question_names_the_person_and_drops_qa_hosts(), an_imperative_social_prompt_names_the_person_not_the_sentence() (+20 more)

### Community 106 - ".order"
Cohesion: 0.22
Nodes (10): checked_serves(), context_additions(), context_tools(), fallback_record(), OrderContext, Ordered, Picker<'a>, serves_for() (+2 more)

### Community 107 - "anyhow"
Cohesion: 0.17
Nodes (9): compile_general_model_prompt(), compile_native(), parse_native_response(), template_claim_relation(), test_acquisition_fixture_native_and_general_model(), test_invalid_probability_distribution_rejected(), test_prompt_injection_resistance(), test_tool_selection_template_compilation() (+1 more)

### Community 108 - "run_turn"
Cohesion: 0.17
Nodes (18): classify_turn_mode(), continue_turn(), enabled_tools(), execute_ordered(), missing_keys(), picker_secret(), picker_snapshot(), previous_synthesis() (+10 more)

### Community 109 - "Architecture"
Cohesion: 0.12
Nodes (17): Admiralty source evaluation (WP:RSP), Architecture, Atlas, Binder and executor, Brain recall, Directives, Intel reports, Investigation flow (+9 more)

### Community 110 - "WorkEvent"
Cohesion: 0.09
Nodes (20): WorkEvent, AnswerDelta, AnswerNote, AtlasDone, BrainRelatedExplained, CatalogDone, DatasetRefreshDone, DatasetRefreshProgress (+12 more)

### Community 111 - ".add_memory"
Cohesion: 0.19
Nodes (13): MemorySource, boost_with_passage_hybrid(), embedding_disabled_degrades_to_jaccard(), embedding_failure_mid_session_degrades_to_jaccard(), fingerprint_mismatch_rebuilds_and_drift_is_reconciled(), like_needle(), local_sqlite_vec_v17_database_is_repaired_on_open(), minilm_lance_upsert_then_search_returns_same_id() (+5 more)

### Community 112 - "ClockSet"
Cohesion: 0.21
Nodes (3): ClockSet, queue_time_does_not_spend_active_but_consumes_wall(), sequential_vs_overlapping_tool_budgets()

### Community 113 - "ProviderAdmission"
Cohesion: 0.18
Nodes (6): shared_rate_limit_cooldown_blocks_admission(), AdmissionGuard, note_shared_rate_limit(), provider_admission_caps_at_two(), ProviderAdmission, rate_limit_cooldown_blocks_acquire()

### Community 114 - "Phase Plan: Home Recon Composer and Investigation Tabs"
Cohesion: 0.11
Nodes (17): 1. Outcome, 2. Home Layout (Section 4), 3. Home Composer (Section 5), 4. Submission & Transition (Section 6), 5. Persistent Investigation Tabs (Section 7), 6. Keyboard & Mouse Contract (Section 8), 7. Recovery & QoL (Section 9), Acceptance Criteria (Spec Section 12) (+9 more)

### Community 115 - "BrainResourceSummary"
Cohesion: 0.19
Nodes (10): bindings_use_brain_evidence_ids(), BrainResourceHit, BrainResourceSummary, candidate_json(), clip(), format_counts(), is_http_url(), parse_brain_scrape_index() (+2 more)

### Community 116 - "gsd-v2.js"
Cohesion: 0.12
Nodes (7): core, home, hooks, names, payload(), run(), setup()

### Community 117 - "now"
Cohesion: 0.22
Nodes (11): answer_evidence_survives_reopen_and_deleted_thread_rejects_late_calls(), claims_deduplicate_and_reject_unsupported_sources(), deleting_an_investigation_removes_its_brain_memories_and_graph_summary(), investigation_memory_ids(), now(), persistence_and_plan(), recon_claims_and_deletions_use_the_durable_index_outbox(), RunLimits (+3 more)

### Community 118 - "briefing_view.rs"
Cohesion: 0.23
Nodes (17): bucket_extracted(), bucket_extracted_with_explanations(), buckets_split_facts_inferences_context_and_links(), claim(), claim_label(), explain_intel_links_with_model(), explain_relation_link(), ExtractedBuckets (+9 more)

### Community 119 - "subscription.rs"
Cohesion: 0.25
Nodes (8): answer_event(), check_login(), command(), complete(), completion_is_ephemeral_and_does_not_inherit_tools_or_configuration(), exec_args(), is_subscription_status(), login()

### Community 120 - "extract"
Cohesion: 0.22
Nodes (9): a_decisions_model_does_not_extract_claims(), context_prompt(), dropped_from_receipt(), extract(), lead_prompt(), packet_diag(), packet_json(), recover_legacy_packet() (+1 more)

### Community 121 - "IndexOutcome"
Cohesion: 0.12
Nodes (7): ReindexReport, IndexOutcome, Disabled, Pending, PermanentFailure, Ready, RetryableFailure

### Community 122 - "chat_model"
Cohesion: 0.25
Nodes (21): a_dropped_provider_stream_keeps_the_text_already_received(), a_provider_that_rejects_streaming_fails_without_a_hidden_retry(), a_streamed_answer_with_a_bad_citation_ends_on_the_repaired_answer(), an_idle_stall_or_the_ceiling_keeps_the_partial_answer(), answer_text(), cancel_during_synthesis_marks_the_run_cancelled_and_keeps_partial_text(), cancel_during_tools_marks_the_run_cancelled(), chat_model() (+13 more)

### Community 123 - "tasks.rs"
Cohesion: 0.07
Nodes (71): lease_fencing_rejects_stale_epoch_and_foreign_owner(), add_missing_columns(), adopt_untracked_index_changes(), block_claimed(), claim_complete_and_retry_round_trip(), claim_index_changes(), claim_index_work(), claim_next() (+63 more)

### Community 124 - "ReconCommand"
Cohesion: 0.17
Nodes (12): ReconCommand, Ask, AskNew, Delete, Limits, List, New, Rename (+4 more)

### Community 125 - "JobStatusFilter"
Cohesion: 0.18
Nodes (8): JobFilter, JobStatusFilter, Active, All, Completed, Failed, Retrying, list_jobs()

### Community 126 - "RouteInput"
Cohesion: 0.13
Nodes (15): RouteInput, FacebookUrl, Handle, HandleOrUserId, Hashtag, LinkedinCompanyUrl, LinkedinProfileUrl, Query (+7 more)

### Community 127 - "modes.rs"
Cohesion: 0.27
Nodes (13): scoped_section_plan(), chat_response_spec(), chat_response_spec_embeds_section_guidelines_and_style_guide(), chat_response_spec_lists_mode_headings(), chat_section_titles(), every_mode_has_bluf_first_and_sources_last(), every_mode_section_has_guideline(), investigation_mode_spec() (+5 more)

### Community 128 - "Value"
Cohesion: 0.17
Nodes (19): a_keyword_inside_the_subjects_name_runs_no_news_or_legal_tool(), a_keyword_outside_the_subjects_name_still_runs_news_or_legal(), ac2_without_a_key_news_and_legal_tools_are_never_picked_and_with_a_key_they_are_eligible(), ac4_a_news_prompt_runs_newsapi_search_with_the_entity_and_who_is_runs_none(), ac5_a_sued_prompt_runs_courtlistener_case_or_docket_search_with_the_entity(), ac7_the_relevance_gate_drops_an_off_topic_article(), action(), bound() (+11 more)

### Community 130 - "agent"
Cohesion: 0.14
Nodes (13): agent, compaction, explore, mode, model, variant, default_agent, mode (+5 more)

### Community 131 - "Functional Requirements"
Cohesion: 0.12
Nodes (15): Atlas (News Pipeline), Brain / Memory, Configuration Requirements, Context Provider Keys, Functional Requirements, Non-Functional Requirements, Non-Goals, OSINT Manual Runs (+7 more)

### Community 132 - "DecisionAdapterKind"
Cohesion: 0.17
Nodes (14): DecisionAdapterKind, JsonMode, NativeDecisions, OutputTool, StrictJsonSchema, ValidatedText, parse_general_model_response(), resolve_adapter() (+6 more)

### Community 134 - "ui.rs"
Cohesion: 0.04
Nodes (82): ACTION_H, ApiKeySlot, atlas_auto_label(), atlas_countdown(), atlas_extracting(), atlas_history_live_label(), atlas_run_label(), atlas_source_anchors_are_not_labeled_deleted_origin() (+74 more)

### Community 135 - "TaskState"
Cohesion: 0.18
Nodes (9): TaskState, Cancelled, Completed, Failed, Paused, Queued, RetryScheduled, Running (+1 more)

### Community 136 - "OsintCommand"
Cohesion: 0.14
Nodes (14): DatasetCommand, Import, Refresh, Status, OsintCommand, Attach, Dataset, Describe (+6 more)

### Community 137 - ".default"
Cohesion: 0.13
Nodes (22): a_budgeted_call_with_a_dependency_from_an_earlier_batch_runs(), a_claimed_email_removes_its_bindings(), a_gap_filler_domain_feeds_hunter_once_firecrawl_observes_it(), a_goal_naming_a_tool_or_provider_is_repaired_then_falls_back(), a_goal_naming_a_tool_outside_the_prompt_entity_is_still_rejected(), a_handle_named_in_a_derived_question_is_an_unverified_binding(), a_prompt_entity_named_like_a_provider_keeps_the_models_directives(), a_weak_firecrawl_search_adds_one_google_search_with_the_same_query() (+14 more)

### Community 138 - "InvestigationEvent"
Cohesion: 0.18
Nodes (5): event_to_chat_block(), is_thinking_expanded(), InvestigationEvent, redact_secrets(), redacts_sensitive_keys()

### Community 139 - "Argos OSINT — Agent Instructions"
Cohesion: 0.13
Nodes (15): Architecture notes, Argos OSINT — Agent Instructions, CI (`.github/workflows/ci.yml`), CLI Entry Points (`argos` binary), Developer Commands, diagrams, Environment Variables, graphify (+7 more)

### Community 140 - "replace_insights.rs"
Cohesion: 0.31
Nodes (11): commit_refined_body(), article(), claim(), commit_replaces_atomically_and_keeps_shared_support(), failed_validation_leaves_prior_insights(), replace_article_insights_from_body(), ReplaceOutcome, snapshot_counts_are_stable_before_commit() (+3 more)

### Community 141 - "Store"
Cohesion: 0.11
Nodes (5): CallProposal, HandoffRecord, TaskRecord, ClaimAssessment, Store

### Community 142 - "wikipedia_rsp.rs"
Cohesion: 0.17
Nodes (14): API, APP_STATE_KEY, CACHE_TTL, index_to_json(), parse_last_year(), parse_rsp_wikitext(), parse_source_name(), parse_summary() (+6 more)

### Community 143 - "rusqlite"
Cohesion: 0.09
Nodes (19): Cached, None, Stale, Valid, Gate, CoolingDown, Ready, Running (+11 more)

### Community 144 - "serde_json"
Cohesion: 0.20
Nodes (4): DecisionState, EvidencePassageState, SubjectStateSummary, TaskStateSummary

### Community 145 - "TurnEvent"
Cohesion: 0.15
Nodes (15): AnswerContext, await_completion(), chat(), finish_recon_job(), investigation_jobs_finish_truthfully_and_link_their_run(), deltas(), Run, Streamed (+7 more)

### Community 146 - "secrets.rs"
Cohesion: 0.15
Nodes (5): accounts_persist_with_owner_only_permissions(), legacy_connection_migrates_without_overwriting_other_accounts(), loading_old_auth_removes_gmail_setup_credentials(), owner_only(), write_private()

### Community 147 - "table_lines"
Cohesion: 0.17
Nodes (8): detail_lines(), optional_columns_drop_instead_of_truncating_headers(), progress(), state_style(), table_lines(), TableColumns, title(), format_duration()

### Community 148 - "QueryExecutionStatus"
Cohesion: 0.18
Nodes (11): QueryExecutionStatus, Cancelled, Completed, Failed, Generated, NoResults, Queued, Running (+3 more)

### Community 149 - "results.rs"
Cohesion: 0.16
Nodes (16): classify_output_quality(), extract_observation_items(), extract_search_results(), extracts_from_array_shaped_observations(), extracts_from_object_with_results_array(), normalize_tool_result(), NormalizedToolResult, OutputQuality (+8 more)

### Community 150 - "Store"
Cohesion: 0.53
Nodes (5): charge_newsapi(), charge_quota(), open_key(), quota_open(), utc_day()

### Community 151 - "ledger.rs"
Cohesion: 0.14
Nodes (13): body_assertion_candidates(), coverage_complete(), ElementStatus, Assessed, Excluded, InProgress, Pending, Superseded (+5 more)

### Community 152 - "QuestionSpec"
Cohesion: 0.25
Nodes (5): DecisionQuestionType, Choice, Noul, Score, QuestionSpec

### Community 153 - "split_vertical"
Cohesion: 0.12
Nodes (26): add_fallback_layout(), brain_form(), brain_hit(), brain_list(), BrainForm, BrainList, chat_areas(), dashboard_areas() (+18 more)

### Community 154 - "§9 implementation order"
Cohesion: 0.15
Nodes (12): §10 acceptance checks, §9 implementation order, Atlas memory completion and System apps — implementation checklist, Phase 1 — what landed, Phase 2 — what landed, Phase 3 — what landed, Phase 4 — what landed, Phase 5a — what landed (+4 more)

### Community 155 - ".new"
Cohesion: 0.23
Nodes (5): a_round_keeps_its_time_and_a_tight_ceiling_still_runs_tools(), aging_the_tool_clock_blocks_new_calls_and_cache_hits_do_not_extend_it(), clock_set_for_turn(), foreground_expiry_continues_in_background(), sequential_raise_calls_budgets_higher()

### Community 156 - "investigation/evidence.rs"
Cohesion: 0.13
Nodes (18): assess_claim_against_passages(), ClaimAssessmentOutcome, ClaimStance, Disputed, Insufficient, Mention, Supported, curate_passages_from_result() (+10 more)

### Community 157 - "InvestigationPattern"
Cohesion: 0.10
Nodes (17): InvestigationPattern, ArticleVerification, Bitcoin, BreakingNews, DomainIp, EmailAttribution, FollowUp, GeneralSubject (+9 more)

### Community 164 - "What Was Learned"
Cohesion: 0.15
Nodes (12): CLI Entry Points, Codebase Structure, Investigation Flow, Model Roles (configured independently in Providers → Defaults), Next Commands, Onboarding Summary — argos-osint, Planning Artifacts Created, Primary Providers (+4 more)

### Community 165 - "AnnPolicy"
Cohesion: 0.17
Nodes (9): AnnMeasurement, AnnPolicy, AnnDeferredUnmeasured, AnnEnabled, ExactSearch, AnnThresholds, decide_ann_policy(), measure_ann_recall() (+1 more)

### Community 166 - "SiteOutcomeStatus"
Cohesion: 0.18
Nodes (10): deserialize_flexible_bool(), SiteOutcomeStatus, Ambiguous, Blocked, Error, Found, NotFound, RateLimited (+2 more)

### Community 167 - "investigation.rs"
Cohesion: 0.05
Nodes (79): Account, account_hits(), ACCOUNT_PLATFORMS, account_platforms_attach_to_the_subject_and_are_never_entities(), ACCOUNTS, accounts_flow(), actions_are_grounded_capped_and_not_a_sweep(), ADAPTIVE (+71 more)

### Community 168 - "Milestone 1 — Core Onboarding (Current)"
Cohesion: 0.18
Nodes (10): Deliverables, Dependencies, Future Milestones (Planned), Milestone 1 — Core Onboarding (Current), Milestone 2 — CLI Verification, Milestone 3 — Test Coverage, Milestone 4 — Integration & Ship, Objectives (+2 more)

### Community 169 - "paths.rs"
Cohesion: 0.30
Nodes (10): write_cache(), auth_path(), config_path(), dataset_dir(), datasets_dir(), db_path(), ensure_home(), hardware_cache_path() (+2 more)

### Community 170 - ".index_now"
Cohesion: 0.14
Nodes (6): ArticleInsightCommit, atlas_answer_id(), AtlasStoredClaim, insight_fingerprint(), new_id(), IndexEnqueue

### Community 171 - "atlas_work.rs"
Cohesion: 0.13
Nodes (16): articles_rev(), AttemptRecord, check_dependency_coverage(), DependencyCoverage, DISPOSITION_EMPTY, DISPOSITION_FAILED, DISPOSITION_INCOMPLETE, DISPOSITION_REJECTED (+8 more)

### Community 172 - "H2 Contracts: Frozen Interface Decisions"
Cohesion: 0.18
Nodes (10): 1. Home Draft Key, 2. Launch State Machine, 3. Persistence Boundary, 4. Tab Identity, 5. Navigation Intent, 6. State Restoration (Per-Tab), 7. Input Precedence Hierarchy, 8. Layout Measurements (Reference Points) (+2 more)

### Community 173 - "the_label_updates_when_a_round_adds_calls_and_when_evidence_is_measured"
Cohesion: 0.28
Nodes (3): format_deadline(), format_span(), the_label_updates_when_a_round_adds_calls_and_when_evidence_is_measured()

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

### Community 179 - "refresh_run_indexing"
Cohesion: 0.20
Nodes (8): CycleOutcome, Blocked, Completed, Failed, Partial, Waiting, MemoryPhase, refresh_run_indexing()

### Community 180 - "JobsView"
Cohesion: 0.27
Nodes (4): button_label(), buttons(), JobsView, reveal()

### Community 181 - "Conventions"
Cohesion: 0.22
Nodes (9): Agent scratch, Checks, Conventions, Diagrams, Graphify, Planning files, Schema, Tests (+1 more)

### Community 182 - "Docs Ingest — argos-osint"
Cohesion: 0.33
Nodes (5): `docs/architecture.md`, Docs Ingest — argos-osint, `docs/providers.md`, Ingest Status, Ingested Documentation

### Community 183 - "poll_device"
Cohesion: 0.47
Nodes (6): Poll, Denied, poll_device(), Pending, SlowDown, Token

### Community 184 - "apply_peer_support"
Cohesion: 0.18
Nodes (15): category_tag(), accept_one(), AcceptMode, Context, Lead, apply_peer_support(), classifier_peers_are_preferred_over_token_overlap(), contains_span() (+7 more)

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

### Community 202 - "App"
Cohesion: 0.05
Nodes (18): add_scroll(), App, AtlasPage, Live, Runs, IntelPage, Briefing, Bulletin (+10 more)

### Community 203 - ".new"
Cohesion: 0.12
Nodes (36): accounts_search_query(), action_order(), adaptive_step(), bitcoin_in(), clip_query(), derived_question_handles(), discovery_batch(), display_name() (+28 more)

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

### Community 209 - "PlanInterval"
Cohesion: 0.33
Nodes (5): PlanInterval, Daily, Monthly, Never, Weekly

### Community 210 - "TaskStatus"
Cohesion: 0.14
Nodes (11): TaskStatus, Cancelled, Completed, Deferred, Failed, Partial, Planned, Ready (+3 more)

### Community 211 - "Value"
Cohesion: 0.29
Nodes (10): clip_chars_ellipsis(), clip_long_strings(), compact_page(), compact_page_evidence(), compact_page_system(), long_page_evidence_compacts_to_a_summary_for_synthesis(), page_excerpt(), page_needs_compact() (+2 more)

### Community 212 - "CycleOutcome"
Cohesion: 0.18
Nodes (11): CycleOutcome, Blocked, Cancelled, Completed, CompletedWithWarnings, Failed, Partial, Pending (+3 more)

### Community 213 - "json"
Cohesion: 0.29
Nodes (3): box(), color(), render()

### Community 214 - "RecordKind"
Cohesion: 0.29
Nodes (6): RecordKind, Claim, DerivedSummary, Memory, Passage, ToolObservation

### Community 215 - "intel_recon/brain.rs"
Cohesion: 0.57
Nodes (5): intel_recon_publishes_durably_with_provenance_and_no_atlas_receipt(), ReconInsightUpdate, update(), upsert_adds_and_updates_claims(), upsert_recon_insights()

### Community 216 - "Unified Investigation Harness"
Cohesion: 0.33
Nodes (6): Catalog projection, Model roles, Persistence, Reasoning channels, Unified Investigation Harness, Validation gates

### Community 217 - "ClaimRelation"
Cohesion: 0.29
Nodes (7): ClaimRelation, AmbiguousEntity, ChangedQuantity, Equivalent, Negation, PlannedVsCompleted, Unrelated

### Community 218 - "draw_overlay"
Cohesion: 0.20
Nodes (14): add_fallback_popup_area(), atlas_run_card(), cover(), draw_choice(), draw_intel_recon_popup(), draw_overlay(), draw_palette(), intel_recon_popup_area() (+6 more)

### Community 219 - "TurnContinuation"
Cohesion: 0.50
Nodes (4): TurnContinuation, Active, Background, Exhausted

### Community 220 - "Argos OSINT"
Cohesion: 0.33
Nodes (6): Applications, Argos OSINT, Documentation, Figures, Limits, Quick start

### Community 221 - "SettingsFile"
Cohesion: 0.07
Nodes (28): a_blank_osint_user_agent_loads_as_unset(), default_credit_reset(), default_firecrawl_credits(), default_hunter_credits(), default_max_calls(), default_max_rounds(), default_one_cost(), default_opening_cap() (+20 more)

### Community 222 - "Severity"
Cohesion: 0.18
Nodes (8): session_event(), EventFilter, NewEvent, Severity, Debug, Error, Info, Warn

### Community 223 - "Argos documentation"
Cohesion: 0.40
Nodes (5): Agent and workflow, Argos documentation, Checklists (historical), Figures, Product

### Community 224 - "Diagram conventions"
Cohesion: 0.50
Nodes (4): Agent rule, Catalog, Diagram conventions, When to draw

### Community 225 - "finish_insights"
Cohesion: 0.23
Nodes (16): a_description_only_span_is_inference_and_an_added_name_is_dropped(), a_fact_requires_both_spans_in_the_title(), accept_claims(), article(), body_spans_accept_entity_and_object_from_full_article(), cap_claims(), context_claims_require_the_entity_in_the_title_and_link_context_for(), finish_insights() (+8 more)

### Community 226 - "Argos UI Interaction Audit"
Cohesion: 0.40
Nodes (5): Argos UI Interaction Audit, Audit Matrix, Field inventory (`field_placeholder`), Remaining limits, Shared contracts

### Community 228 - "run_live"
Cohesion: 0.29
Nodes (6): LiveRun, Fresh, Latest, Run, run_live(), RunInput

### Community 229 - "DecisionContract"
Cohesion: 0.27
Nodes (12): DecisionContract, template_directive_alignment(), template_evidence_relevance(), template_extraction_fidelity(), template_handoff_adequacy(), template_mode(), template_next_action(), template_publication() (+4 more)

### Community 230 - "RspStatus"
Cohesion: 0.21
Nodes (8): clip_summary(), parse_status(), RspStatus, Blacklisted, Deprecated, GenerallyReliable, GenerallyUnreliable, NoConsensus

### Community 231 - "JobRow"
Cohesion: 0.29
Nodes (3): job_row(), JobRow, parse()

### Community 232 - "RspEntry"
Cohesion: 0.27
Nodes (7): normalize_host(), normalize_name(), observation_for(), observation_marks_unlisted(), parses_status_domains_and_maps_reliability(), RspEntry, SourceReliabilityObservation

### Community 233 - "AtlasInsightClaim"
Cohesion: 0.29
Nodes (10): Extraction, InsightStats, Settled, Checkpoint, CheckpointInput, claim(), save_checkpoint(), take_extraction() (+2 more)

### Community 235 - "RspIndex"
Cohesion: 0.40
Nodes (9): cache(), cached_index(), CachedIndex, ensure_index(), fetch_index(), index_from_json(), install_index(), RspIndex (+1 more)

### Community 236 - "RepairStatus"
Cohesion: 0.22
Nodes (9): RepairStatus, Active, IndexingDisabled, IndexQueued, MissingPayload, NoInsights, Partial, Repaired (+1 more)

### Community 237 - "StepState"
Cohesion: 0.22
Nodes (8): StepState, Blocked, Completed, Failed, Partial, Pending, Running, Skipped

### Community 238 - "SourceReliability"
Cohesion: 0.25
Nodes (4): SourceReliability, A, B, C

### Community 239 - "atlas_actions.rs"
Cohesion: 0.29
Nodes (4): record_start(), start_repair(), starts(), infer_app()

### Community 240 - "OriginStat"
Cohesion: 0.36
Nodes (8): insight_packet(), origin(), resume_stats(), select_significant(), significance_keeps_every_tier_and_ignores_domain_and_provider(), stats_from_stored(), tier_key(), OriginStat

### Community 241 - "elon_plan"
Cohesion: 0.39
Nodes (8): ac3_seo_results_that_never_mention_the_subject_add_no_domain_org_email_or_url(), ac4_google_search_always_sends_the_replaced_firecrawl_query(), ac5_every_executed_input_is_grounded_and_an_ungrounded_step_is_skipped(), ac7_a_pronoun_follow_up_takes_the_thread_subject(), elon_plan(), elon_results(), replay_who_is_elon_musk_keeps_queries_short_and_binds_no_seo_pages(), seo_results()

### Community 242 - "LevelFilter"
Cohesion: 0.29
Nodes (5): LevelFilter, All, Error, Info, Warn

### Community 243 - "merge_aliases"
Cohesion: 0.29
Nodes (7): a_country_token_does_not_merge_into_a_longer_name(), aliases_merge_on_shared_tokens_when_the_longer_form_is_a_span(), articles_have_span(), entity_is_country_alone(), longer_alias(), merge_aliases(), tokens()

### Community 244 - "QueryCompatibility"
Cohesion: 0.33
Nodes (5): QueryCompatibility, NeedsInput, Supported, Unsupported, Unverified

### Community 245 - ".job_source"
Cohesion: 0.50
Nodes (4): AtlasRun, JobSource, AtlasRun, ReconThread

### Community 246 - "BrainListMode"
Cohesion: 0.50
Nodes (4): BrainListMode, Create, Graph, List

## Knowledge Gaps
- **1535 isolated node(s):** `$schema`, `default_agent`, `mode`, `model`, `bash` (+1530 more)
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 2072 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **25 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `App` connect `App` to `app.rs`, `ui.rs`, `run`, `.set_focus`, `Store`, `ToolResult`, `hardware.rs`, `Store`, `.handle_key`, `provider.rs`, `.on_work_event`, `brain_detail.rs`, `recon/graph.rs`, `run_atlas_inner`, `ReportMode`, `.activate_button`, `JobsView`, `worker.rs`, `Target`, `src/brain.rs`, `Overlay`, `LogsView`, `FeedArticle`, `DefaultsRole`, `AtlasArticleRow`, `SettingsFile`, `summary_card.rs`, `ModuleId`, `WorkEvent`, `.job_source`, `BrainListMode`, `briefing_view.rs`?**
  _High betweenness centrality (0.069) - this node is a cross-community bridge._
- **What connects `$schema`, `default_agent`, `mode` to the rest of the system?**
  _1535 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `land.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.0076045627376425855 - nodes in this community are weakly interconnected._
- **Why does `ButtonId` connect `ButtonId` to `summary_card.rs`, `app.rs`, `ui.rs`, `contains`, `.activate_button`, `JobsView`, `.job_source`, `DefaultsRole`, `logs.rs`, `Rect`, `Target`?**
  _High betweenness centrality (0.036) - this node is a cross-community bridge._
- **Should `orchestrate.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.061170212765957445 - nodes in this community are weakly interconnected._
- **Why does `ProviderSecret` connect `ProviderSecret` to `atlas_memory.rs`, `recon.rs`, `Store`, `replace_insights.rs`, `TurnEvent`, `secrets.rs`, `gates.rs`, `ToolResult`, `execute_steps`, `provider.rs`, `body.rs`, `ChatMessage`, `brain_detail.rs`, `atlas.rs`, `run_atlas_inner`, `scheduler.rs`, `provider_attempt.rs`, `ReportMode`, `provider_chain.rs`, `.activate_button`, `exec.rs`, `worker.rs`, `intel_recon/jobs.rs`, `atlas_insights.rs`, `body_filter.rs`, `synthesize.rs`, `summarization.rs`, `Value`, `seeded`, `.evaluate`, `AtlasArticleRow`, `SettingsFile`, `graph_explanation/tests.rs`, `actor_review.rs`, `run_live`, `graph_explanation.rs`, `KeptClaim`, `all_tools`, `.order`, `anyhow`, `run_turn`, `briefing_view.rs`, `subscription.rs`, `extract`, `chat_model`?**
  _High betweenness centrality (0.031) - this node is a cross-community bridge._
- **Should `atlas_memory.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.12828282828282828 - nodes in this community are weakly interconnected._