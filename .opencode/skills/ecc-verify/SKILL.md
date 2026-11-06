---
name: ecc-verify
description: Verify Argos changes in the repository CI order.
---

Use the canonical [agent workflow](../../../docs/wiki/code/agent-workflow.md) with the active host's roles and existing authorization. OpenCode-only restrictions apply when that profile is explicitly selected.

Follow `.opencode/agents/tester.md` when the OpenCode tester profile is selected; other hosts retain their actual verification role. Select checks from the affected contracts first. Documentation/config/scripts-only tasks use their relevant policy or harness tests and do not trigger a Rust rebuild. For Rust changes requiring broad workspace/storage verification, run `scripts/local_matrix.sh --storage embedded` in the combined integration tree with ARGOS_EMBED unset. Feature worktrees edit/format only; server is CI-only. Run a real-contract fixture smoke before long suites, using the stable gate-compatible feature graph and RAM-aware jobs. After demonstrated failures, rerun the failing contract and affected closure before the required final gate; repeat a broad gate only for relevant invalidation. An attempt cap never excuses an unmet gate.

TUI: use argos-tui-verify and `docs/wiki/code/tui-verification.md`; Profile adds `docs/wiki/code/profile-analytics-dashboard.md`. Report actual image/interaction/metric evidence separately. Refresh affected wiki routes/tags/diagrams after code edits.

Config/commit/release changes: run relevant `node --test .opencode/tests/*.test.mjs`, release `versions`, and prepared-message checks. Check affected docs/diagrams against final behavior. Record commands, results and limitations under `docs/wiki/code/planning-artifacts.md`; planned or skipped checks are not passes.

## Rust mutation execution restriction

Until further notice, agents must not run Rust mutation tests or mutation baselines locally, including `cargo mutants` and cargo-mutants runner invocations. Use the GitHub Actions `mutation.yml` workflow for execution and inspect its hosted results. Local mutation execution is deliberately skipped, not passed; ordinary targeted checks and the embedded gate remain required.

## Build cache hygiene

Follow [cache retention and disk budgets](../../../docs/wiki/code/contributing.md#build-cache-hygiene) before builds, between large checks and at handoff. The build owner proactively removes verified obsolete inactive artifacts, preserves the selected reusable cache and executable, and records free space. Roles without build ownership report cleanup needs to the owner; do not bypass role permissions or use blanket/global cache cleanup.
