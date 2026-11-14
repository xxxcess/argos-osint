# Phase 8 — affected documentation audit

## Scope and finish line

Audit the changed workstation concerns against current source: native settings/provisioning/setup/views; v2 identities/migration/history/privacy; maintenance inputs/cycles; configured model choices/tool-slot semantics; web Connect/theme/HMR. Follow directly relevant wiki concepts/code routes, READMEs/AGENTS and diagrams. Preserve supplied bundle and archives verbatim. Root owns production source, shared acceptance/handoff/decisions and builds.

Finish when source-proven maintained-doc contradictions are repaired, affected links/HTML–SVG pairs validate and changed figures are inspected. This is a bounded audit, not a full repository review or product-verification pass.

## Findings and disposition

| Concern | Evidence / contradiction | Disposition |
| --- | --- | --- |
| Native identity/setup | Existing new connection/ownership guides agree; architecture, concepts usage/glossary and walkthrough still describe selected child/IP and Profile controls | Repaired in maintained native/architecture/concept guidance |
| Current figures | native-connections/connect-registry/brief-job-context/workstation-readiness SVGs still claim child identity, IP:basename, child filtering, CSV picker and activation UI | Repaired source-proven labels/flows and paired descriptions; seven changed/new figures visually inspected |
| Browser/theme | intel-web and React guidance retained glass/metal instructions while source uses opaque matte tokens and workstation-v2 sessions | Repaired and simplified with preserved web/API/authority obligations |
| Models/tool keys | Source advertises configured model choices and optional preserve/replace/clear slots; native guidance needed concrete semantics | Repaired: configured-route scope, coupled effort/variant setting and independent preserve/replace/clear slots |
| Verification | Existing browser/node/diagram evidence belongs to prior phases; this audit must not invent new runtime results | Reuse unaffected evidence and report audit checks separately |

## Evidence anchors

`crates/fleetworks-contracts/src/{connect,context,readiness}.rs`; `crates/argos-osint-core/src/storage/{connect_registry,job_context}.rs`; `crates/argos-osint-api/src/{configuration,workstation,request_reconciliation}.rs`; native `app.rs`, `configuration.rs`, `maintenance.rs`, `workstation_dashboard.rs`, `views.rs`, `backend_views.rs`; web `connect-session.mjs`, `server.mjs`, `live-relay.mjs`, `connection-scope.ts`; UI/Intel theme authorities.

## Result

Status: complete; documentation frozen. No product builds/tests run by this audit.

## Visual simplification

- Standing native guide: 4,191 → 1,123 words (73% reduction), preserving control/state/token/authority requirements and linking the unchanged detailed source rationale.
- Connections: 1,248 → 729 words (42% reduction); usage: 595 → 348 words (42% reduction).
- Added `native-workstation-layout` schematic; setup, identity, authority, activity and maintenance now lead with diagrams and short tables.
- Sixteen selected maintained pages: 34,117 → 25,230 whitespace-counted words (26.0% reduction). The new standing guide uses its unchanged supplied source guide as the before baseline; other counts use HEAD. This excludes historical archives and source-bundle text.
- Delegated subset: React UI 5,202 → 2,235; Intel web 5,897 → 4,985; web README 514 → 297 (35% combined reduction).
- Directly repaired pairs: native-connections, connect-registry, brief-job-context, workstation-readiness, intel-connect-gate and fleetworks. Reused prior unchanged workspace/scope/lifecycle inspection.

## Audit evidence and limits

- 808 affected local path/heading links across 28 maintained documents checked with no errors; 10 exact HTML/SVG pairs parsed as XML successfully.
- Repository Playwright rendered static local diagram HTML only. All seven new/changed figures were opened and inspected; a connector crossing a section caption was corrected, then the affected figure inspected again. Text bounds check reports no card overflow. This is document figure QA, not app verification.
- Scoped whitespace review passes. All 11 archive entries remain byte-identical to the original ZIP; archives and `.github` were not changed by this audit. No Cargo or product-test processes were started.
- Source fact retained: current cycle summaries expose optional article_count, not richer pipeline counters; unknown values stay unavailable. Configured-route choices are not an exhaustive provider catalog, and effort/variant share the actual provider/model setting.
- No unresolved maintained-doc contradiction remains in the inspected own scope. Delegated web/React/Intel docs are synchronized; root owns product evidence and open platform/storage obligations.

## Audit implementation notes

A scratch diagram generator initially refused a second run because its text replacements were one-shot; it was made idempotent before final rendering. Output-limited source reads were narrowed to relevant symbols/sections. Neither issue changed source requirements or product verification results.

## Added user scope — build-progress visibility

The user subsequently requested native build progress visibility. The backend implementation owner confirmed live stderr stage/Cargo diagnostics, unchanged stdout artifact parsing/results, package validation and launch only with --open while preserving an already running app. Added that concise flow to the native README/contributor build loop and linked the existing accurate development-loop figure. The owner reports 19 fake-runner unit tests passed; this documentation audit did not run them. No separate figure was needed. Documentation is now frozen.

## Added user scope — visual build artifact map

Created [Build artifacts, paths and tools](../../docs/wiki/code/build-artifacts.md), led by a new paired `build-artifacts` HTML/SVG figure. Contributing now explicitly states that large hash-suffixed Cargo storage/compatibility test binaries link embedded SurrealDB/RocksDB as QA harnesses, not additional deployed services; the native API client remains engine-free.

The map covers user/agent implicit and explicit-target debug/release graphs; libraries/metadata/dependency/freshness/incremental/build-script outputs; native app/manifests/archives and live build stages; Rune captures; Next normal/isolated outputs and locks; pnpm diagnostics/unit/browser evidence; BuildKit images/cache/export targets and candidate archives; tracked generated Natural code; the optional Linux Rust-BERT worker/bundle; and separately retained backend settings/data/models. Exact paths were checked against repository scripts/manifests/configuration, not a cache inventory.

Updated navigation: Contributing, code route, docs index and diagram catalog. No retired executable categories were added. No production source, Cargo/Next build, backend or package launch was performed.

Final expanded static review: **29 maintained documents, 831 local links/heading anchors, 11 XML-valid exact HTML/SVG pairs, all 11 original ZIP entries unchanged**. The new static diagram was rendered with repository Playwright from a local HTML file, opened and inspected; text-bound checks found no overflow. Prior unaffected figure inspection is reused. Scoped whitespace checks pass. The new map adds 1,112 whitespace-counted words for the explicitly requested artifact categories, separately from the earlier reduction metrics. Documentation is frozen again.
