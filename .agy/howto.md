# Antigravity teamwork

For Antigravity `/teamwork` only. The [canonical workflow](../docs/wiki/code/agent-workflow.md) ([scope and finish line](../docs/wiki/code/agent-workflow.md#scope-and-finish-line)) governs execution, scope freeze, and evidence. Do not import OpenCode-only restrictions; use this host's configured roles/models without copying another host's IDs or silently substituting unavailable models.

Pick [lite](lite-team.md) for bounded changes or [heavy](heavy-team.md) for independent areas/cross-cutting contracts. Delegate early only when useful, with disjoint ownership and one build owner.

## Host contracts

- **Orient & plans:** Start at [the wiki](../docs/index.md), [contributing](../docs/wiki/code/contributing.md), and [testing](../.opencode/skills/argos-testing/SKILL.md). Preserve unrelated work, locks, and services; `.github` is Code Architect-owned. One owner maintains `.planning/<date-slug>/` per [planning retention](../docs/wiki/code/planning-artifacts.md); keep scratch in `.agent-scratch/`.
- **Builds & loops:** Worktrees are edit-only; one build owner runs `python3 scripts/agent_cargo.py` (`target/agents`), follows [cache hygiene](../docs/wiki/code/contributing.md#build-cache-hygiene), and uses the narrowest [development loop](../docs/wiki/code/contributing.md#development-loops-local-cli-and-docker) (host native first, then at most one authorized Docker/web-dev loop with dedicated workspace, `COMPOSE_PROJECT_NAME`, and ports—never touching the user's stack).
- **Verification:** Honor approved scope: scoped Fleetworks checks for clients/services, native embedded gate for compatibility paths, hosted CI for server/platform matrices and Rust mutation. Unrun checks stay unverified.
- **UI:** Preserve React Compiler, accessible topology/list twin, and offline fixtures. Rune work follows [fleetworks-rune-verify](../.opencode/skills/fleetworks-rune-verify/SKILL.md) and [native verification](../docs/wiki/code/native-verification.md) (same-view Runtime/input, inspected PNG/tree, API-only native client; terminal evidence is historical).

## Handoff

Include scope/non-goals, integration root/base, owned paths, refs, acceptance, and existing evidence. Return changed paths, commands/exits/counts, skips, artifacts, and blockers. Before starting another build on continuation, inspect running processes/logs and explicitly wake idle workers. Commits/pushes require customer authorization.
