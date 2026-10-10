# Argos UI Interaction Audit

Inventory of Home, the nine apps, popups, and field IDs from `ModuleId`, `Overlay`, `FieldId`, `ButtonId`, and `Target` in `crates/argos-osint-bin/src/tui/app.rs`.

- Layout: `ui.rs` `LayoutRegistry`
- Tab order: `(top, left)` of registered rectangles after draw
- Placeholders: `field_placeholder`
- Long panes: `draw_see_more` (`see more` footer)
- Focused panes append `• focused` via `focused_pane`

## Shared contracts

- Tab/Shift+Tab: `focus_order` sorts registered geometry, then raster fallback when the registry is empty.
- Popups: `LayoutRegistry::push_scope` traps hits; Escape closes and restores the invoker.
- Wheel: region under the pointer; keyboard scrolling belongs to the focused inner pane.
- Overhaul fixtures: 160×50, 120×40, 100×32, 80×24, 60×18 and 40×20; Profile also checks 100×24.

## Audit Matrix

| Screen / Overlay | State Variant | Targets / Controls | Visual Order | Input Hints | Scroll / Overflow | Focused Style | Narrow Layout | Verification |
|---|---|---|---|---|---|---|---|---|
| Home | Launcher | App(0..8), Composer, Send | Top-down launcher then composer | Ask an OSINT question… | Composer wraps; Home list fits | Selected row + field gutter | Centered stack, 80×24 | Tests: home_order, home_composer |
| Home | Session tabs | Header tabs when threads exist | Left-to-right | — | Horizontal clip | Selected tab | Tabs shrink | Tests: session_tabs_open_close_reopen |
| Intel bulletin | Empty/filtered/list | IntelTab, IntelDay, IntelSearch, IntelArticle | Tabs, day, search, list | Search title, source, or topic… | List scroll | Selected article | Tabs wrap | Tests: intel_opens_bulletin_filters_and_opens_briefing |
| Intel briefing | Idle | Image, Visit article site, Reload, full article, mode, Summary, View full report, jobs | Center stack then right jobs | — | Center stack + full-article inner scroll; `see more` on long regions | Highlighted middle headings | Stack scrolls; jobs stay | Tests: selected_intel_busy_hides_only_this_article_controls; dump_overhaul_screens |
| Intel briefing | Body/recon busy | Reload, jobs Pause/Cancel | Busy hides mode/Summary/View full report | — | Jobs remain | Jobs buttons | Same | Predicate `selected_intel_busy` |
| Intel overlay | Mode picker | IntelRecon, section toggles, Close | Popup trap | — | Popup scroll | Card | 80×24 clamp | Overlay::IntelRecon |
| Intel overlay | Full report | Report Block, Close, SeeMorePopup | 70%×80% centered | — | Independent Markdown scroll | Detail · focused when see more focused | Tiny terminals clamp | Tests: full_report_popup_is_seventy_percent_of_viewport |
| Atlas live | Running/idle | Run/Pause, Auto, History, origins, insights, feed | Actions, table, insights, feed | — | Origins table + feed | Origins ` · focused` | Columns length-constrained | atlas_table.rs |
| Atlas runs | Cycle list | Live, Resume, Repair, Delete, map, stats, cycles | Map, actions, stats+list | — | Stats table, cycle list | Stats focused | Map shrinks | Tests: atlas_live_feed, news_feed |
| Atlas news | Feed | Articles, World | List | — | Article list | Selected row | Country code | Tests: atlas_news_feed_tags_the_country_code |
| Brain list | Empty/filter/error | BrainQuery, Memory, Create, Pin, Delete, BrainRecall | Query, list, actions, anchors | Search memories by topic… | List + anchors `see more` | Selected memory | Action row wraps | Tests: brain_tab_still_edits_and_finds_sourced_memories |
| Brain create | Form | BrainApp, Conversation, Insight, Save, Back | Fields then buttons | Source app / Optional conversation ID / Write a useful fact… | — | Field gutter | Form stacks | Create mode |
| Brain graph | Detail | Back, graph, Related, Summary, failure actions | Graph, related, summary | — | Related/summary | Pane titles | Related above Summary | Tests: claim_detail, narrow_claim_detail |
| Recon list | Search | ReconSearch, Thread, New, Delete | Search, list, actions | Find investigations… | List | Selected thread | Full width | Tests: recon_and_osint_controls |
| Recon chat | Wide | Transcript, Investigation, composer, Hide investigation | Transcript 70%, panel 30% viewport | Ask an OSINT question… | Transcript + panel | Investigation · focused | Collapse <110 cols | Tests: investigation_panel_is_thirty_percent_of_viewport |
| Recon chat | Narrow | Transcript, Show investigation, composer | Full-width transcript, bottom actions | Ask an OSINT question… | Transcript | Composer gutter | Drawer/bottom actions | Tests: recon_context_hides_on_narrow_screen_without_changing_preference |
| Jobs | List/detail | JobsSearch, rows, Cancel, Retry, logs | Filter, table, actions | Find jobs by name or state… | Table | Selected row | Detail full screen | Tests: jobs_dashboard |
| Logs | List/detail | LogsSearch, rows, filters, Open job | Filter, list | Filter messages or IDs… | List + detail | Selected event | Detail fold | Tests: logs_record_errors |
| Tools | Catalog / Documentation / Test | OsintSearch, ToolCategory, tools, OsintDetail, OsintResponse, keys, input, Run/Cancel/Raw | Category catalog left; documentation and test right | Find tools… / JSON inputs / Paste API key / Optional backup | Independent documentation and response | Pane focus borders | Three button pages | tool_documentation_heading_is_visible_for_filtered_selection; search_reveals_groups_without_mutating_expansion |
| Models Defaults | Role assignment | Tabs, role, Provider, Model, Save, Refresh | Tabs then role form | Choose a provider / Choose a model | Role note | Selected role | Role buttons wrap | Tests: defaults_pick_provider_and_model_from_account_access |
| Models Google/Nvidia/OpenRouter | Key + catalog | Key, Save, Verify, Show endpoint, endpoint, filter, Refresh, catalog | Form then catalog | Paste … key / API base URL / Filter model names or IDs… | Catalog `see more` | Models · N of M | Endpoint hidden until shown | Tests: google_and_nvidia, provider_catalog_filters_locally |
| Profile Overview | Summary / Intel / Recon / Atlas / Models / Tools | Three parent tabs, six child tabs, filters, cards, Report | Shared Dashboard geometry | Loading / empty / stale | Page + independent panels | Active marker / focused card | Single column, too-small notice | analytics_viewports_and_data_states; overhaul_tabs_focus_without_activating_and_configs_suspend_reads |
| Profile System | Host / Paths | Shared parent tabs, Refresh hardware, compact pane tabs | Tabs, action, panes | Cached hardware | Independent panes | Focus border | Host/Paths pages | profile_splits_into_overview_and_system_tabs_and_logs_own_clear |
| Profile Configs | Export above Import | Path, Export, JSON, Verify, Save and apply | Visual order, editor input ownership | Draft / invalid / verified / commit error | JSON caret/offset | Bordered editor | Resize notice below edit floor | overwrite/revision/Control-Enter tests; dump_overhaul_screens |
| Profile Report | Chart + supporting datasets | ProfileReport, ProfileTable, selected-record/owner actions, sort, v, Esc | Scoped card; Tab reveals datasets | Retained snapshot / empty / stale | Fixed sections; table then card at boundary | Sticky headers / selected row | 3–10 data rows | profile_all_20_reports_have_registered_keyboard_and_mouse_routes; nested table tests |
| Overlay: Help | Shortcuts | Close, SeeMorePopup | Centered card | — | Markdown `see more` | Shortcuts · focused | 24–76×8–28 | Keyboard ? |
| Overlay: Memories | Synthesis memories | Close, SeeMorePopup | Card | — | Scroll | Memory · focused | Same | Brain mark |
| Overlay: Block | Generic/report | Close, SeeMorePopup, Atlas news on run cards | Report uses 70% width | — | Scroll | Detail · focused | Clamp | Tests: full_report_popup |
| Overlay: Choice | Provider/model/day | Choice rows, Close | List | — | Popup list | Selected choice | Note + list | Defaults pickers |
| Overlay: Palette | Recent / Current app / Universal | Query, command rows, Esc | Fixed query, headings skipped | Type to filter | List | Selected item | Centered | Tests: unavailable_palette_action |
| Overlay: IntelRecon | Mode + sections | Toggles, start, Close | Title, sections, actions | — | Popup | Selected toggle | Clamp 40–88 | Mode button |
| Overlay: AddFallback | Provider tabs / key / models | FallbackTab, key, verify, model rows, confirm, Close | Tabs, key, models, actions | Masked credential | Model list | Active and focused tabs independent | Clipped popup | defaults_fallbacks_add_delete_reorder_persist; dump_overhaul_screens |
| Overlay: ResumeSession | Saved session | Confirm / dismiss | Message, actions | Enter / Esc | Bounded message | Selected action | Clamped card | draft_isolation_and_persistence; dump_overhaul_screens |

## Field inventory (`field_placeholder`)

All `FieldId` variants have a label plus placeholder. Empty focused fields keep the placeholder until typing. Secrets stay masked; placeholders never save.

| FieldId | Placeholder | Verified |
|---|---|---|
| BrainApp | Source app name | Yes |
| BrainConversation | Optional conversation ID | Yes |
| BrainInsight | Write a useful fact or finding… | Yes |
| BrainQuery | Search memories by topic… | Yes |
| ReconSearch | Find investigations… | Yes |
| IntelSearch | Search title, source, or topic… | Yes |
| OsintSearch | Find tools by name or purpose… | Yes |
| OsintInput | JSON inputs; see example above | Yes |
| JobsSearch | Find jobs by name or state… | Yes |
| LogsSearch | Filter messages or IDs… | Yes |
| RouterKey | Paste OpenRouter API key | Yes |
| GoogleKey | Paste Google AI Studio key | Tests: google_and_nvidia |
| NvidiaKey | Paste NVIDIA API key | Tests: google_and_nvidia |
| Router/Google/NvidiaEndpoint | API base URL, including version | Yes |
| *ModelFilter | Filter model names or IDs… | Tests: provider_catalog_filters_locally |
| Composer | Ask an OSINT question… | Yes |
| *Provider / *Model role fields | Choose a provider / Choose a model | Yes |
| Tool/news keys | Paste API key | Masked |
| *Fallback keys | Optional backup API key | Masked |

## Remaining limits

- Live Google/Nvidia catalog and thought-signature tool-call roundtrips were not exercised with real API keys.
- Raster `legacy_target_at` remains a fallback when draw has not populated the layout registry (unit tests that skip render).
- Atlas insight table still uses centered labels; origin tables are left-aligned.


See [overhaul verification](tui-overhaul-verification-2026-10-10.md) for fresh captures, visual review, interaction/metric results and live limitations. Regenerate non-Profile fixtures with `dump_overhaul_screens` and Profile fixtures with `analytics_viewports_and_data_states` through `scripts/tui_review.py`.
