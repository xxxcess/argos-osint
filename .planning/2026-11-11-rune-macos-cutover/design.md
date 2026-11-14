# Fleetworks — Native client cutover implementation spec

**Consolidated revision · November 12, 2026.** Replaces v12 and superseded interim scope proposals. Inputs: the supplied v12 handoff and the latest global-page, workspace-container, workstation-only activity, and indexing-pace decisions. This is the implementation contract, not evidence that code, CI, a credential reset, or runnable artifacts exist.

**Recorded source baseline:** `xxxcess/argos-osint`, `feat/atlas-zero-shot` at `f8bc447c86f59c369a9259c6d0739681fadd1e94`. **Recorded Rune candidate:** `anomalyco/rune` at `205b8d4f7e60fa6248b83bfbbb4c6a06096c59ff`. These are prior review pins, not a fresh HEAD check or proof of compatibility. Reconcile changed source before implementation; proposed routes/packages/scripts remain proposed until implemented and tested.

**Plan:** maintain `.planning/2026-11-11-rune-macos-cutover/handoff.md` under the repo planning policy. Keep implementation, verification, publication and deployment-reset statuses separate. Read root/scoped `AGENTS.md`, contributing/conventions, testing, CI, versioning and planning guidance first. One integration/build owner; `.github` follows Code Architect ownership. Preserve unrelated work, user processes and caches.

**Navigation:** [Architecture and scope](#1-architecture-scope-and-hard-boundaries) · [Native UI](#2-native-ui-and-brand-contract) · [Workspace and identity](#3-workspaces-identities-and-connections) · [Web access and jobs](#4-web-access-job-provenance-and-workstation-activity) · [Readiness and setup](#5-backend-readiness-and-setup-operations) · [Actor search](#6-tantivy-search-and-durable-ingestion) · [Reset and retirement](#7-one-time-reset-and-client-retirement) · [Local development](#8-local-development-and-rebuild-selection) · [CI and downloads](#9-ci-platforms-and-downloadable-artifacts) · [Verification](#10-verification-and-evidence) · [File and documentation plan](#11-file-documentation-and-agent-guidance-plan) · [Delivery gates](#12-delivery-order-and-definition-of-done).

Requirements appear once under their owning section; later checklists link back instead of restating the whole contract. Historical revision-by-revision commentary is omitted. No unrelated backend algorithm, graph redesign or new build framework is added.

## 1. Architecture, scope and hard boundaries

Deliver one Rune + `rune-kit` **Fleetworks.app**, with downloadable GitHub artifacts for **`x86_64-apple-darwin` and `aarch64-apple-darwin`**. Replace the interactive terminal products with a functional, simple native control hub, not a wrapper around them.

| Maintained component | Ownership |
| --- | --- |
| **Rune/rune-kit native app** | Standard Rust UI components; configured gateway API client; safe local workspace-folder/identity management and OS integration. It opens offline and does not host the backend or a browser. |
| **React Native Web / Next.js** | Independent browser app, same-origin REST/live relay and Connect ID gate. **All topology visualization belongs here.** Theme change is colors only. |
| **Linux backend** | Gateway/private services, durable jobs/workers, classifier/NLP, Tantivy actor search and authoritative SurrealDB domain data. Images/engine components target `linux/amd64` and `linux/arm64`. External tooling owns deployment. |

A local profile connects to an independently running loopback gateway; a remote profile connects to an authenticated HTTPS gateway. Neither means native lifecycle ownership. Opening Intel Web opens an advertised URL; it never starts Next.js. App close, rebuild, reconnect or profile change does not stop services or accepted jobs.

Non-goals: a native topology/graph implementation, mobile/Windows products, altered classifier/NLP semantics, replacement of deterministic actor binding or domain storage, generic remote execution, per-folder copies of shared data, or another orchestrator/build/diagram framework. Required scope/registry, search, attribution, clean configuration cutover and missing endpoint work remain in scope.

### 1.1 UI groups and resource ownership

Use **two sidebar groups**, keeping all eight pages:

| Group | Pages | Scope |
| --- | --- | --- |
| **Backend — selected deployment** | **Atlas, Brain, Tools, Models, Profile** | Shared services/data/settings of this backend. Workspace selection does not partition the corpus or replace provider accounts/default roles. |
| **Workstation — scoped node/IP** | **Workstation, Workspaces, Jobs & Logs** | Local node/connection/folder controls and **only triggered jobs associated with this registered workstation**. A workspace filter may narrow activity; it never widens it to other nodes or system work. |

Global means the selected deployment **within the caller’s permissions**, not universal access or every gateway. Workstation separates local identity/preferences from a visibly labeled **Backend readiness & maintenance** section. Its seed/train/index actions affect shared resources and need separate maintenance permission. There is no native all-backend/all-workstation job or service-log feed.

Distinguish **resource target**, **authorization**, and **request origin**. A job can target the backend-wide actor catalog while retaining the workstation/workspace/Connect ID that requested it. Connect ID supplies registered context, not administrator rights. Retain backend-global configuration authority after reset; do not build workspace key/role copies, inheritance or override machinery merely to preserve earlier drafts. Configure shared keys/default roles once per backend.

Global settings save against the expected backend configuration revision. Registry/session/context revisions remain independent. Accepted work captures effective non-secret configuration revisions and credential references with its immutable origin; UI switching cannot retarget it, and revocation/reset overrides stale references. Never put raw keys in job payloads.

Cache shared data by deployment + authorized principal/access epoch + resource revision/query/index generation. Native activity adds stable workstation ID/scope revision and an optional child-workspace filter; local preferences use installation/profile identity. Preserve still-authorized global data/drafts on workspace-only changes. On gateway/auth changes, isolate caches, close obsolete subscriptions, reject stale replies and prompt about unsaved edits. Reconfirm a pending submission if its origin changed.

### 1.2 Native safety and dependency boundary

The desktop must not import Venture lifecycle code, Tauri/webviews, Ratatui/Crossterm, backend storage/inference/search engines, a Docker SDK/socket or deployment supervisor. No shell/CLI-output adapters, stdin sessions, arbitrary command runner, embedded HTTP server, local backend or fallback to direct database/index access. Build scripts and launch hooks must obey the same rule.

Allowed local work is presentation, protected connection credentials/preferences, managed-folder creation/selection/deletion and identity markers, bounded file transfer, credential store, file dialogs, clipboard and safe browser/Finder integration. Backend operations go through the gateway; local path management uses guarded filesystem APIs, not `rm -rf`. A remote server cannot read a Mac path: upload validated bounded bytes and download exports through authorized responses.

Scoped native Cargo builds, isolated unit/fixture tests and offline shell launch require neither Docker, Node, Python, a running backend nor model/catalog downloads. Python is allowed only in external repository helpers. External test harnesses/operators provision integration services. No native fallback starts infrastructure when an endpoint is unavailable.

### 1.3 Functional endpoint inventory

Maintain one **old command → application operation or external operator task → endpoint → UI control → test** matrix from `apps/fleetworks-cli/README.md`, its implementation and Venture. Required application controls need real endpoints; infrastructure administration stays external. For each operation record: existing/extend/create/blocked status, method/route/live channel, typed payload/response, resource owner/scope, permission, origin requirements, revision/idempotency, failure/disabled states and evidence.

Authentication, validation, request IDs, deduplication and confirmations belong at the server boundary, not just UI labels. A mock-only button is not parity. Capability-based disabling is valid for genuinely unsupported servers, not an excuse to omit required features. Cancelled client requests are not cancelled durable jobs. Use bounded uploads instead of base64 catalogs in small JSON requests; preserve hashes, secret handling and explicit model activation. No generic execute-command endpoint.

## 2. Native UI and brand contract



### 2.1 Required pages and simple composition

Use the same shared shell, state, connection client and scope-aware actions for all pages. Include accessible focus order, keyboard/pointer parity, search/pagination, scrolling, empty/loading/stale/error states, validation and recoverable failures. Missing credentials after reset are setup states, not hidden pages or legacy-default fallback.

| Page | Required components and behavior |
| --- | --- |
| **Atlas** | Backend collection/cycle controls and run state; date/strategic-domain/search/sort/pagination; shared article preview and brief/job links. User requests retain workstation/workspace origin; global corpus/quotas remain global. Detailed job links obey §4.5. |
| **Brain** | Debounced actor search through the gateway; bounded/ambiguity-aware results; keyboard/click selection; fresh SurrealDB dossier with metadata, aliases, claims, briefs and related records in plain lists/tabs. Keep indexing/unavailable distinct from empty. |
| **Tools** | Shared catalog, requirements/schema, supported configuration, validated inputs, guarded execution, request receipt/progress/results. Use permitted global credential references; catalog access does not confer execution/result/key-admin rights. |
| **Models** | Global role catalog, provider/account/model selection, primary and ordered fallbacks, save/clear/validation, expected revision and missing-key feedback. Confirm impact on future backend work. Inference training/activation is the separate Workstation operation. |
| **Profile** | Backend identity/settings and provider account/key add/replace/delete; masked credential/validation metadata; supported protected current-format import/export. Never return raw keys in routine reads/logs. Local app preferences live in Workstation, not this profile. |
| **Workstation** | Node/IP, gateway/deployment and registration; connection/local preferences; separately labeled backend readiness, datasets/upload, Seed Actors, Train Classifier, explicit activation and Rebuild Actor Index. Job/log links remain workstation-scoped. |
| **Workspaces** | Contained folder list/select/create/attach/reveal/copy/remove/delete, protected root/default, registration/Connect ID and pending/recovery states. Creating/deleting a child does not alter shared keys, roles, corpus, models or index. |
| **Jobs & Logs** | Two tabs with workstation identity/IP, status/type/time/search/correlation filters, pagination, details/stages/progress/results, bounded follow/pause/copy/export and guarded retry/cancel. Default all triggered work on this workstation; optional child-workspace filter only narrows it. |

Keep gateway, workstation and submission workspace visibly distinct. Use `Backend-wide`, `Workstation activity: <node/IP>` and optional `Workspace: <name>` labels. A global action’s “Requested from <Connect ID>” is provenance, not a private resource claim. Global read/configuration forms require their backend permission, not an arbitrary folder merely to open. Interactive job submission still establishes its registered origin.

### 2.2 Rune implementation and dependency pin

Create `apps/fleetworks-desktop/` with proposed package/binary **`fleetworks-desktop`** and a small library entry point for shared production/fixture views. Reuse client-safe `fleetworks-contracts`, Tokio, HTTP, tracing and minimal workspace identity/path types. Separate deployment execution from any reused crate/features. Keep modules small; do not create a crate per screen.

Use the following inspected references, at the same pinned revision for **both** dependencies:

| Upstream reference | Required application to this cutover |
| --- | --- |
| [`examples/hello/src/main.rs`](https://github.com/anomalyco/rune/blob/205b8d4f7e60fa6248b83bfbbb4c6a06096c59ff/examples/hello/src/main.rs) | Bootstrap `AppSpec`, application state and `view(&State, &mut Cx<State>) -> El`; use stable keys. Follow its `Runtime` input/state test pattern. |
| [`crates/rune/README.md`](https://github.com/anomalyco/rune/blob/205b8d4f7e60fa6248b83bfbbb4c6a06096c59ff/crates/rune/README.md) — app lifetime | Use `rune::App` and its `AppHandle::update` to deliver asynchronous results on the UI thread. Keep one application-owned Tokio runtime; never block view functions or button handlers on network/file operations, and do not add any Compose execution path. Render the shell before connecting. |
| [`crates/rune-kit/examples/shell.rs`](https://github.com/anomalyco/rune/blob/205b8d4f7e60fa6248b83bfbbb4c6a06096c59ff/crates/rune-kit/examples/shell.rs); [`crates/rune-kit/README.md`](https://github.com/anomalyco/rune/blob/205b8d4f7e60fa6248b83bfbbb4c6a06096c59ff/crates/rune-kit/README.md) | Use `SplitView`, `Sidebar`, `SidebarRow`, `Toolbar` and `PaneState` for one shell with an optional inspector. Use `Kit`/`PaletteSeed` with the logo-led charcoal/gold/ivory contract and restrained native material in §2.3; do not copy the OpenCode example's branding or add its theme crate unnecessarily. |
| [`crates/rune-kit/README.md`](https://github.com/anomalyco/rune/blob/205b8d4f7e60fa6248b83bfbbb4c6a06096c59ff/crates/rune-kit/README.md) — components/table/Markdown | Use fields, selection controls, dialogs, progress and toast components; virtualized `Table`/`TableState` for large lists; selectable Markdown for briefs/results. Mount overlays through `rune_kit::root`. Preserve keyboard, screen-reader and reduced-motion behavior. |
| [`crates/rune/Cargo.toml`](https://github.com/anomalyco/rune/blob/205b8d4f7e60fa6248b83bfbbb4c6a06096c59ff/crates/rune/Cargo.toml); [`crates/rune-kit/Cargo.toml`](https://github.com/anomalyco/rune/blob/205b8d4f7e60fa6248b83bfbbb4c6a06096c59ff/crates/rune-kit/Cargo.toml) | Select explicit needed features. Preserve accessibility and tested macOS rendering; consider `embedded-fonts`, `metal-backend`, `native-file-dialog`, `svg`, and kit `markdown`. Do not blindly enable optional audio, diagrams or development tooling. Do not enable a topology/diagram feature for the native product; ordinary icons, Rune UI rendering and diagnostic element trees remain allowed. Do not remove transitive Unicode processing simply because terminal column-width code is removed. |
| [`scripts/mac-app.md`](https://github.com/anomalyco/rune/blob/205b8d4f7e60fa6248b83bfbbb4c6a06096c59ff/scripts/mac-app.md) | Reference for staged bundle installation, graceful relaunch and readiness/rollback. Implement repository-owned packaging; upstream tooling is not automatically a GitHub release workflow. Do not overwrite a user's installed app during an ordinary build. |
| [`crates/rune/README.md`](https://github.com/anomalyco/rune/blob/205b8d4f7e60fa6248b83bfbbb4c6a06096c59ff/crates/rune/README.md) — Agent tooling/Accessibility; [`src/tooling/args.rs`](https://github.com/anomalyco/rune/blob/205b8d4f7e60fa6248b83bfbbb4c6a06096c59ff/crates/rune/src/tooling/args.rs); hello example tests | Use documented headless PNG/tree/script flags and real `Runtime` behavior assertions. §10 replaces the terminal-image workflow; §11.5 updates agent routing and canonical procedures. Headless rendering and installed-Mac verification remain separate evidence. |

Rune is pre-release and recommends a pinned Git revision ([`README.md`](https://github.com/anomalyco/rune/blob/205b8d4f7e60fa6248b83bfbbb4c6a06096c59ff/README.md)). Its checkout pins Rust **1.99.1** ([`rust-toolchain.toml`](https://github.com/anomalyco/rune/blob/205b8d4f7e60fa6248b83bfbbb4c6a06096c59ff/rust-toolchain.toml)); Fleetworks declares **1.95** ([`Cargo.toml`](https://github.com/xxxcess/argos-osint/blob/f8bc447c86f59c369a9259c6d0739681fadd1e94/Cargo.toml)). Verify the selected graph and toolchain on both Mac architectures first. An upstream toolchain pin is not proof of its minimum supported Rust version. Coordinate any necessary repository toolchain change across CI/images; do not silently use a second toolchain.

Default native edit loop: **incremental Cargo build → relaunch**. Rune's optional `hot` feature brings additional development dependencies; do not promise general Rust hot reload or make it a prerequisite. Keep it out of release features unless separately justified and tested. Upstream readiness receipts and headless screenshots do not establish backend readiness or a passing Finder launch.

### 2.3 Brand theme: native dark metal, web colors only

Use the existing logo/banner, unchanged: `assets/brand/fleetworks-logo.svg`, `fleetworks-mark.svg`, and README header `fleetworks-logo.jpg`. The recorded SVG anchors—not sampled JPEG noise—are charcoal `#111111`, ray gold `#C6A56E`, crescent gold `#C4A265`, ivory `#F3EEE4`, and the mark’s `#141414` cutout. No new logo/banner, blue/mint brand, theme framework, runtime token download or cross-language build pipeline.

Define named tokens in native `theme.rs`/shared controls and existing web authorities; use a lightweight parity fixture rather than making native builds run pnpm. Gold is branding, not a universal success/warning/evidence color.

#### 2.3.1 Palette

| Semantic token | Baseline value | Origin/use |
| --- | --- | --- |
| `background` | `#111111` | Exact logo charcoal; main canvas, not navy. |
| `surface` | `#141414` | Existing mark charcoal; quiet panels and sidebar. |
| `surfaceRaised` | `#232321` | Proposed warm graphite for raised/interactive surfaces. |
| `surfaceInset` | `#0D0D0D` | Proposed inset/input wells. |
| `borderSubtle` | `#3A3833` | Proposed decorative separators only; not the sole control/focus signal. |
| `borderControl` | `#898274` | Proposed stronger neutral where a boundary is required to identify an input/control. |
| `textPrimary` | `#F3EEE4` | Exact logo ivory; readable headings and body text. |
| `textSecondary` | `#C6C0B5` | Proposed warm gray for secondary text. |
| `textMuted` | `#A09A8F` | Proposed readable quiet metadata; never reduce opacity blindly. |
| `accent` | `#C6A56E` | Exact logo gold; primary action accents, active navigation, links and focus. |
| `accentPressed` | `#C4A265` | Exact crescent gold; restrained pressed/selected emphasis. |
| `accentHover` | `#D8BD8C` | Proposed light gold for hover/focus emphasis. |
| `onAccent` | `#111111` | Dark text/icons on a filled-gold background; ivory on gold is not an acceptable small-text pairing. |
| `selectionSurface` | `#322C22` | Proposed muted gold-brown selection fill, with a distinct border/indicator. |

Proposed supporting tones may be adjusted for measured composited contrast while retaining warm dark neutrals. Keep severity, information, evidence-kind and metric-channel colors distinct with labels/icons. Decouple semantic aliases from changed accent values, particularly the existing Intel success/accent alias.

#### 2.3.2 Native material rules

Native remains predominantly **matte**, using simple shared rune-kit composition. Apply dark liquid-metal only where it clarifies interaction: primary/compact toolbar buttons, selected tabs/segments, and appropriate toggle/slider controls. Use a clipped static satin reflection/fine edge—not reflective text or a decoration on every surface. Tables, actor results/dossiers, article prose, logs and passive panels stay matte; inputs use quiet inset fills and readable focus.

Suggested `darkMetal` gradient stops: `#171716`, `#252522`, `#30302C`, `#20201E`, `#151514`, blended into one low-contrast reflection rather than five stripes. Use Rune’s existing gradient/shadow facilities, not a new shader/material dependency. Flat graphite is the fallback. Avoid bright chrome, rainbow highlights, heavy bevels, ripples, parallax, moving noise and persistent time-driven decoration. Static sheen is the default; any brief interaction transition must preserve visible states and stop at rest. Reduced motion uses a static treatment.

Use gold/ivory foreground according to the actual surface; filled-gold controls use dark `onAccent`. Hover, focus, pressed, selected, disabled and loading must remain distinguishable without glow alone. Preserve hit targets, keyboard/accessibility behavior and scrolling; minimize layers/effects on dense lists. Apply the same shared primitives to all eight pages, setup and dialogs.

#### 2.3.3 Web color-only boundary

Update **both** active color authorities—`packages/ui/src/index.tsx` and `packages/intel/src/shell/intel-theme.ts`—plus their existing CSS consumers in `apps/web-desktop/app/style.css`. Recolor shared controls and Home/Bulletin/Focus Brief/dossier/topology/Connect pages consistently. Existing shadows, focus, selection, scrollbar and decorative effects may change color, not structure.

Do not change web layout/component hierarchy, spacing, density/zoom, type/fonts, sizes, radii, breakpoints, navigation, state/data/API flow, graph/camera behavior, motion timing or effect geometry for this theme task. No native liquid-metal treatment, new gradient/refraction/glass layers, shader, animation or UI library in web. Keep evidence/metric semantics, legends and real imagery intact. The Connect gate is separately required functionality, not permission to redesign existing screens.

#### 2.3.4 Appearance acceptance

Use the WCAG 2.2 color-contrast criteria as the measurable color targets: normal text at least 4.5:1; qualifying large text at least 3:1; necessary component/state/focus indicators at least 3:1 against adjacent colors. Decorative hairlines need not be treated as essential control boundaries. Test the real foreground/background composite, including glass, selections, gradients and all readable interactive states. Keep text/icons in addition to color for status, selection, errors and progress. Sources: [Contrast (Minimum)](https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html) and [Non-text Contrast](https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast.html).

The earlier supplied token calculation recorded approximately 8.10:1 for charcoal text on logo gold and 16.33:1 for ivory on logo charcoal; ivory on logo gold is only approximately 2.02:1. These are calculated token-pair ratios, **not** proof of complete rendered accessibility. Use dark `onAccent` on filled-gold controls, and verify material overlays do not invalidate that decision. Native fixtures must exercise matte/metal, hover/pressed/focus/selected/disabled, reduced-motion and fallback treatments. Use all eight pages for the initial shared-theme acceptance, with representative wide/minimum layouts and 1x/2x where applicable, then select only affected scenes for later changes under §10. PNGs must come from the actual Rune renderer and be inspected with their trees/state assertions; do not use generated artwork as verification.

For web, retain Playwright and existing page fixtures. Record before/after screenshots and compare relevant element bounds, typography/spacing/motion tokens and interaction/route tests to demonstrate **color-only** changes. Cover the shared controls, Intel pages, Connect prompt, topology legends/status states and readable errors. Do not accept a palette update that loses keyboard focus or turns a warning/failed/unknown state into success. Review intentional screenshot-baseline updates; no automatic approval. No new visual-test framework or full backend/search rebuild is required solely for colors.

### 2.4 Topology is web-only

**This supersedes every earlier requirement for native topology rendering or visualization parity.** React Native Web owns all node-link graphs, topology canvases, spatial relationship views, layout/physics, pan/zoom, graph selection, depth exploration and associated topology controls. Do not create a reduced native graph, static topology-image preview, mini-map, embedded webview, browser renderer inside Rune, or a separate native Topology page/tab.

Compose the native pages from the existing rune-kit shell, toolbars, tabs, search fields, forms, buttons, dialogs, lists/tables, labels, progress indicators and scrollable detail/Markdown panels. Reuse shared controls and state; no new UI framework or custom visualization subsystem. Keep the approved charcoal/gold/ivory theme and limited native dark-metal control treatment unchanged. This is a composition limit, not permission to replace real application operations with static cards.

**Native Brain layout:** a Search actors field and bounded result list, plus an actor detail pane; at narrow sizes use a simple list-to-detail navigation with Back. The selected actor's SurrealDB-backed dossier may show aliases, metadata, claims, briefs and recorded associations as labeled, bounded lists or tabs. Preserve IDs, provenance, permissions and missing/truncated states. These are ordinary dossier records—not a new topology browser with node/edge/depth controls. Reading a dossier must not eagerly fetch graph layouts, initialize topology state or require the Intel Web server to be available.

**Web handoff:** expose the shared **Open Intel Web** action from Brain and reuse registration, destination validation and the Connect ID prompt in §4. No new deep-link protocol is required by this change. Open the advertised Connect page; do not promise a selected actor/topology destination unless an already-supported, documented same-origin return route provides it after the gate. Never bypass connection authorization, pass bearer credentials/Connect ID in a URL, launch a web server, or introduce an embedded browser. Search and native dossier reading remain usable when Intel Web is unavailable.

**Preserve domain behavior.** SurrealDB relationship records, backend topology/query/summary APIs and the existing React Native Web topology components and accessible list remain supported. The dossier service may use existing relationship/topology readers internally; removing native rendering must not remove those readers, unrelated graph data, or web functionality. This decision changes presentation ownership, not storage authority, indexing, search or article/brief semantics. The web appearance task remains color-only—no graph redesign is requested.

**Trim only unused native presentation code.** Omit any not-yet-created native topology view, layout/renderer, visualization-specific cache/subscription, shader, fixtures or dependency. Remove such code only if already introduced and unused after this cutover; reconcile callers first. Do not remove Rune's required GPU/text/layout machinery, icon SVG support, `--tree` accessibility/debug output, backend topology modules, web graph dependencies or documentation diagrams. The parity inventory records topology as web-owned and verified there, not as a skipped native requirement.

## 3. Workspaces, identities and connections



### 3.1 Protected container and managed workspace lifecycle

The native app’s production root is **`$HOME/.fleetworks`**, resolved from the user home independently of Finder/CWD. It is a protected **container**, not a workspace, backend deployment directory or Connect registration. Every native-created/managed workspace is an immediate child; the reserved `default` child is the protected default mapping:

```text
~/.fleetworks/             protected container; no Connect ID
├── default/              protected default → <IP>:default
├── research/             named workspace → <IP>:research
└── case-review/          named workspace → <IP>:case-review
```

Keep non-secret ownership/default/catalog/recovery/reset metadata outside deletable children, in reserved root files or the protected app support store. Enumerate verified catalog entries, not every unknown subdirectory. Global provider keys/roles remain backend-owned; gateway credentials remain protected client state. Do not mount native workspace directories into backend/database/search containers.

On ordinary GUI setup, create missing root/default with private permissions and current identity markers; reuse identities thereafter. A preexisting file, symlink, unsafe owner or conflicting marker blocks adoption with a repair message—never overwrite it. Builds/package helpers and tests do not initialize the real user root; fixtures inject a disposable home/root internally.

**Never delete, rename, relocate or remove the mapping for the container, `default/`, or an ancestor containing them through ordinary workspace/reset operations.** Enforce canonical object/path identity and persisted protection at UI/filesystem/API boundaries, not a label or body-supplied `is_default=false`. The default registry record is protected; the container has no workspace record. The separately allowlisted reset may clear specified credential/config files inside them, not directories, user contents or safety/identity/reset receipts.

| Action | Required contract |
| --- | --- |
| **List / select** | Show name, absolute child path, owned/default status, target backend registration/Connect ID and pending/error state. Exclude root/internal files. Selection changes future origin and any explicit activity filter, not global backend settings. |
| **Create** | Ask for one validated basename; preview `~/.fleetworks/<name>`; no parent chooser. Create privately without overwriting, establish identity, then register through the gateway. No copied keys/roles, backend storage, seed/train work or service startup. |
| **Attach existing** | Select an immediate child and verify ownership/marker/collisions with consent. Do not adopt unknown legacy config/storage/model/repository/deployment directories. External content requires a separately authorized data-only move/copy into a new child, preserving source until verified; no automatic migration or old-key import. |
| **Reveal / copy** | Reveal/copy the child’s path or confirmed Connect ID. A separate Reveal container utility does not select/register the root. |
| **Remove from app** | Distinct from deletion: retire/revoke known non-default bindings before forgetting association. Root/default cannot be removed. |
| **Delete** | Confirm exact owned non-default child, contents impact, known bindings and active work. Retire/revoke safely, then use guarded filesystem/Trash operation on that child only. Preserve siblings, shared configuration/corpus/index/models and historical origins. |

Reuse Connect ID name validation and reserve `default`, internal/hidden names and case/normalization-equivalent collisions. Reject empty, absolute or multi-component paths, `.`, `..`, separators, traversal, nested workspaces, root/external targets and symlink aliases for root/default/managed children. Revalidate parent/target object identity at mutation time; string-prefix checks are insufficient. Never follow a child’s internal symlink into external data during deletion. Reject filesystem/user-home/repository/deployment roots, overlaps, mount/parent substitution races and unowned folders. No unchecked recursion or `rm -rf`.

Creation and remote registration are separate recoverable operations: local creation can finish offline as `local / not registered`; keep new files and retry the same registration identity. Do not claim a distributed transaction. Before deletion, obtain authorized retirement for **every known backend binding**; unavailable backend/permission leaves `deletion_pending` with files intact. Block on active attributed work or use supported explicit cancel/drain and wait—never cancel another requester’s shared job. Fence new admission, revoke browser/live sessions and retain tombstones/history. A filesystem failure after retirement becomes `cleanup_pending`; retry cannot reactivate sessions. Recreating a name receives a new identity. Switch to default only after resolving its context, never relabel jobs.

Journal operation ID, phase, verified identity and remote receipts for idempotent crash recovery. A missing/moved folder or catalog conflict is a repair state, not automatic resurrection. Trash deletion does not prove secure erasure; the credential reset must not hide old keys in Trash/backups.

**One-time layout transition:** supersede root-as-default and arbitrary-parent behavior without recursive data migration. Inventory root files/old markers/registrations and previously designated defaults. Never move `~/.fleetworks` into its own child, adopt old service-data directories as workspaces, discard unknown files or sweep external locations. Approved data-only moves get receipts; legacy keys follow §7.1, not import. Explicitly rebind/supersede any old root-default registration, preserving justified stable identity/history and revoking old sessions. Do not maintain an active `IP:.fleetworks` compatibility alias or claim inaccessible installations migrated.

`workspace_root` and selected `workspace_dir` are distinct types. Register only a child basename (`default`, `research`), never root or full home path. `--workspace` selects an existing owned child by documented name/path contract; default is `~/.fleetworks/default`. Reject root/external/nested paths. The GUI creates identity/registration; bare `mkdir` is insufficient. Keep native path resolution separate from retained operator deployment paths and storage volumes.

### 3.2 Workstation, workspace and Connect ID contract

A **workstation is the native-client node**. A **workspace is the selected managed immediate child of `~/.fleetworks` on that node**, not the parent container, backend directory/database or a new copy of shared data. The default child is `default`. Register these records in the target backend database via its authenticated gateway; the native app must never connect directly to the database. Registration creates/reuses records, not remote filesystem directories.

| Entity | Required persisted identity and relationships |
| --- | --- |
| Workstation | Stable opaque `workstation_id`; current `workstation_name`/`workstation_ip` based on the canonical selected native-client IP; owning authorization scope; non-secret native installation marker for collision detection; status and timestamps. |
| Workspace | Stable opaque `workspace_id`; `workstation_id`; directory-basename `workspace_name`; local identity marker; server-enforced protected-default flag for the designated default; active/retiring/retired state and revisions. Full local path remains local by default. No folder path is authority to delete backend data. |
| Connect registration | Stable `registration_id`; `deployment_id`; workstation/workspace references; canonical IP/name values; `connect_id`; active/revoked status and revision. Enforce database uniqueness of the canonical IP/folder pair within this target backend scope. |
| Web connection session | Unpredictable opaque session identifier stored hashed at rest; validated registration and revision; authenticated web access scope; deployment identity; creation/expiry/revocation state. Reuse suitable existing session infrastructure; do not add a separate database or identity service. |

**Connect ID format:** `192.168.1.42:research` for IPv4; `[2001:db8::42]:research` for IPv6. The folder is the selected child directory's basename—not a user-entered arbitrary project label or full path. Display the server's canonical returned value with a Copy action. Keep opaque record IDs internally: do not use an unescaped Connect ID as a database record identifier, SQL fragment, URL authority, or filesystem path.

Use one parser/serializer and fixture corpus across Rust and TypeScript. Canonicalize valid IPs (including equivalent IPv6 spellings); define IPv4-mapped-address handling consistently. Require IPv6 brackets in the compound ID. Normalize folder text to NFC while retaining case; reject empty names, `.`/`..`, controls, ambiguous `:`, path separators, and leading/trailing whitespace rather than silently renaming a user's folder. Permit internal spaces/Unicode with tested JSON handling; cap IP/name/ID lengths explicitly. Resolve symlinks/case aliases locally to the same actual folder/marker. Database constraints, not a preflight query alone, prevent concurrent duplicates. Enforce one active workstation per canonical IP in the deployment scope, one canonical folder name per workstation, and one active Connect binding per canonical IP/folder pair. A device or folder marker is a collision-detection aid, not authority to claim an existing record.

**IP selection decision:** use a native-local interface address, normally the local address of the route to the configured gateway. For a loopback gateway or multiple interfaces, select/show a non-loopback LAN/VPN interface explicitly; an offline fallback cannot invent an address. This is client-reported node metadata submitted through authenticated registration, not proof of network origin. Record the gateway-observed peer IP separately for audit; NAT, Docker bridges, VPNs and reverse proxies can make it different. Never register the Next relay/container IP or the gateway address as the native workstation, and never overwrite the workstation IP when a browser submits its Connect ID. Do not use an external “what is my IP” service or trust arbitrary `X-Forwarded-For`/`Forwarded` values. Forwarded peer metadata is accepted only from explicitly trusted, correctly configured proxies.

**Identity limitations and changes:** an IP is not a permanent machine identity; neither IP alone nor IP plus folder basename is globally unique. Reserve uniqueness within the selected deployment/database. A workstation marker identifies the local installation and a workspace marker the selected folder; these detect accidental collisions but are not credentials. Different installations claiming the same active IP, or different folders claiming an existing pair, must produce an explicit collision—not silently merge, add a suffix, or steal the binding. Provide actionable rename/network/profile guidance. Support an authenticated, explicit rebind when IP/folder changes; retain stable entity IDs for the same installation/folder, revoke the previous active binding/sessions, and preserve historical job snapshots. No automatic reassignment of old IDs to a different node. Full tenant/device-attestation infrastructure is out of scope; preserve the existing authorization boundary and state this limitation in the guides.

### 3.3 Gateway profiles and connection ownership

Provide Add/Edit/Test/Connect/Disconnect gateway profiles and the shared Workspaces selector/manager (§3.1). Each profile has a name, API gateway origin and protected credential reference; selected workspace identity/folder is explicit. Both local and remote are connect-only, not deployment modes. Create a named child under `~/.fleetworks` through Workspaces or select `~/.fleetworks/default`; do not infer a path from Finder/CWD or the backend server home. Save secrets in protected storage, not ordinary profile JSON, logs, artifacts or URLs.

Reuse the current client policy in [`crates/fleetworks-contracts/src/client.rs`](https://github.com/xxxcess/argos-osint/blob/f8bc447c86f59c369a9259c6d0739681fadd1e94/crates/fleetworks-contracts/src/client.rs): HTTP only for loopback, authenticated HTTPS for remote connections, origin-bound credentials and no redirect credential forwarding. Keep gateway inputs origin-only initially; supporting API path prefixes is not required. Always show the active destination and distinguish authentication, version, reachability and readiness failures. The connection editor must remain usable offline.

Selecting **any** profile must never invoke Docker/Compose/Venture, install inference tools, alter deployment configuration, or claim ownership of a running stack. A developer/operator provisions the backend separately, then supplies its reachable gateway URL. Readiness failure leaves the GUI in a retryable disconnected/degraded state with an Edit Connection/Retry action and documentation link—not a Start Stack or Install Docker action.

### 3.4 Register before web launch and interactive submission

On **Open Intel Web**, execute:

```text
Selected gateway + owned ~/.fleetworks/<name> child + native interface IP
    -> authenticated capabilities / stable deployment identity
    -> ensure workstation + workspace + Connect registration in that backend
    -> show returned IP, folder, IDs, registration status and Copy Connect ID
    -> open advertised Intel Web Connect form in default browser
    -> user submits Connect ID; backend validates its existing active registration
    -> browser receives a scoped session; application routes become available
```

Proposed gateway operation: **`POST /api/v1/connect/registrations/ensure`**. Input includes request ID, selected IP, folder basename, native installation marker and workspace marker; ownership is derived from the authorized caller, never a body-supplied user ID. Create missing workstation/workspace records and their binding atomically, or reuse the existing owned records. Return `201` for creation or `200` for reuse with `deployment_id`, `workstation_id`, `workstation_ip`, `workspace_id`, `workspace_name`, `registration_id`, `connect_id`, status and revision. Retry/network ambiguity/concurrent launches must converge on the same records; a collision is `409`. Update last-seen metadata without creating duplicates. An ensure call must not reactivate a revoked registration implicitly.

Use the existing repository abstraction and schema/bootstrap mechanism; assign one authoritative backend module/owner. Scope native registration/rebind permissions separately from browser session resolution. The ordinary web relay must not expose native registration through its generic forwarding allowlist. Reuse an existing capability/credential policy where possible; never authorize registration merely because a caller supplies a well-formed IP or local marker. If native and web callers currently share one unrestricted gateway bearer, introduce purpose-scoped permissions/credentials before enabling registration through that boundary; the browser resolver must not inherit workstation-registration authority. A shared deployment credential does not establish individual user/device ownership: document the existing trust scope rather than claiming per-user isolation.

Add **Registration** status to the native connection/workspace surface: active backend, selected absolute local path (local display only), IP/source interface, workstation and workspace names, canonical Connect ID, last verification and readable errors. Read/copy existing registration at any time; reverify before launching. Reuse this ensure operation before any native/headless interactive submission that requires registered context, even when the user has not opened Intel Web yet. Changing the gateway must resolve/register in the new backend, not reuse a cached receipt from another deployment. A local DB receipt cannot unlock a remote site. Do not persist a successful registration state after a failed/ambiguous call; reconcile by the same idempotent operation. A valid existing record is reused rather than “new record on every launch.”

Only open the browser after registration succeeds and the URL is validated. On failure retain a retryable native state; do not launch an unregistered session. If opening the browser itself fails, retain the valid registration and retry the opening step without duplicating records. Starting a stopped web service remains an external operator/development task, never an action performed by the native process. The native app need not stay open after registration/session establishment.

## 4. Web access, job provenance and workstation activity



### 4.1 Gateway capabilities and advertised Intel Web URL

Extend **`GET /api/v1/capabilities`** rather than creating a competing discovery service. Keep currently supported semantics and publish the required current contract/capability versions for registry, workspace controls, configuration epoch and readiness. Coordinate native/web/backend rollout; reject unsupported old contracts with an upgrade message rather than adding legacy adapters. Optional fields remain explicit. Missing required scope/reset contracts cannot authorize legacy loaders or guessed scope; current backend-global settings are intentional, not a legacy fallback. Proposed payload fragment:

```json
{
  "api_version": 1,
  "instance_id": "gateway-instance",
  "deployment_id": "stable-backend-namespace-id",
  "connect": { "version": 1, "registration_required": true },
  "workspace_control": { "version": 1, "purpose": "registration_and_activity" },
  "resource_scopes": {
    "atlas": "backend", "brain": "backend", "tools": "backend",
    "models": "backend", "profile": "backend",
    "native_jobs_logs": "workstation_triggered_activity"
  },
  "search": { "actors": { "version": 1, "engine": "tantivy" } },
  "configuration_epoch": "fresh-config-v1",
  "build": { "version": "0.2.1", "revision": "<full-source-sha>" },
  "links": {
    "intel_web": {
      "url": "https://intel.example.test/",
      "connect_url": "https://intel.example.test/connect"
    }
  }
}
```

The example is proposed schema, not the current response. `links.intel_web` is `null`/absent when unconfigured; a configured URL is **not** a liveness assertion. Preserve existing `categories`, `auth`, `features` and `live` fields. Scope metadata describes capabilities, not permission; advertise/enforce workstation-triggered native activity without an all-node fallback. Authoritative startup identity/readiness stays in the existing readiness contract. `deployment_id` is persisted for the target backend/database namespace and is distinct from a process-specific `instance_id`; it must not change on every gateway restart. Advertise `connect-registry-v1` as an additive feature. An older backend without this contract remains explicitly unsupported for the new registered web-launch flow; never bypass the gate to retain compatibility.

Define a validated gateway runtime setting, proposed **`FLEETWORKS_INTEL_WEB_URL`**, for the externally reachable browser URL. This differs from a container-internal relay upstream. Wire it through host config, Compose and deployment docs. External local deployment tooling derives it from the selected published web port; remote operators set their public HTTPS URL. Keep it and the advertised `connect_url` consistent with web public-origin/base-path configuration. `connect_url` must use the validated Intel Web origin and correct application base path; do not concatenate an unchecked path or trust a browser-supplied redirect. Do not synthesize it from untrusted Host/Forwarded headers, container names, or an assumed port 3000.

On connect and before launch, obtain/refetch capabilities. **Open Intel Web** first ensures registration under §3.4, displays the server-returned Connect ID and destination, and only then opens the advertised Connect form in the OS default browser through a safe URL-launch API, without shell interpolation. Opening `/connect` explicitly always shows the input form, even if that browser has a prior context session; do not silently reuse a different workspace. Do not place the Connect ID in a URL or automatically skip the requested prompt. No bearer in query strings, fragments, clipboard, or browser launch arguments. Remote links must be HTTPS and cannot unexpectedly point to the reviewing Mac's loopback; loopback HTTP is limited to a local profile. Confirm a changed/new advertised web origin and reject unsupported schemes, userinfo and credentials. Missing/invalid URLs disable launch with an explanation; do not silently fall back to localhost. Browser-open success is not web-health success.

If the configured web service is stopped or unavailable, show the advertised destination and a truthful error/Retry action; point to the external deployment guide. **Do not offer Start Intel Web, run Next.js, or execute Docker from the native client.** The same rule applies to local and remote endpoints. Native API-backed control surfaces continue working when their gateway is available even if the web service is not.

### 4.2 Mandatory server-side Connect ID gate

Create an accessible **Connect ID** text input and Connect action at `/connect` (respect the deployment base path), with loading, invalid/unregistered, revoked, incompatible-backend and unavailable states. Preserve safe same-origin return destinations for direct links. The input must resolve an **existing active** registration in the web server's configured target backend. Unknown IDs never register themselves; wrong-backend IDs and invalid/unauthorized registrations do not unlock anything. Rate-limit resolution and return non-enumerating errors outside an authorized context.

Proposed same-origin web endpoint: **`POST /api/connect`**, calling a scoped gateway **`POST /api/v1/connect/sessions`** operation. Accept the Connect ID in the request body, not a URL; verify the current web authorization boundary, registration and deployment. The browser cannot select the gateway, owner, workstation/workspace IDs, or upstream bearer. Issue an opaque, expiring, revocable connection session bound to the validated registry references and current web access scope. Cookie: HttpOnly, host-only, explicit SameSite, Secure for HTTPS, appropriate path; document the loopback-HTTP development exception. Add CSRF and Origin checks for session creation/switch/logout and other mutations. Tokens and backend bearers never enter browser JavaScript, localStorage, query strings, or review logs. Successful registration is a prerequisite to connection, not a replacement for remote ingress authentication.

**This is a server-side gate, not only a React redirect.** Centralize verification and apply it before rendering protected server/client data, processing Route Handlers or Server Actions, forwarding REST, and accepting WebSocket/live upgrades. A Next Proxy/middleware redirect or root-layout check alone is insufficient. The existing custom `server.mjs`/`live-relay.mjs` upgrade path must use the same validation. Missing/expired/revoked sessions redirect page requests to `/connect` before data is rendered; API requests return `401`/`403` and upgrades are rejected before forwarding. No app shell, page prefetch, RSC/Flight response, export, image/data proxy, or cached protected payload may leak data before validation. Allow only the Connect form, necessary static assets, access-login/callback/logout paths, and explicitly reviewed non-sensitive health/bootstrap endpoints; keep this allowlist narrow and tested. No developer header or query parameter disables the gate.

Verify registration/session status authoritatively at data/operation boundaries; fail closed when verification is unavailable. Use no-store/private behavior for session-dependent data and gate responses. On logout/revocation/expiry, close live sessions and stop protected data delivery; validate subscriptions and enforce bounded revocation/expiry handling. Clear client caches on connect/disconnect/switch. Bind query keys to deployment + workspace + context revision. **Cross-tab safety:** session switching must not silently attribute an old tab's brief request to the newly selected workspace; mutations include an expected context revision, checked against the server session, and stale tabs must reconnect (`409`). Never fall back to a process-global or “most recently connected” workspace.

The browser may be on a different IP from the native workstation; its IP is not the registered workspace identity. The native process may close while the valid web session continues. Direct web visits remain supported after supplying an already registered Connect ID. These requirements add registered context and attribution, **not** a promise that a guessed Connect ID authenticates a person or that the shared article corpus becomes a separate tenant database for every folder.

### 4.3 Remote web deployment and relay boundary

[`apps/web-desktop/server-origin.mjs`](https://github.com/xxxcess/argos-osint/blob/f8bc447c86f59c369a9259c6d0739681fadd1e94/apps/web-desktop/server-origin.mjs) currently enforces loopback web origins; [`apps/web-desktop/server.mjs`](https://github.com/xxxcess/argos-osint/blob/f8bc447c86f59c369a9259c6d0739681fadd1e94/apps/web-desktop/server.mjs) uses that policy. Support a validated configured HTTPS external origin behind authenticated ingress, including REST, WebSockets and correct proxy handling. Keep internal upstream addresses separate. Do not loosen all Host/Origin checks or expose the privileged relay unauthenticated. Protect the entire remote web surface with the deployment's authenticated ingress/session boundary, including upgrades. Do not invent new account infrastructure when a documented authenticated ingress satisfies the requirement.

The browser authenticates through its supported web access path; the native bearer is not silently transferred to it. The additional Connect ID gate selects a registered context and does not replace that access boundary. Prove local and remote browser reachability, registration in the intended backend, and correct backend/session association. A discovery endpoint passing alone does not establish remote-web support.

### 4.4 Durable job origin and request associations

Every **new interactive brief job** requested through Intel Web must carry the validated connection context. Cover visit-time ensure, explicit refresh/reload, retry/resume paths that admit new work, and child/stage dispatches—not only the visible Generate button. The reviewed gateway's `/api/v1/brief/ensure` and `/api/v1/brief/reload` handlers currently call `intel_service` without context; extend that path through private Brain forwarding to the actual durable admission code.

At minimum persist:

```text
origin.deployment_id
origin.registration_id
origin.workstation_id
origin.workspace_id
origin.connect_id_at_submission
origin.workstation_ip_at_submission
origin.workspace_name_at_submission
origin.source = intel_web  (or a separately authorized native/system source)
origin.request_id / submitted_at
```

The validated gateway/session supplies these fields; browser-provided strings or headers cannot assign them. The web relay strips untrusted context headers and forwards only a backend-verifiable session/context proof alongside its server credential. The gateway resolves it to an immutable context; private relays forward that context only across their authenticated internal boundary. Direct public brief APIs must not provide a bypass by omitting `source` or pretending to be a system caller. Native/headless interactive submitters supply a registered authorized context; any genuine scheduler/system bypass is an explicit trusted service policy, not an unauthenticated fallback.

Persist the origin snapshot together with durable job acceptance, or in the existing transactional admission/outbox record that guarantees it reaches the job before execution. Do not enqueue first and attach context in a best-effort follow-up. If registry verification or attribution persistence fails, reject admission. Preserve the repository's current SurrealDB/Jobs ownership and fences; do not claim an atomic transaction across unrelated datastores or complete the pending Jobs persistence migration as part of this feature.

**Deduplication:** keep existing article/generation admission fences. If an ensure call reuses an existing brief job, never rewrite its original creator. Persist a separate request-to-job association containing the requesting registration, workspace/workstation, snapshot, request ID and timestamp; the new caller can see its association without duplicating inference. Keep idempotency scoped to the backend, registered context, operation and request key, while preserving legitimate cross-request job reuse. Check active registration at admission and at a new retry request, not by requiring the browser session to remain alive for every accepted background stage. A retry records who requested it while retaining the original creation context; genuinely new jobs receive the initiating context and children inherit it. Reload generations, worker restarts, queue redelivery and publication must not lose attribution.

Expose origin/request associations in native Jobs/Logs only within the workstation membership boundary in §4.5, and in web details under their validated session policy. A workspace filter only narrows native workstation activity. Rebinding/renaming a workstation or folder never rewrites past `*_at_submission` values. Keep historical work without reliable attribution as `legacy/unattributed` in backend history; exclude it from native workstation activity unless a verified request association exists. Never invent provenance. Introduce additive schema defaults for old records, but require valid context for all newly admitted interactive jobs after cutover. Session expiry or closing either client does not cancel already admitted work. A registered ID is necessary for web access; it does not grant unrestricted access to every job or change existing corpus-sharing policy.

### 4.5 Native Jobs & Logs: workstation-only triggered work

This supersedes earlier all-authorized-workspace, backend-activity and system-feed modes. **Native Jobs & Logs shows only work durably associated with the scoped registered workstation**, across its workspace children by default. A workspace filter narrows this set; it never widens access. Backend-global resource configuration does not grant visibility into every backend job/log.

Resolve stable `workstation_id` within the selected deployment and caller’s authority. IP is a display/selection aid: an IP-only request must resolve through the gateway to one authorized registered node. Unknown, ambiguous, mismatched or unauthorized IDs/IPs yield setup/error, never unfiltered fallback. Do not search IP substrings in logs or use browser/relay/proxy IP. Rebind preserves stable-node history; later IP reuse must not transfer another node’s work.

| Category | Eligibility |
| --- | --- |
| Native cycle/tool/brief/seed/train/reindex request | Include when durable origin matches the scoped workstation and caller access. Show backend impact separately. |
| Web brief requested through that node’s Connect ID | Include from validated registry/session origin even if browser IP differs or native has closed. |
| Deduplicated/reused job | Include the workstation’s genuine durable request association and permitted shared status, without changing creator or exposing another requester’s private input/events. Same actor/article alone is not association. |
| Causal child/stage/index job | Include through persisted parent/root/request provenance that survives queue delivery, retries and restart. Do not infer association from time/resource names. |
| Another node’s work; unrelated scheduled/system maintenance; unreliable legacy attribution | Exclude from native lists, suggestions, counts and streams. Preserve backend history without inventing ownership. |
| Global application/service/container logs | Exclude. Bounded readiness summaries are not a global log viewer. |

For mixed-origin coalesced work, filter requester-specific events and expose only allowed shared execution events. If safe separation is unavailable, show permitted progress/result metadata and explicit logs-unavailable status, not another requester’s detail. System-origin tasks need a genuine persisted causal/request association to become visible; the UI cannot attach arbitrary jobs just for access.

Apply one server-side membership/permission policy **before** list/detail/stages/result/log/search/export/count queries, pagination and live delivery. Guessed job IDs, deep links, empty scope, forged context and wildcard exports cannot bypass it. Validate scope at subscription/upgrade and during expiry/revocation. Reuse existing Jobs storage, typed gateway contracts and justified association indexes; do not fetch all jobs then filter in React/Rune or parse raw logs for ownership.

Membership does not grant retry/cancel/admin rights. Recheck action permissions, current scope/revision and shared-job ownership; a requester cannot cancel another node’s useful shared computation because it reused that job. New requests preserve origin and deduplication rules. All View Job/Logs links from Atlas/Brain/Tools/Workstation obey this same boundary. Other-node maintenance may contribute safe global readiness status, but not a bypassing job-detail link.

The header displays backend, stable node ID/IP and optional child filter. No all-backend/all-node/system toggle. Cache/subscription keys include deployment, principal/access epoch, workstation/scope revision and narrower workspace filter. Switching node/backend/auth cancels old reads/streams and rejects late replies. Selecting another child does not change node membership or global configuration; retain retired-child history under its node when authorized.

Tests use two nodes, two children on one node, a second deployment, web/native requests, a shared deduplicated job, causal children, unrelated system and unattributed rows. Assert included/excluded IDs/counts and direct detail/export/live/action denial; exercise IP rebind/reuse/collisions, expired/revoked sessions and stale switching. Native fixtures must show the correct header, narrowing filter and absence of a global mode. These server assertions supplement—not replace—Rune image/state tests.

## 5. Backend readiness and setup operations

Workstation is the local identity/connection page, with a distinct **Backend readiness & maintenance — <deployment>** section. Its shared-resource controls do not imply that the native node hosts or owns the backend. The page opens offline/before registration for connection diagnostics; protected reads need the appropriate credential and context-required jobs need registration.

Reuse one connection/registration state. The header identifies gateway, stable deployment, native IP, selected child, registration and Copy Connect ID. Global safe status can be shown even for other-node/system activity, but detailed Job/Logs links must meet §4.5. Missing registration disables applicable mutations, not setup or permitted diagnostics.

### 5.1 Readiness panels and truthful status

| Panel | Required information | User actions |
| --- | --- | --- |
| Gateway and connection | Client-observed reachability/TLS/authentication result; backend/API compatibility and identity; last successful check and current freshness. A responsive HTTP server is not necessarily a compatible, authorized backend. | Edit Connection, Reconnect, Refresh Checks. |
| Backend services | Gateway-reported datastore/schema, Jobs admission/worker availability, and relevant private Brain/Control/NLP/classifier/search service readiness. Describe the affected capability rather than make unrelated optional services block the whole page. | Refresh; view authorized diagnostics/job logs; link to external operator instructions. No start/restart/container controls. |
| Workstation/workspace | Selected local node/folder, registered records in this deployment, Connect ID, current registration revision and any collision/revocation. | Register/reverify, Copy Connect ID, existing registered Intel Web launch. |
| Actor catalog | Required/optional-for-workflow flag; dataset identity/hash; seed receipt state, processed/total counts and aliases when known; last/active job; completion and discovery/index readiness separately. Preserve `needs_seed`, actors and completion semantics from the existing backend catalog status function. | Select a backend dataset or upload an actor CSV, **Seed Actors**, View Job/Logs. Retry/resume only through supported backend job semantics. |
| Actor search index | Tantivy service, schema/generation, durable ingestion job, seed generation versus reader-visible checkpoint, pending/error/lag state and observation time. A seeded database is not necessarily searchable. | Refresh, View Index Job/Logs and permission-checked Rebuild Actor Index. No local indexing/container control. |
| Classifier inference | Actual active engine, worker/service state, pinned model/checkpoint identity/hash and verification state. Show available/unverified/missing/corrupt/busy conditions distinctly; never label an unverified model inference-ready because the service responds. | Refresh, inspect diagnostics and relevant job. No automatic model load, download, training or activation during a status refresh. |
| Classifier training | Supported trainable engine; selected training dataset/hash, dataset validation, training availability, active/latest job and stage/progress, last outcome, candidate model and active model separately. Untrained/optional is not automatically a failure of another active inference engine. | Select/upload the correct training corpus, **Train Classifier**, View Job/Logs; explicitly confirm supported activation separately from training. |

Each backend check must provide a typed ID, scope, state (`ready`, `not_ready`, `running`, `degraded`, `unknown`, `unavailable`, or `not_applicable`), stable reason code, safe explanation, observation timestamp/freshness, applicable capabilities, and relevant job/artifact references. Include an action's permission/preconditions and a disabled reason, but enforce them again on the server. Render states with text/icons, not color alone; expose accessible names and progress. Missing data is **unknown**, not success; old green results become explicitly stale after timeout/disconnect. Keep client transport/auth failures separate from server-reported resource failures.

Group readiness by use: connection, actor discovery, classification, and training. Distinguish **ready to start seeding/training** from **previous run completed**, **candidate produced**, and **model active/verified**. An unseeded catalog must not disable the very Seed Actors operation that repairs it. An absent trained Natural model must not block a ready Rust-BERT deployment or the Connect form. Downstream features enforce their actual prerequisites server-side; a Connect ID is not a readiness bypass. Do not require every check to be green before viewing this page or using unrelated healthy features.

Use a bounded, read-only gateway aggregate backed by existing readiness and durable status owners. Preserve the existing lightweight `/api/v1/health/ready` contract; do not make load-balancer readiness depend on optional training or full dataset scans. Return partial check results when a private service fails; report a structured unavailable state rather than fabricate empty success. Do not return secrets, unrestricted paths, raw container internals, or accept arbitrary check URLs. Fetch only configured private services. Full catalog parsing, training, mutations, inference warmup and image/container actions are prohibited side effects of a readiness GET.

Provide manual **Refresh Checks** plus conservative, debounced refresh while visible; prefer existing events for active-job changes. Coalesce in-flight checks and bound backend/cache work. Pause unnecessary polling when hidden, back off after errors and ignore replies for an old gateway/registration. Show timestamps and pending states without blocking Rune's UI thread. Reopening the page fetches persisted server state, not a process-local success flag. Stable job IDs survive app closure and reconnect.

### 5.2 Seeding, training, activation and durable admission

**Selection and confirmation.** Support authorized server-configured datasets by opaque ID, and a native file picker plus bounded streaming upload when the input exists only on the Mac. The selected registration folder is not a server-visible filesystem path. The backend stages immutable, hash-verified bytes and validates CSV purpose/schema before admission; never pass a Mac path or arbitrary server path as the job input. Actor catalogs and classifier training corpora are distinct dataset kinds. Respect existing operation-specific limits (reviewed seed worker: 256 MiB; Venture training snapshot: 128 MiB), authorize/validate uploads, expire abandoned uploads, and do not raise every small-JSON endpoint's body limit or base64 a whole catalog into a command request. Advertise/verify the effective limits rather than assume the values never change.

Before submission display deployment/gateway, registered workstation/workspace, operation, dataset/hash and **affected resource scope**. Actor catalog seeding is shared within the target backend database at the reviewed baseline, and training targets server-owned artifacts; Connect ID records request provenance, not a promise of a private actor corpus/model per local folder. Show that shared impact and require the corresponding mutation permission; registration alone does not grant it. Do not let a body-supplied backend directory, workstation or user field redefine the target or caller.

**Seed Actors.** Submit a confirmed, idempotent API request. The gateway admits a durable operation to the existing Linux worker/service machinery and returns a job receipt promptly. Reuse `seed_actors_pinned` and its completion/lease semantics; do not replace or clear the database, erase analyst edits, or create duplicate catalogs per native workspace. Persist dataset/hash, progress and outcome and expose cancellation/interruption honestly. A complete actor count alone is insufficient: use the durable receipt and discovery checks. Partial failure may retain partial progress; never label it complete or silently fall back to article-scoped actors. Reconcile/retry according to the existing seeding contract. Each committed batch must also produce durable actor-index work (§6); show Database seeded and Actor search ready as separate outcomes, and ensure indexing catches up even when seeding reuses existing actor records.

**Train Classifier.** The inspected `services/api-host/src/classifier_train.rs` trains the optional **Natural** classifier and explicitly leaves Atlas admission on **Rust-BERT zero-shot**. Label the button/confirmation accordingly, e.g. **Train Classifier — Natural (optional)**, and display the backend-advertised engine. Do not claim it retrains Rust-BERT or make Natural training a global startup prerequisite. A different training engine is supported only when it has an implemented/versioned backend contract; this UI cutover does not add a new algorithm.

Submit the selected validated dataset to a durable backend job, with activation off by default. Show training stage/progress, candidate artifact identity, report/provenance and errors through Jobs APIs. Successful training is not automatic model promotion or proof of inference readiness. Provide a distinct, authorized activation confirmation/action when supported, pinned to the exact candidate/hash and expected prior model revision. If activation is implemented as an explicit option on an existing request, make that separate consent visible and test it; never silently select it. Retain the prior valid model on failed/cancelled training or failed activation, honor synthetic-baseline safeguards, and show which model/engine the action can actually affect.

**Admission, retries and lifetime.** Both operations target shared backend resources. Their native submissions require backend-maintenance permission plus a registered origin; record immutable `deployment_id`, `registration_id`, workstation/workspace IDs and submission snapshot with `source=native`. Reuse §4.4's attribution conventions, not its web-only source value. Persist attribution with durable admission; propagate it through child tasks, retries and results. Return `202 Accepted` only after durable acceptance, with job ID, operation, dataset/hash, backend resource scope and separate origin/context; return an existing receipt for an idempotent replay. A job acknowledgement is not success. Resolve uncertain responses by the same request ID, not a new click/request identity.

Enforce concurrency/leases at the actual shared resource scope so two different Connect IDs cannot accidentally run competing catalog writers or publish conflicting model artifacts. Reuse an existing authorized job or return a truthful busy/conflict result; never overwrite its creator. Retry/cancel are explicit supported job actions, and cancel acknowledgement is not completed cancellation. The gateway validates permissions, current dataset identity, registration and preconditions again at admission. Closing the page/app, losing a session or changing the gateway cannot cancel already accepted work. Reconnect to the original deployment to recover job history; never relabel that job as belonging to the newly selected backend. Completion triggers a fresh readiness snapshot; running status alone must not erase a previously valid catalog/model readiness state.

### 5.3 Gateway contracts and server implementation seams

The following names are **proposed endpoint contracts, not implemented routes**. Reuse an equivalent existing route if subsequent source changes provide it; document the final mapping once, in the existing API/endpoint coverage inventory. The readiness/setup page adds no separate dashboard database or scheduler; it also surfaces the explicitly requested search service from §6.

| Control | Gateway contract / owner |
| --- | --- |
| Basic connection | Existing `GET /api/v1/health/ready` and `GET /api/v1/capabilities`, preserving their semantics. |
| Readiness dashboard | Proposed authenticated `GET /api/v1/workstation/readiness`; server owns the aggregate and per-check scope/status/freshness. May be read before registration by an appropriately authorized caller; attach registration state when available. |
| Dataset choice/upload | Reuse/add authorized dataset metadata/upload endpoints; return an immutable opaque dataset ID, kind, validated hash and size. Keep this separate from small operation JSON. |
| Seed Actors | Proposed `POST /api/v1/actors/seed`: request ID, dataset reference/hash and expected validated context; backend resolves target and caller, admits work and returns receipt. |
| Train Classifier | Proposed `POST /api/v1/classifier/train`: supported engine, request ID, dataset reference/hash and expected context; default no activation. |
| Candidate activation | Reuse/add an explicit authenticated activation operation/consent with exact candidate and expected-prior-model guard; no generic execute-command endpoint. |
| Progress, result, retry/cancel | Existing Jobs detail/stages/logs and supported retry/cancel routes, extended for these operations as needed; View Job opens the same native Jobs view, not a second job system. |

Expose required capability/permission metadata and contract tests. In the reviewed source, Venture invokes one-shot Compose workers for these tasks, and the inspected public Control routes do not expose the requested dashboard/actions. **Do not import that launcher, remotely wrap its Docker command, or parse its stdout.** Extract/reuse the server-side actor-seed/training domain functions under the existing Linux worker dispatch and persist their status/receipt. Extend the current dispatcher where needed; no Docker socket/SDK or container-launch API is needed in the native client or gateway. Backend process supervisors remain external.

## 6. Tantivy search and durable ingestion

**Required separation.** Add a private Linux actor-search service built with Tantivy. Every native query—including autosuggest/paging—and dossier selection uses the configured authenticated gateway. The gateway/Brain adapter reaches private search; selected actor detail comes from SurrealDB. Never advertise an index origin/token to the desktop or add a direct-index speed shortcut. Tantivy is a library, not a ready-made HTTP server; a small repository-owned Rust service supplies transport, authorization, indexing and operational state. Keep it in the Linux backend component, not a fourth frontend or an embedded native-app engine. SurrealDB remains authoritative for actor identity, dossier fields, lifecycle, relationships and permissions. Search documents are disposable, rebuildable projections—not a replacement actor database.

```text
Seed Actors -> gateway -> durable seed work -> committed SurrealDB actors + index outbox
                                              |
                                     actor-index ingestion worker
                                  reads persisted actor/alias records
                                              |
                                  private Tantivy search-index service
                                      commit -> reader reload

Brain search bar -> gateway -> Brain/search adapter -> Tantivy candidate hits
                         (authorize/filter candidates against current actor state)
Click result    -> gateway -> Brain actor-dossier read -> canonical SurrealDB record
```

**Upstream basis.** The supplied review recorded Tantivy **0.27.0** from the [user-provided documentation](https://docs.rs/tantivy/latest/tantivy/). Select and lock a verified compatible release (0.27.0 is the examined candidate), record the dependency/index-format/schema/tokenizer versions, and check its toolchain/native dependencies on Linux AMD64 and ARM64. Use the documented [`IndexWriter`](https://docs.rs/tantivy/latest/tantivy/indexer/struct.IndexWriter.html), [`IndexReader`](https://docs.rs/tantivy/latest/tantivy/struct.IndexReader.html), and [`Index`](https://docs.rs/tantivy/0.27.0/tantivy/index/struct.Index.html) APIs; do not assume an in-memory test demonstrates durable recovery. Validate exact API signatures against the selected pin before coding.

### 6.1 Private service, projection schema and query contract

Create a small backend package, proposed `services/search-index/` / `fleetworks-search-index`, with one HTTP service and a serialized write owner per actor index directory. Keep Tantivy out of `fleetworks-contracts`, the desktop dependency graph, and unrelated service/client build graphs. Reuse existing HTTP/auth/diagnostic conventions rather than adding another gateway or scheduler. The ingestion worker is an existing-worker task/role, not another orchestration framework. It reads SurrealDB through the existing repository abstraction; the index service accepts only authenticated, bounded internal projection batches. A browser/native client cannot submit arbitrary indexed documents or select an index directory.

Persist indexes in a dedicated backend-owned volume, keyed by stable deployment/database identity, logical index and schema generation. Never mount one writable index into competing service replicas or force-remove a writer lock to make a second process run. Use explicit writer memory/thread/query concurrency budgets and blocking-worker isolation; do not block HTTP async executors with index commits/search. Preserve the index across ordinary service recreation. Do not mount client workspace folders, provider credentials or the Docker socket into this service.

| Field group | Actor projection contract |
| --- | --- |
| Stable identity | Exact `actor_id` from the canonical SurrealDB actor, deployment/database identity, catalog/source ID when present. Document key includes the authority scope; never expose a Tantivy `DocAddress` as an actor ID. |
| Searchable names | Canonical display name, normalized exact-name field, alias values and their supported strength/provenance, tokenized name/alias fields and bounded prefix fields. Use proper-name Unicode analysis; do not stem away identity or discard non-Latin names. |
| Discovery metadata | Allowed short description, actor type, country/operation-country and source facets only when present in authoritative records. Do not index raw secret/configuration payloads or the full dossier graph. |
| Fences | Monotonic **projection revision** that changes for relevant name, alias, metadata, visibility and lifecycle changes; source/catalog generation, schema/analyzer version and content checksum. Do not assume the actor row revision already covers independent alias changes. |
| Visibility/lifecycle | Current eligible/active state and server-controlled access scope. Authorization is never a query-string-selected tenant or workspace. Tombstone/version handling prevents old writes from resurrecting deleted records. |
| Result payload | Canonical actor ID, safe display label/summary, matched field/alias when known, relevant facets, index/source revision and generation. Relevance is a retrieval score, not confidence, identity resolution or corroboration. |

Use **one document per canonical actor**; retain ambiguous/shared aliases as separate actors. Rank exact names and supported aliases ahead of broader token/prefix/description matches; define deterministic tie-breaking by stable identity. Bound query length, token count, prefix/fuzzy expansion, results, execution and response bytes. Plain search text is not unrestricted Tantivy query syntax: use typed queries or a constrained parser/field allowlist. Empty input shows a prompt or explicitly bounded recent state, never an unbounded scan. Start with the existing actor-search limit (up to 25), measure a representative catalog, and expand only with a tested contract. No arbitrary regex or deep-offset query interface.

| Surface | Route and behavior to implement |
| --- | --- |
| Native Brain discovery | Extend the existing `GET /api/v1/actors/search` with a current-contract discovery mode, e.g. `query=...&mode=discovery&limit=20`; gateway/Brain delegates to Tantivy and returns typed hits plus index freshness/completeness. Reuse this route rather than create duplicate UI search APIs. |
| Authoritative dossier | Keep `GET /api/v1/actors/detail?id=<actor_id>`; resolve canonical redirects and read the authorized active actor/associations from SurrealDB. Use the stable ID returned by discovery, not a name-based second search. |
| Index status | Extend Workstation readiness with service reachability, schema/generation, seed generation, pending work/lag, committed and reader-visible checkpoints, counts where known and last safe error. This read must not start a rebuild. |
| Repair/rebuild | Proposed authorized `POST /api/v1/search/actors/reindex` returns an attributed durable job receipt. Internal batch/status endpoints remain private and purpose-authorized; the public web relay does not gain index-write permission. |

Keep the service private on the Compose network; expose native functionality through the configured gateway, with dedicated internal credentials and explicit origin/host validation. Proposed deployment settings are `FLEETWORKS_SEARCH_ORIGIN`, `FLEETWORKS_SEARCH_TOKEN_FILE` and `FLEETWORKS_SEARCH_INDEX_DIR`; names must be reconciled with repository conventions and documented. No default public search-service port, unauthenticated admin route, arbitrary URL forwarding or direct desktop-to-database connection.

Apply authority filters before returning even search labels/snippets. For stale ACL/lifecycle projections, perform bounded canonical eligibility/authorization checks in Brain, omit rejected candidates and cap any overfetch. Fail closed when required authority checks are unavailable. Counts/continuations must not disclose inaccessible actors or claim exhaustive results after filtering. Within one request use one searcher snapshot; across requests bind any opaque continuation to query, scope and index generation, or return an explicit bounded/truncated response. Do not treat segment document addresses as stable pagination tokens.

### 6.2 Seed-triggered outbox, visibility and recovery

**Seeding must automatically arrange indexing of the actors that actually committed to SurrealDB.** Persist index work/outbox records in the same database transaction as each seed batch and its progress. A failed/rolled-back seed batch cannot publish search documents. A request handler's fire-and-forget task or the desktop staying open is not a delivery guarantee. Reuse existing durable job/outbox machinery; publish/consume it after commit, without keeping a database transaction open during an HTTP index request. A completed-seed/no-op retry must also ensure missing indexing work is reconciled; a previously seeded database still needs an initial index/backfill.

The ingestion task reads canonical actors/aliases in bounded ordered batches, builds projections and sends authenticated batches under §6.4 limits. Batch source/alias reads too; no per-actor network/SQL fan-out or full-catalog memory load. Do **not** index the uploaded CSV directly as an alternate truth source. Persist dataset/catalog generation, scan cursor or outbox position, source revision, batch identity and job provenance. Use at-least-once delivery with idempotent replacement/deletion, not a claim of exactly-once transactions spanning SurrealDB and Tantivy.

For each canonical document key, serialize `delete old key -> add current projection` (or delete/tombstone) and reject obsolete revisions; Tantivy does not supply application-level identity uniqueness/version fencing automatically. Persist the necessary revision/checkpoint/tombstone evidence with the committed index generation so a restart cannot accept an older update. Equal revisions with conflicting content fail rather than silently overwrite. Carry source database/reset-generation fences, not just timestamps; Tantivy operation stamps are local to an index and are not database revisions. Retry after an ambiguous response must converge without duplicate actors or stale resurrection.

**Acceptance order per bounded batch, not per actor:** apply bounded updates -> successful `IndexWriter::commit()` -> explicitly refresh the serving `IndexReader` -> verify the generation/checkpoint now served -> acknowledge durable ingestion progress. The documented default reader reload is asynchronous; use explicit reload for deterministic visibility. Distinguish disk-committed work from reader-visible work if reloading fails. Never acknowledge a search-ready checkpoint before it is visible; a crash after index commit but before database acknowledgement must replay safely. Do not “fix” retry by reseeding/clearing SurrealDB.

Hook subsequent actor creations, name/alias/description changes, merges/redirects, retirement/deletion and visibility changes into the same durable projection path. Preserve deletion tombstones long enough to fence delayed work; a source record missing during reconciliation means delete the stale document, not keep a stale hit. Backfill, ongoing writes and rebuilds need an explicit generation/watermark protocol so a slow scan cannot overwrite a newer event. Reuse current domain writer/lifecycle boundaries and test each producer; no new direct UI database writer.

For first adoption, missing/corrupt indexes or schema/tokenizer changes, build a new generation from authoritative actors with a consistent scan/change-catchup boundary. Verify it, catch up durable deltas, then atomically switch the serving generation and record its receipt. Keep the previous valid generation until the replacement is verified; bound retention and disk use. Do not clear the serving index at rebuild start or automatically erase an incompatible volume. An integrity failure is a visible degraded state and an explicit repair operation. Readiness polling does not trigger a rebuild.

**Status is two-stage:** `Database seeded` and `Actor search ready` are separate checks. Track seed and indexing child-job IDs, source/visible generation, processed counts, backlog, timestamp and diagnostic. Initial indexing may show `indexing`/partial results only if clearly labeled; never show “no actors found” as the explanation for an unavailable/unbuilt index. Catalog setup is search-ready only after the required seed generation and catch-up barrier are visible. Search failure does not undo a successful database import or stop unrelated gateway functions. Reindex/ingestion jobs retain original workspace/workstation provenance; automatic maintenance is explicitly system-origin rather than invented user ownership.

### 6.3 Brain search interaction and authoritative dossiers

Add an always-identifiable **Search actors** field, clear/search action, bounded live result list, keyboard navigation and loading/error/indexing/no-match states. Debounce edits (proposed initial 200 ms), coalesce identical input, allow one outstanding request plus the latest pending query, and supersede stale replies by query/deployment/authorized-access revision. Workspace-only changes do not partition the global corpus. No empty-query storm or retry of superseded input; gateway is always the destination. Use the existing asynchronous application runtime; Rune's `Latest`/attempt-fencing pattern is a reference, not a reason to start another executor or embed Tantivy in the app. Focusing or clearing search must not enqueue indexing or other jobs.

Click/Enter requests the gateway dossier API with that hit’s canonical ID; never hydrate every result with full dossiers or access SurrealDB directly. Show current SurrealDB fields and associations in ordinary labeled sections, tabs and bounded lists, including unavailable/truncated states; do not render a complete dossier from stored search snippets or draw a topology graph. Use §2.4 for the web-only visualization boundary. Handle redirects, deletion, changed permissions and unavailable SurrealDB truthfully. An out-of-order dossier response cannot replace the newly selected actor. Returning to results preserves the query/selection/scroll within the same context.

Expose `Actor search` separately in Workstation with View Index Job/Logs and an authorized **Rebuild Actor Index** repair action when needed; never offer container controls. Backend-wide indexing scope is shared where the actor catalog is shared. Deleting a local workspace or executing the user-config reset must not delete this shared index/corpus; invalidate access as required, retain job provenance and re-provision internal service credentials externally only when intended.

This replaces the **native discovery search path**, not deterministic actor identity binding, alias-ambiguity rules, article discovery or unrelated SurrealDB indexes. Keep exact domain lookup functions needed by ingestion/linking; retire duplicate UI search implementations only after callers migrate. No silent fallback to a different engine when Tantivy fails. Add a versioned search capability and reject missing required functionality honestly.

Measure search latency/ingestion throughput, memory, index size and incremental lag on a documented fixture/catalog size. Separate service time from network/debounce time. Do not promise an unmeasured speedup or run a full production catalog/provider workload in every native UI test. Required reliability and end-to-end cases are in §10.

### 6.4 Paced indexing and query protection

Seeding leaves durable projection work; one leased coordinator per deployment/actor-index resource drains it independently of native lifetime. Share limits across workspaces, replicas, live changes and rebuild generations. Repeated seed/reindex requests reconcile or associate with existing work, not duplicate indexes/ingestion floods. Use pooled clients and long-lived writer/readers, bounded source reads, safe revision coalescing and durable unacknowledged backlog; no future/thread/HTTP request per actor, unbounded channel or all-catalog memory load.

**Proposed initial limits—validate and measure, not throughput guarantees or Tantivy defaults:**

| Control | Starting policy |
| --- | --- |
| Batch | Up to **500 projections or 2 MiB serialized uncompressed payload**, whichever first. Bound source/alias expansion; invalid/oversized documents remain visible unresolved work, never silently truncated identity data. |
| Writes / rate | **One write batch in flight**, maximum **2 batch starts/second** with at least 500 ms spacing and no catch-up burst. Resource lease/fencing and private admission prevent multiplication across producers. |
| Staging | At most **two prepared batches plus one in flight**, one bounded source read. Stop fetching when full. The 6 MiB serialized-buffer ceiling is not total memory; budget parser/writer/merge/readers separately. |
| Partial tail | Flush around **1 second** after waiting, or scan end, when capacity permits. Commit/reload per emitted batch, not per document. Backpressure can extend visibility lag. |
| Queries | Separate bounded query capacity, initially e.g. **4 active searches**, with bounded wait/timeouts. Serve the last verified reader snapshot during writes; do not wait for the complete seed or lock every query behind writes. |
| Status | One coalesced visible-page refresh, initially at most every **5 seconds**, plus bounded manual refresh/events. Suspend hidden polling; no status call per actor/result/widget. |

Keep validated limits in one server-owned configuration. Native displays effective state; it never changes container/runtime resource policy implicitly. Move blocking index/search/commit work off async HTTP executors into bounded execution slots. Set writer threads/memory/merge budgets; no new writer, forced optimize/merge or index rebuild for each batch. Give search priority and bounded fairness to live deltas versus backfill; shadow generations share—not double—budgets.

An ingestion success receipt identifies the **reader-visible** checkpoint only after §6.2 commit/reload verification. Async queue acceptance is a different durable receipt; do not acknowledge source work merely because it entered memory/queue. Slow/ambiguous requests keep the same batch identity and cannot create overlapping unlimited attempts. Coalescing cannot advance a contiguous checkpoint past unresolved events or let stale upserts override tombstones.

One retry owner—the coordinator—handles 429/503, transport and uncertain acknowledgements. Honor valid `Retry-After`, including a longer server delay; otherwise use jittered exponential backoff (initial 1 second, cap 30 seconds). After an initial five transient failures, cool down 60 seconds or the longer server delay and allow one probe. Auth/schema/generation/oversize errors require correction. Do not layer immediate retries, lose permits, hot-loop or call a paused queue complete.

Slow/pause bulk work on full queues, query/commit latency, memory or disk high-water marks; recover conservatively below lower thresholds, without bursts. If durable backlog/storage limits approach, pause new bulk seed/backfill at a safe boundary, keep intent durable and expose the reason. Preserve healthy read-only functions and the last valid serving generation. Readiness GET/search does not schedule a rebuild.

Gateway candidate checks use bounded bulk eligibility/authorization reads, not full dossiers/per-hit network calls. Retain typed query limits, cancellation, latest-request fencing and permission-aware caching. Search throttling produces retained-data/retry feedback, never a direct-service fallback.

Expose queue depth/oldest age, processed/total where known, source/visible checkpoints, effective limits/pace, in-flight work, commit/reload timing, retries/cooldown and safe errors. Workstation uses plain status rows and eligible job links. Distinguish `Paused`, `Throttled`, `Indexing`, stale-but-readable and unavailable from fully current readiness.

Verify count/byte/rate/concurrency/queue limits, tail flush, one lease across replicas, Retry-After/backoff/cooldown, slow commits, disk pressure, ambiguous ack/replay, fairness and no-loss checkpoints with controlled clocks/barriers. Measure idle versus concurrent seed/reindex gateway search latency, memory/index size and lag on both Linux architectures using documented catalog/hardware. Set measured thresholds; small fixtures and configured rates do not establish production throughput.

## 7. One-time reset and client retirement



### 7.1 Controlled fresh-configuration cutover

**Mandatory breaking cutover, not a recurring app feature.** As part of implementing this specification, remove the current **Fleetworks/Argos-owned user API keys, authentication/session material and user configuration** from the explicitly inventoried local installations and target backend configuration scope. The user will re-enter gateway/provider credentials as appropriate and recreate model-role defaults. No old user keys or role settings are migrated or restored. This overrides earlier handoff language preserving user credentials or importing legacy configuration. This document update does not itself perform the reset or establish that a remote/local installation has been cleaned.

The scope is this application's data, not every key/configuration file on the machine. Use a reviewed allowlist of absolute paths, exact app-owned Keychain item identifiers, database records/namespaces and cache categories. Inventory applicable registered workspaces and explicitly selected legacy app homes; do not scan-and-delete unrelated directories, all of `$HOME`, all `.env` files, global Keychain entries, SSH keys, Git/GitHub authentication, signing identities or other applications' data. Never print secret contents to produce the inventory.

| Area | Required reset behavior |
| --- | --- |
| Provider/tool credentials and account configuration | Remove all inventoried app-owned saved user keys, tokens, account secrets and configuration entries, including backend-global copies, obsolete workspace/root copies, app-owned secure-store entries, legacy imports, secret exports/backups and credential-bearing caches. Remove cached references that would reload the same old values. New provider accounts are entered explicitly. |
| Model/default-role and user settings | Clear user assignments, ordered fallbacks, account bindings, old preference/config files and server-side equivalents. Preserve only canonical code/schema defaults with **no** user key or automatically assigned role. Global user keys/defaults start empty after reset and are configured once per deployment; new workspace directories contain no copies or overrides. |
| Native connections, user API access and web sessions | Clear saved app connection credentials and user profiles; revoke/rotate old user-facing gateway access material under the operator's controlled cutover and invalidate browser/native session state. Use a durable reset/configuration epoch so old cookies/clients cannot resurrect the prior context. The user configures the new gateway credential explicitly. |
| Deployment-owned infrastructure credentials | Database/internal-service credentials are not user model keys. Preserve them unless the reviewed external operator reset includes their coordinated rotation; never simply delete them and strand persisted data. Keep the minimum independent provisioning/admin route needed to issue a fresh user gateway credential. No unauthenticated recovery endpoint. |
| Files and domain data | Preserve both `~/.fleetworks` and `~/.fleetworks/default`, siblings and the safety/identity/recovery/reset ledger. Preserve datasets, model/checkpoint bytes, actor catalogs, articles, job/log history, repository source and storage volumes except individually inventoried credential/config payloads. Do not reset the database, `docker volume` data or unrelated files under a workspace as a shortcut. |

**Execution order and repeat safety.** Provide a small externally invoked, application-specific reset tool/procedure (proposed `scripts/fleetworks_config_reset.py`, not a GUI container launcher). It defaults to inventory/dry-run; applying requires explicit named targets, a recorded plan and the existing task's destructive-cutover authorization. Revalidate target ownership/default-path protection before changes. Pause new admissions and drain or explicitly stop credential-consuming work through supported external controls before invalidating secrets; no secret copied into a resumable job may silently survive. Invalidate/revoke old sessions/access, remove the inventoried values, clear app-owned credential/config caches, write a non-secret per-installation/backend reset receipt, then run first-use setup. A backend and a local filesystem are separate authorities: record completion independently and fail closed on partial reset. Do not claim the user's Mac is wiped from a remote CI pass.

The operation is idempotent and tied to a specific reset epoch. An ordinary `cargo build`, local helper invocation, CI workflow, first launch or later upgrade **must never wipe credentials**. Re-running a completed reset ID is a no-op and must not erase keys the user subsequently entered. If interrupted, resume only the frozen pre-cutover inventory; first-use writes stay blocked for the affected scope until it is safe. Keep the receipt/protected-path registry outside cleared user configuration. Do not generate secret-bearing backups or move wiped keys to Trash. Do not claim secure physical media erasure or upstream provider-key revocation from local file deletion; report any separately needed provider revocation/rotation or inaccessible backup/host explicitly.

**Remove backwards-compatibility code, not needed domain functionality.** Remove legacy configuration importers, alternate old homes searched automatically, legacy environment aliases, role-name migrations, silent legacy defaults, old client/TUI launch adapters and associated active instructions/tests once the new contract passes. The inspected `crates/fleetworks-workspace/src/import.rs` imports legacy `config.toml`/`auth.json` and role aliases: retire this path instead of reusing it for onboarding. Inventory the actual backend `provider`/`secrets`/configuration readers before editing; tests must prove stale files/environment cannot repopulate cleared settings. An external process's environment cannot be erased from the app—remove old names from supported readers and document removal/restart of inventoried app service configuration by the operator.

Use a single current configuration format and supported client/server capability set. Reject incompatible old clients/contracts with an upgrade/setup message; do not retain shims solely to keep the retired clients working. Minimal one-shot reset inventory adapters belong in the cutover tooling, not the product's normal configuration loader. Keep data/schema integrity and correct historical job reads: dropping configuration compatibility is **not** permission to corrupt retained domain data. Likewise, optional Natural training remains a supported explicit workflow from §5; its old “legacy” CLI label alone does not make its algorithm disposable. Remove obsolete command compatibility, not the promised training action.

**First-use completion.** The native app opens Workstation/connection setup, safely ensures the protected container and default-child identities, registers with the selected freshly configured gateway, and shows Profile/Models setup states. Provider keys are explicitly set once through backend-global Profile APIs and default roles through backend-global Models. The selected child workspace supplies origin, not a private configuration store. No old export/environment default/legacy workspace copy is imported. Authorized workspaces reference current global configuration without copying or inheriting legacy secrets. Verify the reset first with synthetic secrets in disposable local/backend fixtures, then record the separately authorized real cutover result without secrets. Implementation cannot be called fully complete until the real in-scope reset is executed or explicitly reported as an outstanding deployment action.

### 7.2 Retire terminal products only after replacement passes

**Delete terminal clients at cutover, not merely de-emphasize them.** Once both downloaded Mac apps pass the revised native parity matrix, topology is verified in its web owner and their consumers are migrated, remove the six standalone interactive/client executables, TUI/line-session code, client Docker targets, terminal-only tests and CLI release bundles. Audit the second Ratatui consumer, `crates/argos-osint-bin`, and migrate any still-supported behavior before deletion. Remove Tauri remnants if present; it is not an active workspace member in the reviewed root manifest.

Retain only explicitly justified external operator/deployment and headless automation/recovery operations, preferably thin adapters over shared code, and required service entrypoints. Venture may remain an operator tool for Docker/Compose; it is not bundled with, launched by, or required by the native client. Retiring terminal UI products does not require replacing deployment administration with a GUI. Port CI smokes off retired CLIs so tests do not force obsolete products to remain. Clap/Cargo and backend executables are not themselves reasons to retain a TUI. Do not delete domain data, migrations, shared infrastructure credentials or required Linux workers as terminal cleanup; the separate allowlisted reset above still applies. Treat documented command removal as a compatibility change under versioning policy.

Deploy current registry/scope/session/origin contracts and required indexes with explicit capability/configuration epochs; reject obsolete clients/no-context mutations. Keep setup and permitted diagnostics available while disconnected. The layout transition in §3.1 and actual authorized reset are separate recorded deployment actions. Preserve history and domain integrity; no blanket database reset or completion claim for the unrelated Jobs persistence migration.

## 8. Local development and rebuild selection



### 8.1 Prerequisites by development lane

Make this the quick-start structure in the root README, with short matching commands in the app README and detailed, linked procedures in `native-client.md`, `contributing.md` and `deploy/README.md`. The commands adding `build-native`, `platform` or the `backend` Bake group below are **required implementation interfaces, not features present at the reviewed baseline**. Land the helpers/targets and their tests with the documentation; do not publish an executable-looking quick start that relies on missing scripts or groups.

| Intent | Required local tools | Not required for that lane |
| --- | --- | --- |
| Run downloaded native app | Supported macOS; gateway credentials for live operations. | Rust, Python, Node, Docker, repository checkout. |
| Build/run native with Cargo | Git, rustup and repository-pinned Rust/Cargo; compatible Apple SDK and compiler/linker tools. | Docker, host inference engines, Node/pnpm, live backend. |
| Build a complete local `.app` using the repo helper | Native toolchain above plus supported Python 3 for the existing external candidate tooling (standard-library-only; document/test its minimum version). | Docker, Node/pnpm, release-signing credentials, live backend. |
| Build/run containerized backend locally | Docker Engine/CLI, Buildx and Compose v2; on Mac use a supported matching-architecture Docker Desktop or a documented compatible runtime. Git for a source build; Python only for optional repo platform/operator helpers. | Host Rust/LibTorch/PyTorch, native app build, host Node. Build tools inside images are not desktop dependencies. |
| Edit web on host | Repository-pinned Node and pnpm plus frozen dependency install; backend or test fixtures for API work. Containerized web development uses the pinned tools inside its image. | Rune build. |
| Dispatch/download CI outputs | Git and authenticated GitHub CLI (`gh`), only for this optional review step. | Publishing rights for an ordinary unprivileged build. |
| Use the full actor catalog | Git LFS only when materializing the tracked dataset, or an authorized uploaded/backend dataset. | Full catalog/LFS checkout for basic native compilation and UI fixture tests. |

**macOS native setup (explicit human actions, not build-script side effects):**

```sh
# Once, when Apple development tools are not already installed:
xcode-select --install
# Finish the installer, then verify the selected SDK/tools:
xcode-select -p
xcrun --sdk macosx --show-sdk-path
clang --version
git --version

# Once, when rustup is not already installed (official installer):
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
. "$HOME/.cargo/env"

# From the checked-out repository root, use its pinned toolchain:
rustup show
rustc -vV
cargo --version
# Needed for the repository's external build/package helper, not Cargo-only use:
python3 --version
```

Document cloning/selecting the actual implementation ref, or using an existing checkout without discarding its work. Link the official Python macOS installer for a missing `python3`; do not assume Apple ships a suitable Python. Select and test a supported minimum Python version for these existing/new helpers, and print it in prerequisite failures. Explain initial dependency/toolchain downloads versus later cached builds; native build does not mean network-free on first use. Check `RUSTUP_TOOLCHAIN`/directory overrides against the repository pin rather than silently using a globally installed latest release. Resolve the Rune/Fleetworks compiler compatibility in §2.2 before claiming the quick start works. Document whether the verified Rune revision requires full Xcode/Metal tooling, or only Command Line Tools for normal builds; shader regeneration and public notarization can have additional requirements. No silent installers, `sudo`, global toolchain changes or license acceptance in the build helper.

**Backend tools, only when the developer chooses to build/run backend containers:** install and start the compatible runtime separately. Docker Desktop provides the Engine/CLI and Compose; verify Buildx explicitly. On Linux follow Docker's Engine/Buildx/Compose plugin setup instead of a macOS installer. Check the minimum macOS supported by the selected container runtime separately from the native app's deployment minimum; an older Intel Mac may still use the native app with a remote backend even when local Docker is unsupported.

```sh
docker version
docker compose version
docker buildx version
docker context show
docker info --format '{{.OSType}}/{{.Architecture}}'
docker buildx inspect
```

Show the selected Docker context/builder and whether it is local before any build/start. Preserve the user's context and builders; do not silently switch to or create a remote builder. A remote context can build or deploy elsewhere even when the CLI runs on this Mac. Cross-architecture emulation is an explicit choice, not a hidden default. A build-only command may use BuildKit build containers but must not start the application stack, seed actors, train models or mutate a deployment workspace.

For host web development, read the Node pin and `packageManager` pin from tracked repository files; document their supported installer and `pnpm install --frozen-lockfile`. Keep these in the web lane, not native prerequisites. For CI dispatch, document installation of `gh`, `gh auth login` and `gh auth status`; neither login nor publication is a side effect of building locally. Document Git LFS setup/fetch only in dataset preparation. Do not automatically fetch a large actor catalog or model merely to compile the native GUI.

### 8.2 Supported local commands

These interfaces must be implemented and tested with the quick-start docs; their presence here is not proof that the current repository supports them. Build-only commands never start the product stack, seed actors, train models, reset credentials or mutate the real native workspace container. Human Cargo use stays supported; agents retain the established isolated wrapper/cache.

#### 8.2.1 Detect this Mac and build a complete app

Extend the existing `scripts/fleetworks_candidate.py` with a **`build-native`** subcommand; reuse its proposed `package-native` implementation instead of adding a parallel packaging framework. This is a developer command executed outside the application. Default `--target host` resolves the local physical Mac architecture, passes an explicit Rust target, builds the scoped native package and packages the result as `Fleetworks.app`.

```sh
# Build and package the native app for this Mac. Does not launch anything.
python3 scripts/fleetworks_candidate.py build-native --target host --profile debug

# Same build/package operation, followed by an explicitly requested app launch.
python3 scripts/fleetworks_candidate.py build-native --target host --profile debug --open

# Optimized local review artifact (not automatically published):
python3 scripts/fleetworks_candidate.py build-native --target host --profile release

# Preview the detected target, build command and artifact paths only:
python3 scripts/fleetworks_candidate.py build-native --target host --profile debug --dry-run
```

Required default output (print the **absolute** resolved path on success):

```text
dist/native/<rust-target>/<debug-or-release>/Fleetworks.app
```

Also write a local manifest with source revision/dirty status, toolchain, Rune pin, resolved architecture, profile, minimum macOS and development signing/verification status. An optional `--archive` emits a version/SHA/run-or-local-identity ZIP and checksums through the same packager as CI. The plain build command must produce the `.app`, not just print the path to a loose Cargo binary. A locally produced bundle is not automatically a published/notarized release.

**Shared platform detection, not duplicate shell guesses.** Add a small standard-library helper (proposed `scripts/fleetworks_platform.py`) reused by candidate tooling and the existing planner. Expose it through `python3 scripts/fleetworks_verify.py platform --format rust|docker|json` for documentation and external tooling. `rust` means the supported native application target; reject unsupported hosts rather than imply Linux/Windows client support. `docker` maps a supported host's physical architecture to a Linux backend image target. `json` reports OS, physical architecture, process architecture/Rosetta state, Rust toolchain host, and resolved target(s). Detection alone performs no build, installation, Docker invocation or backend request.

| Local machine | Native application target | Matching local backend image platform |
| --- | --- | --- |
| macOS Intel | `x86_64-apple-darwin` | `linux/amd64` |
| macOS Apple Silicon | `aarch64-apple-darwin` | `linux/arm64` |
| Linux x86-64 / ARM64 | No native Mac build in the host quick start. | `linux/amd64` / `linux/arm64` respectively. |

On macOS inspect `uname`/platform data **and** `sysctl.proc_translated`/hardware information where applicable, then compare with `rustc -vV`. A Rosetta process must not silently choose Intel as an Apple Silicon machine's default. For `--target host`, a translated/mismatched toolchain produces an actionable instruction to use the matching native terminal/toolchain; explicit cross-target builds remain supported when SDK/dependencies permit. Validate only the two allowed Mac triples. Installing a rustup target is not a substitute for checking the linker/SDK. `--open` must refuse an artifact for the other physical architecture in this quick-start path; cross-built artifacts are for testing on matching hardware, not a claimed local pass.

Resolve the repository root from the script location; quote paths/pass subprocess argument arrays, not shell interpolation. Honor Cargo's configured target directory and determine the binary from `compiler-artifact` JSON (checking Cargo's exit code, target/package and executable), not a hardcoded `target/debug` assumption. Preserve agent cache rules and user builds; do not run `cargo clean`. Stage packaging in a temporary task-owned directory and replace only the selected local output after success. A failed compile/package must not relaunch a stale binary as if newly built, delete the last good bundle, stop a running app/backend, or overwrite `/Applications`. Explicit `--open` uses macOS Launch Services; report when an already-running app must be gracefully closed/reopened so the exact new artifact can be reviewed. Do not force-kill it.

Required flags: `--target host|x86_64-apple-darwin|aarch64-apple-darwin`, `--profile debug|release`, `--output <directory>`, `--open`, `--archive`, `--dry-run`; document their defaults and tested interactions. Use the same packager and architecture/library/manifest checks in local and CI paths. The helper neither calls Docker/Compose/Venture/Next nor queries a live gateway; the launched app remains API-only.

#### 8.2.2 Direct Cargo build, run and test

These scoped commands must remain usable for rapid executable review without Python/package tooling. The package/binary names are implementation contracts, not available at the reviewed baseline.

```sh
cargo build --locked -p fleetworks-desktop --bin fleetworks-desktop
cargo run --locked -p fleetworks-desktop --bin fleetworks-desktop
cargo test --locked -p fleetworks-desktop
```

Without a configured target override, these use the **Rust toolchain host**, which differs from the physical-machine detection above when running under Rosetta. Explain `CARGO_BUILD_TARGET`/`build.target` and `CARGO_TARGET_DIR`; default untargeted output is `target/debug/fleetworks-desktop`. To choose explicitly:

```sh
rustup target add x86_64-apple-darwin aarch64-apple-darwin

# Intel MacBook
cargo build --locked -p fleetworks-desktop --bin fleetworks-desktop \
  --target x86_64-apple-darwin

# Apple Silicon
cargo build --locked -p fleetworks-desktop --bin fleetworks-desktop \
  --target aarch64-apple-darwin
```

Add `--release` for optimized executable builds. Explicit-target output is `target/<triple>/{debug,release}/fleetworks-desktop`, subject to output overrides. Cargo alone does not assemble a distributable `.app`; use the helper for Finder testing. Do not compile `--workspace --all-features` or host classifier/NLP engines for native review. Client fixture tests use mocks, not Compose. Agents use the equivalent `scripts/agent_cargo.py` commands with `target/agents`; that isolation rule does not replace human direct Cargo usage.

#### 8.2.3 Build backend Linux images separately

The baseline `deploy/docker-bake.hcl` group `server` builds the web/Venture/terminal-client images; it is **not** an accurate backend-only group. Replace/trim obsolete groups as their consumers retire and add **`backend`** for the actual backend image targets. Include the shared `runtime`, the dedicated `search-index` image and managed-database wrapper as appropriate, plus the classifier worker/native-library packaging required by the completed deployment. Keep `web`/`web_dev`, Rune apps and retired CLI images out of that group. Where roles share an image, build it once. The small search service has its own backend image/build graph so Tantivy does not force inference/GUI compilation; reuse existing packaging conventions rather than split other services solely to fill a table.

The existing runtime includes the service host but the Atlas overlay expects a separate mounted Rust-BERT model/library bundle. The completed quick start must include a reproducible **Linux** artifact/image path for that requirement and clearly separate immutable model assets from image compilation. Do not label a base runtime image build as a fully ready classifier deployment, or fall back to a macOS-built worker. Content-neutral weights/data can be reused by verified hash; worker binaries/libraries must match the deployed Linux architecture. Actor seeding and optional Natural training are explicit Workstation API actions after backend startup, not Docker-build steps.

Required build-only recipe after the new group/helper is implemented:

```sh
# From the repository root; the external helper does not start Docker.
export PLATFORM="$(python3 scripts/fleetworks_verify.py platform --format docker)"
export VERSION="$(tr -d '[:space:]' < version.txt)"
# Unique review tags avoid overwriting images selected by another running stack.
export IMAGE_TAG="review-$(git rev-parse --short=12 HEAD)-$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p dist/backend

# Inspect targets/platforms/tags before running the build.
docker buildx bake -f deploy/docker-bake.hcl --print backend
# Build for one selected Linux platform and load results into the chosen engine.
docker buildx bake -f deploy/docker-bake.hcl --load \
  --metadata-file "dist/backend/${IMAGE_TAG}.json" backend
```

Wire `PLATFORM`, `VERSION` and proposed `IMAGE_TAG` into the Bake targets; these are not decorative exports. When `IMAGE_TAG` is supplied, do not also overwrite shared `:local` or release-version aliases. Align Compose image variables with these exact output tags/digests. Emit enough image metadata for the operator to configure the subsequent start command without guessing names. Confirm Docker context/daemon architecture and selected builder against the detected platform; report remote contexts or mismatches instead of silently selecting emulation. Permit an explicit `PLATFORM=linux/amd64` or `linux/arm64` for intentional cross-builds, keeping native Linux builders as the default for dual-platform CI. Use single-platform `--load` locally; multi-platform registry publication stays an explicit CI/release operation.

For the existing shared `runtime` stage, retain this narrow alternative in the backend guide (it does not by itself produce database/model assets or start services):

```sh
docker buildx build --platform "$PLATFORM" \
  -f deploy/Dockerfile --target runtime \
  --build-arg FLEETWORKS_CORE_OPT_LEVEL=1 \
  --tag "fleetworks-runtime:${IMAGE_TAG}" --load .
```

Source builds after adding the desktop workspace member must still resolve valid Cargo workspace manifests inside the Linux build context. Update Docker `COPY`/workspace-resolution inputs accordingly without selecting/compiling the Rune package or making native view-only edits invalidate the expensive service compile. Test this from a clean context; simply changing `-p` does not fix missing workspace-member manifests.

**Build is not start.** In `deploy/README.md`, follow image creation with a separately invoked, tested operator startup recipe: create/select an isolated backend deployment workspace, initialize schema/credential files using retained external tooling, create required bind directories, stage verified Linux model assets, set owner UID/GID and nonconflicting API/web ports, set the exact built image variables, then validate and start selected Compose services with `--no-build`. Keep the same Compose overlays/project name for subsequent status/logs/stop. Include all required secret-file setup and reference scope-limited native/web credentials, not just a bare `up` command with missing prerequisites. No secrets in README, terminal output, artifacts or shell history. Never overwrite existing secrets/data or use `down -v` as ordinary cleanup.

The operator guide must show core backend/search startup separately from the Atlas/NLP/classifier overlay and optional web deployment. Include the private search origin/token, persistent index volume, ingestion-worker task, resource limits and index readiness/repair. A search outage degrades discovery without making seed admission or unrelated gateway diagnostics unavailable; avoid a health dependency cycle between seed completion and search service startup. Use explicit service selection or documented profiles so a backend-only start never unintentionally starts Next.js. Rebuilding an image does not replace a running service until the operator explicitly recreates its consumers. Preserve schema/volume ownership and record the selected platform/image IDs. Connection settings in the GUI point at the resulting gateway; the local folder registered by the native app is **not** the backend deployment directory.

#### 8.2.4 Connect the review app and exercise scoped functions

Open the built app's connection editor, choose the independently running gateway, an appropriate credential and a local review folder. Or implement the equivalent launch flags, kept distinct from Rune tooling arguments:

```sh
cargo run --locked -p fleetworks-desktop --bin fleetworks-desktop -- \
  --api-origin http://127.0.0.1:8080 \
  --api-token-file "$HOME/.config/fleetworks/review-api-token" \
  --workspace "$HOME/.fleetworks/review"
```

For this example, the protected token file and owned `~/.fleetworks/review` child must already exist. Create the child through the GUI’s name-only workspace action; a bare `mkdir` does not create its identity/registration. Without an override use `~/.fleetworks/default`; reject root/external/nested paths. Documentation must explain fresh credential provisioning after the one-time reset without embedding tokens in arguments. Use the remote HTTPS URL for a remote backend. Missing configuration opens setup; an unavailable backend leaves the native shell usable. Do not infer a workspace from the process working directory or register in a different backend silently.

For an existing installation, execute the approved one-time cutover separately, never on each build. Create/select an owned review child; configure global Profile keys and Models defaults once for this backend. Submit a bounded Atlas/tool action and inspect its backend target and workstation/workspace origin in Jobs & Logs. Verify a second child retains global settings and a second node’s jobs remain absent.

In Brain, search and open the selected authoritative dossier; no native topology or dependency on web availability. In Workstation, inspect separate database seed/search readiness and inference/training/activation states. Explicitly seed a synthetic dataset, follow the automatically admitted index work through visible completion, search it, and test supported training without automatic activation. Then ensure/reuse registration and open the advertised Connect form. These actions need an independently provisioned review backend; neither the build nor GUI launches containers/web/services. Capture deterministic Rune fixtures separately from real integration evidence.

### 8.3 One change-to-action planner

Extend `scripts/fleetworks_verify.py` and its tests; do not maintain competing local/CI path lists. Report **checks, native targets, image targets, service recreation and restart/HMR** separately, with reasons. An integration test can reuse compatible images without rebuilding all of them.

| Changed inputs / operation | Local action | Selected verification/build; exclusions |
| --- | --- | --- |
| Native screens, navigation, scoped Jobs/Logs, theme or shared controls | Incremental native rebuild/relaunch; scoped Runtime/transport/PNG/tree checks. | Both Mac lanes when native impacts; shared theme/shell covers affected pages. No web/backend rebuild solely for native views. |
| Native root/default manager, registration/browser launch or preferences | Native build; temporary-home/path, identity, safe launch and stale-context tests. | Both Macs; shared contract consumers only if changed. No real reset or Docker. |
| Native fixture/script/font/material or capture runner | Rebuild relevant fixture once; recapture affected scenes. | Native evidence/runner checks; no terminal cell renderer or mandatory host engine build. |
| Gateway registry/session, global settings/revisions, Jobs/Logs membership, durable origin or schema | Rebuild actual owning Linux image(s), apply supported schema changes and recreate consumers externally. | Auth/scope/storage/admission/gate/stream tests; native/web contract checks. Rebuild clients only for actual compiled inputs. |
| Readiness/seed/train/activation API or worker | Rebuild owning runtime/worker image; fixture and real durable-job checks. | Linux lanes plus changed contracts. No macOS classifier/NLP compilation. |
| Tantivy schema/tokenizer/query/service | Rebuild search image and real query/recovery tests. Schema/analyzer change also needs explicit versioned reindex. | Linux AMD64/ARM64; no actor reseed, native Tantivy or inference rebuild for search-only work. |
| Outbox/projection/ingestion pace/queue/retry code | Rebuild actual worker/search owner; bounded-policy/replay and concurrent-query tests. | Linux integration; no UI build with unchanged contracts. |
| Shared Rust DTO/client or client-safe workspace modules | Rebuild dependency-affected consumers; update web types/fixtures if relevant. | Contracts and actual Mac/Linux/web consumers, not automatic all-images. |
| Mounted React Native Web components | Fast Refresh/HMR or browser reload. | Web lint/type/behavior and distributed web image; no native/runtime rebuild. |
| Next/relay/server/live/config changes | Restart mounted server; rebuild/recreate web image for non-mounted inputs. | Web/relay/gate/REST/live tests. Do not assume server guards are HMR. |
| Web lockfile/dependencies/Node version | Frozen install; affected web dev/production image rebuild. | Web checks; not unrelated native. Distinguish backend bundled JS from web code. |
| Web color tokens/CSS | HMR/reload; palette/contrast and unchanged geometry/behavior checks. | Web artifact only; no native metal renderer or backend search build. |
| General gateway/jobs/core/storage implementation | Rebuild shared Linux runtime once, recreate all selected consumers consistently. | Relevant Linux integration/storage tests; no UI build with unchanged contracts. |
| Classifier/NLP native code/libraries or embedded JS/tooling | Regenerate/verify bundled assets and rebuild owning Linux image(s). | Both Linux real runtime lanes; no host-Mac inference toolchain. Mounted assets may use verified explicit reload instead. |
| Mounted models/datasets | Validate hashes; explicit reload/activation or owner restart. | Asset/behavior checks; image build only if baked-in bytes. No auto model promotion or app compile. |
| Gateway advertised web URL, search limits or other runtime config | Validate/reload if supported, otherwise externally recreate owner. | Config tests for code changes; discovery/status refresh. No native compile or automatic reseed. |
| Compose ports/profiles/mounts/environment | External reconcile/recreate using same overlays/project; preserve volumes. | Compose/connectivity checks. Restart alone does not apply new container config; build only if build inputs changed. |
| Production `NEXT_PUBLIC_*` / browser build-time values | Restart dev as needed; rebuild production bundle/image. | Web build; do not confuse with runtime gateway discovery. |
| Docker stage/base/build arguments/Bake target | Rebuild changed stage and downstream targets; recreate consumers explicitly. | Corresponding Linux/package tests; no native unless its own inputs changed. |
| Cargo/toolchain/lockfile/build scripts | Rebuild actual dependency graphs, widen safely if uncertain. | Relevant Mac/Linux caches/features; not every consumer for every lockfile line. |
| Host detector/native packaging/icon/Info.plist/entitlements | Test detector/packager; repackage/re-sign, recompile only compiled inputs. | Both Mac packages/download checks. No backend rebuild or overwrite of user app. |
| CI/planner/release/tooling | Validate plan/workflows/helper behavior and affected lanes. | Explicit native dispatch always selects both architectures regardless of paths. |
| Retained operator/Venture deployment tooling | Explicit operator tests and affected images/tools. | No native dependency/rebuild unless shared types changed; not launched by GUI. |
| Runtime workspace actions, keys/roles, refresh, seed/train/reindex | Guarded local/API operation or authorized external reset, according to contract. | No product build/CI dispatch from ordinary use. Test code changes separately with disposable fixtures. |
| Docs/agent guidance/diagrams only | Static links/commands/catalog/pairs/routing checks. | No product build or blanket screenshot sweep. |

Account for modified, deleted, renamed and relevant untracked local inputs. CI uses exact pushed SHAs and PR merge-base semantics. Unknown code/build paths or unreadable diffs select conservative coverage, not empty success. Test every row, mixed changes, runtime versus build-time inputs, stage dependencies, docs-only and explicit overrides. The shared runtime image is built once; consumers are recreated consistently, never through volume deletion.

A proposed external `dev`/watch mode may reuse the existing planner/runner. Dry-run first, select an isolated project/workspace, debounce/coalesce edits, serialize image replacement and restart only owned processes after success. Failed builds preserve the last good app. Production containers do not claim HMR. No automatic push, CI dispatch, publication or model promotion; the native process never calls this planner.

## 9. CI, platforms and downloadable artifacts



### 9.1 CI scope, trust and native matrix

Extend [`.github/workflows/fleetworks.yml`](https://github.com/xxxcess/argos-osint/blob/f8bc447c86f59c369a9259c6d0739681fadd1e94/.github/workflows/fleetworks.yml) and the existing candidate/candidate-target workflows. Add a shared native-app reusable workflow only if it removes duplicated steps. Automatic PR/push selection builds native apps for native-impacting changes; web-only changes keep web checks, and backend-only changes select Linux work. Provide explicit dispatch of a pushed branch/tag plus expected SHA; `components=native` must select **both** Mac targets regardless of path filters. `components=backend|all` covers corresponding Linux artifacts. One exact resolved SHA feeds all jobs and metadata; reject head/ref mismatches.

| Build/test target | Initial runner selection to validate |
| --- | --- |
| `x86_64-apple-darwin` | `macos-15-intel` |
| `aarch64-apple-darwin` | `macos-15` (ARM64) |
| `linux/amd64` | Native Linux x86-64 builder |
| `linux/arm64` | Native Linux ARM64 builder |

Runner labels match the existing candidate matrix and current GitHub runner documentation. Recheck availability before implementation; do not rely on `macos-latest` to mean Intel. Install the selected toolchain and use target/feature/lockfile-aware caches. Do not equate a runner OS version with the minimum supported client OS; test the documented minimum on each architecture.

Preserve the required aggregate: failed planning or missing/failed/unexpectedly skipped selected jobs fails the gate. Keep signing/publishing privileges separate from ordinary builds. Untrusted PRs get no signing/release secrets or write tokens; never execute PR code in a privileged `pull_request_target` path. Trusted publishing requires permission and the exact verified revision. Bootstrap dispatch workflows on the default branch as GitHub requires, then select the development ref. Do not rely on tag events from the default `GITHUB_TOKEN` automatically triggering another workflow; wire release creation to publishing explicitly.

Native compile/unit/package jobs must not install or invoke Docker/Compose or build backend engines. Include temporary-home container/default creation, direct-child containment and deletion guards, global settings stable across workspace switches, workstation-only activity, backend/auth isolation and synthetic one-time-reset/no-legacy-reload fixtures. Never run a credential wipe against a CI user home or production backend. Include Workstation fixtures for ready/degraded/stale/offline, permission denial, partial seed, running/failed training, unverified inference and unsupported APIs on both Mac targets. Exercise real bounded seed/train operations and persisted progress/attribution in the selected Linux integration lanes, without promoting fixture models or mutating production resources. Native Brain verification covers simple search/list/dossier composition and browser-launch intent; all topology rendering/interaction and its accessible alternative stay in web tests. Full endpoint/Connect ID integration uses a separately provisioned backend or fixture process owned by the test harness, not by the client under test. Keep actual end-to-end backend evidence in the selected integration lanes; fixture-only native tests do not replace it.

The required aggregate also includes the affected architecture/documentation gate in §11.4. Documentation-only edits run link/catalog/pair checks without building apps or images; component, API, deployment and CI changes trigger verification of their affected documentation surfaces. Keep code/diagram alignment review explicit where static checks cannot prove semantics.

### 9.2 Push an exact revision and dispatch both Mac builds

Keep the external scope-preview and exact-SHA dispatch recipe (new candidate inputs remain proposed until implemented):

```sh
base="$(git rev-parse HEAD^)"
python3 scripts/fleetworks_verify.py plan --base "$base" --head "$(git rev-parse HEAD)"
python3 scripts/fleetworks_verify.py local --base "$base"

git push origin "$(git branch --show-current)"
gh workflow run fleetworks-candidate.yml \
  --ref "$(git branch --show-current)" \
  -f head="$(git rev-parse HEAD)" -f components=native -f publish=none
```

`components=native` builds both Mac targets, not only the dispatching developer's architecture. Show how to identify the exact run/SHA, review its summary, and download the correct artifact; trusted releases and review comments follow §9.3. This quick start never dispatches/publishes merely because a local file changed.

### 9.3 Versioned downloadable app bundles

Use the existing workspace SemVer/`version.txt` policy; do not manually bump versions for each developer push. Make candidate identity unique using source SHA, run ID and run attempt. Required filename templates:

```text
Fleetworks-<version>-<sha12>-<run>.<attempt>-x86_64-apple-darwin.zip
Fleetworks-<version>-<sha12>-<run>.<attempt>-aarch64-apple-darwin.zip
```

Each ZIP contains **`Fleetworks.app`**, not loose terminal binaries. Publish SHA-256 sidecars and a manifest with full SHA, workspace version, target, Rune pin, workflow/run/attempt, Apple-compatible bundle version fields, minimum macOS, backend/API compatibility, dependency provenance, and signing/notarization/test status. Record tested backend image digests as integration evidence where relevant, not as images or deployment manifests the desktop must install. Do not put raw SemVer prerelease text into Apple numeric bundle-version fields without validating their format. Local dirty builds must be labelled; no dirty CI release artifact.

**Two distribution levels:** Actions artifacts for PR/iteration review with an explicit retention period, plus durable GitHub Release assets for a trusted `publish=prerelease|release` operation. `publish=none` is default; never silently publish on every edit. Candidate prereleases get unique tags and must not advance stable “latest.” Final releases follow Release Please and publish only exact-tag verified assets. Promote the already verified final bytes; changing signatures/stapling/resources after verification requires re-verification. Do not overwrite an existing identity.

After uploads/verification, create/update one review comment and the job summary with **real returned artifact/release URLs**, target, version/SHA, checksums, signing status, verification status and expiration where applicable. Associate PRs by validated repository/head SHA; no fabricated links or unauthorised writes. A comment links both architectures and marks failures/partial outputs explicitly. Release assets are the durable download reference; expiring Actions URLs are not permanent release links. Private-repo access still applies.

Verify the **downloaded ZIP by artifact ID**, extract it and inspect executable architecture, permissions, libraries and identity. Launch the extracted `.app` through macOS launch services on each architecture; test readiness, an authenticated read and mutation, a local and remote connection, Workstation readiness and API-backed seed/train controls using an isolated backend/dataset, and registered Intel Web launch: ensure/reuse the correct backend records, enter the returned Connect ID, prove a denied deep link before connection, then submit and inspect an attributed brief. Headless tests are additional evidence. Hosted environments without a usable GUI/GPU session cannot claim a GUI pass; retain a named matching-hardware validation step and report its status. Clearly label unsigned review candidates; public stable distributions require signing/notarization. End users need no Rust/Node/Python/checkout/terminal setup.

### 9.4 Linux engines and search-service alignment

Replace the macOS classifier runtime lane in [`.github/workflows/atlas-bert.yml`](https://github.com/xxxcess/argos-osint/blob/f8bc447c86f59c369a9259c6d0739681fadd1e94/.github/workflows/atlas-bert.yml) with Linux build/link/test and **real-checkpoint inference** on both supported Linux architectures. Audit [`.github/workflows/embedded.yml`](https://github.com/xxxcess/argos-osint/blob/f8bc447c86f59c369a9259c6d0739681fadd1e94/.github/workflows/embedded.yml) and [`scripts/local_matrix.sh`](https://github.com/xxxcess/argos-osint/blob/f8bc447c86f59c369a9259c6d0739681fadd1e94/scripts/local_matrix.sh) so old macOS `embedded,nlp,classifier` legs do not remain by accident. Preserve required Linux runtime/storage coverage; remove lanes used solely by deleted legacy clients instead of keeping dead compatibility code. Move engine work; do not substitute doc-only compilation for runtime tests.

Package the worker, matching native libraries, verified model assets and container-visible configuration. Do not install host PyTorch/classifier/NLP for the desktop. Preserve model pins, hashes, receipts, training activation and inference semantics. If a pinned dependency is unavailable on one architecture, record a blocker rather than dropping that target or silently emulating it. Prefer native builders. External deployment tooling should default to matching Linux image architecture for a local host. The native client has no image-selection/install logic and may connect to any API-compatible local or remote backend architecture.

**Search builds are Linux backend work too.** Build/test/package `fleetworks-search-index` for AMD64 and ARM64, with a persistent index volume and reproducible selected Tantivy dependency. Include its image in the backend-only Bake group and dependency-aware scope planner. Native builds consume only search DTO/client contracts; verify their graph excludes Tantivy and the search service. Schema/analyzer changes select Linux reindex/recovery tests, while Brain view/fixture-only changes do not rebuild backend images. No Mac engine installation or new public service exposure is implied.

## 10. Verification and evidence

**Required verification cutover.** Replace the retired CLI/TUI presentation pipeline with tests and images from the **actual Rune/rune-kit app views**. Read the pinned core README's **Agent tooling** and **Accessibility** sections, `examples/hello/src/main.rs`, and `crates/rune/src/tooling/args.rs` before implementing the harness. Upstream documents `--snap`, `--tree`, `--scene`, `--script`, `--size`, `--scale`, `--at`, `--trace` and `--list-scenes`; there is no need to invent `--headless` or keep terminal cells as the native UI's intermediate format. Preserve correct forwarding/splitting of Rune flags and application connection flags.

### 10.1 Test layers and page/state coverage

| Verification layer | Required evidence / boundaries |
| --- | --- |
| Rust logic and effects | Workspace/default-folder protection, registered context, scoped settings, reset repeat-safety, request state machines, cancellation, stale responses and mutation idempotency. Use temporary owned paths and synthetic keys; no real user home or production writes. |
| Rune `Runtime` / component interaction | Exercise the real app shell/views with deterministic injected state and fake boundary effects. Follow the hello example's `Runtime`, frame stepping and pointer input; assert state, keyed geometry, focus/keyboard/pointer behavior and accessible roles/states. Check response/action effects separately from pixels. |
| Rune rendered PNG + tree | Actual renderer captures of affected page/state/size combinations, with element/accessibility tree assertions and visual inspection. Test overlays, clipping, text wrapping/selection, scroll reachability, disabled controls and responsive breakpoints. A generated PNG is not a reviewed/passing image. |
| Native API contracts | Transport fixtures prove global resource permission/revision versus origin, contained child registration, workstation-only Jobs/Logs and gateway-only search/dossier calls. Mocks do not establish real persistence/index correctness. |
| Real Linux search/domain integration | Small real SurrealDB + durable dispatcher + Tantivy fixtures prove commit/outbox/ingest/reload/search/dossier and restart/recovery. No model/provider calls needed for actor-search tests; classifier/NLP keep separate real runtime suites. |
| Web verification | Retain React component tests and Playwright for the Connect gate, direct routes, sessions, REST/WebSockets, briefs and **all topology visualization/interaction**, including its accessible list. Rune PNGs do not replace browser graph rendering or security checks. |
| Downloaded macOS artifact | Both downloaded `.app` ZIPs must still pass architecture/library checks and real Launch Services/Finder, OS integration and supported Mac accessibility/input smoke. Headless rendering does not prove installation, signing, menus, clipboard, file dialogs or hardware presentation. |

Cover all eight pages at cutover: Atlas, Brain, Jobs & Logs, Tools, Models, Workspaces, Profile and Workstation. The shared-theme cutover also verifies §2.3: logo-token parity, readable metal-control states/static fallback and no idle decorative animation. Web palette work uses its own Playwright color/contrast and unchanged-layout assertions, not Rune captures. Use a maintained **page/action/state fixture inventory**, not a screenshot-count target. Local changes select affected scenes and sizes; shared theme/shell/navigation changes select all affected consumers (all eight when globally affected). Include the supported minimum/window threshold and representative wide layout, 1x/2x where relevant, intended theme and reduced-motion behavior. Do not inherit terminal-cell dimensions as native pixel/point breakpoints.

Required focused journeys include: Brain search text -> stale reply suppression -> arrow/Enter or click -> correct SurrealDB-backed dossier in simple tabs/lists; search unavailable/indexing versus genuine empty; Workstation seed/train/index progress and explicit activation; workspace switching with unsaved edits; protected-default deletion attempt and non-default confirmation/recovery; fresh Profile keys/Models setup; scoped Atlas submissions and Jobs/Logs actions. Assert secrets are absent from PNG/tree/trace/fixture output. Data correctness, permission enforcement and filesystem safety require assertions, not just screenshots.

**Topology ownership regression:** assert native navigation/Brain scenes contain the required search/results/detail controls and no topology view, visualization controls or eager graph-fetch effect. Test the shared Open Intel Web action through a fake browser adapter, with registered context and the existing gate; missing web discovery must not disable actor search/dossiers. Audit native-only graph/layout dependencies and remove obsolete native topology fixtures. Preserve existing browser topology behavior/accessible-list tests and backend relationship/dossier tests. Rune element-tree diagnostics and architecture diagrams are not product topology rendering and stay.

### 10.2 Real actor-search and load/recovery tests

Use real Tantivy RAM indexes for pure query/schema tests and temporary disk indexes for durability/reopen tests. Explicitly commit/reload and use event/barrier conditions, not sleeps that hide races. Test names/aliases/Unicode/ambiguity, exact versus prefix relevance, query/result bounds, stable IDs and protected-scope filtering. Test that indexing an actor never becomes an identity-resolution decision.

Backend integration must cover: committed seed batches only; an already-seeded database gaining its first index; rollback before outbox commit; crash after DB commit/before delivery; index commit before acknowledgement; duplicate/out-of-order batches; alias-only changes; deleted/redirected actors; concurrency during backfill; reader reload failure; worker/service restart; disk-full/corrupt/incompatible generation; writer-lock contention; explicit rebuild/cutover; search unavailability and authorized partial states. Include pending backlog/readiness that becomes current only after the serving generation is visible. Verify no user-config reset or native-workspace deletion clears shared search data.

At least one end-to-end test must seed a bounded synthetic actor catalog through the supported operation, await durable ingestion and reader visibility, search through the public gateway, select the returned actor ID, and retrieve its actual SurrealDB dossier. Change or delete the record and verify updated search/detail behavior and revoked visibility. Native UI mocks cannot substitute for this test. Run search-service tests on both Linux architectures; do not add Tantivy to either macOS native build graph.

Add controlled-clock/barrier regressions for §6.4’s count/byte/concurrency/rate/staging limits, partial tails, shared leases, full-queue pauses, Retry-After longer than local cap, cooldown, slow commit/reload, disk pressure, ambiguous ack and replay. Count actual requests: no per-actor flood or doubled limits when two workspaces seed. Run bounded real search while indexing/rebuilding, recording idle-versus-loaded query latency, memory and recovery; fixture policy success is not a production benchmark.

### 10.3 Same-view headless fixtures and PNG commands

Create a **development/test-only entry point**, proposed `apps/fleetworks-desktop/examples/native_fixture.rs`, that imports the **same** application shell, views, theme and handlers as the production app. Add a small library entry point where needed. Inject deterministic API/file/credential/browser adapters and synthetic fixture data; do not create a second set of fake UI layouts. Fixture code must not read real credentials/default folders, contact a real gateway, open a browser, mutate real workspaces, or run the one-time reset. Do not add an authentication-bypass header/flag to the production server. Native fixture mode is plainly identified and excluded from distributed application behavior.

Register named `AppSpec::scenes` (proposed names below), stable keyed elements and scripted cases; expose a list so selection typos or zero matched scenes fail. Use the virtual clock for fixtures, fixed IDs/timestamps, seeded fake responses and local embedded test assets. Async fixture completion must use explicit deterministic events; `--at` advancing virtual time does not wait for a real network response. Genuine wall-clock integration is a separate lane, not a flaky fixture sleep.

Human developer recipe **after the example/scenes are implemented**; the Rune flags are documented upstream, while package/example/scene paths are proposed Fleetworks contracts:

```sh
mkdir -p .agent-scratch
out="$(mktemp -d .agent-scratch/native-review.XXXXXX)"

# State/interaction/contract tests before accepting visual evidence.
cargo test --locked -p fleetworks-desktop

# Discover the implemented scene names.
cargo run --locked -p fleetworks-desktop --example native_fixture -- --list-scenes

# One real Rune headless frame plus its element/accessibility tree.
cargo run --locked -p fleetworks-desktop --example native_fixture -- \
  --scene brain-search-ready --size 1280x800 --scale 2 --at 0 \
  --snap "$out/brain-search.png" --tree "$out/brain-search.tree.json"

# Drive production handlers with keyed input, then inspect the dossier state.
cargo run --locked -p fleetworks-desktop --example native_fixture -- \
  --scene brain-search-ready \
  --script apps/fleetworks-desktop/tests/fixtures/scripts/brain-search-open.json \
  --size 1280x800 --scale 2 --at 1.0 \
  --snap "$out/brain-dossier.png" --tree "$out/brain-dossier.tree.json" \
  --trace "$out/brain-search.trace.jsonl"
```

Inspect PNGs using an image-capable viewer, assert expected tree/interaction values, and record actual observations. Timestamped script syntax must follow upstream examples; prefer stable keys over brittle coordinates. The scripted capture's `1.0` is a fixture-defined virtual time, not a claim about production latency. Add compact/error/loading and other scenes through the inventory rather than manually editing screenshot output.

Agents use the established `scripts/agent_cargo.py` equivalent with `target/agents`; human commands above remain supported. Extend the **existing** `scripts/fleetworks_verify.py` with a proposed `native-ui` selection/report action if a runner is needed. It builds once, discovers the executable from Cargo output, runs selected tests/captures, checks exit codes and output completeness, and writes a manifest. It must not retain `tui_review.py` as a second mandatory renderer or build the full embedded/engine-enabled workspace for a UI edit. Require explicit scene selection or an explicit full sweep and rationale; unmatched selectors, zero tests, missing/empty PNGs, missing trees and failed assertions fail the selected check.

### 10.4 Evidence handling, CI and terminal-capture retirement

Headless PNG rendering uses Rune's actual renderer and still needs a supported GPU/backend or validated software adapter; it is not synonymous with GPU-free. Document/test the chosen Mac runner capability and dependency setup. Do not add a Linux desktop product merely to make CI render. If an optional Linux software-render lane is useful, follow upstream Linux prerequisites and label its rendering profile separately from Mac evidence. Missing adapter/rendering support is **BLOCKED**, not PASS; a tree-only test is not PNG evidence. No terminal emulator/display server is required merely to render headless PNGs. Optional video/ffmpeg or motion-audit tooling is not a required PNG dependency.

Capture evidence per run/target/scene under ignored task-owned output, then upload a bounded evidence artifact to GitHub Actions with its own retention. Record source SHA/dirty status, Rune revision, executable/fixture/script identities, target, OS/renderer/adapter, fonts, theme, logical size, pixel scale, virtual clock, command/exit and test/inspection statuses. Do not expose real credentials, IPs, home paths or live datasets in fixtures, tree dumps or review comments. Artifact links must be actual returned URLs; review comments link this evidence alongside, but distinctly from, the two installable Mac apps.

Keep approved baselines and updates deliberate. Compare equivalent rendering profiles; do not demand unverified byte-identical pixels across architectures/OS/drivers. Use documented targeted tolerances only after reviewing a real rendering difference, plus geometry/accessibility assertions. A new/different baseline requires an intentional reviewed change; never auto-bless images to make CI green, hand-edit PNGs, use image generation to replace evidence, or mark an unviewed image visually approved. Preserve useful failing images and before/after/diff artifacts; regenerate only evidence invalidated by relevant changes.

After each retired TUI consumer has tested behavior in its assigned owner (native controls, or web for topology), remove its `TestBackend` cell-export/dump fixtures, `ARGOS_SCREEN_DIR` contract, terminal-only capture jobs, CLI screenshot artifacts and cell-to-PNG font setup. Known candidates: `scripts/tui_review.py`, `scripts/render_tui_cells.py`, their terminal-only tests/dependencies, `.opencode/skills/argos-tui-verify/`, and active `tui-verification.md` instructions. Audit all callers first; port valuable state/data assertions rather than deleting them with the renderer. Remove Pillow/font/venv setup **only when exclusively used by this retired pipeline**; preserve unrelated image utilities and browser tests. Remove the old mandatory full native-embedded gate from **Rune-only** checks, while retaining required Linux storage/domain gates for backend changes.

Required CI lanes/reporting: scoped Rust/Runtime tests, headless scene/tree/image checks on supported native verification hosts, Linux search/ingestion integration, existing web tests when affected, and both packaged-Mac artifact checks. A failed or missing selected lane must fail the aggregate; do not relabel unavailable visual verification as skipped-success. Distinguish **test passed**, **PNG generated**, **visually inspected**, **downloaded bundle launched**, and **published/signed** in reports. Update all agent/documentation entry points under §11.5 in the same cutover.

### 10.5 Cross-cutting acceptance matrix

Domain failure cases remain in their owning sections; this table is the consolidated integration checklist rather than a second specification.

| Contract | Required boundary evidence |
| --- | --- |
| Global versus activity scope (§1, §4.5) | Two workspaces share permitted global roles/accounts/corpus; two nodes and two deployments remain isolated. Native Jobs/Logs includes only triggered/associated work for its node, with narrower workspace filtering, safe dedup/children, filtered totals/export/live and guarded direct actions. |
| Managed container/default (§3.1) | Isolated first launch; correct `IP:default`; name-only creation; reject root, external/nested, symlink/case/Unicode/reserved/race/ancestor targets. Test attach/retire/delete/offline/crash recovery, multi-backend revocation, active/shared jobs and unchanged siblings/global data/history. |
| Registry / Connect gate (§3–4) | Duplicate/concurrent/retried ensure; one node/multiple children; collisions, multiple interfaces/NAT/IPv6/rebind. Deny unknown/revoked/wrong-backend registrations, forged cookies/headers, direct pages/RSC/prefetch/API/actions/exports/live, stale tabs and unavailable authority. Native close does not end browser work. |
| Durable provenance (§4.4) | Native and web admission, uncertain response, dedup request associations, queue/restart/retry/child propagation and original creator preservation. Legacy data is not assigned invented origin. |
| Readiness / setup (§5) | Ready/degraded/stale/offline/permission/partial states; seed receipt and reader visibility versus counts; process ready versus unverified model; optional Natural training distinct from Rust-BERT inference. No mutations on GET. Upload schema/kind/hash/size, leases, cancel/restart/reconcile; explicit candidate activation preserves prior valid model on failure. |
| Reset and retirement (§7) | Synthetic allowlisted files/secure-store/backend secrets/sessions removed, defaults emptied; new keys survive completed-epoch rerun and upgrades. Root/default/domain data survive. Actual authorized user-installation reset is separately recorded or pending, never inferred from CI. Terminal consumers/scripts/dependencies are removed only after replacement and reference audit. |
| Developer commands (§8) | Follow documented setup on both physical Mac types; `.app` at printed path, output overrides/spaces, Rosetta mismatch/unsupported host/target, missing SDK/Python/tools, failed-build preservation and Docker-free offline launch. Independently test backend-only Bake plan/build and complete external startup without unintended web/native/CLI targets. |
| Rebuild planner (§8.3) | Native view avoids backend/web; web source reload avoids native/runtime; backend change recreates owners without data loss; discovery/runtime limits need no app compile; shared/mixed inputs select actual consumers. Unknown/zero selection cannot quietly pass. |
| Distribution (§9) | Local edit → pushed SHA → dispatch → both versioned ZIPs → real download links/comments → artifact-ID download/hash/architecture/identity → matching-hardware launch and authenticated read/mutation → trusted release promotion. Record signing/minimum OS/retention and actual GUI status. |
| Product boundaries / appearance (§1–2) | No native Docker/DB/index/topology graph or web-server launch. Search/dossier usable without web. Logo palette and selective native material meet states/contrast; web changes stay color-only and existing topology/list remain functional. |
| Agent/docs/diagrams (§11) | Actual guidance, endpoint inventory, source-linked usage, commands, diagrams and indexes match implementation. Inspect changed HTML/SVG pairs; no stale TUI capture routing or unsupported completion claims. |

Use disposable owned paths/namespaces/ports, synthetic actors/keys and fake providers; no live secrets, user home, production mutations or paid calls in fixtures. Separate native fixture, real Linux integration, web/security and physical-Mac evidence. Preserve original failures and report executed/pass/fail/blocked/skipped/pending honestly. A missing GPU/adapter, zero matched tests, mocked server or mere PNG creation is not a pass for a stronger claim.

## 11. File, documentation and agent-guidance plan

Implement documentation and agent changes alongside their owning code, not as optional post-release cleanup. Existing paths below are baseline candidates; proposed paths must be reconciled with moved files and real callers. One endpoint/scope inventory, one planner and one canonical guide per topic are sufficient. This plan is not an instruction to edit unrelated files or proof that candidate modules already exist.

### 11.1 Code, deployment and CI file candidates

| Action | Files / paths | Work |
| --- | --- | --- |
| Create | `apps/fleetworks-desktop/Cargo.toml`, `src/{main,app,connection,registration,theme}.rs`, `src/views/`, `tests/`, `resources/Info.plist` | Thin API-only Rune app, functional endpoint-backed controls, local-review CLI flags, selected directory/interface, protected local markers, registration/Copy Connect ID, resources and deterministic fixtures. Include only the guarded client workspace filesystem manager, never a runtime orchestration module. Keep modules small; do not create a crate per screen. |
| Create/update | Proposed `apps/fleetworks-desktop/src/theme.rs`, shared native control primitives and `tests/theme*` | Exact logo anchors, semantic palette, native-only dark-metal recipe, accessible state/fallback/reduced-motion behavior, token and real-view fixture tests. No new renderer/material dependency. |
| Update colors only | `packages/ui/src/index.tsx`, `packages/intel/src/shell/intel-theme.ts`, `apps/web-desktop/app/style.css`, affected color-consuming styles and web fixtures | Recolor the existing shared/Intel namespaces and brand effects, preserve geometry/type/motion, decouple semantic success/kind tokens from changed brand accents, and prove no behavior/layout change from this theme task. |
| Preserve/reference | `assets/brand/fleetworks-logo.svg`, `fleetworks-mark.svg`, `fleetworks-logo.jpg` | Existing artwork remains untouched; use exact SVG brand colors and the README banner as visual authority. No new logo/banner generation or raster-derived palette drift. |
| Create | Proposed `services/search-index/{Cargo.toml,src/main.rs,src/lib.rs,src/index.rs,src/query.rs,tests/}` | Small private Linux Tantivy service; single writer, bounded query/write queues, durable generations/checkpoints and real index/load/recovery tests. No dependency from desktop. |
| Create/update | Proposed `services/api-host/src/actor_index_worker.rs`, `crates/argos-osint-core/src/storage/actor_search_outbox.rs`; existing actor seed/catalog/alias/lifecycle writers and job dispatcher | Commit actor projection work with source changes; durable paced batches, bounded staging/shared limits/backoff, reconciliation/rebuild receipts, revision/tombstone fences and distinct resource target/request origin. Reuse current storage/queue ownership. |
| Update | `crates/argos-osint-api/src/{actor_dossier,brain,private_relay,lib}.rs`, proposed `actor_search.rs`, shared DTOs; native `src/views/brain.rs` | Gateway/Brain search -> private Tantivy, scoped candidate checks, stable IDs and separately authorized SurrealDB dossiers; native shows these as simple search/list/detail components. Keep identity-binding and backend topology/dossier readers where needed, not duplicate UI engines. |
| Trim/omit after caller audit | Any native topology view/renderer/layout modules, graph-only state/assets/dependencies and their fixtures under `apps/fleetworks-desktop/`, if introduced | No native topology product surface. Keep simple dossier association rows, Rune rendering/icon support and real-view verification. Do not claim these proposed removal candidates already exist. |
| Preserve and verify | `packages/intel/src/topology/` including `ForceCanvas.tsx` and its accessible list, relevant `tests/web/`; gateway/Brain and core topology/dossier owners | Web is the sole topology presentation owner. Preserve supported graph behavior and domain APIs. Update ownership/docs/tests, not the graph design; web appearance remains palette-only. |
| Create/update | Proposed native `src/lib.rs`, `examples/native_fixture.rs`, `tests/{native_ui,brain_search,workspace_ui}.rs`, `tests/fixtures/scripts/` | Same production views with deterministic fake boundary effects; Rune Runtime/keyboard/pointer/accessibility assertions; PNG/tree/script scenes without real user secrets, DB or reset. |
| Update/trim | `scripts/fleetworks_verify.py`, native fixture/runner tests; `scripts/tui_review.py`, `scripts/render_tui_cells.py` and terminal-only dependents | Add scoped `native-ui` selection/evidence to existing tooling; compile once and fail missing/zero coverage. Retire cell-to-PNG generation after real Rune replacements pass; preserve valuable assertions and unrelated image/web tooling. |
| Create/update | Proposed native `src/{workspaces,context}.rs`, `src/views/{atlas,brain,jobs,logs,tools,models,workspaces,profile}.rs`, targeted `tests/` | Complete Backend/Workstation page groups; global forms, workstation-only Jobs/Logs, contained child-folder lifecycle and root/default protection; use the same context/Jobs/connection primitives. |
| Create/update | Proposed `scripts/fleetworks_config_reset.py` and synthetic-fixture tests; current app-owned credential/config readers and external provisioning hooks | One-shot inventory/apply/receipt cutover, exact ownership allowlists, user-token/session invalidation, fresh setup and no-repeat safeguards. No reset in build hooks or ordinary native startup. |
| Retire after replacement | `crates/fleetworks-workspace/src/import.rs`, `import_legacy` exports/callers, retired role/home/environment aliases and their current doc paths | Remove legacy user-key/config import and runtime compatibility branches; preserve current domain data and supported optional Natural training. Validate each removal against callers. |
| Update | Current `SettingsFile` / `AuthFile` implementations and consumers; `crates/argos-osint-api/src/{control,operations,jobs_api,live}.rs`, job-admission readers and shared contracts | Keep deployment-owned global settings/credentials/role defaults with permission and expected-revision checks. Separate resource scope from workstation/workspace origin; remove obsolete per-folder copies/overrides rather than create them. |
| Create/update | Proposed `apps/fleetworks-desktop/src/views/workstation.rs`, shared readiness/action state module and `tests/workstation*` | First-class sidebar page; typed readiness, dataset picker/upload, explicit seed/train/activation, durable job links, stale-response isolation and offline fixtures. Reuse Jobs and connection state; no orchestration code. |
| Update | [`Cargo.toml`](https://github.com/xxxcess/argos-osint/blob/f8bc447c86f59c369a9259c6d0739681fadd1e94/Cargo.toml), `Cargo.lock`, `rust-toolchain.toml` | Workspace membership, native-only scoped defaults, pinned Rune/kit/features, verified toolchain and removal of unused terminal dependencies. Ensure native build scripts/default features cannot invoke containers/web builds or link deployment/engine code. |
| Update | [`crates/fleetworks-contracts/src/lib.rs`](https://github.com/xxxcess/argos-osint/blob/f8bc447c86f59c369a9259c6d0739681fadd1e94/crates/fleetworks-contracts/src/lib.rs), [`crates/fleetworks-contracts/src/client.rs`](https://github.com/xxxcess/argos-osint/blob/f8bc447c86f59c369a9259c6d0739681fadd1e94/crates/fleetworks-contracts/src/client.rs), `crates/fleetworks-workspace/src/` | Typed capabilities/discovery, Connect ID/context DTOs, workspace lifecycle/configuration epochs, authenticated scoped operations and client-safe protected-default/profile/marker types. Split deployment execution out of any crate/feature used by the GUI; no Docker SDK/socket, Venture or core/storage dependency in the GUI graph. |
| Separate/trim | `apps/venture/src/main.rs` and its lifecycle modules | Retain only justified external operator/automation behavior; retire dispatch to removed clients. Do not extract a stack supervisor into the native app. Separate shared data types from deployment execution where necessary. |
| Update | [`crates/argos-osint-api/src/lib.rs`](https://github.com/xxxcess/argos-osint/blob/f8bc447c86f59c369a9259c6d0739681fadd1e94/crates/argos-osint-api/src/lib.rs), `services/api-host/src/` and API tests | Configured browser URL, additive capabilities and stable deployment identity, registry/session routes, validated context extraction and security tests. Proposed API module: `crates/argos-osint-api/src/connect.rs`. |
| Create/update | Proposed `crates/argos-osint-api/src/workstation.rs`; existing `src/{lib,control,private_relay,jobs_api}.rs`, `crates/fleetworks-contracts/src/` and API tests | Read-only readiness aggregate, capability/permission DTOs, bounded dataset transfer and durable seeding/training admission; authenticated internal forwarding and safe failure states. No command-execution/container-control endpoint. |
| Reuse/refactor on server only | `services/api-host/src/{actor_seed,classifier_train,classifier_service}.rs`; `crates/argos-osint-core/src/storage/actor_catalog.rs`, classifier readiness/training modules and actual job-dispatch writers | Keep seed receipts/leases and model provenance/activation rules; expose durable Jobs status and context. Reuse logic without stdout/CLI coupling. Inspect `apps/venture/src/classifier.rs` for parity only, not a native dependency. |
| Update | [`apps/web-desktop/server.mjs`](https://github.com/xxxcess/argos-osint/blob/f8bc447c86f59c369a9259c6d0739681fadd1e94/apps/web-desktop/server.mjs), [`apps/web-desktop/server-origin.mjs`](https://github.com/xxxcess/argos-osint/blob/f8bc447c86f59c369a9259c6d0739681fadd1e94/apps/web-desktop/server-origin.mjs), `apps/web-desktop/live-relay.mjs`, `packages/api-client/src/`, `tests/web/` | Explicit external-origin policy, safe authenticated remote ingress, REST/WebSocket tests; no browser bearer injection. |
| Create/update | Proposed `crates/argos-osint-core/src/storage/connect_registry.rs` and registry/session tests; existing `storage/mod.rs`, `storage/bootstrap.rs`, repository schema/version/index declarations | Backend-owned workstation/workspace/registration/session storage, default protection, retirement/tombstones, contained-child registry/activity and separate global configuration ownership, uniqueness, revocation and current schema installation/data-safe migration. Do not preserve obsolete user-config schemas. |
| Update | `crates/argos-osint-api/src/{lib,brain,private_relay,jobs_api,live}.rs`; `crates/argos-osint-core/src/intel_service.rs`, `storage/brief_stages*` and their actual admission/job writers; affected `fleetworks-jobs` consumers | Trace both brief routes, authenticated propagation, context-aware idempotency, durable origin/request association and job read models. Locate exact writers before edits; do not assume metadata in a browser request survives admission. |
| Create/update | Proposed `apps/web-desktop/app/connect/page.tsx`, `app/api/connect/route.ts`, shared server-only `connect-session.mjs`/typed adapter as needed; existing `app/layout.tsx`, `app/page.tsx`, `app/server-api.ts`, `app/api/v1/[...path]/route.ts`, `server.mjs`, `live-relay.mjs` | Connect prompt and opaque session, centralized authoritative guards, direct-route/RSC/API/WS protection, safe logout/context switch. Reconcile current paths before editing; one shared validation implementation for Next and the custom Node upgrade server. Optional Proxy is only an early redirect. |
| Update | `packages/api-client/src/`, `packages/intel/src/`, native Jobs/Logs views, `tests/web/`, API/storage/brief tests | Context-keyed caches, expected-revision mutation checks, typed origin display, gate fixtures, durable job provenance and dedup tests. No localStorage unlock flag. |
| Update | [`deploy/Dockerfile`](https://github.com/xxxcess/argos-osint/blob/f8bc447c86f59c369a9259c6d0739681fadd1e94/deploy/Dockerfile), `deploy/docker-bake.hcl`, `deploy/compose.yml`, [`deploy/compose.dev.yml`](https://github.com/xxxcess/argos-osint/blob/f8bc447c86f59c369a9259c6d0739681fadd1e94/deploy/compose.dev.yml), `deploy/compose.atlas.yml`, `deploy/verify.py` | External Linux/backend/web deployment only: add a true backend-only Bake group, isolated review tags/platform selection and required Linux classifier bundle packaging; keep web builds/startup separate. Repair Docker workspace-copy inputs for the new native member without compiling it. Preserve advertised URLs, HMR, image/recreation and artifact smoke. Add private search routing/credentials, its independent image and persisted index volume, and validated ingestion/query limits; operator-only start/rebuild. Never mount native workspace roots as backend data. No Docker/Compose invocation from the desktop. |
| Update | [`scripts/fleetworks_verify.py`](https://github.com/xxxcess/argos-osint/blob/f8bc447c86f59c369a9259c6d0739681fadd1e94/scripts/fleetworks_verify.py), `scripts/test_fleetworks_verify.py` | Single external change/action planner, read-only `platform --format rust\|docker\|json`, watcher integration, narrow build selection and fixture table; reuse the platform helper. Include search-service/ingestion scopes and Rune `native-ui` evidence selection; do not keep independent path-filter lists. |
| Create/reuse helper | Proposed `scripts/fleetworks_platform.py`, `scripts/test_fleetworks_platform.py`; update planner/candidate unit tests | One tested hardware/process/toolchain detector shared by native artifacts and Linux platform selection; no extra orchestrator. Cover Intel, ARM, Rosetta, unsupported OS, explicit target and Docker-context mismatch diagnostics. |
| Update | [`scripts/fleetworks_candidate.py`](https://github.com/xxxcess/argos-osint/blob/f8bc447c86f59c369a9259c6d0739681fadd1e94/scripts/fleetworks_candidate.py), candidate tests | Implement host-detected `build-native` and shared `package-native`, explicit open/archive/dry-run, Cargo artifact discovery and local `.app` outputs; reuse in CI/download verification and replace Mac CLI archives. |
| Update | [`.github/workflows/fleetworks.yml`](https://github.com/xxxcess/argos-osint/blob/f8bc447c86f59c369a9259c6d0739681fadd1e94/.github/workflows/fleetworks.yml), [`.github/workflows/fleetworks-candidate.yml`](https://github.com/xxxcess/argos-osint/blob/f8bc447c86f59c369a9259c6d0739681fadd1e94/.github/workflows/fleetworks-candidate.yml), `.github/workflows/fleetworks-candidate-target.yml`, `.github/workflows/intel-web.yml` | Native matrix, dispatch inputs, scopes, downloadable artifacts, comments and required aggregate; add scoped Rune PNG/tree/test evidence and real Linux search integration, removing retired terminal image jobs. |
| Update | [`.github/workflows/atlas-bert.yml`](https://github.com/xxxcess/argos-osint/blob/f8bc447c86f59c369a9259c6d0739681fadd1e94/.github/workflows/atlas-bert.yml), [`.github/workflows/embedded.yml`](https://github.com/xxxcess/argos-osint/blob/f8bc447c86f59c369a9259c6d0739681fadd1e94/.github/workflows/embedded.yml), [`scripts/local_matrix.sh`](https://github.com/xxxcess/argos-osint/blob/f8bc447c86f59c369a9259c6d0739681fadd1e94/scripts/local_matrix.sh), `.github/workflows/release-please.yml` | Linux engine coverage, removal of obsolete Mac engine lanes, explicit release asset publication. |
| Create if needed | `.github/workflows/fleetworks-native.yml`; `scripts/test_fleetworks_native.py` | Reusable native build/package/verify and package fixtures; do not duplicate existing candidate machinery. |
| Retire after gates | `apps/fleetworks-cli/src/tui/`, interactive `_ui.rs`/line-session modules, six bin entrypoints; [`apps/fleetworks-cli/README.md`](https://github.com/xxxcess/argos-osint/blob/f8bc447c86f59c369a9259c6d0739681fadd1e94/apps/fleetworks-cli/README.md) | Migrate reusable operations/automation consumers; then remove the obsolete package if empty. |
| Audit, then trim/delete | `crates/argos-osint-bin/`, any `src-tauri/` or Tauri config, obsolete CLI image/bake targets and CLI candidate manifests | Preserve required noninteractive service behavior first; remove dead UI dependencies and distribution surfaces. Never mass-delete by name alone. |

**Latest scope changes:** native `src/views/{jobs,logs}.rs` and every cross-page job link use §4.5. Extend actual gateway Jobs/live readers and request/parent-association query indexes before UI filtering; test other-node/system exclusion on counts, details, exports and streams. Native `src/workspaces.rs` differentiates `workspace_root` from selected child, and `src/context.rs` differentiates backend resource scope from workstation/workspace origin. The search coordinator/service share one validated limits configuration (§6.4); no new per-page or per-workspace stores.

### 11.2 Canonical usage and architecture documents

Keep long procedures in their canonical guide and short matching links/commands in entry points. Publish new commands only with tested helpers/targets; otherwise retain a clear implementation-pending label.

| Files | Required update/create/trim |
| --- | --- |
| `README.md`, `apps/fleetworks-desktop/README.md`, `docs/index.md`, `docs/wiki/code/index.md` | Short choose-a-lane quick start; host-detected `.app` and direct Cargo commands, separate Docker backend build; both Mac downloads. Distinguish end user, developer and operator. Remove terminal-first/Tauri/native-managed-stack framing. |
| **Create** `docs/wiki/code/native-client.md` | Native prerequisites, host/explicit target builds, artifact paths/Finder review, connections, contained `~/.fleetworks/{default,<name>}`, two UI groups, workstation-only Jobs/Logs, global Profile/Models setup, Workstation actions and registered browser handoff. |
| `docs/wiki/code/{architecture,fleetworks,interfaces}.md` | Source-linked component/endpoint/scope ownership. Native simple controls → gateway; private Linux services/search/DB; web-only topology; external deployment. Global target versus immutable origin, root versus default child, enforced node-only activity and security boundaries. Preserve accurate domain explanation and separately labeled Jobs persistence work. |
| `docs/wiki/code/contributing.md`, `deploy/README.md`, `deploy/atlas-dispatch.md` | Native/web/backend development loops and §8.3 mapping; Docker tools/context/build-only versus complete external startup, overlays/profiles/tags/ports/secrets/model assets. No native script/container launch. Respect actual operator workspace paths, not native-folder paths. |
| `docs/wiki/code/{ci,versioning}.md` | Both Mac matrices, Linux engines/search, exact-SHA dispatch/download/review comments, trusted signing/publishing, Actions retention versus Releases, minimum OS and actual launch status. |
| **Create** `docs/wiki/code/workspaces.md`; update `connect-context.md` | Protected container/default, name-only contained creation/attachment/deletion/recovery, child-only registration and IP:basename; collision/rebind/session scope, target backend and job attribution. Include safe old-layout transition; no root-as-workspace alias. |
| `docs/wiki/code/{actor-catalog,classifier}.md`; **create** `actor-search.md` | Readiness versus durable results; dataset upload/hash/limits, optional Natural training versus Rust-BERT, explicit activation; source/outbox/batched index/reader/dossier flow, global scope, pace/backpressure, repair and Linux deployment. |
| **Create** `docs/wiki/code/{brand-theme,native-ui-verification}.md`; update `react-ui.md`, `react-ui-testing.md`, `intel-web.md` | Exact logo tokens, native material allowlist versus web colors only; same-view Runtime/PNG/tree/script evidence and web/real-Mac limits; Connect gate and retained browser topology. Remove old TUI-palette-as-brand authority without changing valid web layout/Compiler rules. |
| Jobs/API/live/storage guides and applicable concept pages | Workstation-only triggered/native and web-origin jobs, causal children/dedup associations, filtered export/live/counts and global config revisions. Preserve data ownership and authorized external admin paths. |
| `docs/diagrams.md`, `docs/wiki/tags.md`, concept/code indexes and embeds | Register current pages/figures/anchors, repair source links and remove active retired navigation. Keep history only in labeled archive with replacement links. |
| `tui-{design-spec,components,verification}.md`, legacy CLI/Tauri/import manuals and screenshot references | Retire after supported consumers migrate. Preserve useful backend/browser content; remove contradictory old scope, arbitrary-parent and reset-on-launch instructions. |

Architecture prose must distinguish runtime from build topology, native workstation/root/children from deployment storage, public browser URLs from internal origins, safe readiness from maintenance mutation, and immutable origin from resource ownership. A passing build does not make a diagram’s planned behavior current. Keep one small coverage inventory: path/pair, update/create/retire/no-change, owner, source/implementation reference and verification status.

### 11.3 Diagram update/create/retire map

`{name}` means `docs/diagrams/{name}.html` **and** matching standalone `.svg`. Reuse an equivalent maintained figure; do not introduce another rendering framework. Preserve self-contained HTML/inline SVG, accessible titles/descriptions, typography, orthogonal connectors and limited accent/complexity conventions. Split unreadable combined stories. Do not replace these sources with screenshots or Mermaid.

| Action / figure | Required content |
| --- | --- |
| Rewrite `fleetworks` | Simple native controls and web-only topology, relay/gateway/private Linux/search/DB boundaries; deployment is external. No native-to-Docker, index or DB edge. |
| Update `workspace`; rewrite `workspace-scope` | Two UI groups, backend-global five pages, workstation-only activity, root containing protected `default` and named children, child registrations and global target versus origin. No folder-owned key/role copies. |
| Create/update `workspace-lifecycle` | Contained create/attach/select/retire/delete, dual directory protection, active/shared work, multi-backend revocation and filesystem/registry partial recovery. |
| Create/update `native-connections` | Gateway/child/interface selection → auth/capabilities → ensure/reuse → Connect ID → validated Connect form; collisions/failures/reconnect and no implicit browser/server launch. |
| Reframe/retire `venture-navigation` | Retain only a supported external operator deployment story. Native startup belongs in `native-connections`; migrate every embed before removal/rename. |
| Create/update `workstation-readiness` | Bounded partial checks; separate database seeded/search visible/model verified/training candidate/activation; shared-impact job admission, paced ingestion and only eligible node-associated job links. |
| Create/update `actor-search` | SurrealDB commit/outbox → bounded leased worker → private Tantivy commit/reader checkpoint → gateway hits → selected ID → authorized SurrealDB dossier. Show delayed/failed/throttled recovery and optional web topology, not native canvas. |
| Update `intel-web`; create/update `intel-connect-gate` | Browser Connect form/auth/session and denied direct pages/data/REST/live until validated; remote HTTPS/public versus internal URLs; sole topology rendering owner. Native need not remain open. |
| Create/update `connect-registry` | Node → contained child workspace → active backend binding; uniqueness, stable IDs, mutable IP/rebind/revocation, no root registration or remote-folder creation. |
| Create/update `brief-job-context`; targeted `jobs-lifecycle` | Validated origin → durable admission → stages/results; reuse adds request associations without creator overwrite. Workstation membership enforced before lists/counts/exports/live; exclude unrelated system/legacy work without deleting it. |
| Rewrite `dev-loops`; replace `dev-loop-cli` with `dev-loop-native` | External change planner; native host detection/Cargo/package/explicit open versus web HMR/server restart and Linux image recreate. Print paths; no native service startup. Retire old figure/embeds. |
| Update `dev-loop-web`, `dev-loop-runtime`, `dev-loop-images` | Actual mounts/build inputs, shared runtime reuse and dedicated search image, platform/tag/assets, backend-only group, preserved data, explicit recreation. |
| Rewrite `ci-scopes`; create/update `native-release` | Dependency-scoped checks vs builds; dispatch forces both Macs; exact SHA → package/sign → upload/download verify → review links → explicit trusted release promotion. Expiring Actions versus durable release assets. |
| Create/update `native-verification` | Real views → Runtime/input/accessibility → Rune PNG/tree → inspection; separate Linux integration, web and actual Mac launch. No terminal image reconstruction. |
| Audit `classifier-runtime`, `nlp-preprocessing.svg` | Linux-only engine boundary; optional Natural training/activation versus Rust-BERT readiness; preserve algorithms/pins. Add matching `nlp-preprocessing.html` when materially revising its SVG-only source. |
| Targeted `persistence`, `model-roles`, domain figures, `wiki-navigation` | Correct owners/entry points/navigation only; preserve valid domain and migration status. Record no-change where accurate. “Terminal job state” is not a TUI reference. |
| Optional `configuration-cutover` | One-time inventory/drain/revoke/reset/receipt/fresh global setup, protected root/default/data and no auto rerun. Prefer prose if a figure adds no value. |

### 11.4 Documentation and diagram completion gate

Follow `docs/wiki/maintenance.md`; integration owns coherence, with the scoped documentation editor/Scribe and component owners. Update alongside behavior, reconcile the affected surface before freeze, then recheck only relevant deltas.

Verify each command/route/arrow/target/URL/ownership against actual source, planner and packaging. Run existing documentation/link checks (including `scripts/fleetworks_doc_links.py` where applicable), heading/catalog/tag/embed checks and HTML/SVG semantic parity. Open/render changed pairs and inspect clipping, overlaps, legibility and connector endpoints. Existence or an uninspected screenshot is not semantic/visual review. Retire stale active diagrams/links only after replacement; label retained history.

Reject active instructions that make all five Backend pages per-workspace, register `.fleetworks` itself, allow arbitrary-parent native creation, offer global native Jobs/Logs, bypass gateway search, leave indexing unbounded or route Rune changes through terminal capture. Attach the path/pair coverage inventory, actual checks/results and remaining gaps to the PR/handoff. Missing architecture/guidance/pairs or broken build/download instructions block completion even if applications compile.

### 11.5 Agent guidance migration

Update actual repository guidance when corresponding code lands; this document alone does not satisfy the task. Preserve root policy, active host role boundaries, one build owner, cache isolation and existing authorization. Do not introduce extra tools/roles or rewrite unrelated model assignments.

| Entry point | Required direction |
| --- | --- |
| Root `AGENTS.md` | Current three-component architecture, two UI scope groups, Docker-free native boundary and dependency-scoped verification. Native is Rune Runtime/PNG/tree, not full host embedded-engine gate; topology is web. |
| Proposed `apps/fleetworks-desktop/AGENTS.md` | Simple controls, exact fixture commands, global versus origin semantics, contained root/default safety, node-only Jobs/Logs, no native graph/DB/index/container access and fixture-only effects. |
| Proposed `services/search-index/AGENTS.md` | Private service, single writer/lease, source truth, bounded queues/batches/pace, revision/tombstone/commit/reload fencing and real Linux recovery/load evidence. |
| Replace `.opencode/skills/argos-tui-verify/SKILL.md` with proposed `fleetworks-rune-verify/SKILL.md` | Read pinned Rune docs; select real-view scenes; assert behavior, capture PNG/tree, inspect and report. Retire TestBackend/Pillow/font reconstruction and irrelevant Mac backend gates after migration. |
| `.opencode/skills/argos-testing/SKILL.md` | Distinguish native unit/Runtime/render/package, Linux domain/search and React/Playwright evidence; include two-node and scope/path/overload tests. Keep isolation, synthetic secrets, no assertion weakening and honest blocked/zero-test reporting. |
| `.opencode/skills/argos-react-ui/SKILL.md` and actual aliases | Logo color-only scope, retained Compiler/Hooks/layout rules and web-only topology; no native liquid-metal effects on web. |
| `.opencode/AGENTS.md`, actual agents/commands/config/skill loaders, `.agents/` aliases, `.agy/howto.md` and active team guides | Repoint live references and delegated prompts to canonical native/search/usage/testing guides; specify owned files, contracts, scope/scene cases and evidence handoff. |

Native procedure: read guidance/pinned Rune docs → identify affected views/contracts → add regression/real-view fixtures → run scoped logic/Runtime/transport tests → render selected PNG/tree/scripts → inspect/fix → rerun affected checks → report exact evidence. Search requires real source/outbox/index/gateway/dossier recovery and pressure checks. Do not regenerate all screenshots for docs-only edits.

Audit tracked references before retirement:

```sh
git grep -n -E 'argos-tui-verify|tui_review[.]py|render_tui_cells[.]py|ARGOS_SCREEN_DIR|dump_overhaul_screens'   -- AGENTS.md .opencode .agents .agy docs scripts .github apps crates
```

No matches is expected after cleanup except labeled archive/history or an explicitly still-supported migration consumer. Preserve valuable behavior/data assertions rather than deleting by name. Trace **guidance → implemented test/fixture → actual evidence → canonical docs**. Unimplemented commands, stale active capture rules and unverified image approval remain blockers.

## 12. Delivery order and definition of done

Maintain a small live status table with owner, implementation reference, checks/evidence and blockers. Keep work in coherent increments; no endless parallel audits or duplicate mandatory builds. Apply existing authorization, preserve unrelated work and honor the repo’s freeze/finish-line policy.

| Phase | Exit requirement |
| --- | --- |
| **1 — Contracts and foundation** | Reconcile pins/current source; endpoint/resource/activity matrix, reset/layout inventory and docs/figure map. Prove Rune/toolchain on both Macs; API-only offline shell with grouped navigation and real-view fixture. |
| **2 — Identity and access** | Protected container/default and child lifecycle; saved connections/registry; discovery and registered web gate; global configuration ownership, immutable origin and enforced workstation-only activity. |
| **3 — Functional services/UI** | All eight pages have tested real operations; truthful readiness, seed/train/activation, durable actor outbox and paced Tantivy ingestion; gateway search → authoritative dossier. Web-only topology retained. |
| **4 — Developer/review loop** | Tested prerequisites/host helper/Cargo and separate Docker quick starts; planner/HMR/rebuild matrix; scoped Runtime/PNG/tree and real Linux/web tests; exact-SHA dual-Mac versioned artifacts/download links. |
| **5 — Verified cutover** | Matching downloaded-app launch evidence, docs/agent/diagram consistency, terminal/capture retirement and actual authorized one-time configuration/layout cutover receipts or explicitly outstanding deployment action. Measure comparable build/app/full-stack footprint; no inferred savings. |

Completion requires every applicable contract/check in §§1–11, not merely compiling or capturing a dashboard. A missing selected target/lane, endpoint, workstation access guard, real search chain, required visual inspection, updated guidance or runnable download is not a successful cutover. Public release readiness additionally requires its signing/notarization and minimum-OS evidence.

Final handoff records exact repository/Rune/Tantivy pins; changed/removed paths; commands and executed/pass/fail/blocked/skipped/pending results; scope/default/reset safety and actual reset status; source/index schema/generation/pace and measured load evidence; PNG/tree identities and inspection; real workflow/run/artifact/release URLs, both architectures, checksums/signing/minimum OS; docs/diagram/agent coverage and remaining blockers. Never include secrets. **Implementation, fixture success, real Mac launch, deployment reset and publication are separate statuses.** This specification authorizes no implicit publication or mutation of a running user stack.

## Appendix A — Pinned implementation references and baseline findings

The following are recorded references from the supplied handoff, not newly checked repository state. Reconcile their symbols, feature flags and owners against the chosen implementation revision; pin compatible dependency versions rather than following `latest`/`dev` silently.

| Reference | Baseline fact / implementation seam |
| --- | --- |
| [Fleetworks source baseline](https://github.com/xxxcess/argos-osint/tree/f8bc447c86f59c369a9259c6d0739681fadd1e94) | `feat/atlas-zero-shot` source reviewed previously. Follow its root guidance, wiki/planning conventions and existing service/Jobs ownership. |
| [Rune source pin](https://github.com/anomalyco/rune/tree/205b8d4f7e60fa6248b83bfbbb4c6a06096c59ff) | Core/kit READMEs, manifests, hello/shell examples, tooling arguments/headless and Mac packaging references are mapped in §2.2. Pin both crates consistently; verify candidate toolchain compatibility. |
| `crates/argos-osint-api/src/{lib,brain,actor_dossier,control,private_relay}.rs` | Existing actor search/detail routes and private forwarding; reviewed brief ensure/reload path lacks proposed registered origin. Control baseline does not supply the requested readiness/seed/train dashboard endpoints. Extend real owners, not a native shell adapter. |
| `crates/argos-osint-core/src/{intel_service,storage/actor_dossier,storage/actor_catalog}.rs` | Brief reuse/dedup and authoritative dossier/seed receipts. Preserve canonical redirect/active-actor reads and association provenance; Tantivy is derived discovery only. |
| `services/api-host/src/{actor_seed,classifier_train,classifier_service}.rs`; `apps/venture/src/classifier.rs` | Seed hash/lease/progress; Natural training candidate versus explicit activation; service ready can coexist with unverified model. Venture’s Compose launch path is parity reference only, never a native dependency. |
| `crates/fleetworks-workspace/src/{lib,import}.rs` | Old operator default resolves explicit/env/CWD `.fleetworks`; native now uses a protected home container with `default/`. Retire legacy key/config/role import, not domain data. |
| `apps/web-desktop/{server,server-origin,live-relay}.mjs`, server API and catch-all route | Existing loopback policy and REST/live guards must support protected remote ingress and registered sessions. Browser credentials are not supplied by native launch. |
| `deploy/{Dockerfile,docker-bake.hcl,compose.yml,compose.atlas.yml}` | Shared runtime; old `server` group includes web/operator/terminal targets. New backend-only group and host helper are proposed. Atlas depends on matching Linux model/native-library assets; desktop workspace membership must not break Docker manifest resolution. |
| Logo SVG/mark/JPEG blobs | `af831975e4b27d36f7d0bb26873cff0db1ad551b`, `ad3752b8a93d5de4b822994c4af833de3ac38750`, `280a5716b692c11d5ae26aadf284736eb0eea1f2`; exact SVG colors drive §2.3. Banner JPEG pixels were not separately sampled in the recorded review. |

- [Tantivy library](https://docs.rs/tantivy/latest/tantivy/), [IndexWriter commit/delete](https://docs.rs/tantivy/latest/tantivy/indexer/struct.IndexWriter.html), [IndexReader visibility/reload](https://docs.rs/tantivy/latest/tantivy/struct.IndexReader.html), and [Index ownership/test directory APIs](https://docs.rs/tantivy/0.27.0/tantivy/index/struct.Index.html).
- [Pinned Rune headless tooling and accessibility](https://github.com/anomalyco/rune/blob/205b8d4f7e60fa6248b83bfbbb4c6a06096c59ff/crates/rune/README.md#agent-tooling-headless-no-window-same-renderer-fixed-virtual-clock), [argument parser](https://github.com/anomalyco/rune/blob/205b8d4f7e60fa6248b83bfbbb4c6a06096c59ff/crates/rune/src/tooling/args.rs), and [hello Runtime test](https://github.com/anomalyco/rune/blob/205b8d4f7e60fa6248b83bfbbb4c6a06096c59ff/examples/hello/src/main.rs).

- [Apple Command Line Tools setup](https://developer.apple.com/documentation/xcode/installing-the-command-line-tools), [Rosetta detection/background](https://developer.apple.com/documentation/apple-silicon/about-the-rosetta-translation-environment), [official rustup installer](https://rustup.rs/) and [toolchain selection/overrides](https://rust-lang.github.io/rustup/overrides.html).
- [Python macOS downloads](https://www.python.org/downloads/macos/) for the external packaging/helper prerequisite; do not bundle Python in the native app.
- [Cargo JSON artifact output](https://doc.rust-lang.org/cargo/reference/external-tools.html) for finding the actual produced executable with custom output paths.
- [Docker Desktop Mac installation](https://docs.docker.com/desktop/setup/install/mac-install/), [Compose setup](https://docs.docker.com/compose/install/), [Buildx Bake/outputs](https://docs.docker.com/reference/cli/docker/buildx/bake/), [Buildx build/platform selection](https://docs.docker.com/reference/cli/docker/buildx/build/), and [Compose build](https://docs.docker.com/reference/cli/docker/compose/build/). These support command semantics, not proof that new repo targets have been implemented.


- [Rust macOS targets and SDK/deployment considerations](https://doc.rust-lang.org/rustc/platform-support/apple-darwin.html).
- [Cargo build target selection](https://doc.rust-lang.org/cargo/commands/cargo-build.html) and [Cargo run, target selection and argument forwarding](https://doc.rust-lang.org/cargo/commands/cargo-run.html). v6 command semantics were checked against these references; commands for the proposed package remain unexecuted.
- [GitHub runner architecture labels](https://docs.github.com/en/actions/reference/runners/github-hosted-runners).
- [GitHub manual dispatch and default-branch requirement](https://docs.github.com/actions/managing-workflow-runs/manually-running-a-workflow).
- [GitHub workflow trigger/security semantics](https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/trigger-a-workflow).
- [GitHub artifact storage/retention](https://docs.github.com/en/actions/tutorials/store-and-share-data).
- [Next Fast Refresh](https://nextjs.org/docs/15/architecture/fast-refresh) and [runtime/build-time environment configuration](https://nextjs.org/docs/app/guides/self-hosting).

- [Next.js authentication and authorization guidance](https://nextjs.org/docs/app/guides/authentication) and [Proxy limitations](https://nextjs.org/docs/app/getting-started/proxy): secure data/operation boundaries, not layout-only protection.
- [OWASP Authorization Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Authorization_Cheat_Sheet.html), [Session Management](https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html) and [CSRF Prevention](https://cheatsheetseries.owasp.org/cheatsheets/Cross-Site_Request_Forgery_Prevention_Cheat_Sheet.html): deny by default, scoped authorization, opaque protected sessions and mutation safeguards. These inform implementation; the proposed endpoints/entity model are Fleetworks design decisions.
- [RFC 7239 §11](https://www.rfc-editor.org/rfc/rfc7239.html#section-8): forwarded-IP trust/privacy limits; [RFC 5952](https://www.rfc-editor.org/rfc/rfc5952.html): canonical IPv6 text.

For guarded local file deletion and narrowly scoped secret cleanup, consult the official [Rust filesystem operations](https://doc.rust-lang.org/std/fs/) and [remove_dir_all](https://doc.rust-lang.org/std/fs/fn.remove_dir_all.html) documentation, plus Apple [SecItemDelete](https://developer.apple.com/documentation/security/secitemdelete(_:)). These describe primitives and their limits; the ownership, default protection, confirmation and recovery checks in this specification remain application responsibilities.

Repository/upstream links above are pinned for review. Reconcile with the implementation revision, and record changes to the dependency pin rather than silently following `dev`.
