# Shared React packages

Follow the shared [agent workflow](../docs/wiki/code/agent-workflow.md) for execution and evidence. Reuse authorized scoped checks, preserve the active host's roles, and suggest evidence-backed reversible process improvements within assigned ownership.

Keep work bounded by the shared [scope and finish line](../docs/wiki/code/agent-workflow.md#scope-and-finish-line).

Follow root [AGENTS.md](../AGENTS.md) and [React UI rules](../docs/wiki/code/react-ui.md), including the [Fleetworks look and feel](../docs/wiki/code/react-ui.md#fleetworks-look-and-feel). Design components creatively inside that theme. Load `argos-react-ui` for visual/interaction work and `argos-react-patterns` for components, hooks, state or performance. `ui` owns shared tokens/primitives, `intel` owns screens, and `api-client` owns Query/connection state. Topology graphs in `intel` use `ForceCanvas` (`react-force-graph-3d`) plus its accessible list twin; keep mutable engine data isolated from Query state. Use React Native primitives and keep React Compiler/Hooks diagnostics enabled. Keep renders pure, hooks unconditional and dependencies complete; fix diagnostics rather than suppressing them. Browser persistence/UUIDs belong in app adapters.

Choose frontend checks from [CI contracts](../docs/wiki/code/ci.md) and `.github/workflows/intel-web.yml`; preserve opaque cursors and distinguish loading, empty collections and valid numeric zero.

Follow [build cache hygiene](../docs/wiki/code/contributing.md#build-cache-hygiene) before builds and at handoff; reclaim verified obsolete artifacts while preserving the current reusable Rust/web cache and running services.

## Rust mutation execution restriction

Until further notice, agents must not run Rust mutation tests or mutation baselines locally, including `cargo mutants` and cargo-mutants runner invocations. Use the GitHub Actions `mutation.yml` workflow for execution and inspect its hosted results. Local mutation execution is deliberately skipped, not passed; ordinary targeted checks and the embedded gate remain required.
