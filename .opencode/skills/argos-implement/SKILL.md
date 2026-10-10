---
name: argos-implement
description: On the build agent, edit the current plan phase through parallel ecc-edit subagents, then test in the parent.
---

Run this on the `build` agent. `plan` does not launch `ecc-edit`.

1. Read the in-progress phase in `.planning/<id>/task_plan.md`. Split its files into disjoint sets. One set is one unit. A file that several units need belongs to one unit only.
2. Call `subagent` once per unit. Two or more units go in the same turn with `background` true. One unit stays in the foreground.

```json
{
  "agent": "ecc-edit",
  "description": "Edit binder inputs",
  "prompt": "Phase, the exact file list, the change, and constraints. The child has no other context.",
  "background": true
}
```

Do not set `model`. Do not poll. Do not edit those files in the parent while a child owns them.

3. After the children finish, run the checks in this session with shell `timeout` set to `600000` and `ARGOS_EMBED` unset. Do not enable the `lancedb` feature.

```sh
cargo fmt --all --check
python3 scripts/agent_cargo.py clippy --workspace --all-targets --locked --no-default-features -- -D warnings
python3 scripts/agent_cargo.py test --workspace --locked --no-default-features
```

4. On failure, map each error to the unit that owns the file. Launch `ecc-edit` again the same way. Pass `sessionID` to continue the editor that already owns those files, and put the failing command plus the relevant output in `prompt`.
5. Re-run the failed command, then the full trio. After the trio passes, run `graphify update .`.

For UI phases, load the relevant `docs/tui-components.md` entries and `docs/tui-design-spec.md`. For Profile, also read `docs/profile-analytics-dashboard.md`: preserve exactly 20 primary views (Intel 4 / Recon 5 / Atlas 3 / Models 4 / Tools 4), Summary reuse, theme RGB/tokens and metric/history contracts. Use AnalyticsCard, TimePlot, DetailTable, Meter and ScrollPane through shared `profile_components`; the Dashboard/Report presets share pure `profile_layout::LayoutResult` rectangles for draw/focus/hit testing. Child prompts include chosen components/preset, concise API contracts, exact owned files, read-only references, and visual acceptance at 160×50, 120×40, 100×32, 80×24, 60×18 plus too-small/empty/stale and interaction/data states. Preserve disjoint ownership and report actual checks. Use these existing instruction files; do not invent OpenCode config keys or change model defaults/permissions for this workflow.

TUI process and artifact retention: `docs/tui-verification.md` / `argos-tui-verify`. The parent owns capture and image/action/metric verification; editors remain formatting-only.


TUI overhaul navigation/component contract: read `docs/tui-design-spec.md` and `docs/tui-components.md`, including Shared navigation and portable configuration. Preserve three Profile pages, independent TabBar focus/activation, nested dataset offsets, background image resources, exact-revision Verify/Save and usize transcript scroll. Include `dump_overhaul_screens` alongside Profile captures in the six-size acceptance matrix; generation does not complete visual review.
