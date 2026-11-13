# Fleetworks Native Immersive UX — Design Specification

[Task plan](task_plan.md) · [Decisions](decisions.md) · [Findings](findings.md)

Grounded in `anomalyco/rune` revision `205b8d4` (`crates/rune`, `crates/rune-kit`, `crates/rune-motion`, `crates/rune-fx`, `crates/rune-light`) and tailored to `apps/fleetworks-desktop`.

---

## 1. Visual Language, High-Contrast Brand Palette & Lighting

### 1.1 Color Tokens (`apps/fleetworks-desktop/src/theme.rs`)
Rooted in [fleetworks-mark.svg](../../assets/brand/fleetworks-mark.svg) (`#060709`, `#F3EEE4`, `#C6A56E`, `#C4A265`) and harmonized with [intel-theme.ts](../../packages/intel/src/shell/intel-theme.ts):

| Role | Token | Hex | Purpose & Contrast Note |
| --- | --- | --- | --- |
| Deep Canvas | `BACKGROUND` | `#060709` | Window / split-view obsidian void |
| Base Card | `SURFACE` | `#0C0E12` | Primary cards & sidebar surface (lifted slightly from `#0B0C0F` for clear separation over `#060709`) |
| Raised Surface | `SURFACE_RAISED` | `#15181F` | Elevated interactive tiles, inputs, neutral button base |
| Hover Surface | `SURFACE_HOVER` | `#1C2029` | Active hover plate & focus container |
| Inset Well | `SURFACE_INSET` | `#08090C` | Nested checklists, code/log wells, progress tracks |
| Subtle Rim | `BORDER_SUBTLE` | `#47433B` | Card dividers & decorative hairlines (stronger contrast than `#3A3833`) |
| Control Bevel Rim | `BORDER_CONTROL` | `#9C9382` | Essential interactive boundaries & metallic rims (raised from `#898274`) |
| Primary Text | `TEXT_PRIMARY` | `#F3EEE4` | Brand ivory (~15:1 over `#0C0E12`) |
| Supporting Text | `TEXT_SECONDARY` | `#D6D2C4` | Secondary labels & metadata |
| Muted Text | `TEXT_MUTED` | `#A9A497` | Captions, timestamps, unavailable placeholders |
| Primary Gold | `ACCENT` | `#C6A56E` | Brand rays; primary liquid-metal base |
| Hover Gold | `ACCENT_HOVER` | `#E0C696` | Luminous ivory-gold specular crest & focus outline |
| Pressed / Crescent Gold | `ACCENT_PRESSED` | `#B38D4F` | Deep brushed gold pressed state & metallic shadow stop |
| Bronze Shadow Stop | `ACCENT_DEEP` | `#7A5E31` | Lower bevel gradient stop for gold liquid metal |
| Selection Surface | `SELECTION_SURFACE` | `#322C22` | Active nav / list row tint paired with `#C6A56E` rim |

### 1.2 Brand-Harmonized Status & Label Diversity
Status and categorical badges preserve the warm obsidian/mineral tone of the brand without neon clashing:

| Semantic / Domain Role | Foreground (`fg`) | Tint Fill (`bg`) | Rim (`border`) | Usage |
| --- | --- | --- | --- | --- |
| Ready / Verified / Complete | `#5FD39A` | `#0F271E` | `#2B6B4D` | Connected gateway, healthy checks, completed jobs, verified datasets |
| Pending / Loading / Uncertain / Stale | `#E5B45B` | `#291F0F` | `#7A5E29` | In-flight async requests, pairing wait, `OutcomeUnknown`, stale checks |
| Error / Failed / Denied / Destructive | `#F06A8A` | `#2B111A` | `#7A293F` | Failed requests, denied permissions, delete confirmations |
| Info / Telemetry / Active Stream | `#5AB8F0` | `#0F2130` | `#285C80` | Active Connect ID, host metrics, fixture banner, informational cues |
| Inference / Model Role | `#B48EFF` | `#1E1630` | `#4E397A` | Model assignments, effort variants, fallback chains |
| Search / Claims / Index | `#21D9EE` | `#0D252B` | `#216A75` | Brain Search hits, Tantivy index status |
| Actor / Dataset / Seed | `#FFB347` | `#2B1D0D` | `#754E1E` | Actor CSVs, Natural candidate training, entity pills |
| Host / Hardware / Source | `#CAD9F0` | `#171D26` | `#425169` | Chipset, OS arch, provider badges |

### 1.3 Liquid Metal Buttons, Light-Area Glow & Pointer-Cast Shadows
Grounded in `rune_fx::LiquidMetalParams`, `rune_fx::GlowParams`, `rune_light::{Light, Prim, LightRenderer}`, and `examples/{showreel,cursed-settings,site-scenes,live-values}`:

1. **Liquid Metal Button Finish (`src/material.rs` + `src/fx_compositor.rs`)**:
   - **Primary / Submit CTA (`LiquidMetalGold`)**: `El::effect(MARK_METAL_GOLD)` backed by `LiquidMetalParams::gold()` tuned to Fleetworks stops (`tint: 0xC6A56E`, `light: 0xF3EEE4`, `bevel: 10.0`, `dome: 0.45`, `flow: 0.22` at rest → `0.55` on hover, `iridescence: 0.06`, `contrast: 1.18`, `anchor: FillAnchor::Element`). Crisp dark `#060709` label + icon on top.
   - **Secondary / Interactive Tile (`LiquidMetalObsidian`)**: `El::effect(MARK_METAL_DARK)` backed by `LiquidMetalParams::gunmetal()` tuned to `#171A21` / `#2A2E38` (`tint: 0x262A34`, `light: 0xE0C696`, `bevel: 8.0`, `dome: 0.35`, `flow: 0.14` at rest → `0.42` on hover, `contrast: 1.25`).
   - **Hover Motion**: Every liquid-metal button attaches:
     - `El::hover_motion(tok::motion::SNAPPY, tok::motion::SETTLE)`
     - `El::press_scale(0.97)` + subtle hover scale/lift (`scale_motion` or `offset_motion` + specular intensity bump via `effect_data` `[hover_mix, press_mix, glow_mix, seed]`)
     - `El::on_hover_change` / `El::on_pointer_move_local` updating `Live` state so hover highlights glide smoothly with zero layout shift.
   - **Non-Compositor / Reduced-FX Fallback**: Multi-stop `El::linear_gradient` + `El::border_gradient` (`#E0C696` top specular rim → `#7A5E31` bottom rim) + `El::radial_gradient` tracking `Live<[f32; 2]>` local pointer coordinates.

2. **Faint Glow on Light UI Areas & Pointer-Tracking Cast Shadows**:
   - Light UI elements (ivory `#F3EEE4` active emblems, `#E0C696` gold active badges/headers, luminous status pills, and `LiquidMetalGold` triggers) emit a faint warm radial glow (`GlowParams { style: GlowStyle::Soft, color: 0xE0C696 / 0xF3EEE4, intensity: 0.18..0.32, radius: 12.0 }` or soft `Shadow` halo).
   - When the cursor moves (`rt.pointer()` in `Compositor` / `on_pointer_move_local`), a soft warm point light (`rune_light::Light::point(px, py, 140.0 * scale)` tinted `#F3EEE4`/`#E0C696`) illuminates nearby raised cards and liquid-metal buttons (`Prim::from_rune(rect, radii).raised(4.0).bevel(4.0, 1.2).gloss(0.65, 0.3)`), casting real-time directional soft SDF shadows and moving rim highlights away from the cursor—especially pronounced when hovering over light/gold UI zones!
   - **Idle & Accessibility Budget**: When the pointer is stationary, animations have settled, and no `Spinner`/`Loader` is active, `Compositor::animating()` returns `false` so Rune sleeps at 0% GPU/CPU. Under `preferences.matte()` or `reduced_motion`, pointer light and shader flow freeze cleanly to deterministic static lighting.

---

## 2. Motion Choreography, Transitions & Async Feedback

| Mechanism | Rune / rune-kit API | Fleetworks Behavior | Reduced-Motion Behavior |
| --- | --- | --- | --- |
| **Page / Screen Switch** | `El::transition(Transition::offset(0.0, 10.0).combined(Transition::opacity()).combined(Transition::scale(0.985)))` keyed by `state.page` + `state.setup_gateway` | Smooth vertical micro-glide + cross-fade when switching sidebar pages or entering Setup Gateway | Instant cut (settled immediately via `cx.reduced_motion(true)`) |
| **Active Sidebar Pill** | `El::matched("sidebar-active-pill").matched_crop()` (`examples/matched/src/main.rs`) | Gold indicator rail/pill glides seamlessly from previous `SidebarRow` to newly selected `SidebarRow` | Snaps directly to selected row |
| **Sidebar Collapse & Detail Panes** | `PaneState` + `El::animate_layout()` + `El::hold_landing_width()` | Main workstation content reflows once and glides cleanly when sidebar toggles or inspector drawers open | Instant resize |
| **Card → Detail / Modal Zoom** | `El::matched(format!("card-{}", id))` or `Dialog` + `Transition::scale(0.96)` | Clicking a cycle card, job row, provider card, or readiness tile smoothly opens its detail sheet or `Dialog` prompt | Instant mount |
| **Staggered Dashboard Entrance** | `El::enter_delay(Duration::from_millis(i * 35))` + `Transition::offset(0.0, 8.0)` | Hero KPI tiles / cards cascade cleanly on page entry without blocking interaction | Zero delay |
| **Async Loading & Polling** | `Spinner`, `Loader`, `Progress` (indeterminate & determinate), `ProgressCircle`, `ButtonVariant::Loading`, `El::looping(LoopSpec)` | Every `Load::Pending`/`Loading`, `SubmissionPhase::{Preparing, Posting}`, and pairing poll renders an animated spinner/pulse + concise status badge (`role("status")`) | Static indicator icon + `"Loading…"` status text |
| **Live Telemetry & Counters** | `RollingText` (`examples/odometer/src/main.rs`) + `ProgressCircle` / horizontal meter bars | Host CPU/Memory/Disk %, job counts, and readiness fresh-age counters roll smoothly when updated | Immediate numeric update |

---

## 3. Workstation Dashboard Layout & Creative Sidebar Context Controls

### 3.1 Shell Architecture (`src/app.rs`)
Retains `SplitView` with the canonical 6-item sidebar order and gateway status header, reimagined as a high-contrast workstation cockpit:
1. **Top Brand & Workstation Status Pill (`gateway-status-header`)**:
   - Displays canonical [fleetworks-mark.svg](../../assets/brand/fleetworks-mark.svg) (compact 18×18 SVG), deployment label, live pulsing/solid status dot (mint emerald = Ready, amber = Pairing/Degraded, rose = Error/Offline), and a compact Connect ID badge. Clicking opens `Workstation Status`.
2. **Icon-Led Navigation Rail (`SidebarRow`s)**:
   - Each of the 6 destinations carries a crisp 16px vector `Icon` + concise title + trailing `Badge` or mini status indicator (e.g., active job count `Spinner`/badge on `Jobs & Logs`, configured provider count on `Model Assignments`, readiness dot on `DB Seed & Train`).
   - Active item hosts the `El::matched("sidebar-active-pill")` gold accent marker + subtle luminous rim.
3. **Creative Sidebar Context Panel (`SidebarFooter` / contextual slot)**:
   - **Rule**: *Only shows controls & telemetry relevant to the currently selected page or active control!*
   - Examples by page:
     - `Workstation Status` / `Setup Gateway`: Quick Gateway URL chip + Refresh Readiness icon trigger + Setup Gateway liquid-metal trigger.
     - `DB Seed & Train`: Active 3-step pipeline mini-checklist (`CSV` · `Upload` · `Index`) showing live checkmarks/spinners.
     - `News Cycles`: Active cycle count + `OutcomeUnknown` recovery shortcut if a submission is unresolved.
     - `Brain Search`: Index readiness pill (`Tantivy` status icon + actor count) + clear-query action when query non-empty.
     - `OSINT Tools`: Configured vs unconfigured credential ratio (`Progress` mini-bar).
     - `Model Assignments`: Active role pill + pending unsaved-change count badge.
     - `Jobs & Logs`: Live queue summary (`Running` / `Failed` / `Total` `RollingText` pills) + selected job quick-badge.
4. **Slim Workstation Toolbar**:
   - Page icon + title + concise `Badge` for resource scope (`Backend-wide` vs `Workstation · <Connect ID>`) + right-aligned liquid-metal `Setup Gateway` button (using `IconButton` / compact `Button` with tooltip).

---

## 4. Screen-by-Screen Progressive Disclosure & Component Mapping

Each screen replaces long stacked prose walls with **concise visual cards, inline checklists/toggles, icon-first metrics, and click-to-reveal drawers or `Dialog` prompts**.

### 4.1 Setup Gateway (`setup_gateway` view)
- **Layout**: Centered 3-step progressive stepper card (`max-w: 560pt`) with top `Checkbox`/step-indicator pills (`1. Gateway` → `2. Pair` → `3. Register`).
- **Incremental Reveal**:
  - **Step 1 (Gateway URL)**: Only URL `TextInput` + liquid-metal `Connect & Pair` CTA + collapsible `Collapsible::new("Operator token-file options", ...)` (collapsed by default so normal setup stays ultra-clean).
  - **Step 2 (Operator Approval)**: Once pairing starts, Step 1 collapses into a crisp checked summary chip with a `Change URL` ghost trigger; Step 2 expands with pairing code badge, live `Spinner` + `"Waiting for approval…"` status pill, and `Retry` / `Pair again` liquid-metal button.
  - **Step 3 (Keychain & Registration)**: Reveals origin-bound Keychain check icon + Workstation registration confirmation & `Enter Workstation` primary CTA.

### 4.2 Workstation Status (`Page::Workstation`)
- **Layout**: 3-column bento KPI row at top + interactive Readiness Matrix below (no tall text dump).
  1. **Native Client Tile** (clickable → opens `Dialog` with full version, arch, registration receipt, and `Copy Connect ID` liquid-metal button): Icon + OS/arch badge + Connect ID pill + registration status icon.
  2. **Gateway Backend Tile** (clickable → opens Readiness Breakdown `Dialog` or refreshes checks): Origin chip + deployment badge + overall readiness `ProgressCircle` (e.g., `4/4` checks passed).
  3. **Host Resources Tile**: Visual telemetry meters for **CPU**, **Memory**, **Disk** using `Progress` / `ProgressCircle` + `RollingText` percentages + color-coded metric icons (`#5FD39A` CPU, `#C08CF5` Disk, `#5AB8F0` Memory), plus compact `platform` / `chipset` pills. Unavailable telemetry renders a muted dash pill (`—`) rather than prose.
- **Readiness Chips Strip**: Compact horizontal grid of readiness check pills (`Icon` + short title + status dot). Clicking any check pill reveals its exact observation message and freshness age in an inline `Popover` or detail strip.

### 4.3 DB Seed & Train (`Page::Seed`)
- **Layout**: Visual 3-stage pipeline (`1. Dataset` → `2. Upload` → `3. Seed & Index`) using a clickable `SegmentedControl` or step-cards + live pre-flight `Checkbox` checklist.
- **Incremental Reveal**:
  - Top bar: Stage selector + `Refresh checks` icon button (shows `Spinner` while refreshing).
  - **Pre-flight Checklist**: Compact `Checkbox` indicators (`Dataset selected`, `Validated`, `Actors seeded`, `Search index ready`) — read-only visual state pills that jump to or highlight the required stage on click.
  - Only the **active stage** expands its action controls (CSV picker button, Upload liquid-metal CTA with `Progress` bar, or Seed/Natural Train/Index Build triggers).
  - Triggering a maintenance operation opens a crisp **`Dialog` confirmation prompt** showing target dataset/revision metadata instead of injecting inline confirmation walls; `MaintenanceReceipt` renders as a compact success/status card with check icon.

### 4.4 News Cycles (`Page::Atlas`)
- **Layout**: Top hero strip (liquid-metal `New Cycle` CTA + status badge / `Spinner` if `Preparing`/`Posting` / amber `Resolve pending` CTA if `OutcomeUnknown`) + concise grid/list of cycle cards.
- **Incremental Reveal**:
  - Cycle cards display concise visual telemetry: Cycle ID badge, status icon + pill, timestamp, and item count (`RollingText`).
  - Clicking `New Cycle` opens a focused **`Dialog` confirmation prompt** (`Confirm New Cycle` with deployment/revision badges).
  - Clicking any **Cycle Card** opens a contextual **Cycle Inspector `Dialog` / slide-over drawer** displaying cycle details and the guarded `Delete cycle` action with an inline `Switch`/confirmation step (`"Confirm permanent removal"` toggle + destructive confirm button).

### 4.5 Brain Search (`Page::Brain`)
- **Layout**: Focused command-style search header (`TextInput` with search glyph, live `Spinner` during `Load::Loading`, and index-status `Badge`) + split master-detail workspace (or zoom detail on narrow viewports).
- **Incremental Reveal**:
  - Before searching: minimal centered empty-state prompt with index status icon.
  - Results list: compact interactive rows (actor category icon in `#FFB347` coral + name + match score pill).
  - Clicking a result reveals the **Authoritative Dossier** pane (using `El::matched` or smooth slide-in `Transition`) with a `SegmentedControl` (`Overview` | `Claims` | `Associations`) or `Accordion` sections so only one dossier facet is shown at a time.

### 4.6 OSINT Tools (`Page::Tools`)
- **Layout**: Responsive 2-column / 3-column grid of compact **Provider Service Cards**.
- **Minimal Text & Visual Status**: Each card shows only: Provider Icon/Monogram, Provider Name, Credential Status `Badge` (Emerald `Configured` vs Amber `Key needed` vs Muted `External`), and primary/fallback slot indicator dots.
- **Click-to-Provision Modal**: Clicking any provisionable provider card opens the **`Dialog`** modal (`"{Provider} API keys"`) featuring:
  - Masked `TextInput`s for Primary and Fallback keys
  - **`Switch` toggles** for `"Clear primary key"` and `"Clear fallback key"` (replacing long text toggle buttons)
  - Liquid-metal `"Review key replacement"` CTA → reveals concise in-dialog confirmation strip (`Confirm` / `Cancel`).

### 4.7 Model Assignments (`Page::Models`)
- **Layout**: Top `SegmentedControl` (`Roles` | `Providers`) so the user sees only the active configuration surface.
- **`Providers` view**: Clean checklist/table of configured provider accounts with status icons (`Configured` / `Unconfigured`) and inline `"Configure key"` trigger → clicking opens a focused **`Dialog` or `Popover`** with masked key input, `Switch` for `"Clear key"`, and confirmation.
- **`Roles` view**: Concise role cards (`Role` badge in `#B48EFF` violet, current Provider + Model + Effort chip, Fallback count badge). Clicking a role row expands its inline **`Collapsible` / `Dialog` Editor** containing only that role's `Select` pickers (Provider, Model, Effort/Variant, Ordered Fallbacks) and liquid-metal save/confirm flow.

### 4.8 Jobs & Logs (`Page::Jobs`)
- **Layout**: Top concise metric strip (`Active`, `Completed`, `Failed` with `RollingText` counts and status icons) + bounded workstation queue list (`List` / `Table` rows with status `Spinner` or glyph, job type icon, and relative timestamp).
- **Click-to-Inspect**: Clicking a job row opens a **Job Operational Summary Drawer / `Dialog`** showing the safe authorized summary and bounded log entries (monospace well, wrapped text), keeping the main queue list clean and high-signal. No retry/cancel/export toolbars are added (preserving allowlist boundaries).

---

## 5. Verification, CI Workflows & Accessibility Strategy

1. **Automated Unit & Geometry Tests (`cargo test`)**:
   - Extend `src/appearance_tests.rs` to assert that `LiquidMetal` / glow / hover motion / `matte()` modes preserve exact layout bounds (`presented(key)`) and identical `accessibility_json()` across all 7 destinations + Setup.
   - Verify `cx.reduced_motion(true)` settles all `Transition`s, `LoopSpec`s, and `Compositor` animations cleanly.
   - Add unit tests for progressive disclosure state transitions (e.g., Setup step progression, Tool/Model modal open/close, Atlas/Jobs card selection, Checklist/Switch state bindings).
2. **Selected Same-View Runtime & Visual Evidence (`scripts/fleetworks_verify.py native-ui --agent`)**:
   - Update `examples/native_fixture.rs` + `tests/fixtures/scripts/` so scripted verification tests:
     - Baseline settled views (`1280×800` @ `2×` and compact `640×480` width)
     - Interactive transitions & progressive disclosure (`--script-at` after click/hover/navigation)
     - Async loading states (`Load::Loading`, `SubmissionPhase::Posting`, pairing wait)
     - Hover/pointer lighting over liquid-metal buttons and light surfaces
     - Reduced-motion / matte parity captures.
3. **Current CI Workflow Alignment (`.github/workflows/`)**:
   - `fleetworks.yml` (`scope` → `desktop` / `server` / `web` / `bert` / `embedded` / `images` / `compose` → `source_checks` → `release_images` → `prepublish` → `publish` → `required`) selects the dual-Mac [fleetworks-native.yml](../../.github/workflows/fleetworks-native.yml) lane on native/contract changes (`macos-15-intel` and `macos-15`).
   - `fleetworks-native.yml`'s `package` job caches `target/agents` on `Cargo.lock` + `apps/fleetworks-desktop/Cargo.toml`, runs `cargo fmt --check`, `agent_cargo.py test --locked -p fleetworks-desktop -p fleetworks-workstation -p fleetworks-contracts`, and `fleetworks_verify.py native-ui --agent --require-clean` across its explicit `--scene` / `--script` list (`workstation-offline`, `atlas-offline`, `brain-search-ready`, `brain-dossier`, `brain-search-indexing`, `brain-search-error`, `tools-offline`, `models-setup`, `profile-setup`, `workspaces-offline`, `jobs-offline`, plus `--script brain-search-ready=...`).
   - In Phase 3, coordinate any new/renamed `--scene` or `--script` entries in `.github/workflows/fleetworks-native.yml` with Code Architect, keep `docs/wiki/code/ci.md` and `docs/wiki/code/native-verification.md` aligned with `.github/workflows/`, and update `docs/wiki/code/native-design-system.md` + `docs/diagrams/native-workstation-layout.*`.
