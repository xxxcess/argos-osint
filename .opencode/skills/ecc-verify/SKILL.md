---
name: ecc-verify
description: Verify Argos changes in the repository CI order.
---

Follow `.opencode/agents/tester.md`. In the combined integration tree run `scripts/local_matrix.sh --storage embedded` with ARGOS_EMBED unset. Feature worktrees edit/format only; server is CI-only. Respect the full-gate cap (normally 1–2, max 3). After architect-directed fixes, target `python3 scripts/agent_cargo.py check --workspace --all-targets --locked` and failing tests before the final gate; preserve embedded defaults.

TUI: use argos-tui-verify and `docs/wiki/code/tui-verification.md`; Profile adds `docs/wiki/code/profile-analytics-dashboard.md`. Report actual image/interaction/metric evidence separately. Refresh affected wiki routes/tags/diagrams after code edits.

Config/commit/release changes: run relevant `node --test .opencode/tests/*.test.mjs`, release `versions`, and prepared-message checks. Check affected docs/diagrams against final behavior. Record commands, results and limitations under `docs/wiki/code/planning-artifacts.md`; planned or skipped checks are not passes.

## Rust mutation execution restriction

Until further notice, agents must not run Rust mutation tests or mutation baselines locally, including `cargo mutants` and cargo-mutants runner invocations. Use the GitHub Actions `mutation.yml` workflow for execution and inspect its hosted results. Local mutation execution is deliberately skipped, not passed; ordinary targeted checks and the embedded gate remain required.
