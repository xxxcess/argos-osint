---
name: argos-react-ui
description: Create, update or review Argos React Native Web UI, layout, styling and accessibility in apps/web-desktop, packages/ui and packages/intel. Excludes the Rust Rune app.
---

# Argos React UI

Use the [React UI guide](../../../docs/wiki/code/react-ui.md), [Intel Web semantics](../../../docs/wiki/code/intel-web.md) and [agent workflow](../../../docs/wiki/code/agent-workflow.md). Preserve Intel Web composition, routes, typography and product scope.

- **Design.** Choose the layout, density and nearest shared primitive. Apply the [Matte Canvas theme](../../../docs/design/matte-canvas/theme.md) and [interaction-state contract](../../../docs/design/matte-canvas/interaction-state.md). Keep resolved rows and scroll position during refresh. Use localized pending and retry states, reduced-motion alternatives and opaque cursors for continuous lists.
- **Implement.** Use React Native primitives and `@argos/ui`. Keep feature composition in `@argos/intel` and browser code in the app. Load `argos-react-patterns` for component APIs, remote state or render performance. Keep Compiler and Hooks lint enabled. Use [ForceCanvas and its accessible list twin](../../../docs/wiki/code/react-ui.md#canvas-visualizations) for topology.
- **Verify.** Cover affected wide/narrow, keyboard/focus, accessibility and loading/empty/error states in the compiled build. Use repo Playwright fixtures. For authorized local browser runs, `ARGOS_WEB_TEST_DEV=1` isolates test output and locks from the user's frontend.
- **Review.** In [Nightly GSD](../../../docs/wiki/code/ci.md#nightly-gsd-order), Web builds and checks run in pass two after exact-head first-pass CI is green. Inspect affected Playwright captures before reporting visual approval. Use [supplemental visual review](../../../docs/wiki/code/native-verification.md#dispatchable-ui-visual-review) for selected captures at the same SHA. Report findings with file and line references and record the evidence on the PR.

The [React UI design baseline](../../../docs/wiki/code/react-ui.md) records upstream design provenance.
