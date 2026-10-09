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
