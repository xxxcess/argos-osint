# Argos OSINT repository — Native design system guide

**Status:** Historical structural audit with the approved visual direction updated on **2026-11-17**.  
**Historical baseline:** `xxxcess/argos-osint` · `feat/atlas-zero-shot` · `d219d5964a29d50fa99de4cdcf2b228391576b4a` · audited November 12, 2026.  
**Current authorities:** [Native control contract](../../wiki/code/native-design-system.md), [matte canvas theme](../matte-canvas/theme.md) and [interaction/state](../matte-canvas/interaction-state.md).  
**Original companion:** `argos_workstation_identity_implementation_spec.md` records the initial migration proposal.

Sections 2–4 and 6–9 preserve that dated structural reasoning. They are not a current source audit or a replacement control allowlist: later pairing, Settings, permitted retry and other maintained contracts take precedence. Section 5 replaces the former gold/ivory and satin-metal visual direction. The current change is documentation only; runtime adoption and verification remain pending.

## 1. Purpose and scope

Make the native application understandable at an initial glance. Each destination has one clear purpose, a short status summary and only the controls specified in `argos_osint_workstation_ui_specification.md`. The supplied reference governs the application structure and allowed interaction points. Existing source determines reusable components, real operations and contracts that require migration.

The original structural proposal applies to the Rune/rune-kit native client; the maintained native wiki now owns its allowlist. The approved shared visual direction is **Matte canvas**: a uniform `#060709` canvas, cool neutral text, diffuse grouped backgrounds, flat semantic colors and coarse matte rock only on eligible semantic actions. Reflective material belongs to the symbol artwork, not general controls. Section 5 links the reproducible current contract. Intel Web retains its navigation, controls, reading order and topology/list behavior.

The target product uses one registered workstation identity. Remove workspace selection, workspace registration, folder management and workspace-based Connect IDs. Do not retain an invisible default workspace behind the new interface. Local settings storage and backend deployment directories remain implementation concerns, not user-facing identities.

The native app remains an API client. The gateway and Linux services own execution, storage, indexing, inference and authoritative configuration. Opening the app or completing setup does not start those services. Product identity remains Fleetworks; visible UI branding uses the symbol alone under the current theme.

## 2. Historical audited starting point

At the November 12 audit, the app had a native `Sidebar`, `Toolbar`, `SplitView` and one selected page. Its initial Workstation page combines connection profiles, credential fields, registration, interface selection, readiness, dataset management, seeding, training, reindexing and activation in one scroll. Models and Profile also append complete editors below their lists. The structural problem is the amount and ordering of content inside pages, rather than every application page being mounted together. [S1][S2][S3]

Reuse the existing shell, asynchronous effects, view helpers and theme implementation, updating the visual tokens and material recipes in place. Brain already supplies an adaptive result/dossier arrangement. The existing operational log implementation supplies bounded rendering and visibility-aware reads. These are implementation foundations; their additional controls are not automatically part of the target interface. [S1][S2][S10]

The branch comparison belonged to the November 12 audit. It establishes neither the current branch state nor installed-app visual inspection, performance or completion of the later target.

## 3. Historical control allowlist

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

## 4. Historical shell and navigation

### Persistent shell

Retain the native titlebar integration and current Rune shell. Use actual macOS window controls rather than painted traffic-light substitutes. The title area identifies the current destination, with **Setup Gateway** at the trailing edge. Current visible branding uses the symbol alone; it does not add a Fleetworks wordmark.

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

## 5. Approved visual direction — Matte canvas

The [canonical theme guide](../matte-canvas/theme.md) replaces this section's former gold/ivory palette and satin-metal recipes. It contains exact tokens, compositing and texture formulas, responsive group widths, typography, preference behavior, performance boundaries and four approved mockups. The [interaction/state companion](../matte-canvas/interaction-state.md) owns stable realtime text, retained refresh, localized loading and cursor continuation.

| Element | Current target |
| --- | --- |
| Canvas | Uniform opaque `#060709`, shared by native/web and matched at logo/splash edges. No working-screen starfield, grain or vignette. |
| Text and controls | Cool gray primary `#D5DCE5`, supporting `#A8B6C5`, muted `#8192A4`; cyan interaction `#24DCE3`. No ivory/gold primary UI accents. |
| Grouping | Flat `#0B0F14` background, 20px rounded bounds and a 16px inward smoothstep alpha fade into the canvas; crisp text and controls above the mask. Functional borders and focus remain independent. |
| Branding | Symbol only; no visible Fleetworks wordmark. Preserve the approved crescent, sphere and five triangles, aspect and canvas-matched edges. Reflective/gold/ivory detail is confined to the material emblem. |
| Action material | Static coarse crescent-rock texture only for semantic submit/confirm/create/delete, including Connect. Tint by intent and apply the deterministic crop/scale/opacity recipe. All ordinary navigation, Settings, filters, pop-out, inspect, read-retry, metric and readiness surfaces remain flat. |
| Motion | Zero persistent control/card sheen, specular bands, pointer lights or cursor shadows. Only genuinely pending unresolved regions may translate a restrained band over an unchanged base; text never pulses. Reduced motion is static. |
| Semantic color | Flat green, amber, rose, blue, cyan and distinct chart categories retain their actual meanings. Unknown values are not fabricated zero; warning amber is not generic branding. |
| Composition | Grouped responsive grids for overview/options/status; tables remain for comparisons. Preserve native allowed controls and web navigation/reading order, focus, scroll and authoritative state. |

Use existing native `theme.rs`, `material.rs`, `ui_components.rs` and corresponding `@argos/ui`/Intel theme ownership. Helper names such as `apply_liquid_metal`, `luminous_card`, `glass` or `mutedGoldPrimary` describe existing code; they do not exempt a consumer from the new material rules. New semantic-action metadata or mask/grid helpers must be identified as proposed implementation, not claimed as existing APIs.

Keep the 2px cool focus ring, functional contrast and full-opacity glyphs outside decorative masks. Preserve opaque fallbacks, static reduced-motion states and readable disabled reasons. The narrow surface change creates no backend authority or new operations. Runtime screenshots, temporal stability, keyboard behavior and actual composited contrast must be verified later; the documentation and generated mockups are not implementation evidence.

## 6. Historical workstation connection and identity

### Two-step setup

**Step 1 — Gateway URL.** Normalize and validate the gateway origin. Resolve an externally provisioned Native-purpose credential binding for that exact origin. Keep the existing protected credential-file identity and permission checks. A deployment setup step or supported launch configuration provisions the binding; the wizard adds no token picker, account form or profile manager.

**Step 2 — Connectivity checks.** Authenticate, check protocol compatibility, and ensure/reuse the workstation registration. Group the visible checklist around gateway/workstation connection, advertised backend service readiness and model-role availability. Retain the reference's concise checklist rather than exposing the internal request sequence as extra controls.

Missing or mismatched credentials are a clear setup error. Never reuse a credential from the previous origin against a new origin. A gateway switch invalidates old authority, results, pending view reads and confirmations. Optional services can be unavailable without falsely marking the gateway disconnected; show their actual state and disable only the operations they prevent.

### Workstation-only contract

Use the existing opaque backend `workstation_id` as the canonical returned **Connect ID**, resolved within the configured deployment and authorized scope. The backend already derives workstation reuse from authenticated scope plus the stable local installation marker. Retain that marker in protected app settings; it is not a credential. [S5][S6]

Remove workspace name, marker and ID from new active registration/context/request-origin contracts. Keep internal registration IDs, revisions and access/configuration epochs for authority and stale-response checks. IP is mutable network metadata, not the Connect ID. Prefer a best-effort local interface observation; when it is unavailable or ambiguous, show unknown rather than fabricating an identity or adding an interface-management control. Making that metadata optional is an explicit contract change from the current source.

Display the returned Connect ID as selectable text in the Workstation identity card. The existing Intel Web Connect form accepts that ID and resolves the same workstation. The browser retains its server-side WebRelay credential and protected session. No new native Copy ID or Open Browser button is added by this guide. [S7]

## 7. Historical destination composition and API behavior

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

## 8. Historical shared state and request rules

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

## 9. Historical structural acceptance

- The connected landing screen makes the workstation, gateway and current readiness understandable without scrolling through forms or maintenance controls.
- Exactly the six reference control destinations appear beneath the gateway/status entry. Every visible interactive control maps to the allowlist.
- No workspace destination, selector, child name, hidden default requirement or workspace-based Connect ID remains in the target flow.
- The two-step wizard can connect and register using a correctly provisioned origin-bound credential; changing origins never forwards the old credential.
- Native and Intel Web resolve the same workstation ID. IP changes preserve that identity. Different workstations and deployments remain isolated.
- Every action reaches a supported typed API contract with actual authorization and capability gates. No static example data is presented as live state.
- Visual acceptance is now defined by [Matte canvas](../matte-canvas/theme.md): cool neutral UI, symbol-only branding, diffuse grouped backgrounds with sharp content, role-limited matte-rock actions and no persistent sheen outside the emblem.
- Read-only groups do not imply clickability. Independent focus and readable state cues survive reduced preferences; unresolved loading alone can use the bounded translated sheen.
- The native app retains its API-only architecture. Setup does not launch backend processes.
- Intel Web uses the current approved symbol and legible small-size fallback without a mismatched background patch. Its control responsibilities, reading order and topology behavior remain intact; the native allowlist applies only to native screens.
- At 1180 × 780 and 640 × 480, headings/actions remain reachable, normal text wraps, and list/detail scrolling and keyboard focus remain coherent. Actual Rune runtime captures and physical-Mac checks must verify this during implementation.
- Old sessions and pending operations follow the companion migration spec. Historical activity is preserved without granting access to unrelated jobs.

## Sources

These repository links remain pinned to the historical November 12 audit; they do not establish current implementation. Current theme/source-baseline references are in [Matte canvas](../matte-canvas/theme.md), with the maintained native wiki owning the active control contract.

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

