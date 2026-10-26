# Shared React packages

Follow root [AGENTS.md](../AGENTS.md) and [React UI rules](../docs/wiki/code/react-ui.md). Load `argos-react-ui` for visual/interaction work and `argos-react-patterns` for components, hooks, state or performance. `ui` owns shared tokens/primitives, `intel` owns screens, and `api-client` owns Query/connection state. Use React Native primitives and keep React Compiler/Hooks diagnostics enabled. Keep renders pure, hooks unconditional and dependencies complete; fix diagnostics rather than suppressing them. Browser persistence/UUIDs belong in app adapters.
