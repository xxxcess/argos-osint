# Antigravity teamwork

Use this guide only for Antigravity `/teamwork`. The host's current model catalog and configured role assignments are authoritative; do not copy model IDs from another host or silently substitute unavailable models.

## Choose the smallest useful team

Use [lite](lite-team.md) for bounded changes with known ownership. Use [heavy](heavy-team.md) for independent implementation areas or cross-cutting contracts. File count alone does not justify another delegation layer. Reuse explicit customer authorization; ask only for missing decisions that prevent safe progress.

## Shared contracts

- Start at [the wiki](../docs/index.md), then [contributing](../docs/wiki/code/contributing.md) and [testing](../.opencode/skills/argos-testing/SKILL.md).
- One owner maintains `.planning/<date-slug>/task_plan.md`, findings and progress. These are local working memory; only curated design/decision/acceptance/handoff documents are eligible for Git. Follow [retention](../docs/wiki/code/planning-artifacts.md).
- Temporary artifacts belong in `.agent-scratch/`. Cargo artifacts are the deliberate exception: `python3 scripts/agent_cargo.py` owns `target/agents`. Never override it or create per-worker build caches.
- One tester owns serialized Cargo builds in the integration tree. Worktrees are edit-only. Match verification to changed contracts; docs-only work needs document/policy checks, not a full Rust build.
- Agents must not run Rust mutation tests or baselines locally until further notice. Execution belongs to GitHub Actions `mutation.yml`; report absent hosted evidence honestly.
- Preserve unrelated work, locks, servers, caches and unresolved evidence. Remove only clean, integrated, task-owned worktrees; never force cleanup to make status look clean.
- Keep existing source conventions, React Compiler, accessible 3D topology/list twin, offline fixtures and host-native embedded constraints. Hosted runners own server/non-host verification.

## Handoff

Every assignment states task/phase, integration root and base revision, owned paths, read-only references, contract, acceptance checks, and existing evidence. Return changed paths, command/exit/test counts, skips, artifact locations, blockers and next action. A queued command or a generated screenshot is not verification. After interruption, inspect existing process/log/artifact state before starting another build.
