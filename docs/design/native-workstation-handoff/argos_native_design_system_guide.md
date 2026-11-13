# Argos OSINT repository — Native design system guide

**Status:** Working draft, adapted to the current Fleetworks native app.  
**Baseline:** `xxxcess/argos-osint` · `feat/atlas-zero-shot` · `d219d5964a29d50fa99de4cdcf2b228391576b4a` · audited November 12, 2026.  
**Proposed repository location:** `docs/wiki/code/native-design-system.md` (new, following the existing wiki structure).  
**Companion:** `argos_workstation_identity_implementation_spec.md` defines the contract migration and adoption work.

## 1. Purpose and scope

Make the native application understandable at an initial glance. Each destination has one clear purpose, a short status summary and only the controls specified in `argos_osint_workstation_ui_specification.md`. The supplied reference governs the application structure and allowed interaction points. Existing source determines reusable components, real operations and contracts that require migration.

The structure, component rules and control allowlist in this design system apply to the Rune/rune-kit native macOS app. The shared visual direction for native and Intel Web is **Matte Obsidian with Satin Metal Accents**, expressed minimally using the existing brand logo's gold and ivory. Matte black dominates the canvas and card bodies. Interactive controls use dim, low-sheen satin graphite edges with barely perceptible depth; cards have only a very faint static edge. Section 5 defines that treatment and the brand-logo alignment. Intel Web's existing layout, controls and interaction patterns remain governed by the web UI guidance. The shared API and Intel Web Connect changes separately support the requested workstation identity transition.

The target product uses one registered workstation identity. Remove workspace selection, workspace registration, folder management and workspace-based Connect IDs. Do not retain an invisible default workspace behind the new interface. Local settings storage and backend deployment directories remain implementation concerns, not user-facing identities.

The native app remains an API client. The gateway and Linux services own execution, storage, indexing, inference and authoritative configuration. Opening the app or completing setup does not start those services. Product branding remains Fleetworks; its theme adopts matte obsidian with minimal satin accents.

## 2. Audited starting point

The current app already has a native `Sidebar`, `Toolbar`, `SplitView` and one selected page. Its initial Workstation page combines connection profiles, credential fields, registration, interface selection, readiness, dataset management, seeding, training, reindexing and activation in one scroll. Models and Profile also append complete editors below their lists. The structural problem is the amount and ordering of content inside pages, rather than every application page being mounted together. [S1][S2][S3]

Reuse the existing shell, asynchronous effects, view helpers and theme implementation, updating the visual tokens and material recipes in place. Brain already supplies an adaptive result/dossier arrangement. The existing operational log implementation supplies bounded rendering and visibility-aware reads. These are implementation foundations; their additional controls are not automatically part of the target interface. [S1][S2][S10]

The current branch is newer than `main` for this native work. This document records a source audit, not an installed-app visual inspection, a performance measurement or an implemented change.

## 3. Control allowlist

This table is the complete native interaction surface. A control must map to one of these entries. Ordinary selection, submission, dismissal, loading, confirmation and error states needed to operate a listed control are included; they must not grow into separate administration screens.

| Reference surface | Allowed controls and target behavior |
| --- | --- |
| Window shell | Real native window controls; **Setup Gateway** action; sidebar collapse control; clickable gateway/status header; six navigation items. |
| Two-step gateway wizard | Gateway URL field and normal step progression. The second step shows connectivity checks and completes the connection. |
| Workstation status | Read-only identity, connection and resource information. Text may be selected using native text behavior. |
| DB Seed & Train | Corresponding backend seed, index-build and supported training triggers; read-only dataset/model/status information. |
| News Cycles | New-cycle trigger; cycle cards; selected-cycle delete action. |
| Brain Search | One catalog search field; result selection; dossier inspector. |
| OSINT Tools | Provider/service cards and the API-key provisioning modal opened from a card that needs credentials. |
| Model Assignments | Provider table with inline key configuration; role controls for provider, model, reasoning effort, variant and ordered fallbacks. |
| Jobs & Logs | Job queue/list and its operational log viewer, including selection of the job whose logs are shown. |

Do not carry over separate Test/Connect/Disconnect buttons, saved-profile CRUD, token-file pickers, workspace controls, dataset upload/selection screens, classifier activation controls, account-ID/base-URL administration forms, configuration import/export, article filters, manual tool-input/Run/result forms, extra tabs, job retry/cancel controls, or log filters/follow/export/copy controls. Those controls are outside the supplied reference. Existing backend capabilities can remain available to their authorized non-native callers.

An unavailable action stays within its listed control: show the concrete reason adjacent to it and disable submission. Do not add a repair toolbar or a settings destination to compensate. A revision-sensitive action may use an in-place review/confirmation state of that same action. Backend work must never be submitted by merely opening a page.

## 4. Shell and navigation

### Persistent shell

Retain the native titlebar integration and current Rune shell. Use actual macOS window controls rather than painted traffic-light substitutes. The title area identifies the app and current destination, with **Setup Gateway** at the trailing edge.

The sidebar header shows a short gateway label, connection state and a bounded endpoint label. Selecting it opens **Workstation Status**. Connection status describes the gateway connection; it does not imply that every worker or model is ready.

Below the header, use exactly these six destinations, in reference order:

1. **DB Seed & Train**
2. **News Cycles**
3. **Brain Search**
4. **OSINT Tools**
5. **Model Assignments**
6. **Jobs & Logs**

Workstation Status is the gateway-header destination, not a seventh control item. Remove the separate Profile and Workspaces navigation entries. Provider configuration permitted by the reference belongs in Model Assignments; tool-key configuration belongs in OSINT Tools.

Render only the selected destination in the main stage. Put the destination heading and its permitted primary action above its content. Do not repeat the whole workstation overview above every page. Avoid a persistent explanatory paragraph under every heading; keep necessary ownership and failure information short and local.

### Resizing and scrolling

Reuse the current initial window size of 1180 × 780 logical points, minimum 640 × 480, and expanded sidebar defaults of 220 points within its existing 180–340 range as initial implementation values. These are source defaults, not a declaration that the redesigned layouts are already verified at those sizes. [S1]

The collapsed sidebar retains accessible navigation and tooltips as in the reference. At the minimum size, content becomes a single column; a selected dossier or job detail replaces its list within the same destination using ordinary navigation. Wider layouts may use list/detail panes. Keep one clear scroll owner for each pane, a stable heading/action region, and no horizontal page scrolling for normal text.

## 5. Shared visual direction — Matte Obsidian with Satin Metal Accents

Use matte obsidian-black canvas and card bodies, with the **canonical logo's muted gold and ivory as the brand palette**. Make the satin-metal finish dim and low-sheen: graphite controls have faint edges and barely perceptible depth, while cards receive only a very subtle static edge. Bright silver/chrome borders, broad glossy reflections, shiny surface gradients and sweeping sheen are excluded. The current native and web palettes are the audited starting point; the values below are target design tokens to implement in both clients. [S4][S12][S14]

### Target palette from the brand logo

| Target token | Value | Use |
| --- | --- | --- |
| `BACKGROUND` | `#060709` | Flat matte obsidian window/page canvas |
| `SURFACE` | `#0B0C0F` | Matte black base for cards and ordinary controls |
| `SURFACE_RAISED` | `#111317` | Solid tonal separation for raised controls and panels |
| `SURFACE_INSET` | `#060709` | Quiet inner beds for long text, tables and logs |
| `TEXT_PRIMARY` | `#F3EEE4` | Main text and the canonical logo's ivory disc |
| `TEXT_SECONDARY` | `#D6D2C4` | Supporting text |
| `TEXT_MUTED` | `#B8B4A8` | Readable lower-priority metadata |
| `ACCENT` | `#C6A56E` | Canonical ray gold; primary action, selection and focus emphasis |
| `ACCENT_PRESSED` | `#C4A265` | Canonical crescent gold; solid pressed-state color |
| `CARD_EDGE` (new) | `rgba(243,238,228,0.02)` | Very faint static card edge derived from logo ivory |
| `BORDER_DECORATIVE` (new) | `#3A3833` | Dim warm-graphite framing where the edge does not identify an essential control |
| `BORDER_CONTROL` | `#898274` | Functional boundary where needed to identify a control; preserve actual contrast checks independently of decorative satin edges |
| `SATIN_CONTROL_EDGE` (new) | `rgba(243,238,228,0.03)` | Faint, static satin edge on interactive controls; never a bright border or broad reflection |

Keep the Kit's distinct success, warning, danger and information roles, plus the web's evidence-kind and metric colors. Pair state colors with text or icons. Gold identifies the brand and interaction emphasis; status meanings retain their own colors.

### Minimalism rules

- Let obsidian and charcoal occupy most of the interface. Use the logo's gold for selected navigation, focus and one compact primary action in the active view.
- Use solid matte bodies and, at most, one faint static edge per card/control. Depth should be barely perceptible; add a small shadow only when elevation needs to be explained. Avoid bright silver/chrome outlines, broad reflections, shiny gradients, stacked decorative borders, heavy glow and ornamental card nesting.
- Use aligned content, consistent spacing and a short type hierarchy. Keep labels and necessary status information clear; avoid repeated explanatory blocks and decorative icons.
- Keep the canonical logo in its existing branding position, at a restrained size. Preserve its five-ray geometry and exact gold/ivory colors; do not repeat it on cards or animate the emblem.
- Keep surfaces stable at rest and during interaction. Hover changes only a control's border or solid fill; it never launches a sheen sweep. Read-only cards remain matte and static.

The intended appearance is a clean intelligence workstation dominated by matte black, muted brand colors and minimal satin graphite detail. Tonal separation and spacing establish hierarchy; the dim finish should be barely perceptible and never become reflective focal content.

### Surface recipes

| Surface | Required finish |
| --- | --- |
| Window/page canvas | Uniform, opaque matte obsidian black with minimal decoration. |
| All card surfaces | Solid matte black/charcoal fill with a very faint static logo-ivory edge, initially about 1–2% opacity. Use a minimal shadow only for meaningful elevation. No broad reflective overlay or glossy gradient. |
| Interactive dark controls | Matte graphite body, dim satin edge initially about 2–3% opacity, a functional boundary and barely perceptible depth. Apply to the permitted buttons, fields, selectors, navigation items and selectable cards/rows. Keep text-entry areas quiet while typing. |
| Primary action | One compact, muted gold control with a solid `#C6A56E` body, `#C4A265` pressed state and dark `#060709` text. Keep its finish flat and low-sheen; secondary controls remain matte graphite. |
| Long text, table and log interiors | Solid obsidian/inset fill within the matte container, with stable foreground contrast and no reflective overlays. |

These edge-opacity ranges are starting design values to verify in actual rendered views. The 1–3% satin edges and dim decorative border never replace functional control boundaries or keyboard-focus tokens. Preserve the existing numeric contrast checks after alpha compositing; adjust the appropriate functional token when a control needs a clearer boundary instead of weakening those checks. Use solid tonal separation, a faint decorative edge and minimal shadow to establish depth; do not add noisy textures, glossy gradients, reflective sweeps or shimmer. A card's finish does not create an action: read-only cards have no pointer cursor, interactive focus or hover treatment.

### Interaction states

| State | Visual behavior |
| --- | --- |
| Rest | Solid matte body, fine static edge and readable label; noninteractive cards remain in this state. |
| Hover on an interactive surface | Make a small border or solid-fill change within the muted graphite/gold palette. Keep the finish dim; no bright silver rim, sheen sweep or moving reflection. |
| Pressed | Darken the solid fill or reduce apparent elevation; keep the hit target and surrounding layout fixed. |
| Keyboard focus | Persistent visible focus outline independent of the satin edge or state transition. Use a contrasting outline on the gold primary variant where needed. |
| Selected | Stable selection fill/border and existing selection semantics. A highlight animation alone cannot indicate selection. |
| Disabled or unavailable | Reduce border emphasis/elevation and keep the label plus reason readable on the matte body. |
| Loading or error | Keep the existing state indicator and message clear against the matte body. Motion cannot imply progress that the API has not reported. |

Use short 120–180 ms color/border transitions as an initial value. Reduce Motion uses immediate or reduced transitions. Surfaces are opaque by default; Reduce Transparency retains the same matte fills and static edges without backdrop blur. Keep decorative surface animation absent, including when the app/view is hidden or unfocused. Use the existing renderer's solid fills, borders and shallow shadows. This theme adds no renderer dependency or continuous animation loop.

### Type, spacing and implementation

Use current Kit body/small/medium typography and monospace for logs and machine identifiers. Start with the existing 20-point page padding and 14-point section gap, normalizing the smaller 4/6/8-point gaps by purpose. Long content wraps and foreground text remains clear through every interaction state. The fine satin edge must not substitute for a necessary control boundary or focus indicator. [S2]

Implement reusable matte-card, matte-dark-control and muted-gold-primary recipes in the current native theme/material helpers, and equivalent tokens/recipes in the existing web theme/CSS. Centralize the canonical brand colors and satin-edge settings within each existing theme system. Avoid per-screen decorative effects. The native structure and control allowlist still determine which controls exist.

### Intel Web and brand alignment

Apply these target palette and material recipes to Intel Web's existing controls and cards, including the Connect surface and Intel shell/content panels. Its audited palette matched the old native palette; update both clients to this new target together. Preserve distinct semantic, metric and topology colors and keep graph rendering/reading areas quiet. [S12]

- **Brand mark:** replace the three-solid-ray approximation in `packages/ui/src/FleetworksMark.tsx` with the existing canonical five-outlined-ray emblem. Preserve its `#C6A56E` rays, `#C4A265` crescent and `#F3EEE4` disc. Reuse its geometry. If the existing hardcoded charcoal knockout produces a patch on obsidian, make that knockout transparent with equivalent vector geometry/masking; use the new background only for intentionally opaque icon variants. [S13][S14]
- **Placement:** retain the existing `IntelStatusBar` logo slot and `FleetworksMark` size API. Preserve aspect ratio, accessible labeling and base-path-safe asset resolution. Keep browser/app icon variants consistent with the same emblem. [S15]
- **Scope:** retain the web's current typography scale, layout, navigation, controls, responsive behavior and topology/list interactions. The native control allowlist does not remove web controls. Theme and material styling add no API calls or UI workflow.

Verify actual native and browser views for matte canvas/card coverage, restrained static control edges, rest/hover/press/focus/disabled states, long-text readability, small-logo fit at 1×/2×, reduced motion and opaque surfaces. Source tokens alone do not establish visual approval.

## 6. Workstation connection and identity

### Two-step setup

**Step 1 — Gateway URL.** Normalize and validate the gateway origin. Resolve an externally provisioned Native-purpose credential binding for that exact origin. Keep the existing protected credential-file identity and permission checks. A deployment setup step or supported launch configuration provisions the binding; the wizard adds no token picker, account form or profile manager.

**Step 2 — Connectivity checks.** Authenticate, check protocol compatibility, and ensure/reuse the workstation registration. Group the visible checklist around gateway/workstation connection, advertised backend service readiness and model-role availability. Retain the reference's concise checklist rather than exposing the internal request sequence as extra controls.

Missing or mismatched credentials are a clear setup error. Never reuse a credential from the previous origin against a new origin. A gateway switch invalidates old authority, results, pending view reads and confirmations. Optional services can be unavailable without falsely marking the gateway disconnected; show their actual state and disable only the operations they prevent.

### Workstation-only contract

Use the existing opaque backend `workstation_id` as the canonical returned **Connect ID**, resolved within the configured deployment and authorized scope. The backend already derives workstation reuse from authenticated scope plus the stable local installation marker. Retain that marker in protected app settings; it is not a credential. [S5][S6]

Remove workspace name, marker and ID from new active registration/context/request-origin contracts. Keep internal registration IDs, revisions and access/configuration epochs for authority and stale-response checks. IP is mutable network metadata, not the Connect ID. Prefer a best-effort local interface observation; when it is unavailable or ambiguous, show unknown rather than fabricating an identity or adding an interface-management control. Making that metadata optional is an explicit contract change from the current source.

Display the returned Connect ID as selectable text in the Workstation identity card. The existing Intel Web Connect form accepts that ID and resolves the same workstation. The browser retains its server-side WebRelay credential and protected session. No new native Copy ID or Open Browser button is added by this guide. [S7]

## 7. Destination composition and API behavior

### Workstation Status

Use the reference's three compact identity/telemetry areas: **Native Client**, **Gateway Backend**, and **Host Resources**. They appear only on this destination. Remove the reference's active-workspace area.

Show native app/platform information and the registered workstation identity; gateway origin/deployment, compatibility and readiness; then CPU, memory and disk percentages. `GET /api/v1/host/metrics` already returns optional percentages plus platform/chipset metadata. These measurements describe the API host, not every service machine. Do not invent GPU details, memory totals, uptime or the reference's sample hardware/counts. Missing values are unavailable, not zero. Poll only while this status surface is visible and focused, with a bounded cadence and stale labeling. [S8][S11]

### DB Seed & Train

Use compact operation cards for configured actor seeding, actor-search index building and supported classifier training. Bind each trigger to a server-authoritative dataset or operation configuration and exact revision. No local CSV picker, upload manager or arbitrary dataset selection appears in this UI. If a required configured dataset is missing, the related trigger explains that requirement and stays disabled.

Reuse the existing seed, reindex and training endpoint families. Current `TrainableEngine` supports **Natural**; Rust-BERT inference readiness is a separate capability. Label the implemented operation accurately. Training produces a candidate and never activates it automatically. The reference does not authorize an activation control or a new Rust-BERT training service. Show returned model identifiers and validation metrics only when the API supplies them. [S3][S9]

### News Cycles

Place the new-cycle trigger at the top and show cycle cards below it. Remove the current article-day/category/search controls and article-list-first layout. Cards show the cycle's reported time, state and authoritative ingestion/admission/extraction counts. Unknown counts remain unknown.

Reuse the existing Atlas submission boundary. The audited public router binds cycle submission; the reference's cycle-history and deletion presentation needs a defined gateway read/delete contract before it can be enabled. Scope cycle history to this workstation's authorized associations. Define delete as a cycle-history tombstone, preserving durable job audit and shared articles/evidence; it is not job cancellation. Use an in-place confirmation state for the listed Delete action. This deletion behavior is a proposed contract, not a claim about an existing endpoint. [S8]

### Brain Search

Keep one search input, bounded results and a selected dossier. Reuse the current debounce/latest-response fencing and canonical actor-ID lookup. The dossier leads with identity/summary, then supported confidence, linked entities and verified claims. Keep long sections within the inspector scroll; do not add filter panels or tabs. A missing summary/confidence field is unavailable, not synthesized for decoration. Native topology rendering is outside this surface; linked entities can remain text. [S2]

### OSINT Tools

Render the current provider/service catalog as cards. Show availability and credential presence. Selecting a card that requires a key opens the reference's key-provisioning modal. Route its changes through the actual protected backend configuration contract, retaining revision checks and explicit preserve/replace/clear semantics where applicable.

Do not expose the current manual tool execution form, result workflow, unrelated account administration or a separate vault screen. Service cards without provisionable credentials are status displays, not prompts to enter invented API keys. [S3]

### Model Assignments

Use a compact provider table followed by the role-assignment table, matching the reference. Provider entries come from configured backend accounts; inline key configuration uses the real credential contract. Role rows expose only provider, model, supported reasoning effort, variant and ordered fallbacks. Derive roles and choices from the live catalog rather than copying the example role names.

Reuse current configuration revision and permission checks. Saving a row must not reset hidden account fields or unrelated settings. If the server reports a conflict, preserve the pending values and show the conflict within that row. Do not append full account/model forms or add tabs, account creation/deletion, base-URL editing or import/export controls. [S3]

### Jobs & Logs

Use one queue-and-log destination. A compact job list shows operational state; selecting a job sets the log viewer. Retain bounded or virtualized rendering and automatic finite log updates only while the viewer is visible and focused. Monospace text, line numbering and status colors may follow the reference; content remains authorized safe operational summaries, not unrestricted provider stdout/stderr. [S10]

Both native and browser activity use server-resolved workstation membership. Remove workspace narrowing from new requests. No global-job fallback exists when registration is absent. No additional tabs, filters, retry/cancel, follow toggle or export controls appear. Selection and automatic bounded continuation supply the listed queue/log interaction without extra toolbars.

## 8. Shared state and request rules

| Situation | Required presentation and behavior |
| --- | --- |
| Initial or stale destination data | Start a bounded read on entry/focus when needed; do not fetch every destination on every navigation. Show loading in the affected content region. |
| Gateway authenticated, workstation registration unavailable | Keep explicitly permitted backend reads usable; explain the registration requirement on activity and mutations. |
| Capability missing or permission denied | Disable only the affected listed action and show its reason. Capability advertisement, permission and readiness are separate checks. |
| Mutation accepted | Show accepted/queued and its returned job identity. A durable receipt is not completion. |
| Response lost after submission | Retain the exact request ID/body/context. Reconcile that request; do not send a new action because the user revisits the page. |
| Job completes | Show the authoritative completed result. Training completion does not imply activation or inference readiness. |
| Navigation or focus loss | Cancel or pause obsolete client reads. Accepted backend jobs continue. |
| Gateway, registration or access changes | Fence old responses, clear affected caches and pending confirmations, and apply the current authority before reading or mutating. |
| Empty versus unavailable | Distinguish no records, no matching authorized records, unsupported service, access denied, stale observation and failed request. |

Keep these mechanics in shared state/effects and service contracts. The screen should show the relevant state plainly, rather than displaying protocol internals as permanent explanatory content.

## 9. Acceptance criteria

- The connected landing screen makes the workstation, gateway and current readiness understandable without scrolling through forms or maintenance controls.
- Exactly the six reference control destinations appear beneath the gateway/status entry. Every visible interactive control maps to the allowlist.
- No workspace destination, selector, child name, hidden default requirement or workspace-based Connect ID remains in the target flow.
- The two-step wizard can connect and register using a correctly provisioned origin-bound credential; changing origins never forwards the old credential.
- Native and Intel Web resolve the same workstation ID. IP changes preserve that identity. Different workstations and deployments remain isolated.
- Every action reaches a supported typed API contract with actual authorization and capability gates. No static example data is presented as live state.
- Native and Intel Web use matte obsidian-black canvas and card bodies. Controls use dim, low-sheen satin graphite edges and barely perceptible depth; bright silver/chrome borders, broad reflections, glossy gradients and sheen sweeps are absent.
- The logo's exact gold/ivory colors inform both clients. Gold is used sparingly, the active view has one compact primary action, and ordinary cards use a very subtle static edge without repeated logos or heavy decorative framing.
- Read-only cards do not imply clickability. Reduced-motion and reduced-transparency modes retain the opaque matte finish and readable static focus/state cues.
- The native app retains its API-only architecture. Setup does not launch backend processes.
- Intel Web uses the canonical five-ray logo without a mismatched background patch. Its existing controls, layout and topology behavior remain intact; the native allowlist applies only to native screens.
- At 1180 × 780 and 640 × 480, headings/actions remain reachable, normal text wraps, and list/detail scrolling and keyboard focus remain coherent. Actual Rune runtime captures and physical-Mac checks must verify this during implementation.
- Old sessions and pending operations follow the companion migration spec. Historical activity is preserved without granting access to unrelated jobs.

## Sources

All repository links are pinned to the audited commit. The user's supplied structural Markdown and subsequent constraints are the target-design authority; these sources establish the existing implementation.

[S1]: https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/apps/fleetworks-desktop/src/app.rs
[S2]: https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/apps/fleetworks-desktop/src/views.rs
[S3]: https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/apps/fleetworks-desktop/src/backend_views.rs
[S4]: https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/apps/fleetworks-desktop/src/theme.rs
[S5]: https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/crates/argos-osint-core/src/storage/connect_registry.rs
[S6]: https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/crates/fleetworks-contracts/src/connect.rs
[S7]: https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/apps/web-desktop/connect-session.mjs
[S8]: https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/crates/argos-osint-api/src/lib.rs
[S9]: https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/crates/fleetworks-contracts/src/readiness.rs
[S10]: https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/apps/fleetworks-desktop/README.md
[S11]: https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/crates/argos-osint-api/src/host_metrics.rs
[S12]: https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/packages/intel/src/shell/intel-theme.ts
[S13]: https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/packages/ui/src/FleetworksMark.tsx
[S14]: https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/assets/brand/fleetworks-mark.svg
[S15]: https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/packages/intel/src/shell/IntelShell.tsx

