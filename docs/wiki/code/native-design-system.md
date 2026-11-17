# Native workstation design

[Native client](../../../apps/fleetworks-desktop/README.md) · [Connections](native-connections.md) · [Identity](native-workspaces.md) · [UX verification status](../../../.planning/2026-11-13-native-immersive-ux/acceptance.md) · [Pairing verification](../../../.planning/2026-11-12-native-pairing-implementation/acceptance.md)

Rune macOS and Linux x86_64 client (`apps/fleetworks-desktop`). One selected destination; backend boundaries remain governed by the connection/runtime contracts. Intel Web shares paint/branding (`@tag:look-and-feel`) and workstation Connect while keeping its navigation and topology/list behavior.

**Approved target · 2026-11-17:** [Matte canvas theme](../../design/matte-canvas/theme.md) and [interaction/state contract](../../design/matte-canvas/interaction-state.md). Cool neutral text, symbol-only branding, diffuse grouped surfaces, flat semantic colors and narrowly eligible matte-rock actions supersede the earlier reflective/gold interface. This update changes documentation only; runtime integration and verification remain pending.

[![Native shell and selected destination](../../diagrams/native-workstation-layout.svg)](../../diagrams/native-workstation-layout.html)

## Historical runtime reference · November 2026

These [inspected Rune Runtime captures](https://github.com/xxxcess/argos-osint/pull/67#issuecomment-6879191295) come from the passing [macOS ARM CI job](https://github.com/xxxcess/argos-osint/actions/runs/43606326381) at `fddb8405`. They show synthetic UI states; the installed native app has a separate review phase.

| Wide workstation shell | Workstation Settings |
| --- | --- |
| [![Wide workstation shell](https://github.com/user-attachments/assets/4da95468-dd85-41e7-8e8d-ed8705f3b6ba)](https://github.com/user-attachments/assets/4da95468-dd85-41e7-8e8d-ed8705f3b6ba) | [![Workstation Settings dialog](https://github.com/user-attachments/assets/a72e60ce-ed34-40c7-8b31-0347d176afdc)](https://github.com/user-attachments/assets/a72e60ce-ed34-40c7-8b31-0347d176afdc) |
| Readiness checks | Compact paged jobs |
| [![Workstation readiness checks](https://github.com/user-attachments/assets/9137e43a-6853-4ab3-b310-aae4423d34de)](https://github.com/user-attachments/assets/9137e43a-6853-4ab3-b310-aae4423d34de) | [![Compact jobs page at 640 by 480](https://github.com/user-attachments/assets/5c63db9d-d177-4d04-b42f-0cd436510d87)](https://github.com/user-attachments/assets/5c63db9d-d177-4d04-b42f-0cd436510d87) |

The [compact Settings capture](https://github.com/user-attachments/assets/cd303831-684c-44bb-a4e4-1accf037b421) records the modal at the `640×480` minimum window size.

## Authority and references

Explicit user decisions govern. This page owns the maintained native control/design boundary; [migration/authority](native-connections.md) owns active wire semantics. The [matte canvas guide](../../design/matte-canvas/theme.md) is the canonical current paint/composition target, and its [four approved mockups](../../design/matte-canvas/theme.md#approved-composition-references) are synthetic references. [Interaction/state](../../design/matte-canvas/interaction-state.md) owns loading, retained updates and cursor continuation. The [supplied structural handoff](../../design/native-workstation-handoff/argos_workstation_identity_implementation_spec.md) and [original rationale](../../design/native-workstation-handoff/argos_native_design_system_guide.md) preserve their dated audit; their earlier two-step setup, control exclusions and gold/ivory recipes do not override these maintained contracts.

Source baseline: `nightly` commit `00b700afd77ddac6cba2e2bd7ae91ab3802b7197`, tree `5051474185a8eb14b38e607480bb24c65352fff0`. Pointer lighting, reflective finishes, rolling values and explicit page buttons described below are baseline facts, not approved target guidance. Historical CI captures above do not verify the new target.

Normal setup implements [pairing and macOS Keychain](native-connections.md#pairing-and-keychain-agreed-target). Explicit operator/headless file mode stays independent. Runtime/backend/live Keychain evidence remains separately tracked.

## Complete native control allowlist

Ordinary selection, progression, dismissal, confirmation and recovery belong to each listed control. The user-requested Workstation Settings dialog groups existing workstation and configuration routes; it adds no backend administration authority.

| Surface | Allowed controls and content |
| --- | --- |
| Shell | Actual native window controls; `Setup Gateway` (`ActionButton`, `connection-editor`); sidebar collapse; clickable `gateway-status-header` showing connection status; `Workstation Status` (`nav-workstation`) always visible; Workstation/Operations section labels and bottom `Settings` (`open-workstation-settings`) entry. When disconnected (`!state.connected()`), sidebar shows only `Workstation Status`, `Setup Gateway` (`nav-setup`) and Settings; `Page::ALL` and `⌘1–6` remain guarded to Workstation Status. When connected, sidebar reveals all six `Page::ALL` destinations with live busy status and count badges, and `scope_bar` shows resource scope + syncing/online pill. The selected destination occupies the center, with a read-only context rail on wide windows. |
| Workstation Settings | Centered `workstation-settings` dialog over the retained destination; compact General, Appearance, Gateway, Models and Tools category rail; bounded scrolling settings rows. General shows registration, readiness and deployment state; Appearance reflects macOS transparency/motion preferences; Gateway shows URL and opens Setup Gateway; Models and Tools open their existing destination controls. |
| Setup | 3-stage `stage_stepper` (`Origin URL` → `Workstation Pair` → `Gateway Ready`); Gateway URL → operator pairing approval → Keychain → authenticated checks/registration. Waiting request/code, Retry/Pair again, Change gateway URL and `setup-advanced-toggle` (`Switch` to reveal `readiness_cards::checks`) reflect the current stage; no user-managed token file. |
| Workstation Status, via header / default landing | Default home page (`Page::Workstation`, `setup_gateway: false`). When disconnected, renders only `Native Client` (`"Connect to a gateway to register this workstation."`) and `Gateway Backend` (with `workstation-setup-gateway` CTA, hiding `Host Resources` and `readiness_cards::checks`). When connected, renders `Host Resources` (clickable CPU/Memory/Disk `metric_ring_tile` controls opening a `Dialog` `modal_prompt` via `WorkstationInspector`), `Native Client` (with `Workstation registered` pill) and `Gateway Backend` (showing the deployment badge and expandable `readiness_cards::checks` tiles), and optional authoritative API-host percentages/platform/chipset. Eligible actor-catalog, index and classifier checks offer direct repair actions with existing permission and preflight guards. |
| DB Seed & Train | 3-stage `stage_stepper` + `SeedStage` (`Seed`, `Classifier`, `Maintenance`) `SegmentedControl`; preflight readiness relayed directly via each action button's inline status chip; actor/training CSV selection, validated upload and confirmed seed/Natural candidate jobs; index build only after completed seeding with actors present. Eligible submit/confirm controls use the target matte-rock material; genuine pending state uses only the restrained loading treatment. |
| News Cycles | `New Cycle`; compact cursor-paged cycle tiles expanding inline metadata; `Inspect summary` opening `Dialog` (`modal_prompt`); selected-cycle history Delete with in-place confirmation |
| Brain Search | One search field; bounded interactive actor tiles; selected authoritative dossier with `DossierFacet` (`Overview`, `Associations`) `SegmentedControl` and `Collapsible` alias/attribute sections; no native graph |
| OSINT Tools | Provider/service cards; interactive credential rows opening `Dialog` (`tool-api-key`) with masked primary/fallback inputs and explicit `Switch` toggles (`inline_toggle_row`) to clear primary or fallback slots |
| Model Assignments | `ModelsTab` (`Roles`, `Providers`) `SegmentedControl`; configured provider cards with inline masked keys and `Switch` preserve-if-blank toggle; canonical role cards expanding `role_row_editor` (`Select` pickers for provider, model, reasoning variant and ordered fallback Move up/Move down/Remove actions) |
| Jobs & Logs | Compact bounded workstation queue/logs, selectable job rows and truthful count/progress; `Pop out summary` opens `Dialog` (`modal_prompt`); `Retry job` (`job-retry-preview`) retains in-place confirmation (`job-action-confirmation`) and receipt strip for permitted failed retryable jobs; selection supplies authorized safe operational summaries in `safe_log_table`. Cursor continuation, retained rows and localized unresolved regions follow the [interaction/state target](../../design/matte-canvas/interaction-state.md), preserving membership and bounded requests. Existing Previous/Next controls are recorded as source baseline below. |

Sidebar order when connected is `Workstation Status` followed by `DB Seed & Train`, `News Cycles`, `Brain Search`, `OSINT Tools`, `Model Assignments`, `Jobs & Logs`, then Settings in the footer; when disconnected (`!state.connected()`), `Workstation Status`, `Setup Gateway` and Settings remain available (`Page::ALL` is hidden and `navigate()`/`⌘1–6` clamp to `Workstation Status`). Render only the selected destination behind the Settings dialog. There is no chat transcript or composer.

Excluded: separate Test/Disconnect, saved-profile CRUD, workspace/folder/Trash controls, classifier activation, account creation/deletion/base-URL forms, configuration import/export, native article filters, manual tool Run/input/results, and job/log cancel/filter/follow/copy/export toolbars. Backend APIs may retain these independent capabilities for authorized callers.

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
| Cycles | Bounded cursor-paged `GET /api/v1/atlas/cycles`; `POST /api/v1/atlas/cycles/delete` tombstones this workstation association, preserving shared evidence and durable audit. Deletion is not cancellation. Current summaries supply time/state and optional `article_count`; unreported pipeline counts stay unavailable. |
| Dossier | Canonical actor lookup, debounce/latest-response fences, identity/summary first, then supported claims/links. Missing confidence or summaries stay unavailable; long content scrolls in the inspector. |
| Tool keys | Independent slot patches: omitted/null preserves, explicit empty clears, nonempty replaces. Blank native fields preserve; Clear actions are explicit (`Switch` toggles in `tool-api-key`). Saved bytes are never returned. |
| Models | Server-advertised `configured_routes` catalog, not exhaustive live provider discovery. Provider selection resolves its account/model tuple. Effort and variant address the same supported `provider::model` setting. Preserve hidden fields, unchanged secrets and pending edits on conflict. |
| Activity | Server membership precedes rows, counts, paging, logs/results/streams/actions. Shared status does not imply private access. Never fall back globally when unregistered; summaries are not provider stdout. |

## Approved matte canvas target

The complete reproducible palette, edge-mask formula, texture recipe, action eligibility and loading timing live in [theme.md](../../design/matte-canvas/theme.md). Use its exact values in native and web rather than deriving UI colors from the logo.

| Concern | Required target |
| --- | --- |
| Canvas and branding | Uniform opaque `#060709`; material emblem with transparent/canvas-matched edges and preserved aspect. Symbol only in the interface: no visible Fleetworks wordmark. Keep OS metadata and necessary accessible naming. |
| Neutral hierarchy | Cool primary `#D5DCE5`, supporting `#A8B6C5`, muted `#8192A4`; group `#0B0F14`, raised `#121820`, elevated `#19212B`. No ivory/gold primary UI accents. |
| Group surfaces | Flat group background masked inward over 16 logical pixels with a smoothstep falloff and 20px outer radius; compact recipe 12px/16px. Content is drawn above the mask at full opacity. No bright card strokes, reflection, backdrop blur or ambient halo. |
| Essential controls | Preserve functional `#647587` borders; 2px `#72E9FF` focus plus 2px canvas gap, independent of diffuse decoration and selection. |
| Actions | Static coarse crescent-rock texture only on semantic submit/confirm/create/delete controls, including Connect. Context tints remain local. Cancel, Settings, navigation, filters, pop-out/inspect, read retry, metric and readiness surfaces stay flat. |
| Motion | Zero persistent sheen or pointer lighting; the emblem is the reflective-art exception. Only genuinely pending unresolved regions use the guide's low-alpha translated band over a steady base. Text/live values never blink, pulse or roll on refresh. |
| Color | Flat, diverse semantic and chart hues with labels/icons. Preserve severity, evidence and category meanings; unknown is never fabricated zero. |
| Layout | Responsive grouped grids for overview/options/status; retain tables for comparisons, existing reading order, keyboard traversal and supported compact layouts. |

### Existing implementation to adapt

The audited [`theme.rs`](../../../apps/fleetworks-desktop/src/theme.rs) still contains warm ivory/gold tokens; [`material.rs`](../../../apps/fleetworks-desktop/src/material.rs) implements reflective `MetalFinish` variants, `luminous_card`, `interactive_tile`, `apply_liquid_metal`, `configure_kit`, `skeleton_block` and `compact_toolbar`. `ActionButton` and status/metric/skeleton helpers live in [`ui_components.rs`](../../../apps/fleetworks-desktop/src/ui_components.rs). Reuse these shared ownership points; their names are not exceptions to the target.

Baseline `app.rs` updates `State::pointer_offset` and `shadow_offset` using a fixed `1180×780` reference. Material layers include specular/pointer glow, cursor shadows, hover sheen, spring/press effects and `LoopSpec::slide` loading bands. These are source facts to replace or remove in implementation, not recipes to reproduce. Do not retain a fixed-pixel pointer-lighting system under the new theme.

`MetalFinish::{GoldCta,Gunmetal,CrimsonDanger,ObsidianPill}` does not express submit/read/navigation semantics; explicit role metadata for eligible texture is proposed new work. Never texture every `ActionButton`. Native `BRAND_MARK` currently uses a diamond rather than the supplied crescent/sphere/five-triangle material identity; asset adoption must deliberately resolve that mismatch, with a legible small-size fallback. Other custom 16×16 Rune destination/action glyphs retain their roles.

Reuse [`Accessibility::for_view` and `ViewPreferences::matte()`](../../../apps/fleetworks-desktop/src/accessibility.rs). The target is opaque by default; reduced transparency keeps its readable content and hit bounds, reduced motion makes loading/static states immediate, and high-contrast needs override decorative masks/textures. Existing [`appearance_tests.rs`](../../../apps/fleetworks-desktop/src/appearance_tests.rs) is a baseline validation integration point, not evidence that the new recipes already pass.

## Layout, transitions and progressive disclosure

| Concern | Audited baseline and retained boundary | Approved target / implementation work |
| --- | --- | --- |
| Window and chrome | Initial `1440×860`; minimum `640×480`; `Sidebar` range `180..340` with 220 default; Workstation/Operations labels and Settings footer; read-only 236pt context rail when main width is at least 1120pt. | Preserve native window controls and destinations. Group grids use measured available content width; optional rail hides before essential content clips. |
| Compact layout | Toolbar titles shorten below 520pt content width; Brain list/detail switches below 720px; `safe_log_table` adapts below 320/500px; Settings has bounded scrolling. | Preserve order, selection, focus and scroll owners. Collapse groups without forcing horizontal page scrolling or shrinking text. |
| Transitions | Existing destination/dossier entry uses offset plus opacity, omitted for reduced motion. | No replay on polling, live updates or retained refresh. Follow the state companion for stable content and explicit navigation transitions. |
| Telemetry and loading | `async_status_strip`, `metric_ring_tile`, `RollingText`, `ProgressCircle` and `Progress` supply current state presentation. | Keep labels and retained values stable; localize unresolved skeletons; static numeral updates replace rolling refresh effects. Genuine reported progress stays distinct from indeterminate loading. |
| Pagination | Native Jobs & Logs currently renders explicit Previous page / Next page controls and cursor-scoped reads. | Use the approved continuation/recovery contract in [interaction-state.md](../../design/matte-canvas/interaction-state.md); preserve finite bounds, cursor identity and request deduplication. Do not describe that target as already implemented. |
| Navigation gating | Workstation Status/Settings remain available; disconnected setup hides `Page::ALL`; `navigate()` and `⌘1–6` guard hidden destinations. | Unchanged. Paint/grid adoption cannot expose unauthorized operations. |
| Progressive disclosure | Existing stage steppers, segmented controls, switches, collapsibles and allowed dialogs retain their responsibilities and keys. | Group/readability changes add no controls, mutation or backend authority. |
| Keyboard and states | Existing selection and local loading/empty/unsupported/denied/stale/failed/uncertain states remain distinct. | Crisp independent focus, source-order traversal and preference parity; unknown metrics stay unavailable, never invented zero. |

## Required evidence

The current change is documentation only; no new native/browser runtime pass is claimed. Implementation must inspect production Rune Runtime PNG/tree/input evidence for all seven surfaces plus setup and relevant failures at wide and `640×480` minimum sizes, 1×/2×, keyboard/scroll/selection and reduced preferences. Check masked backgrounds with crisp text/focus, role-limited rock textures, no persistent sheen, canvas-matched symbol branding, retained live content and finite cursor continuation. Supplied mockups are synthetic composition references. Generated-but-uninspected images, unit tests, installed GUI/accessibility, backend integration and hosted platform evidence remain separate claims. [Verification workflow](native-verification.md) · [UX overhaul verification](../../../.planning/2026-11-13-native-immersive-ux/acceptance.md) · [Pairing acceptance baseline](../../../.planning/2026-11-12-native-pairing-implementation/acceptance.md)
