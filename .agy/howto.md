# Antigravity teamwork

Use this guide only for Antigravity `/teamwork`; the [canonical agent workflow](../docs/wiki/code/agent-workflow.md) owns execution, scope control and evidence. Do not import OpenCode-only role restrictions. Use this host's configured roles/models; never copy another host's model IDs or silently substitute unavailable models.

## Scope and team

Follow the canonical [scope and finish line](../docs/wiki/code/agent-workflow.md#scope-and-finish-line): agree outcome/non-goals/acceptance, review risk early, freeze after required changes and limit late work to demonstrated in-scope blockers. Defer independent improvements; wrap-up stops new work with an honest handoff. Existing authorization remains valid; no extra approval gate.

Use [lite](lite-team.md) for bounded changes and [heavy](heavy-team.md) for independent areas or cross-cutting contracts. Delegate early only when useful, with disjoint ownership and one build owner; follow the canonical workflow for worker wakeups, resources and evidence reuse.

## Host-specific contracts

- Start at [the wiki](../docs/index.md), then [contributing](../docs/wiki/code/contributing.md) and [testing](../.opencode/skills/argos-testing/SKILL.md). Preserve unrelated work, locks and services; `.github` changes remain Code Architect-owned.
- One owner maintains `.planning/<date-slug>/` under the [planning/retention contract](../docs/wiki/code/planning-artifacts.md). Scratch belongs in `.agent-scratch/`; Cargo uses `python3 scripts/agent_cargo.py` and its shared `target/agents` cache. Worktrees are edit-only; the build owner follows [cache hygiene](../docs/wiki/code/contributing.md#build-cache-hygiene).
- Respect the user's approved verification scope. Rust runs target the native host with embedded storage; the required final embedded gate remains required for applicable Rust changes. Server/all-platform and mutation execution belong to hosted CI, never local agents. Missing evidence stays unverified.
- Keep existing source/UI conventions, React Compiler, accessible topology/list twin and offline fixtures. Use canonical instructions for visual previews and final evidence.

## Handoff

Assignments include scope/non-goals, integration root/base, owned paths, references, acceptance and existing evidence. Return changed paths, actual checks/exits/counts, skips, artifacts and blockers. On continuation, inspect existing processes/logs before starting another build; use the host's explicit wake/resume operation for idle workers. Commits/pushes require the customer's authorization.
