# Fleetworks native cutover implementation handoff

Prepared November 12, 2026 and maintained during implementation. This handoff defines delivery; acceptance.md records checked results and pending gates separately.

## Contract, baseline and status

The user-supplied consolidated specification is preserved verbatim in [design.md](design.md); it remains the full requirements authority. Its original location is `/Users/successndalama/Downloads/fleetworks-native-cutover-spec.md`; SHA-256 is `23054b611c436ccc2f979d6078f18a194f2f95862a4aab78aa35d5ae1e6cdf8a`. Requirement references below use its section numbers. This handoff organizes delivery without weakening that contract.

| Item | Recorded status |
| --- | --- |
| Source reviewed | `feat/atlas-zero-shot`, `7fb262b062275d0bd159fb4c7ba18be7321c4e4b`; clean at session start, one commit ahead of the locally tracked upstream |
| Spec source baseline | `f8bc447c86f59c369a9259c6d0739681fadd1e94`; implementation-start HEAD adds one scheduler lease-election serialization fix; retain it |
| Rune and rune-kit | Same candidate revision `205b8d4f7e60fa6248b83bfbbb4c6a06096c59ff`; pinned in the implementation; initial Intel and ARM compilation passed on Rust1.95; newest ARM and installed GUI evidence remain pending |
| Tantivy | Spec candidate `0.27.0`; locked in the implementation; current host disk/HTTP/pressure cases passed; real Linux and durable-pump evidence remain pending |
| Planning | Prepared from supplied spec and current source by one integration owner with independent runtime and surface research |
| Product implementation / verification | Phases 1–4 active; scoped local/native/browser/isolated-Docker verification and local commits authorized; current results in acceptance.md |
| Real Mac download/launch / publication | Pending / not performed |
| Real configuration reset / layout transition | Pending external deployment actions; no user installation, credentials or running stack changed |

Use [decisions.md](decisions.md) for contracts and risks and [acceptance.md](acceptance.md) for verified and pending evidence. The ignored `task_plan.md`, local research/progress and cleanup inventory are working memory; this durable handoff does not depend on them. Preserve the recorded baseline as local commits advance; do not confuse a dirty worktree with an exact published artifact.

## Outcome and boundaries

Deliver one API-only Rune/rune-kit `Fleetworks.app` for Intel and Apple Silicon macOS. It opens offline, owns safe local connection/workspace identity, and sends operations to an independently provisioned gateway. Linux owns services, jobs, inference, authoritative SurrealDB and private Tantivy; Next/React Native Web owns browser access and all topology visualization. The app never starts or stops backend/web processes.

All eight pages share one shell/state/runtime/client:

| Sidebar group | Pages and scope |
| --- | --- |
| Backend — selected deployment | Atlas, Brain, Tools, Models, Profile; permitted backend-global corpus/settings/accounts/roles |
| Workstation — scoped node/IP | Workstation, Workspaces, Jobs & Logs; local identity/folders and only durably associated triggered work for the registered workstation; optional child filter narrows it |

Resource target, permission and immutable request origin are separate. A second local workspace does not create key/role/corpus/model/index copies. Registration supplies context, not administrative permission or personal authentication. No native topology, webview, terminal adapter, storage/search/inference engine, Docker socket/SDK, embedded server, arbitrary command runner or backend supervisor. No new orchestrator, rendering framework, classifier algorithm, mobile/Windows product or unrelated Jobs migration.

Protected native layout is `~/.fleetworks/` container plus `default/` and named immediate children. The container is never a workspace/Connect registration; container/default cannot be removed through ordinary actions. Keep native local paths/types distinct from retained operator deployment workspaces.

## Baseline code and implementation seams

This map records the inspected preimplementation baseline. The endpoint inventory and acceptance tracker identify current landed interfaces and remaining work; baseline descriptions are not current completion claims.

| Concern | Current seam | Work to perform |
| --- | --- | --- |
| Product graph | Root `Cargo.toml` defaults to Venture/CLI; no desktop/search package | Add thin `apps/fleetworks-desktop`; pin one compatible repository toolchain and both Rune crates; keep backend engines out of its resolved graph |
| Gateway client | `crates/fleetworks-contracts/src/client.rs::Client::new` | Preserve origin-only input, loopback HTTP, authenticated remote HTTPS and redirect credential isolation; extend typed DTOs |
| Purpose permissions | `crates/argos-osint-api/src/network.rs`, `AccessMode::{Loopback,Bearer}` | Current single-bearer policy is insufficient; establish native registration, web resolution, maintenance, configuration and activity authority before exposing new operations |
| Stable deployment | `storage/bootstrap.rs::ensure_network_identity`, persisted `storage_meta:identity` | Reuse persisted authority, not process-specific `ApiState.instance_id`; coordinate additive schema/bootstrap changes (reviewed schema version 6) |
| Native folder reuse | `crates/fleetworks-workspace/src/lib.rs::Workspace::resolve` currently supports explicit/env/CWD operator layout | Extract minimal client-safe types; implement separate home/container/child semantics and recoverable lifecycle rather than adopting service-data layout |
| Registration/session | No current Connect registry | Add one authoritative registry owner via existing repository/schema abstractions; database uniqueness, default protection, retirement/rebind, hashed opaque web sessions and epochs |
| Capabilities | API `lib.rs::capabilities` | Extend existing categories/auth/features/live response with stable deployment, versions/scopes/config epoch/search and validated advertised browser URLs |
| Durable brief origin | API `brain.rs` → `lib.rs::{ensure,reload}` → `intel_service` → `storage/brief_stages.rs::ensure` | Persist validated origin/request association inside existing `run_tx` with `jobs::ensure_chain_in` and input capture, including reused chains; never enqueue then attach later |
| Atlas/Tools origin | `operations.rs::submit_atlas_cycle`, `storage::job_dispatch::submit`, `tools.rs::submit`, `osint::manual::Tools::submit` | Extend actual durable admissions and causal children; reconcile request-key scope without breaking legitimate shared work |
| Native activity | API Jobs/log/live readers and `storage/jobs_view.rs` currently lack registered-node membership | Apply one server policy before counts/pagination/details/stages/results/logs/exports/live/actions, not after fetching all rows |
| Global configuration | API `control.rs`, `ConfigLock`, settings/provider/secrets readers | Add missing account/key CRUD, protected byte import/export and expected revision; existing server-file export path is not a remote download contract |
| Setup operations | `services/api-host/src/{actor_seed,classifier_train}.rs`, Venture classifier parity | Expose bounded uploads/readiness/durable seed and optional Natural training/explicit activation through server functions and worker dispatch; never wrap Compose/stdout |
| Source/index transaction | `storage/actor_catalog.rs::ingest` already writes actors/aliases/progress and derivative invalidation in `run_tx` | Add durable projection intent in the same transaction; cover no-op seed reconciliation and every relevant actor/alias/lifecycle writer |
| Discovery | API `actor_dossier.rs::search`, catalog name/alias lookup and `/actors/detail` | Extend existing search route with versioned Tantivy discovery; preserve deterministic binding and canonical SurrealDB dossiers |
| Web gate | `proxy.ts`, `app/server-api.ts::proxyRust`, `server.mjs`, `live-relay.mjs` | One authoritative guard for protected render/RSC/prefetch, REST/export/actions and upgrades; no middleware-only unlock |
| Remote web | `server-origin.mjs` currently requires loopback HTTP | Add configured authenticated external HTTPS origin without loosening Host/Origin/upstream separation |
| Web palette | `packages/ui/src/index.tsx`, `packages/intel/src/shell/intel-theme.ts`, app CSS | Gold/charcoal/ivory colors only; decouple success/kind/metric aliases, preserve geometry/type/motion and ForceCanvas/list behavior |
| Delivery | Existing candidate/planner scripts, Dockerfile/Bake, candidate workflows | Extend these authorities, not competing frameworks; manifest-copy inputs must tolerate desktop workspace membership without compiling Rune in Linux images |

## Delivery phases and exit gates

### Phase 1 — Contracts and foundation

Owner: integration; parallel native spike and backend contract research after interfaces are assigned.

1. Reconcile current source, scoped instructions and upstream pinned Rune examples/manifests/tooling; record exact dependency/features and one repository toolchain. The spec records Rune's checkout toolchain as 1.99.1 versus repo 1.95; neither establishes actual minimum compatibility. Prove both Mac graphs before broad screen work and coordinate any required toolchain change across images/CI.
2. Build one durable endpoint matrix from CLI README/implementation and Venture: old command → app operation or justified operator task → method/route/live channel → typed request/response → resource/permission/origin/revision/idempotency → UI → test/status. Cover every required operation; no generic execute endpoint or mock-only parity.
3. Freeze identity/session/origin/activity/configuration DTOs and authority model. Inventory actual configuration readers/reset targets and root/default layout transition without reading or deleting secrets.
4. Create offline grouped eight-page shell, small library entry point, production-view fixture and one asynchronous gateway read using injected effects. Confirm dependency exclusions, UI-thread updates and startup without backend/Node/Docker/Python runtime/model downloads.
5. Establish docs/diagram disposition inventory and same-view fixture inventory with owners. Record baseline source/build/footprint evidence only when measured.

Exit: both-Mac compatibility evidence, API-only offline shell/one real-view fixture, reviewed endpoint/scope/reset-layout inventories and frozen shared interfaces. Compatibility failure blocks downstream claims; do not silently change compiler or drop Intel.

### Phase 2 — Identity, access and durable context

Dependencies: Phase 1 contracts. Backend registry and native folder manager can develop independently against frozen DTOs; wire together after boundary tests.

1. Purpose-scoped credentials/permissions and persisted deployment/configuration/reset epochs; backend registry with transactional uniqueness, canonical IPv4/bracketed IPv6 + NFC basename shared fixture corpus, explicit collision/rebind/revocation/default protection.
2. Protected root/default and name-only immediate-child management; saved connection credential references, native interface selection, identity markers, ownership/object-identity guards, journaled creation/retirement/deletion and all-known-backend revocation. Local creation may remain unregistered offline; blocked deletion keeps files intact.
3. Extend capabilities and implement authenticated ensure/reuse, registration display/copy, permission-checked browser session resolution and validated advertised web launch. Never register root, proxy IP or arbitrary full path; launch browser only after successful registration and URL validation.
4. Persist immutable native/web origins, original creator plus separate reused-job request associations, configuration/credential references and causal propagation at actual durable admission. Legacy rows stay unattributed. Shared-resource leases remain global.
5. Enforce workstation-only membership before all activity reads/counts/exports/live/actions. Membership is not cancel/retry permission; mixed-origin private events must not leak.
6. Add central web Connect gate with secure opaque cookie, CSRF/Origin, fail-closed authority, revision-bound stale-tab mutations, revocation/stream closure and no-store/private cache policy; preserve authenticated ingress and separate server bearer.

Exit: two nodes/two children/second deployment plus dedup/system/legacy fixtures prove correct global settings, exact activity membership, gate denial before data, collisions/rebind, retry convergence and protected filesystem recovery. A Connect ID alone grants no admin or personal identity claim.

### Phase 3 — Functional services, private search and complete pages

Dependencies: Phase 2 authority/origin; independent search-service work can begin once its projection/batch contract is frozen.

1. Complete backend-global Profile/Models endpoints and forms, protected current-format byte transfer, missing-key setup and expected configuration revisions. Workspace switches preserve authorized global data/drafts; gateway/auth changes isolate caches and unsaved edits.
2. Bounded dataset upload/metadata/expiry by kind/hash; read-only partial readiness aggregate separate from lightweight health. Durable Seed Actors, optional Natural training and exact-candidate explicit activation use existing workers/Jobs and resource-level concurrency, not native/HTTP lifecycle wrappers.
3. Implement private Linux `services/search-index` with isolated Tantivy graph, one serialized writer, bounded query capacity, typed constrained search, durable generation/checkpoint/tombstone evidence and authorized projection batches. Indexes are disposable projections; SurrealDB is truth.
4. Add transactional actor outbox + leased ingestion coordinator, canonical bulk actor/alias reads, no-op/backfill reconciliation, all lifecycle producers, crash/replay fencing and atomic verified generation switch. Batch acceptance is commit → explicit reader reload → verified visible checkpoint → durable acknowledgement.
5. Implement server-owned pacing: proposed ≤500 documents or 2 MiB/batch, one in flight, ≤2 starts/sec, two prepared batches, bounded source reads, partial-tail flush, shared budgets/live priority, Retry-After/backoff/cooldown and disk/memory/query-pressure pauses. Validate/measure these starting policies; do not claim throughput from configured numbers.
6. Wire Brain debounce/latest-response fencing → bounded gateway hits → selected canonical ID → authorized fresh SurrealDB dossier. No fallback engine, native graph or eager full-dossier hydration.
7. Complete all eight production pages/actions/states and cross-page activity links. Workstation distinguishes seed versus visible index, inference readiness versus optional Natural candidate/activation. Web palette change stays colors only and topology/list behavior remains in web.

Exit: endpoint-backed parity matrix complete; one real Linux seed → durable index → gateway search → authoritative dossier journey plus update/delete/revoke and recovery tests; truthful readiness/train/activation; complete native real-view state coverage and retained web topology.

### Phase 4 — Developer, verification and review delivery

Dependencies: shell/fixture can enable early packaging; final packages require Phase 3 parity. Reuse one planner, candidate helper and packager.

1. Shared physical-machine/Rosetta/toolchain detector, planner `platform` interface, dependency-aware checks/builds/recreation/HMR, deletion/rename/untracked/unknown-path handling and explicit dual-Mac dispatch selection.
2. Candidate `build-native` and shared `package-native`: explicit target/profile/output/open/archive/dry-run; Cargo compiler-artifact discovery; staged `.app` at `dist/native/<target>/<profile>/Fleetworks.app`; manifest/checksums; retain last good build and never overwrite installed app or launch stale failed output.
3. True backend-only Bake group including runtime/search/database wrapper and reproducible matching Linux classifier artifacts; separate web/Atlas overlays; unique review tags/platforms, clean Docker workspace manifests and complete external startup guide with secrets/volumes/ports/UIDs/image identities. No stack mutation from app/build-only commands.
4. Extend existing verification runner for selected real Rune Runtime/PNG/tree/script scenes. Build once, fail zero/missing evidence, inspect actual images and keep headless, interaction, Linux integration, web and physical-Mac results distinct.
5. Code Architect updates `.github`: exact-SHA graph, both Mac targets, both Linux architectures, required aggregate, trusted signing/publication separation and downloaded-artifact verification. Review artifacts default to `publish=none`; comments/messages/releases need applicable authorization.
6. Validate documented native build/package/Cargo lanes and separate Docker backend build/start lanes. Record minimum OS/signing/accessibility/GUI statuses rather than inferring them from runner labels or screenshots.

Exit: tested commands/planner, complete isolated backend recipe, selected checks and actual inspected native evidence, both versioned `.app` ZIPs with real artifact IDs/URLs and exact manifest/checksum identity. Publication remains separate from artifact construction.

### Phase 5 — Verified retirement and external cutover

Dependencies: both downloaded matching-hardware app parity checks and verified web topology. Synthetic reset/tool tests should be developed earlier; real reset is late and separately authorized.

1. Download both ZIPs by artifact ID, verify bytes/architecture/libraries/permissions and launch via Launch Services on matching hardware/minimum OS. Exercise local/remote gateway read/mutation, setup seed/train, protected folders and registered web handoff/attributed brief. Missing GUI/GPU support is blocked evidence, not a pass.
2. Audit/migrate callers then remove six standalone CLI products/TUI/line sessions, CLI images/archives, second Ratatui consumer behavior, legacy key/config loaders/aliases and terminal-only capture pipeline/dependencies. Retain justified thin external operator/headless/service entrypoints; do not delete by name alone.
3. Migrate actual docs, paired diagrams, indexes/tags and agent/skill routing alongside owning changes. Review affected pairs and terminal references before freeze; retain historical evidence and open obligations with explicit dispositions.
4. Finish/test externally invoked reset inventory/apply/receipt tool with synthetic secrets. Execute a real reset only against frozen explicit local/backend targets under separate destructive-cutover authorization: drain, revoke, clear allowlisted user credentials/settings, retain infrastructure credentials/data/root/default/ledgers, persist epoch and provision fresh access. Never reset on build/first launch/upgrade or erase post-reset new keys on repeat.
5. Execute explicit old-layout transition with independent local/backend receipts; no recursive move/adoption and no active root-as-default alias. Record inaccessible installations and outstanding operator actions.
6. Freeze scoped inputs, run one affected final verification pass and publish only when explicitly authorized. Compare measured build/app/full-stack footprint; no assumed savings.

Exit: all applicable [acceptance](acceptance.md) rows supported by exact evidence; implementation, tests/images/inspection, real Mac launch, configuration reset/layout transition and publication each retain their own status. Outstanding real reset or GUI validation prevents a full cutover claim.

## Subagent execution and ownership

Use four concurrent slots at most: one integration/build owner plus three workers. Workers use disjoint assigned edit paths/worktrees; root integrates, owns shared manifests/lock/toolchain/schema registration/DTO contract files and all builds. Edit worktrees remain format-only. Wake/reuse workers by wave; no competing Cargo graphs or broad duplicate audits.

| Wave | Worker A | Worker B | Worker C | Integration owner |
| --- | --- | --- | --- | --- |
| 1 | Native shell/one real-view fixture under new desktop app | Endpoint/auth/registry/reset-layout contract design, read-only source map | Platform/package/planner interface design and docs/diagram inventory | Baseline/pins/toolchain proof; freeze shared interfaces and integration files |
| 2 | Desktop connection/context/workspace filesystem modules and fixtures | New registry/session/auth modules and tests | Actual durable origin/request association/activity readers and tests | Schema/DTO integration; serialize shared API wiring and builds |
| 3 | Web session guard/Connect/custom relay/upgrade and context-keyed cache | Readiness/dataset/seed/train/configuration API and worker changes | Private search service, projection protocol and its isolated tests | Coordinate API `lib.rs`/control overlap and approve explicit worker handoffs |
| 4 | Native Atlas/Brain/Tools views after shared state/client freeze | Native Models/Profile/Workstation/Workspaces/Jobs & Logs views | Actor outbox/producers/coordinator/discovery integration | Own desktop app/state/exports wiring; preserve actor writer and API ownership boundaries |
| 5 | Platform/planner/candidate packager and their tests | Linux image/Bake/external startup guide | Code Architect: `.github` matrices/artifact/release integration | One build owner, real-view/web/Linux/Mac evidence and affected docs/figures |
| 6 | Audited terminal/import/capture retirement | Canonical docs/diagram/agent-routing migration | Synthetic reset tool and reviewed operator runbook/receipts | Final scope review, freeze, affected verification and explicit handoff |

These are roles to reuse, not six persistent teams. Integration assigns exact file lists before each wave. API `lib.rs`, `control.rs`, storage `mod.rs`/bootstrap, shared DTOs, app state and root manifests have one editor. Search producer hooks cannot race provenance edits to actor/job writers. Web palette work follows the web gate owner on overlapping files. Code Architect ownership applies to every `.github` edit.

## Documentation and historical dispositions

Create canonical native-client, workspaces, actor-search, brand-theme and native-ui-verification guides; create/reconcile connect-context. Update architecture/Fleetworks/interfaces, contributing/deploy/Atlas, CI/versioning, actor-catalog/classifier, Jobs/API/live/storage and React/Intel guides with owning behavior. Do not publish proposed commands as working instructions.

Maintain one path/pair inventory with update/create/retire/no-change, owner, source reference and actual review status. Spec §11.3 names the figure map: Fleetworks/workspace/scope/lifecycle/connections/registry/gate/readiness/search/job-origin, developer loops, CI/release/native verification and affected classifier/persistence/model-role figures. Use paired HTML/SVG and inspect modified figures; repair embeds before retirement.

Migrate root/scoped AGENTS, actual OpenCode/.agents aliases/skills/commands/loaders and active Antigravity guides under §11.5. Replace TUI routing only after real Rune replacements pass; preserve generated Next guidance and valid React Compiler/Hooks rules. No prose-only claim that guidance has already migrated.

Retain `.planning/2026-11-09-fleetworks-overhaul/{design,decisions,acceptance}.md` and its unresolved obligations; record a scoped disposition when superseding active terminal instructions. Keep the Jobs persistence plan independent. No archive/plan deletion is authorized by this handoff.

## Current delivery checkpoint — November 12, 2026

Implementation is active across Phases 1–4; Phase 5 retirement/external cutover remains pending. See [acceptance.md](acceptance.md) for the 63 native, 25 Core, 44 API, 13 search and 8 browser scoped results and their revision limits. Initial planning language above defines the finish line; it is not evidence that the remaining work passed.

Current owners: native_foundation holds the 71-case lifecycle checkpoint frozen and prepares system preferences/material/freshness; native_delivery_tools integrates bounded local parsing/RSS and exact claimed-task seed fences; cutover_contracts prepares explicit exact-candidate Natural activation. Reset startup and durable rebuild have compiled, and their focused server fixtures passed; required embedded and Linux evidence remain separate. Root owns shared integration, all builds, source freeze, curated plans and commits. No worker stages or commits.

## Immediate next action

Complete the running native lifecycle suite (expected 71 cases), preserving the verified 63-case action checkpoint. Latest unified host/search check passed in 1m26s with zero warnings; 10 new required-server cases passed for strict network reads, reset admission barrier, transactional outbox and durable rebuild. Integrate exact task-fenced bounded seed execution and separate explicit activation, then run their affected checks before the required embedded gate, isolated Docker journeys and final affected documentation.

The user explicitly authorized local commits and requested current planning updates. Commit reviewable checkpoints, without pushing or publishing. Actual reset, layout transition, installed dual-Mac validation and terminal retirement remain separately gated; preserve all unrelated work and the baseline scheduler fix.

Local commit `294a634` records the curated plans and requested cache-first build guidance. Root owns subsequent source commits and records actual validation limits with each checkpoint; no push or publication has occurred.
