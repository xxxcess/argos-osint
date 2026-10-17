# Topology scope summary screenshots (phase 3)

Real `TestBackend` cell dumps rendered by `scripts/render_tui_cells.py`.
Regenerate with `ARGOS_SCREEN_DIR=<dir> python3 scripts/agent_cargo.py test --locked -p argos-osint-bin dump_topology_summary_screens`.

Sizes are topology content cells (160x50, 120x40, 100x32, 80x24, 60x18); the app chrome adds two rows to the terminal.

- `before/` Graph page on main (two-row findings strip).
- `after/summary-<state>-<size>.png` Graph page with the panel (Desktop/Compact) or the one-line headline (Stacked).
- `after/page-<state>-<size>.png` Summary page (80x24 and 60x18 only).
- States: `idle` (deterministic), `writing-first`, `writing-stale`, `ready`, `stale`, `failed`, `unavailable`, `filtered`, `isolated`, `bounded`, `long-top`, `long-scrolled`.
