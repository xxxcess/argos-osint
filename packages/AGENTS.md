# Shared React packages

Follow root [AGENTS.md](../AGENTS.md) and [React UI rules](../docs/wiki/code/react-ui.md), including the [Argos look and feel](../docs/wiki/code/react-ui.md#argos-look--feel-reference). Design components creatively inside that theme. Load `argos-react-ui` for visual/interaction work and `argos-react-patterns` for components, hooks, state or performance. `ui` owns shared tokens/primitives, `intel` owns screens, and `api-client` owns Query/connection state. Topology graphs in `intel` use `ForceCanvas` (`react-force-graph-3d`) plus its accessible list twin; keep mutable engine data isolated from Query state. Use React Native primitives and keep React Compiler/Hooks diagnostics enabled. Keep renders pure, hooks unconditional and dependencies complete; fix diagnostics rather than suppressing them. Browser persistence/UUIDs belong in app adapters.

Choose frontend checks from [CI contracts](../docs/wiki/code/ci.md) and `.github/workflows/intel-web.yml`; preserve opaque cursors and distinguish loading, empty collections and valid numeric zero.

## Rust mutation execution restriction

Until further notice, agents must not run Rust mutation tests or mutation baselines locally, including `cargo mutants` and cargo-mutants runner invocations. Use the GitHub Actions `mutation.yml` workflow for execution and inspect its hosted results. Local mutation execution is deliberately skipped, not passed; ordinary targeted checks and the embedded gate remain required.
