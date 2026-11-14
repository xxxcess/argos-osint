# ARGOS_NATIVE_WORKSTATION_IMPLEMENTATION_HANDOFF

Baseline: `xxxcess/argos-osint` · `feat/atlas-zero-shot` · `d219d5964a29d50fa99de4cdcf2b228391576b4a` · revalidated November 13, 2026. Remote ref and comparison confirm the branch is unchanged from the earlier audit; latest commit: November 12, 20:21 UTC. Read-only source audit; no checked-out repository changes or product tests. [Branch baseline][baseline]

Proposed repository location: `.planning/2026-11-13-native-workstation-redesign/handoff.md` (new curated handoff). Standing design authority: `docs/wiki/code/native-design-system.md` (new, supplied as `argos_native_design_system_guide.md`).

## Outcome and audit conclusions

Deliver the reference's focused native workstation UI, use a stable workstation identity throughout native/API/web Connect, and apply **matte obsidian with dim satin graphite accents and canonical gold/ivory**. Native structure is native-only; Intel Web receives the theme/logo and Connect changes while retaining its layout and controls.

- Recent pushes retired Venture/terminal executables, retained independent Linux services, and added direct-Compose provisioning with private root `compose.env`. Preserve that completed cutover and existing dual-Mac build lanes. Do not restart it. [Current orientation][orientation] · [Recent deployment commit][deployment]
- `app.rs::Page` still contains eight destinations; `views.rs::workstation` and `backend_views.rs` stack administration and maintenance. Recompose the existing Rune shell and effects. The URL-only wizard and six-destination layout are new work. [Native views][views]
- `connect_registry::ensure_observed` already returns stable `workstation_id`; active Connect/context contracts still require IP/workspace. Migrate the complete contract, not just labels. [Registry][registry] · [Connect contracts][connect]
- `material::compact_toolbar` retains a broad gradient and web `FleetworksMark` approximates the emblem with three rays. Change material recipes and reuse the canonical five-ray asset. Generated mockups are visual references with synthetic data, not runtime evidence.

The supplied structural Markdown governs controls. Later user decisions override its workspace, Tauri/Linux-client, hardware-example, Rust-BERT-training and glossy-surface assumptions. Keep the current Rune macOS API-client architecture and implemented backend capabilities.

## Edit map

All existing paths below were checked at the baseline; only explicitly marked additions are new.

| Verified location | Work |
| --- | --- |
| `apps/fleetworks-desktop/src/app.rs::{Page,State,view,on_key,complete_connection}`; `views.rs::{workstation,brain,dossier,workspaces}`; `backend_views.rs::{maintenance,atlas,tools,models,profile,jobs}` | Reuse shell/list-detail composition; replace page model and stacked forms with the allowed destinations. |
| `apps/fleetworks-desktop/src/{launch.rs,connection_profiles.rs,connection_profile_views.rs,registration.rs,rebind.rs}`; `crates/fleetworks-workspace/src/native/{mod.rs,state.rs,connections.rs}` | Settings-only installation startup, protected credential provisioning, URL wizard, workstation registration; remove child-folder dependence. |
| `crates/fleetworks-contracts/src/{connect.rs,context.rs,capabilities.rs,operational_logs.rs,client.rs}` | Coordinated protocol/context version, workstation IDs, optional IP metadata, typed SDK and removal of active workspace query fields. |
| `crates/argos-osint-core/src/storage/{connect_registry.rs,schema/connect_registry.surql,bootstrap.rs,job_context.rs,jobs_view.rs,jobs_paging.rs}` | Forward migration, historical origins, admission/reconciliation, membership and paging. |
| `crates/argos-osint-api/src/{connect.rs,context_boundary.rs,activity.rs,activity_paging/cursors.rs,operational_logs.rs,request_reconciliation.rs,workstation.rs,lib.rs}` | Enforce new scope everywhere; preserve reconciliation; add bounded cycle summaries/tombstones and configured maintenance inputs. |
| `apps/fleetworks-desktop/src/{configuration.rs,configuration_views.rs,submission.rs,maintenance.rs,jobs_query.rs,jobs_effects.rs,logs.rs,log_effects.rs,readiness_cards.rs}` | Preserve permission/revision/idempotency fences; automatic bounded reads and allowed in-place action states. |
| `apps/web-desktop/{connect-id.mjs,connect-id.d.mts,connect-session.mjs,connect-session.d.mts,live-relay.mjs,app/server-api.ts,app/api/connect/route.ts,app/connect/connect-form.tsx}`; `packages/api-client/src/connection-scope.ts` | Workstation parsing, session/context validation, cache namespaces, relay and existing Connect form. |
| `apps/fleetworks-desktop/src/{theme.rs,material.rs}`; `packages/ui/src/{index.tsx,FleetworksMark.tsx}`; `packages/intel/src/shell/{intel-theme.ts,IntelShell.tsx}`; `apps/web-desktop/app/style.css` | Shared visual target through each client's existing theme authority. Preserve semantic colors and functional contrast. |
| `assets/brand/fleetworks-mark.svg`; `apps/web-desktop/public/fleetworks-mark.svg`; `apps/web-desktop/app/{layout.tsx,icon.svg,apple-icon.svg}`; `deploy/native_review.py` | Canonical logo/background handling and non-secret native credential-provisioning instructions. |

## Implementation

### 1. Establish workstation-only contracts and migration

Use the existing opaque backend `workstation_id` as Connect ID. Preserve authenticated scope plus protected installation-marker identity; make IP optional mutable metadata. Remove workspace names, markers, IDs and narrowing from new registrations, contexts, origins, queries and SDKs. Reject legacy ID/query shapes explicitly; advertise the coordinated incompatible contract version.

Implement an idempotent forward migration with a transaction/checkpoint and compatible-writer gate before reopening admissions. Preserve each workstation ID and its `(scope_id, installation_hash)` relationship; create one current registration per authorized workstation/deployment. Updating `DEFINE ... IF NOT EXISTS` alone is insufficient. Preserve immutable legacy origins and old registration-to-workstation mappings through version-aware readers; old registrations cannot admit new work.

Update `fence_causal_child_in` and `job_context::record_child_in` together so accepted chains can finish under the correct workstation lifecycle fence. Apply authorized membership before list totals, facets, pagination, details, logs, results, exports, streams and mutations. Preserve private-detail restrictions on shared jobs; never assign unknown/system work to a workstation.

Reuse `GET /api/v1/workstation/requests/detail` and its `original_registration_id` for authenticated read-only recovery. Preserve original registration/operation/request ID, exact body and context. Update both workspace checks in `job_context::accepted_request`, retaining other authority checks. An unknown lookup retains the unresolved handle; it never causes a new POST under the new registration.

Revoke workspace-bound browser sessions and invalidate old cursors/cache namespaces. Require a fresh Connect operation to establish workstation scope. Keep deployment/registration/access/configuration revisions, WebRelay credentials, HttpOnly/same-origin protections and stale-stream closure. Identical IPs must not merge installations.

### 2. Make fresh-install URL-only setup usable

Split protected installation/settings initialization from `NativeContainer::open`'s current default-child creation. Preserve installation markers, credential references and unresolved journals; normal startup must not create a hidden workspace. Retire folder/Trash workflows without deleting users' existing folders. Cargo workspaces, deployment directories and Jobs' private SQLite/RocksDB stores are unrelated.

Add **`--provision-gateway-credential`** (new mode on the existing native app executable), requiring existing `--api-origin` and `--api-token-file` arguments. Add a narrow `provision_credential_binding` manager method using `CredentialReference::{capture,open_verified}` and current locked, revisioned atomic storage. Persist an unambiguous exact-origin file reference, never token bytes; preserve other origins. Explicit provisioning handles rotation. Exit after a non-secret result without GUI, network or service startup.

Have `deploy/native_review.py` print the matching provisioning command using its existing private Native token path. Remote operators supply a private local token file through the same mode. This is app configuration, not another standalone terminal client.

The wizard has **Gateway URL → Connectivity Check**. Normalize origin, resolve/verify its binding, authenticate, check compatibility, and ensure workstation registration. Show gateway/registration, service readiness and model availability distinctly. Missing bindings or replaced files produce a checklist error with provisioning guidance. Never forward the previous origin's credential. Invalidate old confirmations, caches and pending responses on gateway/authority changes. Optional-service failure disables only dependent actions; permitted shared reads remain usable.

### 3. Supply the API behavior needed by the reference

- Bind seed/index/training actions to server-configured inputs and exact dataset/configuration revisions. Missing configuration disables the corresponding action with a reason. Reuse seed/reindex endpoints and **Natural** training; Rust-BERT inference readiness is separate. Training produces a candidate and never activates it.
- Reuse `POST /api/v1/atlas/cycles`. Add typed, capability-advertised cycle-summary reads and a cycle-history tombstone mutation alongside it. Scope summaries to authorized workstation associations; use authoritative timestamps/states/counts, with unknown counts unavailable. Delete only that workstation's cycle-history association, preserving shared articles/evidence and durable job audit. Require current context/revision/idempotency and in-place confirmation; deletion is not cancellation.
- Reuse protected tool-key/provider/role mutations. Inline edits preserve hidden fields and unchanged secrets, validate supported catalog choices and retain pending values on revision conflict.
- A mutation receipt means accepted/queued, not completed. Reconcile uncertain delivery through retained request identity; navigation cannot resubmit work. Display only server-reported outcomes.

### 4. Recompose the native UI

Use actual macOS window controls, the existing titlebar, trailing **Setup Gateway**, collapsible sidebar and clickable gateway/status header. That header opens **Workstation Status**, not a seventh nav item. Render only the selected destination.

| Destination, in sidebar order | Entire allowed content/control surface |
| --- | --- |
| **DB Seed & Train** | Actor seed, search-index build and supported training triggers with read-only metadata; no upload/picker or activation. |
| **News Cycles** | New Cycle, cycle cards and selected-cycle Delete; no article filters or list-first dashboard. |
| **Brain Search** | One search field, bounded results and dossier inspector; no tabs, extra filters or native graph. |
| **OSINT Tools** | Provider/service cards and key-provisioning modal; no manual tool-run/input/results workflow. |
| **Model Assignments** | Provider table with inline keys; role provider/model/effort/variant/ordered-fallback controls. |
| **Jobs & Logs** | Job queue/list and selected-job safe operational summaries; no tabs, filters, retry/cancel/follow/copy/export toolbar. |

Status contains read-only **Native Client**, **Gateway Backend**, and **Host Resources** cards, selectable Connect ID and real optional `GET /api/v1/host/metrics` values. No extra Copy/Test/Disconnect controls or fabricated hardware. Remove separate Profile, Workspaces, saved-profile CRUD and account/base-URL/import-export administration.

Start bounded reads on active-view entry/focus when needed; reuse fresh observations and cancel/fence obsolete reads. Keep logs bounded and update only while visible/focused; accepted jobs continue. Distinguish loading, empty, unsupported, denied, stale and failed states locally. Never fall back to global activity when unregistered.

Start at 1180×780, minimum 640×480, with the current 220-point sidebar defaults. At narrow widths use one column and ordinary list/detail navigation. Preserve wrapping, one scroll owner per pane, top-left-to-bottom-right keyboard order, visible focus and collapsed-nav labels/tooltips.

### 5. Apply matte brand styling and update maintained documentation

Canvas `#060709`; matte card/control bodies `#0B0C0F`/`#111317`; primary ivory `#F3EEE4`; supporting `#D6D2C4`/`#B8B4A8`; muted gold `#C6A56E`, pressed `#C4A265`. Use one compact flat gold primary action at most.

Create reusable matte-card, graphite-control and gold-primary recipes. Remove broad gradients, chrome rims, glossy sweeps, backdrop sheen and decorative animation. Static decorative card edges start at 1–2% ivory and control accents at 2–3%; **these never replace contrast-checked functional boundaries or focus tokens**. Hover/press use small fill/border changes without layout movement. Keep read-only cards noninteractive, reduced motion static and surfaces opaque.

Use the canonical five outlined rays, ivory disc and gold crescent; preserve size/accessibility and base-path-safe assets. Make the old charcoal knockout transparent where needed; intentionally opaque icon backgrounds use obsidian. Apply the same theme to existing Intel Web cards/controls while preserving its typography, navigation, responsive layouts, semantic colors, ForceCanvas and list twin. Layout names such as DOM `#intel-web-workspace` do not establish workspace identity.

Add the standing design guide and update affected native/web AGENTS/README, `native-connections.md`, `native-workspaces.md`, interfaces/Fleetworks docs and linked workspace/scope/lifecycle HTML+SVG diagrams. Replace active workspace instructions with workstation ownership and clearly bounded legacy-history notes. Update affected links/tags only; keep historical cutover evidence and existing CI lanes. Track work in the named plan with one integration/build owner.

## Acceptance and validation

- Fresh install provisions a protected binding, completes the URL-only wizard, displays Connect ID and connects Intel Web to the same workstation without creating a child workspace.
- IP changes/missing IP preserve identity; two installations/deployments remain isolated. Old IDs, sessions, query fields and cursors fail explicitly. Legacy receipts remain readable; uncertain submissions never duplicate; accepted causal jobs continue.
- Every visible native control maps to the table. Inspect all seven destinations plus setup, empty/loading/disabled/error/uncertain states at both native sizes. Verify keyboard traversal, selection, scrolling and 1×/2× logo fit. Generated mockups do not establish runtime or installed-app approval.
- Extend existing suites: core `tests/{storage_connect_registry,storage_workstation_activity,storage_workstation_paging,storage_setup_jobs}.rs`; API `tests/{activity_actions,activity_paging}.rs`; native `src/submission/tests.rs`, connection/appearance tests and production `examples/native/*_scenes.rs`. Preserve exact-request reconciliation, stale-response, credential masking, privacy and revision-conflict regressions.
- Update shared `crates/fleetworks-contracts/tests/fixtures/connect_ids.json`, web Connect/auth/relay fixtures and deliberate theme snapshots. Preserve numeric contrast, focus, forged-session, stale-tab and stream-revocation assertions.
- Follow existing verification authorization and reuse unaffected evidence. Run the planner, focused contracts, actual Rune captures and selected web checks; complete required embedded-storage and hosted scopes. Rust uses the shared debug cache and one build owner.

Verified command entry points for the execution agent (not run for this handoff):

```sh
python3 scripts/fleetworks_verify.py plan --base d219d5964a29d50fa99de4cdcf2b228391576b4a --head "$(git rev-parse HEAD)"
python3 scripts/fleetworks_verify.py local --base d219d5964a29d50fa99de4cdcf2b228391576b4a
python3 scripts/agent_cargo.py test --locked -p fleetworks-desktop -p fleetworks-contracts -p fleetworks-workspace
python3 scripts/fleetworks_verify.py native-ui --agent --list-scenes
python3 scripts/fleetworks_candidate.py build-native --agent --target host --profile debug
pnpm test packages/api-client/src/connection-scope.test.ts apps/web-desktop/__tests__/server-api-auth.test.ts
node --test apps/web-desktop/__tests__/live-connect-gate.test.mjs apps/web-desktop/__tests__/live-relay.test.mjs
pnpm test packages/ui/__tests__/theme-tokens.test.tsx packages/ui/__tests__/palette-contrast.test.ts
ARGOS_WEB_TEST_DEV=1 ARGOS_WEB_PROFILE=dev pnpm test:e2e tests/web/connect-gate.spec.ts tests/web/visual-regression.spec.ts
```

Use `native-ui --scene … --size 1180x780 --size 640x480` with actual adapted scene names and inspected PNG/tree/script evidence on a supported physical Mac. The planner/local wrapper already selects checks: avoid duplicating unchanged runs. For affected embedded persistence, the required gate is `env -u ARGOS_SURREAL_MODE -u ARGOS_SURREAL_BIN -u ARGOS_SURREAL_REQUIRED bash scripts/local_matrix.sh --storage embedded`. Hosted scopes retain both `x86_64-apple-darwin` and `aarch64-apple-darwin` app artifacts plus selected server/web gates; installed GUI/input/accessibility checks remain separate. [Verification workflow][verification]

**Audit checks performed:** remote ref/zero-diff comparison, recent commit review, bounded source/contract/instruction/test-command inspection and document consistency. No implementation, migration, builds, tests, deployment or publishing.

**Execution:** Verify the baseline and listed symbols; inspect only the relevant diff if HEAD moved. Implement in dependency order, run authorized focused/required checks, fix demonstrated failures and rerun affected checks. Report changed files, actual pass/fail/skip results, artifact paths and material blockers; keep pending platform evidence explicit.

[baseline]: https://github.com/xxxcess/argos-osint/commit/d219d5964a29d50fa99de4cdcf2b228391576b4a
[orientation]: https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/AGENTS.md
[deployment]: https://github.com/xxxcess/argos-osint/commit/d219d5964a29d50fa99de4cdcf2b228391576b4a
[views]: https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/apps/fleetworks-desktop/src/views.rs
[registry]: https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/crates/argos-osint-core/src/storage/connect_registry.rs
[connect]: https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/crates/fleetworks-contracts/src/connect.rs
[verification]: https://github.com/xxxcess/argos-osint/blob/d219d5964a29d50fa99de4cdcf2b228391576b4a/docs/wiki/code/contributing.md
