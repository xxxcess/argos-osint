---
name: argos-implement
description: Implement one architect phase through child coders and tester
---

Follow `.opencode/agents/coder.md` and `docs/wiki/code/opencode-v2.md`. Rust EDIT_UNITs load `argos-rust-style`. Delegate disjoint EDIT_UNITs, integrate/format one phase, run Scribe documentation review at the scheduled phase checkpoint, then call tester in the combined tree. Include affected wiki/tags/diagrams; report completed checklist items promptly. Relay evidence to architect for corrections/acceptance. Pass the coder-worker model to editor/tester/CONSULT children; use the configured Scribe role model for Scribe; honor user overrides/exclusions. Failed delegation is not permission to bypass roles.

## Rust mutation execution restriction

Until further notice, agents must not run Rust mutation tests or mutation baselines locally, including `cargo mutants` and cargo-mutants runner invocations. Use the GitHub Actions `mutation.yml` workflow for execution and inspect its hosted results. Local mutation execution is deliberately skipped, not passed; ordinary targeted checks and the embedded gate remain required.
