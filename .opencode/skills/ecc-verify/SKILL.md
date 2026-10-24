---
name: ecc-verify
description: Verify Argos changes in the repository CI order.
---

Follow `.opencode/agents/tester.md`. In the combined integration tree run `scripts/local_matrix.sh --storage embedded` with ARGOS_EMBED unset. Feature worktrees edit/format only; server is CI-only. Respect the full-gate cap (normally 1–2, max 3). After architect-directed fixes, target `python3 scripts/agent_cargo.py check --workspace --all-targets --locked` and failing tests before the final gate; preserve embedded defaults.

TUI: use argos-tui-verify and `docs/tui-verification.md`; Profile adds `docs/profile-analytics-dashboard.md`. Report actual image/interaction/metric evidence separately. Refresh graphify after code edits.

Config/commit/release changes: run relevant `node --test .opencode/tests/*.test.mjs`, release `versions`, and prepared-message checks. Check affected docs/diagrams against final behavior. Record commands, results and limitations under `docs/planning-artifacts.md`; planned or skipped checks are not passes.
