---
name: argos-react-ui
description: Create, update or review Argos React Native Web UI, layout, styling and accessibility in apps/web-desktop, packages/ui and packages/intel. Excludes Rust terminal UI.
---

# Argos React UI

Read `docs/wiki/code/react-ui.md` for the design baseline and evidence workflow; follow `docs/wiki/code/intel-web.md` for product semantics. Paths are relative to the repository root.

- Before editing, name the screen type and density intent (default or compact), then the nearest reusable primitive, layout constraints and acceptance. Apply the `theme.rs` palette with the HUD/glass treatment (tokens in `packages/ui`, see react-ui.md) while preserving layout semantics and product contracts; apply explicit user design direction within the product contracts. Follow `docs/wiki/code/react-ui.md#argos-look--feel-reference`.
- Use React Native primitives and keep the Next React Compiler pipeline and Hooks/compiler lint enabled; fix diagnostics and verify compiled-build behavior.
- Reuse `@argos/ui`; keep feature composition in `@argos/intel` and browser-only behavior in the web app. For component APIs, remote state or render performance also load `argos-react-patterns`.
- Cover wide/narrow layouts, long content and relevant loading/empty/error/offline states. Preserve source attribution, evidence distinctions, image fallback and ready-gated actions.
- Verify keyboard/focus and accessible control names/states. Run appropriate frontend checks and fixture E2E; capture affected states at proportional viewports and inspect the PNGs before reporting visual approval.
- For reviews, inspect the requested scope (or current UI delta if unspecified) and return concrete `file:line` findings with impact/correction. Report checks and unverified visual evidence separately. Do not turn a review request into unrelated edits.

This skill adapts selected Vercel ideas; provenance and exclusions are in the design baseline. No upstream fetch or dependency installation is required to use it.
- Topology graphs use the force-graph canvas (`ForceCanvas` / `react-force-graph-2d`) with an accessible list twin and tokens passed into draw code; load it lazily client-only and honor reduced motion (see react-ui.md#canvas-visualizations). Do not add another graph renderer.
