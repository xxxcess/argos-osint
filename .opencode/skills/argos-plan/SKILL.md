---
name: argos-plan
description: Start or resume architect planning with planning-with-files
---

Use the canonical [agent workflow](../../../docs/wiki/code/agent-workflow.md) with the active host's roles and existing authorization. OpenCode-only restrictions apply when that profile is explicitly selected.

Follow `.opencode/agents/architect.md` when that OpenCode planning role is selected; otherwise use the active host role. Rust phases load `argos-rust-style`. Load planning-with-files (fallback: `~/.agents/skills/planning-with-files/SKILL.md`); pin one named PLAN_ID. Recover current planning/evidence before rediscovery. Audit wiki/current source, write phases with actionable checkbox items and acceptance, and maintain shared plans under `docs/wiki/code/planning-artifacts.md`. Update item checkboxes as work completes, plus evidence and Next Step. Return implementation needs to the task owner; delegate only as the active host/profile permits; plan-only requests stop before implementation.

## Build cache hygiene

Follow [cache retention and disk budgets](../../../docs/wiki/code/contributing.md#build-cache-hygiene) before builds, between large checks and at handoff. The build owner proactively removes verified obsolete inactive artifacts, preserves the selected reusable cache and executable, and records free space. Roles without build ownership report cleanup needs to the owner; do not bypass role permissions or use blanket/global cache cleanup.
