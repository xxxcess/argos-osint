# Native workstation design

[Native client](../../../apps/fleetworks-desktop/README.md) · [Connections](native-connections.md) · [Identity](native-workspaces.md) · [UX verification status](../../../.planning/2026-11-13-native-immersive-ux/acceptance.md) · [Pairing verification](../../../.planning/2026-11-12-native-pairing-implementation/acceptance.md)

Rune macOS API client (`apps/fleetworks-desktop`). One selected destination; the backend owns data and execution. Intel Web shares paint/branding (`@tag:look-and-feel`) and workstation Connect while keeping its layout, typography, navigation and topology/list behavior.

[![Native shell and selected destination](../../diagrams/native-workstation-layout.svg)](../../diagrams/native-workstation-layout.html)

## Authority and references

Explicit user decisions govern. This page is the maintained control/design contract; [migration/authority](native-connections.md) owns active wire semantics. The unchanged [supplied handoff](../../design/native-workstation-handoff/argos_workstation_identity_implementation_spec.md) and [detailed design rationale](../../design/native-workstation-handoff/argos_native_design_system_guide.md) preserve source requirements and mockups. Their November 2026 baseline descriptions are historical, not current verification. The original structural reference's Tauri/Linux-client, workspace identity, sample hardware and Rust-BERT-training assumptions are superseded; the flat matte-only surface restriction is superseded by the adaptive liquid-metal and pointer-lighting system below with strict `matte()` parity.

Normal setup implements [pairing and macOS Keychain](native-connections.md#pairing-and-keychain-agreed-target). Explicit operator/headless file mode stays independent. Runtime/backend/live Keychain evidence remains separately tracked.

## Complete native control allowlist

Ordinary selection, progression, dismissal, confirmation and recovery belong to each listed control; they do not authorize extra administration pages.

| Surface | Allowed controls and content |
| --- | --- |
| Shell | Actual native window controls; `Setup Gateway` (`ActionButton`, `connection-editor`); sidebar collapse; clickable `gateway-status-header` showing connection status (opens `Workstation Status` when connected or `Setup Gateway` when disconnected); `Workstation Status` (`nav-workstation`) always visible; when disconnected (`!state.connected()`), sidebar shows ONLY `Workstation Status` and `Setup Gateway` (`nav-setup`), `views::page`/`toolbar`/`screen_key` guard `Page::ALL` and `⌘1–6` to `Workstation Status`, and `scope_bar` shows `Local Client` (`local-client-scope`) + status pill; when connected, sidebar reveals all six `Page::ALL` destinations with live busy status and count badges (no bottom sidebar footer card) and `scope_bar` shows resource scope + syncing/online pill |
| Setup | 3-stage `stage_stepper` (`Origin URL` → `Workstation Pair` → `Gateway Ready`); Gateway URL → operator pairing approval → Keychain → authenticated checks/registration. Waiting request/code, Retry/Pair again, Change gateway URL and `setup-advanced-toggle` (`Switch` to reveal `readiness_cards::checks`) reflect the current stage; no user-managed token file. |
| Workstation Status, via header / default landing | Default home page (`Page::Workstation`, `setup_gateway: false`). When disconnected, renders only `Native Client` (`"Connect to a gateway to register this workstation."`) and `Gateway Backend` (with `workstation-setup-gateway` CTA, hiding `Host Resources` and `readiness_cards::checks`). When connected, renders read-only `Host Resources` (clickable CPU/Memory/Disk `metric_ring_tile` controls opening a `Dialog` `modal_prompt` via `WorkstationInspector`), `Native Client` (with `Workstation registered` pill) and `Gateway Backend` (showing the deployment badge and expandable `readiness_cards::checks` tiles), and optional authoritative API-host percentages/platform/chipset |
| DB Seed & Train | 3-stage `stage_stepper` + `SeedStage` (`Seed`, `Classifier`, `Maintenance`) `SegmentedControl`; preflight readiness relayed directly via each action button's inline status chip; actor/training CSV selection, validated upload and confirmed seed/Natural candidate jobs; index build only after completed seeding with actors present; busy async job/upload buttons trigger a clipped liquid-metal `LoopSpec::slide` shimmer surface |
| News Cycles | `New Cycle`; cycle count badge and interactive cycle tiles expanding inline metadata; `Inspect summary` opening `Dialog` (`modal_prompt`); selected-cycle history Delete with in-place confirmation |
| Brain Search | One search field; bounded interactive actor tiles; selected authoritative dossier with `DossierFacet` (`Overview`, `Associations`) `SegmentedControl` and `Collapsible` alias/attribute sections; no native graph |
| OSINT Tools | Provider/service cards; interactive credential rows opening `Dialog` (`tool-api-key`) with masked primary/fallback inputs and explicit `Switch` toggles (`inline_toggle_row`) to clear primary or fallback slots |
| Model Assignments | `ModelsTab` (`Roles`, `Providers`) `SegmentedControl`; configured provider cards with inline masked keys and `Switch` preserve-if-blank toggle; canonical role cards expanding `role_row_editor` (`Select` pickers for provider, model, reasoning variant and ordered fallback Move up/Move down/Remove actions) |
| Jobs & Logs | Bounded workstation queue with `RollingText` count; interactive job tiles; `ProgressCircle` + determinate `Progress` bar; `Pop out summary` opening `Dialog` (`modal_prompt`); selecting a job supplies authorized safe operational summaries in `safe_log_table` |

Sidebar order when connected is `Workstation Status` followed by `DB Seed & Train`, `News Cycles`, `Brain Search`, `OSINT Tools`, `Model Assignments`, `Jobs & Logs`; when disconnected (`!state.connected()`), only `Workstation Status` and `Setup Gateway` appear (`Page::ALL` is hidden and `navigate()`/`⌘1–6` clamp to `Workstation Status`). Render only the selected destination.

Excluded: separate Test/Disconnect, saved-profile CRUD, workspace/folder/Trash controls, classifier activation, account creation/deletion/base-URL forms, configuration import/export, native article filters, manual tool Run/input/results, and job/log retry/cancel/filter/follow/copy/export toolbars. Backend APIs may retain these independent capabilities for authorized callers.

## Identity, setup and actions

[![Pairing, Keychain and authenticated reconnect](../../diagrams/native-connections.svg)](../../diagrams/native-connections.html)

| Contract | Required behavior |
| --- | --- |
| Installation | Protected settings initialize without a default child. Existing folders, markers and journals remain intact. |
| Connect | Opaque backend workstation ID; authenticated scope + installation marker establish identity. Optional mutable IP never merges installations or defines identity. |
| Credential | Origin-bound Keychain secrets after operator approval; revocable scoped Native authority. Settings/journals retain only references/recovery. Explicit file mode never becomes a fallback. |
| Authority change | Fence old requests, caches, live responses and pending confirmations. Keep permitted shared reads usable when registration/optional services are unavailable. |
| Mutation | Explicit action + current permission/capability/readiness + context/revision/idempotency fences. Accepted/queued is not completed. Navigation never submits work. |
| Uncertain delivery | Retain original request ID/body/context/registration. Read-only reconciliation never creates replacement work. |

Setup shows one actionable state: pairing approval, Keychain access or network progress. Registration/readiness follows successful authentication. Focus loss pauses transport; URL changes/navigation fence callbacks while retaining recovery. No CLI flag error paragraphs in normal setup. Source implementation is distinct from inspected Runtime, backend and live Keychain evidence.

## Destination contracts

[![Readiness, configured inputs and durable maintenance](../../diagrams/workstation-readiness.svg)](../../diagrams/workstation-readiness.html)

| Area | Data and effect boundary |
| --- | --- |
| Seed/index/train | Server-configured validated inputs and exact configuration revision. Missing/stale inputs disable only their operation. Natural training produces a candidate; activation and Rust-BERT inference are separate facts. |
| Cycles | Bounded `GET /api/v1/atlas/cycles`; `POST /api/v1/atlas/cycles/delete` tombstones this workstation association, preserving shared evidence and durable audit. Deletion is not cancellation. Current summaries supply time/state and optional `article_count`; unreported pipeline counts stay unavailable. |
| Dossier | Canonical actor lookup, debounce/latest-response fences, identity/summary first, then supported claims/links. Missing confidence or summaries stay unavailable; long content scrolls in the inspector. |
| Tool keys | Independent slot patches: omitted/null preserves, explicit empty clears, nonempty replaces. Blank native fields preserve; Clear actions are explicit (`Switch` toggles in `tool-api-key`). Saved bytes are never returned. |
| Models | Server-advertised `configured_routes` catalog, not exhaustive live provider discovery. Provider selection resolves its account/model tuple. Effort and variant address the same supported `provider::model` setting. Preserve hidden fields, unchanged secrets and pending edits on conflict. |
| Activity | Server membership precedes rows, counts, paging, logs/results/streams/actions. Shared status does not imply private access. Never fall back globally when unregistered; summaries are not provider stdout. |

## Brand palette, liquid-metal surfaces and matte parity

Implemented in [`theme.rs`](../../../apps/fleetworks-desktop/src/theme.rs), [`icons.rs`](../../../apps/fleetworks-desktop/src/icons.rs), [`material.rs`](../../../apps/fleetworks-desktop/src/material.rs) and [`ui_components.rs`](../../../apps/fleetworks-desktop/src/ui_components.rs) using the `--locked` `rune` + `rune-kit` + `rune-motion` GPU gradient and shadow pipeline (retaining `Cargo.lock` without adding unlocked `rune-fx`/`rune-light` crates).

### Palette and semantic tokens (`theme.rs`)

| Token / role | Hex / value | Purpose |
| --- | --- | --- |
| `BACKGROUND` / `SURFACE_INSET` | `#060709` | Window obsidian canvas and inset wells |
| `SURFACE` / `SURFACE_RAISED` / `SURFACE_ELEVATED` | `#0B0D11` / `#12151C` / `#181C26` | Card base, raised interactive tiles and elevated hover/dialog layers |
| `TEXT_PRIMARY` / `TEXT_SECONDARY` / `TEXT_MUTED` | `#F7F3EA` / `#DCD6C8` / `#A8A296` | High-contrast ivory primary ink, supporting copy and quiet captions |
| `ACCENT` / `ACCENT_PRESSED` / `ACCENT_HOVER` / `ACCENT_HIGHLIGHT` / `ACCENT_DEEP` | `#C6A56E` / `#C4A265` / `#E5C993` / `#F2DFB6` / `#8E6E3B` | Brand gold metallic ramp, focus ring (`ACCENT_HIGHLIGHT`) and `ON_ACCENT` (`#060709`) ink |
| `SELECTION_SURFACE` | `#322C22` | Selected nav/pill fill |
| `BORDER_CONTROL` / `BORDER_SUBTLE` | `#948B7B` / `#3A3833` | Essential control boundary vs decorative card framing |
| Gunmetal & lighting stops | `METAL_DARK_TOP` `#222631`, `METAL_DARK_MID` `#14171F`, `METAL_DARK_BOTTOM` `#0D0F14`, `METAL_EDGE_TOP` `#5A5448`, `METAL_EDGE_BOTTOM` `#24211C`, `LIGHT_SURFACE_GLOW` `#F7E8C9`, `GOLD_GLOW` `#D8BD8C`, `CAST_SHADOW` `#020304` | Multi-stop metallic gradients, ambient luminous rim/glow and directional cast shadows |
| Semantic `StatusTone` (`Ok`, `Warn`, `Err`, `Info`) | `#5FD39A` (`#10241B` / `#2B6E4E`), `#E5B45B` (`#282010` / `#7A5D28`), `#F06A8A` (`#291219` / `#7D2B3F`), `#5AB8F0` (`#10202D` / `#2B5E80`) | Aligned with web semantic tokens (`StatusTone::{fg, bg, border}`) |
| Entity `StatusTone` (`Violet`, `Cyan`) & entity constants | `ENTITY_GRAPH` `#B48EFF` (`Violet`), `ENTITY_TECH` `#21D9EE` (`Cyan`) at 14% fill / 45% border; plus `ENTITY_PERSON` `#C08CF5`, `ENTITY_ORG` `#5AB8F0`, `ENTITY_COMPUTE` `#FFB347`, `ENTITY_CLUSTER` `#CAD9F0` | Dossier subtypes, OSINT tool categories and telemetry categorization |
| Decorative edge alpha | `CARD_EDGE_ALPHA` `0.04` / `SATIN_CONTROL_EDGE_ALPHA` `0.07` | Subtle inset rim; never substitutes for `BORDER_CONTROL` or focus rings |

### Vector icons (`icons.rs`)

All shell destinations, telemetry tiles and action buttons use custom 16×16 (`view: 16.0`) `rune::Icon` SVG glyphs: `BRAND_MARK`, `WORKSTATION`, `SEED`, `ATLAS`, `BRAIN`, `TOOLS`, `MODELS`, `JOBS`, `CPU`, `MEMORY`, `DISK`, `KEY`, `SHIELD_CHECK`, `ALERT`, `SERVER`, `REFRESH`, `TRASH`, `CHEVRON_RIGHT` and `PLUS`.

### Liquid-metal buttons, luminous surfaces and `matte()` parity (`material.rs`, `app.rs`)

| Primitive / mechanism | Active behavior (`!preferences.matte()`) | Accessible fallback (`preferences.matte()`) |
| --- | --- | --- |
| `ActionButton` + `MetalFinish` (`GoldCta`, `Gunmetal`, `CrimsonDanger`, `ObsidianPill`) | Brushed low-sheen 4-stop `linear_gradient(PI * 0.92, ...)`, softened `border_gradient`, `continuous_corners()`, dual `box_shadow` (drop shadow + dim ambient glow), `a11y_hidden` `liquid-specular` (`group_opacity(0.02, 0.55)` + 140px radial gradient with `live_offset(pointer_offset)`) and dim `liquid-pointer-light` (150px radial falloff illuminated by cursor), `press_scale(0.965)` and `hover_motion(Spring::new(0.18, 0.12), Spring::new(0.28, 0.18))`. Optional inline preflight status chip renders on the button, and busy/async job state renders an inline `Spinner`, disables clicks, and triggers a clipped `liquid-shimmer` (`LoopSpec::slide`) sweep unless `reduced_motion`. | Solid opaque fills (`ACCENT`, `SURFACE_RAISED`, `STATUS_ERR_BG`), 1px solid borders (accent border when active/busy), no specular/pointer-light/shimmer overlay, no `press_scale` or spring hover motion. |
| `luminous_card` & `interactive_tile` | `SURFACE` (`#0B0D11`) / `SURFACE_RAISED` (`#12151C`) fill, softened `border_gradient`, `radius(10.0).continuous_corners()`, drop shadow + dim `LIGHT_SURFACE_GLOW` / tone halo, clipped `a11y_hidden` `pointer-glow` (210px dim radial falloff, `live_offset(&state.pointer_offset)`) and opposing `cast-shadow` (190px, `0.22` alpha, `live_offset(&state.shadow_offset)`). `interactive_tile` adds `press_scale(0.985)`, spring `hover_motion` and low-sheen `hover-metal-sheen` (`group_opacity(0.0, 0.42)`). | Flat opaque fill (`SURFACE` / `SURFACE_RAISED`), 1px solid border (`BORDER_SUBTLE` or `tone.border()`), zero shadow/glow layers, identical outer bounds and `accessibility_json()`. |
| Pointer tracking (`app.rs`) | `rune_kit::root(...).on_pointer_move_local(...)` normalizes pointer coordinates against `1180×780`, updating `State::pointer_offset` (`[-28..28, -19..19]`) and opposing `State::shadow_offset` (`[-8..8, -7..7]`) with `tx.live_only()` so the dim cursor light glides across liquid-metal surfaces without rebuilding the view tree. | `pointer_ref` and `shadow_ref` are `None`. |
| `compact_toolbar` & `configure_kit` | `configure_kit` sets button/raised/overlay elevation and sheen tokens; `compact_toolbar` passes toolbar buttons through without adding reflection wrappers. | `configure_kit` clears `submit_highlight`, `depth_top`, `depth_bottom` and glow/drop shadows, keeping 1px functional inset edges. |
| Brand mark & reduced preferences | Canonical `BRAND_MARK` / [`fleetworks-mark.svg`](../../../assets/brand/fleetworks-mark.svg) geometry preserved; `reduced_transparency || matte_fallback` triggers `matte()`; `reduced_motion` propagates via `cx.reduced_motion(true)`. | Identical layout hit-bounds (`presented(key)`) and `accessibility_json()` verified across `Page::ALL` in [`appearance_tests.rs`](../../../apps/fleetworks-desktop/src/appearance_tests.rs). |

## Layout, transitions and progressive disclosure

| Concern | Implemented behavior |
| --- | --- |
| Window & chrome | Initial `1180×780`; minimum `640×480`; `Sidebar` `PaneState::new(220.0).range(180.0, 340.0)`; full-size content chrome with macOS traffic lights (`launch.rs`). |
| Compact layout | `Brain Search` switches from a `340px` results split + detail pane to single-column list/detail navigation below `720px` content width; `safe_log_table` adapts columns below `320px` and `500px`; single scroll owner per pane. |
| Transitions | Destination container (`screen-setup` or `screen-<key>`) animates with `Transition::offset(0.0, 12.0).combined(Transition::opacity())`; wide `Brain Search` `dossier-pane-*` animates with `Transition::offset(8.0, 0.0).combined(Transition::opacity())`. Both are omitted when `preferences.reduced_motion` is true. |
| Async & live telemetry | `async_status_strip` pairs a `Spinner` + indeterminate `Progress::new(None)` bar during active requests with a static `status_dot` when settled. `metric_ring_tile` pairs `ProgressCircle` (34px) + `RollingText` for valid `0.0..=100.0` host percentages (`N/A` icon fallback when unavailable). `Jobs & Logs` pairs `RollingText` total count with `ProgressCircle` + determinate `Progress::new(Some(frac)).show_value()`. |
| Sidebar indicators & navigation gating | Always shows `Workstation Status` (`nav-workstation`); when `!state.connected()`, shows only `Workstation Status` + `Setup Gateway` (`nav-setup`) and guards `navigate()` / `⌘1–6` against hidden destinations; when connected, reveals `Page::ALL` with `sidebar_row_status` (`RowStatus::Busy`) and `sidebar_row_badge` live counts on `News Cycles` and `Jobs & Logs` (no bottom footer card). |
| Progressive disclosure & modals | `stage_stepper` + `SegmentedControl` (`SeedStage`, `ModelsTab`, `DossierFacet`); button-relayed preflight status chips on `DB Seed & Train`; `inline_toggle_row` (`Switch`) on `Setup Gateway`, `OSINT Tools` and `Model Assignments`; `Collapsible` alias/attribute blocks on `Brain Search`; `Dialog` (`modal_prompt`) inspectors for `WorkstationInspector` (`workstation-telemetry`), `selected_cycle_id` (`cycle-inspector`), `selected_job_id` (`job-operational-summary`) and `tool-api-key`. |
| Keyboard & states | Top-left → bottom-right traversal with `⌘1–6` page shortcuts when connected (`app.rs`), `2px` `ACCENT_HIGHLIGHT` focus rings, independent selection states, and distinct local loading/empty/unsupported/denied/stale/failed/uncertain representations. Unknown metrics stay `N/A` / unavailable, never invented zero. |

## Required evidence

Inspect actual production Rune Runtime PNG/tree/input evidence for all seven surfaces plus setup and relevant failure states at both sizes (`1180×780` / `1280×800` and `640×480`), keyboard/scroll/selection, reduced preferences and `matte()` visual parity. Supplied mockups are synthetic composition references. Generated-but-uninspected images, unit tests, installed GUI/accessibility, backend integration and hosted dual-Mac evidence are separate claims. [Verification workflow](native-verification.md) · [UX overhaul verification](../../../.planning/2026-11-13-native-immersive-ux/acceptance.md) · [Pairing acceptance baseline](../../../.planning/2026-11-12-native-pairing-implementation/acceptance.md)
