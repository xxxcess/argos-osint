---
name: argos-implement
description: Implement one assigned phase directly and return verification needs
---

Use the canonical [agent workflow](../../../docs/wiki/code/agent-workflow.md) with the active host's roles and existing authorization. OpenCode-only restrictions apply when that profile is explicitly selected.

When an OpenCode stack role is selected, follow its assigned `.opencode/agents/rust-coder.md` or `.opencode/agents/react-coder.md` and `docs/wiki/code/opencode-v2.md`. Rust work loads `argos-rust-style`. Investigate and implement the authorized unit directly. Delegate only when the active host/profile permits it and ownership is disjoint. Integrate/format one unit, smoke-test the real affected contract through the build owner, and return incremental evidence plus documentation and final verification needs to the task owner. Include affected wiki/tags/diagrams; report completed checklist items promptly. Request architecture input only for material unresolved contracts. Honor user overrides/exclusions. Failed delegation is not permission to bypass roles.

## Rust mutation execution restriction

Until further notice, agents must not run Rust mutation tests or mutation baselines locally, including `cargo mutants` and cargo-mutants runner invocations. Use the GitHub Actions `mutation.yml` workflow for execution and inspect its hosted results. Local mutation execution is deliberately skipped, not passed; ordinary targeted checks and the embedded gate remain required.

## Build cache hygiene

Follow [cache retention and disk budgets](../../../docs/wiki/code/contributing.md#build-cache-hygiene) before builds, between large checks and at handoff. The build owner proactively removes verified obsolete inactive artifacts, preserves the selected reusable cache and executable, and records free space. Roles without build ownership report cleanup needs to the owner; do not bypass role permissions or use blanket/global cache cleanup.
