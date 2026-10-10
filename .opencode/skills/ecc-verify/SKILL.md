---
name: ecc-verify
description: Verify Argos changes in the repository CI order.
---
Run `cargo fmt --all --check`, then `python3 scripts/agent_cargo.py clippy --workspace --all-targets --locked --no-default-features -- -D warnings`, then `python3 scripts/agent_cargo.py test --workspace --locked --no-default-features`. Keep `ARGOS_EMBED` unset. Do not pass `--features lancedb`. After code changes, run `graphify update .`. Report each result and changed graph artifacts.

Profile verification follows `docs/profile-analytics-dashboard.md`: all 20 unique primary IDs and per-app counts, scales/denominators/units, unknown/overflow capacity, missing/low-N lines, stable colors, Unicode/no wrapping, independent scroll/focus restoration, mouse parity and System/Configs. Render actual terminal fixtures with `scripts/render_tui_cells.py` at 160×50, 120×40, 100×32, 80×24, 60×18 and too-small/empty/stale. Compare Summary/Recon reference composition when images are available and explicitly report missing references or unverified live behavior. Acceptance instructions are not completed checks.

TUI process and artifact retention: `docs/tui-verification.md` / `argos-tui-verify`. The parent owns capture and image/action/metric verification; editors remain formatting-only.
