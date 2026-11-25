---
name: argos-react-ui
description: Create, update or review Argos React Native Web UI, layout, styling and accessibility in apps/web-desktop, packages/ui and packages/intel. Excludes the Rust Rune app.
---

# Argos React UI

Follow [React UI guide](../../../docs/wiki/code/react-ui.md) (design baseline/provenance), [Intel Web](../../../docs/wiki/code/intel-web.md), and [agent workflow](../../../docs/wiki/code/agent-workflow.md). Preserve Intel Web routes, composition, typography, and product scope.

- **Design:** Pick the nearest shared primitive and apply [Glass Canvas](../../../docs/design/matte-canvas/theme.md) (`#060709` backdrop, layered smoked/frosted glass, static liquid-metal rims, opaque reduced-transparency fallbacks, molten-gold/cyan accents, fractured crescent textures on committing/brand actions) and the [interaction-state contract](../../../docs/design/matte-canvas/interaction-state.md). Retain resolved rows/scroll on refresh; use localized pending/retry, reduced-motion alternatives, and opaque cursors for continuous lists.
- **Implement:** Use React Native primitives and `@argos/ui`; keep screens in `@argos/intel` and browser adapters in the app. Load `argos-react-patterns` for APIs, state, or perf; keep Compiler/Hooks lint enabled; use [ForceCanvas + accessible list twin](../../../docs/wiki/code/react-ui.md#canvas-visualizations) for topology.
- **Verify & review:** Cover wide/narrow viewports, keyboard/focus, a11y, and loading/empty/error states via repo Playwright fixtures (`ARGOS_WEB_TEST_DEV=1` isolates local output/locks). In [Nightly GSD](../../../docs/wiki/code/ci.md#nightly-gsd-order), run Web builds/checks/captures in GitHub Actions during pass two after pass-one CI is green; use [dispatchable visual review](../../../docs/wiki/code/native-verification.md#dispatchable-ui-visual-review) for targeted same-SHA captures. Inspect captures before visual sign-off and report `file:line` findings on the PR.
