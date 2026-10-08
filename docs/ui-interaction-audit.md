# Argos UI Interaction Audit

Inventory of Home, the nine apps, popups, and field IDs from `ModuleId`, `Overlay`, `FieldId`, `ButtonId`, and `Target` in `crates/argos-osint-bin/src/tui/app.rs`.

- Layout: `ui.rs` `LayoutRegistry`
- Tab order: `(top, left)` of registered rectangles after draw
- Placeholders: `field_placeholder`
- Long panes: `draw_see_more` (`see more` footer)
- Focused panes append ` · focused` via `focused_pane`

## Shared contracts

- Tab/Shift+Tab: `focus_order` sorts registered geometry, then raster fallback when the registry is empty.
- Popups: `LayoutRegistry::push_scope` traps hits; Escape closes and restores the invoker.
- Wheel: region under the pointer; keyboard scrolling belongs to the focused inner pane.
- Viewports checked in tests: 160×50, 120×40, 100×36, 80×24.

## Audit Matrix

| Screen / Overlay | State Variant | Targets / Controls | Visual Order | Input Hints | Scroll / Overflow | Focused Style | Narrow Layout | Verification |
|---|---|---|---|---|---|---|---|---|
| Home | Launcher | App(0..8), Composer, Send | Top-down launcher then composer | Ask an OSINT question… | Composer wraps; Home list fits | Selected row + field gutter | Centered stack, 80×24 | Tests: home_order, home_composer |
| Home | Session tabs | Header tabs when threads exist | Left-to-right | — | Horizontal clip | Selected tab | Tabs shrink | Tests: session_tabs_open_close_reopen |
| Intel bulletin | Empty/filtered/list | IntelTab, IntelDay, IntelSearch, IntelArticle | Tabs, day, search, list | Search title, source, or topic… | List scroll | Selected article | Tabs wrap | Tests: intel_opens_bulletin_filters_and_opens_briefing |
| Intel briefing | Idle | Preview, Reload, full article, mode, Summary, View full report, jobs | Center stack then right jobs | — | Center stack + full-article inner scroll; `see more` on long regions | Section heading ` · focused` (middle column borderless) | Stack scrolls; jobs stay | Tests: selected_intel_busy_hides_only_this_article_controls |
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
| Tools | List/docs/run | OsintSearch, tools, OsintDetail, keys, input, Run…Docs | Search, list, detail, keys, input, actions | Find tools… / JSON inputs / Paste API key / Optional backup | Detail `see more` | tool · focused | Detail card when short | Tests: tool_documentation_heading_is_visible_for_filtered_selection |
| Models Defaults | Role assignment | Tabs, role, Provider, Model, Save, Refresh | Tabs then role form | Choose a provider / Choose a model | Role note | Selected role | Role buttons wrap | Tests: defaults_pick_provider_and_model_from_account_access |
| Models Google/Nvidia/OpenRouter | Key + catalog | Key, Save, Verify, Show endpoint, endpoint, filter, Refresh, catalog | Form then catalog | Paste … key / API base URL / Filter model names or IDs… | Catalog `see more` | Models · N of M | Endpoint hidden until shown | Tests: google_and_nvidia, provider_catalog_filters_locally |
| Profile | Hardware | Refresh hardware, paths | Actions, panes | — | Path panes | — | Stack | Tests: system_shows_only_hardware |
| Overlay: Help | Shortcuts | Close, SeeMorePopup | Centered card | — | Markdown `see more` | Shortcuts · focused | 24–76×8–28 | Keyboard ? |
| Overlay: Memories | Synthesis memories | Close, SeeMorePopup | Card | — | Scroll | Memory · focused | Same | Brain mark |
| Overlay: Block | Generic/report | Close, SeeMorePopup, Atlas news on run cards | Report uses 70% width | — | Scroll | Detail · focused | Clamp | Tests: full_report_popup |
| Overlay: Choice | Provider/model/day | Choice rows, Close | List | — | Popup list | Selected choice | Note + list | Defaults pickers |
| Overlay: Palette | Commands | Query, items | Query then list | Type to filter | List | Selected item | Centered | Tests: unavailable_palette_action |
| Overlay: IntelRecon | Mode + sections | Toggles, start, Close | Title, sections, actions | — | Popup | Selected toggle | Clamp 40–88 | Mode button |

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
