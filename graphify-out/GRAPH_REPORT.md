# Graph Report - argos-osint  (2026-10-08)

## Corpus Check
- 171 files · ~387,364 words
- Verdict: corpus is large enough that graph structure adds value.
- Unclassified: 9 file(s) not represented in the graph (top: (none) 3, .toml 2, .diff 2)

## Summary
- 6158 nodes · 15199 edges · 230 communities (204 shown, 26 thin omitted)
- Extraction: 98% EXTRACTED · 2% INFERRED · 0% AMBIGUOUS · INFERRED: 283 edges (avg confidence: 0.85)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `e40375fd`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- land.rs
- orchestrate.rs
- .activate_button
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
- Call
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
- ModelGate
- InvestigationPart
- body.rs
- rule_bindings
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
- graph_explanation/tests.rs
- unix_now
- events.rs
- exec.rs
- home_rows
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
- SourceReliability
- Category
- body_filter.rs
- synthesize.rs
- dataset.rs
- .activate_target
- rusqlite
- theme.rs
- Overlay
- pipeline.rs
- summarization.rs
- InvestigationSurface
- LevelFilter
- .run_configured
- tui/jobs.rs
- .new
- FeedArticle
- budget.rs
- store.rs
- Command
- DefaultsRole
- logs.rs
- src/evidence.rs
- .is_empty
- DecisionContract
- AtlasArticleRow
- dispatch
- dork_generator.rs
- TurnClock
- How
- H3 Task Packet: State and Persistence Foundation
- summary_card.rs
- contains
- ModuleId
- Region
- graph_explanation.rs
- validate.rs
- KeptClaim
- .default
- .order
- .memory
- run_turn
- Architecture
- WorkEvent
- inset
- ClockSet
- ProviderAdmission
- Phase Plan: Home Recon Composer and Investigation Tabs
- BrainResourceSummary
- gsd-v2.js
- Request
- briefing_view.rs
- subscription.rs
- InsightStats
- .new
- synthesize
- mem
- ReconCommand
- JobStatusFilter
- RouteInput
- poll_device
- context_turn
- JobsView
- agent
- Functional Requirements
- Severity
- super
- ui.rs
- TaskState
- OsintCommand
- .new
- the_label_updates_when_a_round_adds_calls_and_when_evidence_is_measured
- Argos OSINT — Agent Instructions
- replace_insights.rs
- TaskStatus
- wikipedia_rsp.rs
- tasks.rs
- DecisionState
- Service
- url
- Store
- QueryExecutionStatus
- results.rs
- Store
- ledger.rs
- QuestionSpec
- button_areas
- §9 implementation order
- TurnContinuation
- investigation/evidence.rs
- InvestigationPattern
- Part
- argos-osint-bin
- What Was Learned
- JobRow
- SiteOutcomeStatus
- investigation.rs
- Milestone 1 — Core Onboarding (Current)
- paths.rs
- HypothesisRecord
- atlas_work.rs
- H2 Contracts: Frozen Interface Decisions
- youtube_pair
- Codebase Map — argos-osint
- PROJECT.md — argos-osint
- Current Phase State
- SettingsFile
- RecoveryAction
- ModelAssignment
- BodyFetchEvent
- Conventions
- Docs Ingest — argos-osint
- atlas_actions.rs
- anyhow
- OpenCode V2 workflow
- Unified reliability / summarization / semantic pipelines — completion checklist
- Concepts
- package.json
- explore.rs
- App
- subject_of
- Providers
- Decision roles and output contracts
- H0-H2 Completion Summary
- Terminal UI
- PlanCall
- json
- Value
- enqueue_article_body
- intel_recon/brain.rs
- .new
- F
- Unified Investigation Harness
- select_strategy
- PlanInterval
- IndexOutcome
- Argos OSINT
- .job_source
- LaunchState
- Argos documentation
- Diagram conventions
- AcceptMode
- Argos UI Interaction Audit
- ChainReport<T>
- ProviderKeys

## God Nodes (most connected - your core abstractions)
1. `App` - 295 edges
2. `ProviderSecret` - 130 edges
3. `ButtonId` - 126 edges
4. `FieldId` - 72 edges
5. `Store` - 67 edges
6. `Target` - 64 edges
7. `ToolResult` - 62 edges
8. `Store` - 61 edges
9. `AtlasArticleRow` - 55 edges
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
- `table_lines()` --calls--> `format_duration()`  [INFERRED]
  crates/argos-osint-bin/src/tui/jobs.rs → crates/argos-osint-core/src/jobs_view.rs

## Import Cycles
- 2-file cycle: `crates/argos-osint-core/src/osint.rs -> crates/argos-osint-core/src/osint/results.rs -> crates/argos-osint-core/src/osint.rs`
- 2-file cycle: `crates/argos-osint-bin/src/cli.rs -> crates/argos-osint-core/src/hardware.rs -> crates/argos-osint-bin/src/cli.rs`

## Communities (230 total, 26 thin omitted)

### Community 0 - "land.rs"
Cohesion: 0.01
Nodes (262): CODES, LABELS, R0, R1, R10, R100, R101, R102 (+254 more)

### Community 1 - "orchestrate.rs"
Cohesion: 0.05
Nodes (64): a_claimed_email_removes_its_bindings(), a_gap_filler_domain_feeds_hunter_once_firecrawl_observes_it(), a_handle_named_in_a_derived_question_is_an_unverified_binding(), a_weak_firecrawl_search_adds_one_google_search_with_the_same_query(), a_zero_email_count_skips_the_paid_domain_search(), ac3_seo_results_that_never_mention_the_subject_add_no_domain_org_email_or_url(), ac4_google_search_always_sends_the_replaced_firecrawl_query(), ac5_every_executed_input_is_grounded_and_an_ungrounded_step_is_skipped() (+56 more)

### Community 2 - ".activate_button"
Cohesion: 0.08
Nodes (9): open_external_url(), ProviderEvent, Finished, ProviderPage, .ALL, Defaults, Google, Nvidia (+1 more)

### Community 3 - "atlas_memory.rs"
Cohesion: 0.06
Nodes (90): Extraction, atlas_memory_count(), background_indexing_then_refresh_upgrades_a_partial_run(), Barrier, Checkpoint, CheckpointInput, claims(), clear() (+82 more)

### Community 4 - "tool_io.rs"
Cohesion: 0.04
Nodes (90): Binding, canonical(), dependency_order(), depends_on(), question_bindings_and_dependency_fix(), accept_bindings(), allowed_producer(), best_handle() (+82 more)

### Community 5 - "app.rs"
Cohesion: 0.06
Nodes (73): a_click_folds_an_event_and_incoming_entries_do_not_move_a_paused_list(), a_finished_tool_call_is_logged_once_and_the_transcript_keeps_a_summary(), all_provider_actions_render_with_hit_areas_at_80x24(), ATLAS_AUTO_SECS, atlas_auto_toggle_persists_the_next_trigger(), atlas_claim(), atlas_headlines_scroll_and_request_failures_reach_the_system_log(), atlas_history_resume_is_offered_only_for_resumable_cycles_and_repair_starts_once() (+65 more)

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
Nodes (69): ac6_citation_groups_split_validate_each_id_and_normalize(), answer_step_rate_limit_is_explicit_and_keeps_results(), attribute_word(), auxiliary(), BRIEF_SYNTHESIS, broad_question_collapses_to_grounded_lookups(), char_ceil(), char_floor() (+61 more)

### Community 10 - "ButtonId"
Cohesion: 0.02
Nodes (102): ButtonId, Add, AddFallback, AtlasAuto, AtlasDelete, AtlasLive, AtlasNews, AtlasNewsFeed (+94 more)

### Community 11 - "Store"
Cohesion: 0.07
Nodes (16): claims_deduplicate_and_reject_unsupported_sources(), CreditHold, deleting_an_investigation_removes_its_brain_memories_and_graph_summary(), investigation_memory_ids(), now(), settle_holds(), persist_claims(), persistence_and_plan() (+8 more)

### Community 12 - "IntelligenceCategory"
Cohesion: 0.09
Nodes (22): all_61_tools_mapped_to_categories(), argument_contract(), ArgumentBuilderContract, compact_capability_catalog(), CompactToolCapability, IntelligenceCategory, Bitcoin, DomainNetwork (+14 more)

### Community 13 - "job_registry.rs"
Cohesion: 0.05
Nodes (48): begin(), begin_cancellable(), db(), db_path(), finish(), beat(), BEAT_INTERVAL, beats() (+40 more)

### Community 14 - "Call"
Cohesion: 0.14
Nodes (24): build_blocks(), clip_chars(), coverage_only_counts_explicit_assessments(), coverage_summary(), decision_row_shows_directives_picker_order_bindings_and_fallbacks(), extract_log(), footer_line(), input_brief() (+16 more)

### Community 15 - "directives.rs"
Cohesion: 0.06
Nodes (64): apply_context_targets(), asks(), asks_about_judge(), asks_for_headlines(), clip_words(), context_entity(), CONTEXT_FILLER, context_gate() (+56 more)

### Community 16 - ".set_focus"
Cohesion: 0.07
Nodes (15): backspace_after_a_sent_question_deletes_one_character(), brain_anchors_follow_memory_focus_and_scroll_stops_at_ends(), draft_isolation_and_persistence(), duplicate_submission_prevention(), keyboard_navigation_esc_and_shortcuts(), multiline_paste_stays_in_composer_and_does_not_run_shortcuts(), pointer_and_chords_do_not_switch_apps_on_their_own(), recon_chat_folds_decisions_and_opens_synthesis_memory() (+7 more)

### Community 17 - "absorb_hit"
Cohesion: 0.12
Nodes (22): absorb_hit(), account_platform_host(), Candidate, content_tokens(), distinct_queries(), domain_label(), EntityIdentifier, first_domain() (+14 more)

### Community 18 - "providers.rs"
Cohesion: 0.06
Nodes (52): clip_page(), assert_host_locked(), BATCH_SCRAPE_DEFAULT_URLS, BATCH_SCRAPE_MAX_URLS, batch_urls(), company_card(), CRAWL_MAX_PAGES, every_firecrawl_tool_builds_a_host_locked_post() (+44 more)

### Community 19 - "atlas_news.rs"
Cohesion: 0.08
Nodes (44): api_error(), ATLAS_TOOLS, authors(), clip(), country_arg(), country_code(), CURRENTS_COUNTRIES, CURRENTS_HOST (+36 more)

### Community 20 - "Store"
Cohesion: 0.07
Nodes (18): article_body_from_row(), ArticleBodyRow, element_from_row(), IntelAssessmentRow, IntelEvidenceRow, IntelInvestigationRow, IntelReportJobRow, IntelReportSectionRow (+10 more)

### Community 21 - "publication.rs"
Cohesion: 0.10
Nodes (35): claim(), AtlasInsightClaim, bump_memories_changed(), claim(), clear(), CoverageReport, delete_run_memory_state(), DELETED_REVISION (+27 more)

### Community 22 - "gates.rs"
Cohesion: 0.08
Nodes (13): execute_harness_step(), InvestigationRuntime, GateOutcome, Passed, Rejected, validate_claim_assessment(), validate_evidence_admission(), validate_publication() (+5 more)

### Community 23 - "ToolResult"
Cohesion: 0.17
Nodes (7): ToolResult, answer_evidence_survives_reopen_and_deleted_thread_rejects_late_calls(), cacheable(), evidence_summary(), evidence_notes(), sample_evidence(), signatures()

### Community 24 - "news_legal.rs"
Cohesion: 0.07
Nodes (48): a_rate_limit_switches_to_the_fallback_key_and_a_rejection_does_not(), ac1_catalog_has_55_tools_and_the_news_and_legal_entries(), ac1_each_request_has_its_url_params_and_auth_header_and_no_key_in_the_url(), an_echoed_key_is_redacted_from_the_stored_body_and_error(), article(), articles(), CATEGORIES, choice_arg() (+40 more)

### Community 25 - "hardware.rs"
Cohesion: 0.15
Nodes (18): CACHE_TTL_SECS, classify_arch(), disk_totals(), HardwareProfile, metal_vram_gb(), now_secs(), parse_apple_gpu_cores(), partial_gpu() (+10 more)

### Community 26 - "App"
Cohesion: 0.07
Nodes (68): active_popup_area(), atlas_auto_label(), atlas_cycle_stats_room(), atlas_cycle_stats_scroll_max(), atlas_extracting(), atlas_feed_room(), atlas_feed_room_for(), atlas_live_areas() (+60 more)

### Community 27 - "Store"
Cohesion: 0.07
Nodes (10): UnitManifest, atlas_answer_id(), atlas_article_from_row(), atlas_brief_id(), AtlasRunRow, AtlasStoredClaim, has_table(), repair_embed_tables() (+2 more)

### Community 28 - "embed.rs"
Cohesion: 0.08
Nodes (31): active(), DIM, disable(), disabled(), DisableGuard, download_file(), DOWNLOAD_TIMEOUT_SECS, embed_batch() (+23 more)

### Community 29 - "osint.rs"
Cohesion: 0.08
Nodes (38): test_all_catalog_tools_mapped_to_categories(), CACHE_DAY_SECONDS, cache_follows_the_provider_plan_interval(), CACHE_MONTH_SECONDS, cache_seconds(), CACHE_WEEK_SECONDS, canonical_tool_id(), default_enabled() (+30 more)

### Community 30 - "execute_steps"
Cohesion: 0.14
Nodes (30): after_step(), context_block(), context_dispatched(), directive_for(), execute_steps(), expand_dork_children(), expand_per_platform(), extract_bindings() (+22 more)

### Community 31 - ".handle_key"
Cohesion: 0.10
Nodes (11): ctrl_arrows_cycle_app_tabs_forward_and_back(), ctrl_tab_cycles_app_tabs_forward_and_back(), IntelReconFocus, Section, Start, Tab, is_picker_field(), PaletteItem (+3 more)

### Community 32 - "provider.rs"
Cohesion: 0.04
Nodes (49): a_blank_osint_user_agent_loads_as_unset(), complete_errors_with_finish_reason_when_response_is_empty(), complete_stream_of_reasoning_deltas_yields_text(), decide(), DecisionAnswer, decisions_client_posts_to_alpha_decisions_and_parses_answers(), DECISIONS_MODELS, decisions_url() (+41 more)

### Community 33 - "ModelGate"
Cohesion: 0.18
Nodes (16): a_recon_model_429_trips_the_gate_and_later_calls_skip_the_provider(), call_cached(), cancelled(), derive_directives(), derived_note(), directive_user(), directive_user_includes_brain_resource_summary_when_present(), DirectivePrompt (+8 more)

### Community 34 - "InvestigationPart"
Cohesion: 0.10
Nodes (17): event_to_chat_block(), is_thinking_expanded(), InvestigationPart, DirectiveAssessment, EvidencePassage, GateValidation, Handoff, Plan (+9 more)

### Community 35 - "body.rs"
Cohesion: 0.17
Nodes (18): body_fetch_routes(), direct_http_extract(), discover_and_scrape(), FAILURE_COOLDOWN_SECS, fetch_article_body(), FetchedBody, full_markdown_from_result(), has_firecrawl_key() (+10 more)

### Community 36 - "rule_bindings"
Cohesion: 0.10
Nodes (33): binder_maps_kinds_to_inputs_and_never_hands_hunter_a_social_host(), binding(), normalize_platform(), question_bindings(), a_gap_filler_only_domain_waits_for_a_primary_observation_before_hunter(), batch_scrape_takes_ranked_urls_and_gates_order_before_hunter(), bind_arguments(), bitcoins_in() (+25 more)

### Community 37 - "brain_detail.rs"
Cohesion: 0.05
Nodes (45): areas(), BrainDetail, DetailAreas, DetailPane, Path, Related, Summary, DetailSnapshot (+37 more)

### Community 38 - "recon/graph.rs"
Cohesion: 0.07
Nodes (45): glyph(), basic_explanation(), build_claim_graph(), build_memory_graph(), call_for(), choose_directive(), claim_tokens(), directive_ids() (+37 more)

### Community 39 - "atlas.rs"
Cohesion: 0.09
Nodes (29): article_card_labels_title_publisher_author_and_classification(), Band, band_sizes(), band_temperature(), category_from_decisions(), CATEGORY_IDS, CLUSTERS, country_label() (+21 more)

### Community 40 - "run_atlas_inner"
Cohesion: 0.09
Nodes (34): AtlasEvent, Classified, InsightProgress, MemoriesChanged, MemoryProgress, Note, Replaced, Stats (+26 more)

### Community 41 - "whatsmyname.rs"
Cohesion: 0.06
Nodes (39): AccountTuple, ACTIVE_SNAPSHOT, ADAPTER_VERSION, apply_strip_bad_char(), benchmark_14_4_parse_index_and_selection(), cache_get(), cache_set(), CACHE_TTL (+31 more)

### Community 42 - "scheduler.rs"
Cohesion: 0.09
Nodes (22): apply_index_change(), apply_index_change_rebuild_reports_outcome(), apply_index_with(), DEFAULT_LEASE_SECS, drain_index_once(), drain_summary_flush(), drain_summary_flush_cached(), drain_summary_flush_completes_cached_tasks() (+14 more)

### Community 43 - "jobs_view.rs"
Cohesion: 0.11
Nodes (22): summary_text(), active_time_is_live_for_open_attempts_and_unavailable_only_without_a_start(), AttemptRow, event_apps(), event_counts(), EventCounts, format_duration(), get_job() (+14 more)

### Community 44 - "provider_attempt.rs"
Cohesion: 0.09
Nodes (38): Acc, attempt(), AttemptReport, check_final(), complete_stream_and_json_answers_succeed(), Deadlines, delta(), fill() (+30 more)

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
Nodes (14): AttemptOutcome, categorize_http(), execute_with_retries(), other_llm_allows_three(), summarization_stops_at_two_attempts(), TimeoutProfile, .CLASSIFIER, .EXTRACTION (+6 more)

### Community 50 - "graph_explanation/tests.rs"
Cohesion: 0.27
Nodes (16): auth_failure_consumes_primary_budget_gives_guidance_and_redacts(), conn(), count(), failure_is_durable_correlated_bounded_and_harmless(), fixture(), Fx, GOOD, late_completion_for_a_changed_or_deleted_memory_is_not_published() (+8 more)

### Community 51 - "unix_now"
Cohesion: 0.22
Nodes (7): atlas_auto_future_tick_waits_and_past_tick_reschedules(), atlas_auto_while_running_only_arms_the_next_slot(), atlas_history_button_counts_down_while_auto_run_is_on(), atlas_poll_wait(), manual_run_moves_the_next_auto_trigger_out_by_90_minutes(), unix_now(), Atlas

### Community 52 - "events.rs"
Cohesion: 0.13
Nodes (19): clear_events(), DEFAULT_RETENTION_HOURS, event_row(), EventRow, get_event(), job_filter_includes_descendants_and_prune_keeps_jobs(), list_events(), MAX_DETAIL_CHARS (+11 more)

### Community 53 - "exec.rs"
Cohesion: 0.14
Nodes (20): admission_account(), admission_contention_consumes_no_attempt(), AttemptEvent, Finished, Started, AttemptLog, cancelled(), complete_summary_report() (+12 more)

### Community 54 - "home_rows"
Cohesion: 0.16
Nodes (20): center_row(), cursor_blink_visible(), gap_row(), home_composer_and_all_content_centered_vertically_and_horizontally(), home_group(), home_layout_metrics(), home_line(), home_line_text() (+12 more)

### Community 55 - "ProviderSecret"
Cohesion: 0.08
Nodes (34): defaults_json(), account_secret(), active_text_secret(), authorize(), bearer_token(), catalog_error(), complete(), effective_kind() (+26 more)

### Community 56 - "ErrorCategory"
Cohesion: 0.10
Nodes (15): ErrorCategory, AuthOrQuota, CancelledOrStale, ConfigurationMissing, ContextLimit, IndexFailure, InterruptedStream, InvalidResult (+7 more)

### Community 57 - "grok_oauth.rs"
Cohesion: 0.15
Nodes (20): auth_path(), bearer(), bearer_from_path(), check_login(), Entry, entry_from(), failed_login_keeps_the_cli_reason_and_recovery_action(), login() (+12 more)

### Community 58 - "MemoryKind"
Cohesion: 0.09
Nodes (22): AdmissionPolicy, admit_memory(), BrainQuery, pack_context(), AssessmentState, Contradicts, Insufficient, Mentions (+14 more)

### Community 59 - "Rect"
Cohesion: 0.13
Nodes (48): AbsRect, Chrome, composer_height(), composer_parts(), draw(), draw_atlas(), draw_atlas_cycle_stats(), draw_atlas_news() (+40 more)

### Community 60 - "brain_resources.rs"
Cohesion: 0.10
Nodes (22): source_anchor_label(), BRAIN_SCRAPE_PREFIX, CLAIM_CHARS, classify_resource(), file_link_url(), hit(), is_brain_binding(), is_brain_scrape_pick() (+14 more)

### Community 61 - "worker.rs"
Cohesion: 0.15
Nodes (19): IntelReportEvent, InsightsUpdated, JobCreated, JobDone, Section, Stage, cancel_if_needed(), claim_relevant_evidence() (+11 more)

### Community 62 - "Target"
Cohesion: 0.07
Nodes (27): brain_article_source_opens_intel_brief(), FocusEntry, hit(), LayoutRegistry, Target, App, AtlasCycleStats, BrainMark (+19 more)

### Community 63 - "src/brain.rs"
Cohesion: 0.14
Nodes (19): AGREEMENT_WEIGHT, CATEGORIES, category_hint(), format_injection(), hybrid_recall(), hybrid_recall_adds_paraphrases_and_rewards_agreement(), identity_query_prefers_name_memory(), jaccard() (+11 more)

### Community 64 - "intel_recon/jobs.rs"
Cohesion: 0.11
Nodes (29): active_job_for_mode(), cancel_job(), create_job_inserts_all_verify_sections(), create_report_job(), finish_canonical(), intel_recon_links_its_legacy_job_to_one_canonical_registry_job(), pause_job(), register_canonical() (+21 more)

### Community 65 - "cli.rs"
Cohesion: 0.16
Nodes (20): ask(), ask_thread(), atlas_command(), defaults_command(), defaults_show_includes_the_tool_picker_transport(), DefaultsCommand, Set, Show (+12 more)

### Community 66 - "atlas_insights.rs"
Cohesion: 0.07
Nodes (40): a_country_token_does_not_merge_into_a_longer_name(), a_decisions_model_does_not_extract_claims(), aliases_merge_on_shared_tokens_when_the_longer_form_is_a_span(), articles_have_span(), ask_claims(), AskedClaims, BODY_CLAIM_LIMIT, BODY_SPAN_CHARS (+32 more)

### Community 67 - "SourceReliability"
Cohesion: 0.09
Nodes (22): article_information_credibility(), AdmiraltyCode, best_credibility(), CredibilityInputs, information_credibility(), InformationCredibility, CannotBeJudged, Confirmed (+14 more)

### Community 68 - "Category"
Cohesion: 0.05
Nodes (45): bounded(), Category, Auth, Cancelled, Configuration, Empty, InvalidModel, InvalidResult (+37 more)

### Community 69 - "body_filter.rs"
Cohesion: 0.17
Nodes (22): BATCH_SIZE, BodyChunk, chunk_article_body(), CHUNK_MAX, CHUNK_MIN, CHUNK_TARGET, chunks_preserve_absolute_offsets(), classify_chunk_batch() (+14 more)

### Community 70 - "synthesize.rs"
Cohesion: 0.17
Nodes (19): BODY_REFINE_INPUT_CHARS, deterministic_bluf_mentions_title(), deterministic_section(), model_refine_body(), model_synthesize(), parse_json_object(), refine_retrieved_article_body(), refine_without_secret_keeps_validated_scrape() (+11 more)

### Community 71 - "dataset.rs"
Cohesion: 0.13
Nodes (24): acquire_refresh_lease(), active_manifest_path(), ACTIVE_REFRESHES, dataset_root(), DatasetManifest, DatasetStatus, get_status(), import_from_file() (+16 more)

### Community 72 - ".activate_target"
Cohesion: 0.12
Nodes (3): AtlasArticle, AtlasFeed, IntelArticle

### Community 73 - "rusqlite"
Cohesion: 0.09
Nodes (19): Cached, None, Stale, Valid, Gate, CoolingDown, Ready, Running (+11 more)

### Community 74 - "theme.rs"
Cohesion: 0.05
Nodes (61): char_width(), column_widths(), columns_consume_inner_width(), display_width(), draw_origins(), east_asianish(), header_line(), OriginsView (+53 more)

### Community 75 - "Overlay"
Cohesion: 0.15
Nodes (13): ChoiceKind, IntelDay, Investigation, Model, Provider, Overlay, AddFallback, Choice (+5 more)

### Community 76 - "pipeline.rs"
Cohesion: 0.12
Nodes (26): apply_recon_directive_coverage(), ClaimRelation, AmbiguousEntity, ChangedQuantity, Equivalent, Negation, PlannedVsCompleted, Unrelated (+18 more)

### Community 77 - "summarization.rs"
Cohesion: 0.09
Nodes (39): cache_invalidates_when_source_revision_changes(), cache_get(), cache_key_changes_with_revision_and_focus(), cache_put(), cache_round_trip(), complete_summary(), CoverageMeta, deterministic_article_description() (+31 more)

### Community 78 - "InvestigationSurface"
Cohesion: 0.10
Nodes (12): InvestigationSurface, HomeComposer, IntelBrief, JobsResume, ReconChat, atlas_only_tools_blocked_on_all_surfaces(), check_need_gate(), sociavault_blocked_on_intel_surface() (+4 more)

### Community 79 - "LevelFilter"
Cohesion: 0.29
Nodes (5): LevelFilter, All, Error, Info, Warn

### Community 80 - ".run_configured"
Cohesion: 0.17
Nodes (18): a_claimed_email_keeps_no_person_data(), annotate(), claimed_email(), custom_user_agent(), effective_user_agent(), Executor, get(), hunter_observations() (+10 more)

### Community 81 - "tui/jobs.rs"
Cohesion: 0.11
Nodes (21): areas(), BASE_COLUMNS, button_label(), detail_lines(), draw(), hit(), JobsAreas, LOGS_COLUMN (+13 more)

### Community 82 - ".new"
Cohesion: 0.25
Nodes (19): a_spent_primary_quota_uses_the_fallback_key(), classification_request(), country_labels_show_the_name_and_the_code(), daily_cap(), format_run_card(), HttpCall, HttpReply, insights_do_not_call_a_decisions_model() (+11 more)

### Community 83 - "FeedArticle"
Cohesion: 0.16
Nodes (20): apply_hits(), article_from(), article_from_row(), article_row(), Article, canonical_url(), dedup_drops_a_url_an_exact_title_and_keeps_the_higher_domain(), FeedArticle (+12 more)

### Community 84 - "budget.rs"
Cohesion: 0.11
Nodes (24): courtlistener_spacing_and_firecrawl_polling_are_counted(), CUT_NOTE, CUT_SHORT, deadline_seconds(), live(), more_calls_mean_more_tool_time_and_cache_hits_add_nothing(), PHASE_WINDOW_SECONDS, RECON_DEADLINE (+16 more)

### Community 85 - "store.rs"
Cohesion: 0.10
Nodes (20): ArticleInsightCommit, AUTO_REBUILD_HINT, embedding_disabled_degrades_to_jaccard(), embedding_failure_mid_session_degrades_to_jaccard(), fingerprint_mismatch_rebuilds_and_drift_is_reconciled(), GraphSummary, IDS, insight_fingerprint() (+12 more)

### Community 86 - "Command"
Cohesion: 0.10
Nodes (20): AtlasCommand, Repair, Resume, Verify, Cli, Command, Atlas, Defaults (+12 more)

### Community 87 - "DefaultsRole"
Cohesion: 0.06
Nodes (19): ChoiceItem, codex_models(), DefaultsRole, .ALL, ClaimAssessor, Classifier, EntityResolver, EvidenceCurator (+11 more)

### Community 88 - "logs.rs"
Cohesion: 0.11
Nodes (18): stamp(), areas(), button_label(), buttons(), count(), draw(), hit(), in_list() (+10 more)

### Community 89 - "src/evidence.rs"
Cohesion: 0.09
Nodes (30): AGREEMENT_WEIGHT, AnnMeasurement, AnnPolicy, AnnDeferredUnmeasured, AnnEnabled, ExactSearch, AnnThresholds, chunk_text() (+22 more)

### Community 90 - ".is_empty"
Cohesion: 0.12
Nodes (24): chat_body(), ChatMessage, complete_once(), complete_one(), complete_reads_reasoning_only_non_stream_json(), concrete_free_models(), delta_text(), is_concrete_free() (+16 more)

### Community 91 - "DecisionContract"
Cohesion: 0.08
Nodes (36): compile_general_model_prompt(), compile_native(), parse_general_model_response(), parse_native_response(), DecisionContract, DecisionService, DecisionPolicy, EnforcementOutcome (+28 more)

### Community 92 - "AtlasArticleRow"
Cohesion: 0.15
Nodes (26): category_tag(), accept_one(), apply_peer_support(), article_with_body_spans(), body_lead_prompt(), catalog_json(), classifier_peers_are_preferred_over_token_overlap(), classify_peers_chat() (+18 more)

### Community 93 - "dispatch"
Cohesion: 0.19
Nodes (15): Fault, CallSpec, dispatch(), fetch_with_spare_key(), json_message(), one_line(), parse_fault(), phase1_call() (+7 more)

### Community 94 - "dork_generator.rs"
Cohesion: 0.10
Nodes (31): ACTIVE_SNAPSHOT, compose_single_query(), compute_query_id(), compute_template_id(), DATASET_NAME, DEFAULT_MAX_QUERIES, DorkCatalog, DorkCategory (+23 more)

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
Cohesion: 0.12
Nodes (24): abs_rect(), atlas_row_at(), contains(), focus_order(), hit_test(), intel_body_loading(), intel_body_progress_lines(), intel_brief_full_lines() (+16 more)

### Community 100 - "ModuleId"
Cohesion: 0.15
Nodes (16): ModuleId, .ALL, Atlas, Brain, Intel, Jobs, Logs, Osint (+8 more)

### Community 101 - "Region"
Cohesion: 0.11
Nodes (19): Region, AtlasFeed, AtlasInsights, AtlasNews, AtlasOrigins, AtlasRuns, Chat, Detail (+11 more)

### Community 102 - "graph_explanation.rs"
Cohesion: 0.12
Nodes (14): BASIC_HEADING, event(), explain(), ExplainOutcome, Failed, Saved, Superseded, ExplainReport (+6 more)

### Community 103 - "validate.rs"
Cohesion: 0.18
Nodes (16): paywall_and_snippet_fail_validation(), BLOCK_MARKERS, BodyQuality, Complete, Partial, Unavailable, Uncertain, BodyValidation (+8 more)

### Community 104 - "KeptClaim"
Cohesion: 0.16
Nodes (26): a_description_only_span_is_inference_and_an_added_name_is_dropped(), a_fact_requires_both_spans_in_the_title(), accept_claims(), admiralty_scales_claim_confidence_from_rsp_and_peers(), apply_admiralty_evaluation(), article(), body_spans_accept_entity_and_object_from_full_article(), brief_text() (+18 more)

### Community 105 - ".default"
Cohesion: 0.21
Nodes (31): a_429_stops_picker_calls_and_the_fallback_finishes_the_list(), a_dispatch_error_fails_the_step_and_the_loop_goes_on(), a_failed_email_finder_gets_one_fallback_and_no_third_picker_call(), a_fallback_with_no_bindings_still_runs_the_satisfied_steps_and_reaches_synthesis(), a_goal_naming_a_tool_or_provider_is_repaired_then_falls_back(), a_goal_naming_a_tool_outside_the_prompt_entity_is_still_rejected(), a_prompt_entity_named_like_a_provider_keeps_the_models_directives(), a_question_handle_fills_the_handle_steps_without_a_fallback() (+23 more)

### Community 106 - ".order"
Cohesion: 0.22
Nodes (10): checked_serves(), context_additions(), context_tools(), fallback_record(), OrderContext, Ordered, Picker<'a>, serves_for() (+2 more)

### Community 107 - ".memory"
Cohesion: 0.15
Nodes (14): article(), atlas_articles_for_intel_dedupes_same_article_id_across_runs(), atlas_articles_for_intel_filters_category_day_and_query(), atlas_auto_next_round_trips_through_app_state(), atlas_claims_for_article_returns_linked_claims(), atlas_prune_drops_finished_runs_older_than_the_cutoff(), atlas_recent_articles_lists_retained_rows(), atlas_run_days_lists_distinct_days_newest_first() (+6 more)

### Community 108 - "run_turn"
Cohesion: 0.15
Nodes (20): a_follow_up_keeps_names_from_the_previous_synthesis(), ac7_a_pronoun_follow_up_takes_the_thread_subject(), classify_turn_mode(), continue_turn(), enabled_tools(), execute_ordered(), missing_keys(), picker_secret() (+12 more)

### Community 109 - "Architecture"
Cohesion: 0.12
Nodes (17): Admiralty source evaluation (WP:RSP), Architecture, Atlas, Binder and executor, Brain recall, Directives, Intel reports, Investigation flow (+9 more)

### Community 110 - "WorkEvent"
Cohesion: 0.11
Nodes (17): WorkEvent, AnswerDelta, AnswerNote, AtlasDone, CatalogDone, DatasetRefreshDone, DatasetRefreshProgress, Deadline (+9 more)

### Community 111 - "inset"
Cohesion: 0.20
Nodes (19): add_fallback_layout(), add_fallback_popup_area(), atlas_run_card(), choice_list_room(), cover(), draw_add_fallback(), draw_choice(), draw_intel_recon_popup() (+11 more)

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

### Community 117 - "Request"
Cohesion: 0.18
Nodes (18): bitcoin(), bounded(), domain(), email_address(), ip(), linkedin_handle(), number_arg(), crawl_limit() (+10 more)

### Community 118 - "briefing_view.rs"
Cohesion: 0.24
Nodes (13): bucket_extracted(), buckets_split_facts_inferences_context_and_links(), claim(), claim_label(), ExtractedBuckets, ExtractedLine, MAX_ACTORS, MAX_CONTEXT (+5 more)

### Community 119 - "subscription.rs"
Cohesion: 0.25
Nodes (8): answer_event(), check_login(), command(), complete(), completion_is_ephemeral_and_does_not_inherit_tools_or_configuration(), exec_args(), is_subscription_status(), login()

### Community 120 - "InsightStats"
Cohesion: 0.16
Nodes (17): insight_header(), insight_packet(), insight_row(), insight_row_line(), insight_stats_line(), insight_table_lines(), InsightRow, InsightStats (+9 more)

### Community 121 - ".new"
Cohesion: 0.17
Nodes (11): MemorySource, change_watcher_sees_only_committed_changes_and_coalesces(), MemoriesChanged, memory_writes_and_their_outbox_rows_commit_together(), MemoryChangeWatcher, retag_source(), source(), worker_killed_after_the_lance_write_is_recovered_by_revision() (+3 more)

### Community 122 - "synthesize"
Cohesion: 0.23
Nodes (18): a_dropped_provider_stream_keeps_the_text_already_received(), a_provider_that_rejects_streaming_fails_without_a_hidden_retry(), a_streamed_answer_with_a_bad_citation_ends_on_the_repaired_answer(), an_idle_stall_or_the_ceiling_keeps_the_partial_answer(), answer_text(), cancel_during_synthesis_marks_the_run_cancelled_and_keeps_partial_text(), chat_server(), ChatReply (+10 more)

### Community 123 - "mem"
Cohesion: 0.12
Nodes (32): lease_fencing_rejects_stale_epoch_and_foreign_owner(), adopt_untracked_index_changes(), claim_complete_and_retry_round_trip(), claim_index_changes(), claim_index_work(), claim_next(), claim_next_in(), complete_index_change() (+24 more)

### Community 124 - "ReconCommand"
Cohesion: 0.17
Nodes (12): ReconCommand, Ask, AskNew, Delete, Limits, List, New, Rename (+4 more)

### Community 125 - "JobStatusFilter"
Cohesion: 0.18
Nodes (8): JobFilter, JobStatusFilter, Active, All, Completed, Failed, Retrying, list_jobs()

### Community 126 - "RouteInput"
Cohesion: 0.17
Nodes (11): RouteInput, FacebookUrl, Handle, HandleOrUserId, Hashtag, LinkedinCompanyUrl, LinkedinProfileUrl, Query (+3 more)

### Community 127 - "poll_device"
Cohesion: 0.28
Nodes (9): DeviceGrant, Poll, Denied, poll_device(), Pending, SlowDown, Token, start_device() (+1 more)

### Community 128 - "context_turn"
Cohesion: 0.39
Nodes (9): a_keyword_inside_the_subjects_name_runs_no_news_or_legal_tool(), a_keyword_outside_the_subjects_name_still_runs_news_or_legal(), ac2_without_a_key_news_and_legal_tools_are_never_picked_and_with_a_key_they_are_eligible(), ac4_a_news_prompt_runs_newsapi_search_with_the_entity_and_who_is_runs_none(), ac5_a_sued_prompt_runs_courtlistener_case_or_docket_search_with_the_entity(), context_keys(), context_turn(), missing_providers() (+1 more)

### Community 130 - "agent"
Cohesion: 0.18
Nodes (10): agent, compaction, explore, mode, model, variant, default_agent, mode (+2 more)

### Community 131 - "Functional Requirements"
Cohesion: 0.12
Nodes (15): Atlas (News Pipeline), Brain / Memory, Configuration Requirements, Context Provider Keys, Functional Requirements, Non-Functional Requirements, Non-Goals, OSINT Manual Runs (+7 more)

### Community 132 - "Severity"
Cohesion: 0.18
Nodes (8): session_event(), EventFilter, NewEvent, Severity, Debug, Error, Info, Warn

### Community 134 - "ui.rs"
Cohesion: 0.05
Nodes (64): ACTION_H, ApiKeySlot, atlas_countdown(), atlas_history_live_label(), atlas_source_anchors_are_not_labeled_deleted_origin(), BrainForm, BrainList, call_stamp() (+56 more)

### Community 135 - "TaskState"
Cohesion: 0.18
Nodes (9): TaskState, Cancelled, Completed, Failed, Paused, Queued, RetryScheduled, Running (+1 more)

### Community 136 - "OsintCommand"
Cohesion: 0.14
Nodes (14): DatasetCommand, Import, Refresh, Status, OsintCommand, Attach, Dataset, Describe (+6 more)

### Community 137 - ".new"
Cohesion: 0.23
Nodes (5): a_round_keeps_its_time_and_a_tight_ceiling_still_runs_tools(), aging_the_tool_clock_blocks_new_calls_and_cache_hits_do_not_extend_it(), clock_set_for_turn(), foreground_expiry_continues_in_background(), sequential_raise_calls_budgets_higher()

### Community 138 - "the_label_updates_when_a_round_adds_calls_and_when_evidence_is_measured"
Cohesion: 0.28
Nodes (3): format_deadline(), format_span(), the_label_updates_when_a_round_adds_calls_and_when_evidence_is_measured()

### Community 139 - "Argos OSINT — Agent Instructions"
Cohesion: 0.13
Nodes (15): Architecture notes, Argos OSINT — Agent Instructions, CI (`.github/workflows/ci.yml`), CLI Entry Points (`argos` binary), Developer Commands, diagrams, Environment Variables, graphify (+7 more)

### Community 140 - "replace_insights.rs"
Cohesion: 0.31
Nodes (11): commit_refined_body(), article(), claim(), commit_replaces_atomically_and_keeps_shared_support(), failed_validation_leaves_prior_insights(), replace_article_insights_from_body(), ReplaceOutcome, snapshot_counts_are_stable_before_commit() (+3 more)

### Community 141 - "TaskStatus"
Cohesion: 0.07
Nodes (16): CallProposal, HandoffRecord, TaskRecord, TaskStatus, Cancelled, Completed, Deferred, Failed (+8 more)

### Community 142 - "wikipedia_rsp.rs"
Cohesion: 0.09
Nodes (38): API, APP_STATE_KEY, cache(), CACHE_TTL, cached_index(), CachedIndex, clip_summary(), ensure_index() (+30 more)

### Community 143 - "tasks.rs"
Cohesion: 0.09
Nodes (46): operation_kind(), add_missing_columns(), backoff_delay(), block_claimed(), can_retry(), claim_task(), claim_with(), ClaimedTask (+38 more)

### Community 144 - "DecisionState"
Cohesion: 0.20
Nodes (4): DecisionState, EvidencePassageState, SubjectStateSummary, TaskStateSummary

### Community 145 - "Service"
Cohesion: 0.15
Nodes (15): AnswerContext, chat(), cut_footer(), cut_short_answer(), finish_recon_job(), Message, credit_map(), note_cache() (+7 more)

### Community 146 - "url"
Cohesion: 0.24
Nodes (10): bind_request(), credential_key(), header_for(), provider_credential(), public_source_url(), rebase(), redact(), redact_key() (+2 more)

### Community 147 - "Store"
Cohesion: 0.22
Nodes (4): active_index_rows(), clear_index_state(), Store, IndexEnqueue

### Community 148 - "QueryExecutionStatus"
Cohesion: 0.17
Nodes (11): QueryExecutionStatus, Cancelled, Completed, Failed, Generated, NoResults, Queued, Running (+3 more)

### Community 149 - "results.rs"
Cohesion: 0.16
Nodes (16): classify_output_quality(), extract_observation_items(), extract_search_results(), extracts_from_array_shaped_observations(), extracts_from_object_with_results_array(), normalize_tool_result(), NormalizedToolResult, OutputQuality (+8 more)

### Community 150 - "Store"
Cohesion: 0.53
Nodes (5): charge_newsapi(), charge_quota(), open_key(), quota_open(), utc_day()

### Community 151 - "ledger.rs"
Cohesion: 0.13
Nodes (13): body_assertion_candidates(), coverage_complete(), ElementStatus, Assessed, Excluded, InProgress, Pending, Superseded (+5 more)

### Community 152 - "QuestionSpec"
Cohesion: 0.25
Nodes (5): DecisionQuestionType, Choice, Noul, Score, QuestionSpec

### Community 153 - "button_areas"
Cohesion: 0.13
Nodes (36): Block, abs_contains(), api_key_slot(), atlas_hit(), atlas_news_areas(), atlas_runs_areas(), brain_form(), brain_hit() (+28 more)

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

### Community 165 - "JobRow"
Cohesion: 0.29
Nodes (3): job_row(), JobRow, parse()

### Community 166 - "SiteOutcomeStatus"
Cohesion: 0.18
Nodes (10): deserialize_flexible_bool(), SiteOutcomeStatus, Ambiguous, Blocked, Error, Found, NotFound, RateLimited (+2 more)

### Community 167 - "investigation.rs"
Cohesion: 0.08
Nodes (73): Account, account_hits(), ACCOUNT_PLATFORMS, account_platforms_attach_to_the_subject_and_are_never_entities(), ACCOUNTS, action_order(), actions_are_grounded_capped_and_not_a_sweep(), ADAPTIVE (+65 more)

### Community 168 - "Milestone 1 — Core Onboarding (Current)"
Cohesion: 0.18
Nodes (10): Deliverables, Dependencies, Future Milestones (Planned), Milestone 1 — Core Onboarding (Current), Milestone 2 — CLI Verification, Milestone 3 — Test Coverage, Milestone 4 — Integration & Ship, Objectives (+2 more)

### Community 169 - "paths.rs"
Cohesion: 0.33
Nodes (9): auth_path(), config_path(), dataset_dir(), datasets_dir(), db_path(), ensure_home(), hardware_cache_path(), home_dir() (+1 more)

### Community 170 - "HypothesisRecord"
Cohesion: 0.18
Nodes (13): Alternative, classify_hypothesis(), contradiction(), draft_hypotheses(), hypothesis_absence_stays_unresolved(), hypothesis_status(), HypothesisRecord, leading_name() (+5 more)

### Community 171 - "atlas_work.rs"
Cohesion: 0.08
Nodes (27): articles_rev(), AttemptRecord, check_dependency_coverage(), CycleOutcome, Blocked, Cancelled, Completed, CompletedWithWarnings (+19 more)

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

### Community 179 - "ModelAssignment"
Cohesion: 0.35
Nodes (3): ModelAssignment, ModelRoute, role_fallbacks_add_delete_reorder_and_reject_duplicates()

### Community 180 - "BodyFetchEvent"
Cohesion: 0.22
Nodes (9): BodyFetchEvent, Attempt, Failed, InsightsFailed, InsightsRefreshing, InsightsReplaced, Progress, Ready (+1 more)

### Community 181 - "Conventions"
Cohesion: 0.22
Nodes (9): Agent scratch, Checks, Conventions, Diagrams, Graphify, Planning files, Schema, Tests (+1 more)

### Community 182 - "Docs Ingest — argos-osint"
Cohesion: 0.33
Nodes (5): `docs/architecture.md`, Docs Ingest — argos-osint, `docs/providers.md`, Ingest Status, Ingested Documentation

### Community 183 - "atlas_actions.rs"
Cohesion: 0.29
Nodes (4): record_start(), start_repair(), starts(), infer_app()

### Community 184 - "anyhow"
Cohesion: 0.13
Nodes (14): DecisionAdapterKind, JsonMode, NativeDecisions, OutputTool, StrictJsonSchema, ValidatedText, resolve_adapter(), DecisionValidationStatus (+6 more)

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
Cohesion: 0.04
Nodes (28): a_failed_turn_keeps_the_streamed_answer_on_screen(), a_log_or_late_tool_row_does_not_drop_the_live_answer(), add_scroll(), App, atlas_countdown_visible(), atlas_extracting_visible(), atlas_log_level(), AtlasPage (+20 more)

### Community 203 - "subject_of"
Cohesion: 0.12
Nodes (17): accounts_flow(), accounts_search_query(), clip_query(), complementary_queries(), derived_question_handles(), DiscoveryQuery, investigation_frame(), InvestigationFrame (+9 more)

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

### Community 209 - "PlanCall"
Cohesion: 0.17
Nodes (15): a_budgeted_call_with_a_dependency_from_an_earlier_batch_runs(), action_call(), bound(), BudgetedOutcome, cancel_mid_dispatch_releases_credit_holds(), execute_budgeted(), isolation_lines(), isolation_lines_show_what_ran_what_was_held_and_why() (+7 more)

### Community 210 - "json"
Cohesion: 0.29
Nodes (3): box(), color(), render()

### Community 211 - "Value"
Cohesion: 0.23
Nodes (12): await_completion(), clip_chars_ellipsis(), clip_long_strings(), compact_page(), compact_page_evidence(), compact_page_system(), long_page_evidence_compacts_to_a_summary_for_synthesis(), page_excerpt() (+4 more)

### Community 212 - "enqueue_article_body"
Cohesion: 0.33
Nodes (6): enqueue_article_body(), EnqueueOutcome, AlreadyRunning, Cached, Cooldown, Start

### Community 213 - "intel_recon/brain.rs"
Cohesion: 0.57
Nodes (5): intel_recon_publishes_durably_with_provenance_and_no_atlas_receipt(), ReconInsightUpdate, update(), upsert_adds_and_updates_claims(), upsert_recon_insights()

### Community 214 - ".new"
Cohesion: 0.23
Nodes (10): D, CompiledSite, DatasetSnapshot, deserialize_flexible_code(), deserialize_headers(), deserialize_optional_string_or_empty(), deserialize_string_or_list(), slugify() (+2 more)

### Community 215 - "F"
Cohesion: 0.43
Nodes (5): F, refresh(), refresh_from_upstream(), refresh_with_progress(), refresh_with_progress_and_client()

### Community 216 - "Unified Investigation Harness"
Cohesion: 0.33
Nodes (6): Catalog projection, Model roles, Persistence, Reasoning channels, Unified Investigation Harness, Validation gates

### Community 217 - "select_strategy"
Cohesion: 0.40
Nodes (6): has_concrete_identifier(), hypothesis_question(), select_strategy(), strategy_change_reason(), strategy_follows_the_question_and_can_change_without_erasing_work(), StrategyChoice

### Community 218 - "PlanInterval"
Cohesion: 0.33
Nodes (5): PlanInterval, Daily, Monthly, Never, Weekly

### Community 219 - "IndexOutcome"
Cohesion: 0.12
Nodes (7): IndexOutcome, Disabled, Pending, PermanentFailure, Ready, RetryableFailure, IndexWork

### Community 220 - "Argos OSINT"
Cohesion: 0.33
Nodes (6): Applications, Argos OSINT, Documentation, Figures, Limits, Quick start

### Community 221 - ".job_source"
Cohesion: 0.50
Nodes (4): AtlasRun, JobSource, AtlasRun, ReconThread

### Community 222 - "LaunchState"
Cohesion: 0.40
Nodes (5): LaunchState, Accepted, Accepting, Editable, RecoverableFailure

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

### Community 228 - "ProviderKeys"
Cohesion: 0.29
Nodes (7): LiveRun, Fresh, Latest, Run, run_live(), RunInput, ProviderKeys

## Knowledge Gaps
- **1526 isolated node(s):** `$schema`, `default_agent`, `mode`, `model`, `bash` (+1521 more)
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 2058 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **26 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `App` connect `App` to `JobsView`, `.activate_button`, `app.rs`, `ui.rs`, `Call`, `.set_focus`, `Service`, `Store`, `ToolResult`, `hardware.rs`, `Store`, `.handle_key`, `brain_detail.rs`, `recon/graph.rs`, `run_atlas_inner`, `ReportMode`, `SettingsFile`, `unix_now`, `ProviderSecret`, `Target`, `src/brain.rs`, `.activate_target`, `Overlay`, `FeedArticle`, `DefaultsRole`, `logs.rs`, `AtlasArticleRow`, `.job_source`, `LaunchState`, `summary_card.rs`, `ModuleId`, `WorkEvent`, `briefing_view.rs`?**
  _High betweenness centrality (0.067) - this node is a cross-community bridge._
- **What connects `$schema`, `default_agent`, `mode` to the rest of the system?**
  _1526 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `land.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.0076045627376425855 - nodes in this community are weakly interconnected._
- **Why does `ButtonId` connect `ButtonId` to `JobsView`, `.activate_button`, `summary_card.rs`, `app.rs`, `ui.rs`, `inset`, `tui/jobs.rs`, `DefaultsRole`, `logs.rs`, `button_areas`, `Rect`, `.job_source`, `Target`?**
  _High betweenness centrality (0.032) - this node is a cross-community bridge._
- **Should `orchestrate.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.049899396378269616 - nodes in this community are weakly interconnected._
- **Why does `ProviderSecret` connect `ProviderSecret` to `.activate_button`, `atlas_memory.rs`, `recon.rs`, `Store`, `replace_insights.rs`, `Service`, `gates.rs`, `execute_steps`, `provider.rs`, `ModelGate`, `body.rs`, `atlas.rs`, `scheduler.rs`, `provider_attempt.rs`, `ReportMode`, `provider_chain.rs`, `graph_explanation/tests.rs`, `exec.rs`, `anyhow`, `worker.rs`, `intel_recon/jobs.rs`, `atlas_insights.rs`, `body_filter.rs`, `synthesize.rs`, `summarization.rs`, `Value`, `.is_empty`, `DecisionContract`, `AtlasArticleRow`, `ProviderKeys`, `graph_explanation.rs`, `.default`, `.order`, `run_turn`, `subscription.rs`, `synthesize`, `poll_device`?**
  _High betweenness centrality (0.028) - this node is a cross-community bridge._
- **Should `.activate_button` be split into smaller, more focused modules?**
  _Cohesion score 0.08097165991902834 - nodes in this community are weakly interconnected._