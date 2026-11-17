---
name: argos-react-ui
description: Create, update or review Argos React Native Web UI, layout, styling and accessibility in apps/web-desktop, packages/ui and packages/intel. Excludes the Rust Rune app.
---

Use the canonical [agent workflow](../../../docs/wiki/code/agent-workflow.md) with the active host's roles and existing authorization. OpenCode-only restrictions apply when that profile is explicitly selected.

# Argos React UI

Read `docs/wiki/code/react-ui.md` for the audited web composition and evidence workflow; follow `docs/wiki/code/intel-web.md` for product semantics. The approved [matte-canvas theme](../../../docs/design/matte-canvas/theme.md) and [interaction-state contract](../../../docs/design/matte-canvas/interaction-state.md) own future paint/loading/scroll targets; mockups are not runtime evidence. Paths are relative to the repository root.

- Before editing, run the design process in `docs/wiki/code/react-ui.md#argos-look--feel-reference`: screen type, density (default or compact), nearest primitive, then type, space, semantic accent, component treatment and states. Preserve existing web composition, typography, routes and topology/list behavior; share paint and generic interaction contracts without importing native setup/service workflows.
- Use symbol-only branding, cool-gray text and smooth matte surfaces that blend into the canvas. Only the logo retains permanent metallic sheen; coarse matte crescent-rock texture is restricted to Submit/Confirm/Create/Delete actions (Connect qualifies). Utility/navigation controls stay smooth. The canonical theme owns exact treatments and semantic colors; do not restore primary ivory/gold accents.
- Keep headings and resolved content steady. Pending regions use layout-matched, constant-base skeletons with localized cool translated shimmer and static reduced-motion alternatives. Long data lists use continuous near-end loading, stable rows/scroll anchors and local tail error/Retry, without Prev/Next or page-number chrome; preserve opaque cursor and request-identity contracts.
- Use React Native primitives and keep the Next React Compiler pipeline and Hooks/compiler lint enabled; fix diagnostics and verify compiled-build behavior.
- Reuse `@argos/ui`; keep feature composition in `@argos/intel` and browser-only behavior in the web app. For component APIs, remote state or render performance also load `argos-react-patterns`.
- Cover wide/narrow layouts, long content and relevant loading/empty/error/offline states. Preserve source attribution, evidence distinctions, image fallback and ready-gated actions.
- Verify keyboard/focus and accessible control names/states. Run appropriate frontend checks and fixture E2E; capture affected states at proportional viewports and inspect the PNGs before reporting visual approval.
- For an open PR with web UI changes, dispatch the [UI visual review workflow](../../../.github/workflows/ui-visual-review.yml) at the frozen exact head with `scope=web` and the affected `web_surface`. Inspect the generated Playwright captures; attach selected images inline to the PR and record a durable reference in affected docs or an exact-head release. Follow the [dispatch recipe](../../../docs/wiki/code/native-verification.md#dispatchable-ui-visual-review).
- For reviews, inspect the requested scope (or current UI delta if unspecified) and return concrete `file:line` findings with impact/correction. Report checks and unverified visual evidence separately. Do not turn a review request into unrelated edits.

This skill adapts selected Vercel ideas; provenance and exclusions are in the design baseline. No upstream fetch or dependency installation is required to use it.
- Topology graphs use the force-graph canvas (`ForceCanvas` / `react-force-graph-3d`) with an accessible list twin and tokens passed into draw code; load it lazily client-only and honor reduced motion (see react-ui.md#canvas-visualizations). Do not add another graph renderer.

