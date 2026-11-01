---
name: argos-implement
description: Implement one assigned phase directly and return verification needs
---

Follow the assigned `.opencode/agents/rust-coder.md` or `.opencode/agents/react-coder.md` and `docs/wiki/code/opencode-v2.md`. Rust work loads `argos-rust-style`. Investigate and implement assigned work directly; do not spawn children. Integrate/format one phase, then return documentation and verification needs to the parent for Scribe/tester dispatch. Include affected wiki/tags/diagrams; report completed checklist items promptly. Relay evidence to architect for corrections/acceptance. Honor user overrides/exclusions. Failed delegation is not permission to bypass roles.

## Rust mutation execution restriction

Until further notice, agents must not run Rust mutation tests or mutation baselines locally, including `cargo mutants` and cargo-mutants runner invocations. Use the GitHub Actions `mutation.yml` workflow for execution and inspect its hosted results. Local mutation execution is deliberately skipped, not passed; ordinary targeted checks and the embedded gate remain required.
