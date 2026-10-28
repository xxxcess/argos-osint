# Shared React packages

Follow root [AGENTS.md](../AGENTS.md) and [React UI rules](../docs/wiki/code/react-ui.md), including the [Argos look and feel](../docs/wiki/code/react-ui.md#argos-look--feel-reference). Load `argos-react-ui` for visual/interaction work and `argos-react-patterns` for components, hooks, state or performance. `ui` owns shared tokens/primitives, `intel` owns screens, and `api-client` owns Query/connection state. Topology graphs in `intel` use `ForceCanvas` (`react-force-graph-2d`); do not add another graph renderer. Use React Native primitives and keep React Compiler/Hooks diagnostics enabled. Keep renders pure, hooks unconditional and dependencies complete; fix diagnostics rather than suppressing them. Browser persistence/UUIDs belong in app adapters.

Choose frontend checks from [CI contracts](../docs/wiki/code/ci.md) and `.github/workflows/intel-web.yml`; preserve opaque cursors and distinguish loading, empty collections and valid numeric zero.
