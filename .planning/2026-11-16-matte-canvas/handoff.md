# ARGOS_MATTE_CANVAS_IMPLEMENTATION_SPEC

Baseline: `xxxcess/argos-osint` · `nightly` · `00b700afd77ddac6cba2e2bd7ae91ab3802b7197` · tree `5051474185a8eb14b38e607480bb24c65352fff0` · source/documentation audit 2026-11-16 America/New_York. This documentation-only change records the approved target; runtime source, product tests and installed-app evidence remain pending. Preserve unrelated work and use `nightly` as the intended PR base.

## Outcome and audit conclusions

Reproduce the approved Matte Canvas screens, the selected three-second native Quiet Orbit introduction, stable realtime states and continuous long-list scrolling through the existing native/shared web architecture.

- Native `app::view` aggregates unrelated reads into **Syncing**; readiness refresh errors discard last-good cards; empty log follows can reenter skeletons; Brain Pending/Loading changes presentation. Use the [shared state contract](../../docs/design/matte-canvas/interaction-state.md).
- Native Jobs/cycles expose Previous/Next/Page and replace a visible cursor page. Convert the presentation to bounded continuous scrolling while preserving opaque cursors, authorization and response fences.
- Shared web `Skeleton` loops opacity; native matte preferences suppress shimmer. Use a steady base and a translated cool loading sheen, with static reduced motion.
- Approved mockups are visual references, not runtime evidence. Canonical specifications are [theme](../../docs/design/matte-canvas/theme.md), [interaction/state](../../docs/design/matte-canvas/interaction-state.md) and [motion](../../docs/design/matte-canvas/motion.md). They supersede older gold/ivory, reflective-panel and coarse-utility treatments.

## Edit map

Native source paths in this table are under `apps/fleetworks-desktop/src/`; web paths are repository-relative.

| File / symbol | Required change |
| --- | --- |
| Native `theme.rs`, `material.rs`, `accessibility.rs`, `ui_components.rs::ActionButton::build`, `section_header`, `skeleton_metric_tiles`, `async_status_strip` | Shared matte recipes, action-only coarse rock, stable state geometry, cool loading sheen and reduced preferences. |
| Native `app.rs::State`, `spec`, `poll`, `view`; `views.rs::page`, `setup_gateway`, `workstation`, `brain` | Finite splash phase, measured shared canvas/grids, connection/activity separation and faithful setup/service compositions. |
| Native `readiness_cards.rs::checks`; `app.rs::complete_readiness`; `workstation_dashboard.rs::load_cycles_page`, `complete_cycles_page`, `apply_host_metrics` | Retained freshness/error presentation, bounded cycle accumulation and existing metrics retention. |
| Native `jobs_query.rs::JobsQueryState`; `jobs_effects.rs::request_jobs`, `project_jobs_page`; `backend_views.rs::jobs`, `safe_log_table`; `logs.rs::LogState`; `log_effects.rs::poll_scoped_logs` | Continuous cursor windows, stable row/scroll identity, verified empty and local boundary recovery. |
| `packages/ui/src/Skeleton.tsx`, `index.tsx`, `FleetworksMark.tsx`; `packages/intel/src/shell/IntelShell.tsx`, `intel-theme.ts`; `apps/web-desktop/app/style.css` | Shared tokens, symbol-only brand, surface/state parity and replacement of recurring skeleton opacity animation. |
| `packages/intel/src/views/bulletin/BulletinView.tsx`, `FeedHeader.tsx`; `packages/api-client/src/index.tsx::useArticles`; `packages/intel/src/views/brief/StageContent.tsx`, `BriefProgressStepper.tsx`, `ActorChips.tsx`, `BriefSection.tsx` | Reuse continuous-feed admission/query identity; preserve generation precedence and stable retained content while tightening copy. |
| `assets/brand/source/fleetworks-liquid-metal-glass.png`; `assets/brand/reference/fleetworks-quiet-orbit.gif`; `assets/brand/reference/quiet-orbit/playback.json` | Immutable approved source/reference bytes, provenance and bounded derived runtime media. |
| `apps/fleetworks-desktop/examples/native_fixture.rs`, `examples/native/appearance_scenes.rs` under that app, and `examples/native/splash_scenes.rs` (new); affected native/web tests | Shared production-path state, timing, scrolling and visual evidence; new scene names must be registered before use. |

## Implementation

1. **Reproduce the approved references.** Use the four images under `docs/design/matte-canvas/mockups/`: `01-workstation-status.png`, `02-setup-gateway.png`, `03-jobs-logs.png`, `04-compact-workstation.png`. Follow the theme's geometry, typography, spacing and component recipes. Use softly blended smooth matte canvas/cards/rows without decorative panel strokes, cool text and symbol-only branding. Coarse matte moon rock belongs only to Submit/Confirm/Create/Delete action buttons. Utilities/navigation stay smooth matte. Permanent sheen belongs only to the logo; loading has its own restrained cool sheen. Preserve functional focus/error affordances.

2. **Adopt the selected motion.** Preserve source PNG/GIF bytes and recorded hashes; derive runtime assets according to `motion.md`, not by altering the canonical files. Show fully formed materials from the first visible logo frame, with no flat/outline material reveal. Exactly five glass triangle satellites follow the approved full inclined orbit and remain visibly clear of the crescent, including their transformed silhouettes and fringe. Keep sphere rotation and internal molten flow distinct. The review GIF's loop does not set startup duration: native startup lasts three seconds, then enters the existing gateway-selection/setup path. Use a persistent shared canvas, whole-orbit contain scaling and the motion guide's still/reduced-motion/error fallbacks. Decode asynchronously outside `view`; no webview, video-player runtime or new 3D engine. Preserve reconnect/pairing state, block underlying input during startup, transfer focus once and never replay on resize/reconnect/navigation.

3. **Resolve state centrally.** Derive presentation from accepted snapshot, identity, activity, authoritative domain state, transport and freshness. Keep same-scope data and verified empty results during refresh. Retain generation-specific pending/terminal rules and independent content; do not treat old output as current-generation completion. Preserve epoch/query/generation fences. Keep connection health separate from busy requests. Use stable headings, keys and reserved local status slots; no poll-driven remount, label cycling or height change. Results/errors resolve immediately, without cosmetic timing delays.

4. **Match loading and compact grids.** Use the same measured slots for loading/populated/empty/stale content. Fix native telemetry's always-horizontal skeleton versus its below-540-pixel populated layout. Preserve semantic/keyboard order and stable card positions; live values do not reorder grids. Translate one dim cool sheen over a constant-opacity skeleton base at 1.2–1.8 seconds per cycle; reduced motion is static. Keep known labels readable, pause hidden decoration and retain the same material quality through gateway and service loading/live states.

5. **Make long lists continuous.** Replace Previous/Next/page numbers with near-boundary continuation and an inline accessible Load more/Retry fallback. Reuse opaque cursors and query keys; single-flight and deduplicate reads. Accumulate/reconcile by canonical row ID, preserving selected detail and scroll anchors. Bound payload/media/cursor retention, virtualize with overscan, and record measured caps. Preserve current response/display limits unless separately changed and verified. Stop on end, repeated cursor, no progress, error or bound; distinguish each outcome. Pause failed auto-loads and retry locally. Log following preserves a reader who leaves the live edge; show a quiet unread affordance within the existing header, with no new toolbar. Never widen workstation authorization to obtain more data.

## Acceptance and validation

| Phase | Completion evidence |
| --- | --- |
| Documentation | Approved references and source symbols mapped; canonical links and scope reviewed. Runtime behavior remains pending. |
| Implementation | Shared recipes/state/continuous windows and finite splash implemented; unrelated state/authentication behavior preserved. |
| Focused checks | Controlled ordering: initial/empty, repeated refresh, failed retry, out-of-order scope/query/generation responses, duplicate/cyclic cursor, memory bound, scroll/focus retention and actual completion. |
| Native/browser review | Temporal sequences and approved compositions inspected at compact/wide sizes, 1×/2× and motion preferences. Static image generation is not visual approval. |

Use [native verification](../../docs/wiki/code/native-verification.md#matte-canvas-target-review) and existing scoped tests (`log_tests.rs`, `jobs_query_tests.rs`, inline view/dashboard cases; web Skeleton/LoadingState/Bulletin/Brief suites). After implementation and within authorized verification scope, run `cargo fmt --all -- --check`, focused `python3 scripts/agent_cargo.py test --locked -p fleetworks-desktop -- --test-threads=1`, and affected `pnpm test` files; follow required repository gates. Register/select actual native scenes before running the existing capture helper. Record runtime timing, memory, visual inspection and installed-app evidence separately.

Execution: verify baseline and symbols; inspect relevant changes if HEAD moved. Implement dependency-first, fix demonstrated failures and rerun affected checks. Report changed files, actual results and blockers. Current evidence is source/documentation inspection only; no product checks were run for this documentation update.
