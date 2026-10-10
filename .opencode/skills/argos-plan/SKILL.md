---
name: argos-plan
description: Keep multi-step Argos work in .planning/ so the chat stays short. Use when a task needs five or more tool calls or more than one phase.
---

Write the plan to disk and keep the chat to the current step.

1. Run shell `graphify query "<question>"` before a repo-wide search. Read only paths the query returns.
2. Create `.planning/<YYYY-MM-DD-slug>/` with `task_plan.md`, `findings.md`, and `progress.md`. One owner writes `task_plan.md`.
3. `task_plan.md` holds the goal, phases, decisions, errors, and a single next step. `findings.md` holds research. `progress.md` holds checks.
4. Re-read the next step before starting a phase. Do not paste those files back into the chat.
5. Do not load another planning skill. Phase and milestone state stays in `.planning/`.
6. Code for the current phase is implemented on the `build` agent with the `argos-implement` skill.

UI phases read `docs/tui-design-spec.md` and relevant `docs/tui-components.md` entries. Identify chosen components, layout preset, affected screens, and viewport/interaction/data acceptance checks before editing.

Profile phases also read `docs/profile-analytics-dashboard.md`. Plan exactly 20 primary views (Intel 4 / Recon 5 / Atlas 3 / Models 4 / Tools 4), Summary reuse, simultaneous Dashboard and expanded Report presets, shared `profile_components` and pure `profile_layout::LayoutResult` geometry. Preserve theme tokens/RGB, metric/history contracts, System/Configs and provider scheduler ownership. Include 160×50, 120×40, 100×32, 80×24, 60×18 and too-small/empty/stale acceptance. Existing skills/agent prompts carry this guidance; no OpenCode config key, model default or permission change is required.

TUI process and artifact retention: `docs/tui-verification.md` / `argos-tui-verify`. The parent owns capture and image/action/metric verification; editors remain formatting-only.
