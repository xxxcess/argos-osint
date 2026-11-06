# Antigravity teamwork

Use this guide only for Antigravity `/teamwork`; follow the canonical [agent workflow](../docs/wiki/code/agent-workflow.md) for execution and evidence. Do not import OpenCode-only orchestrator or editor restrictions into this host. The host's current model catalog and configured role assignments are authoritative; do not copy model IDs from another host or silently substitute unavailable models.

## Choose the smallest useful team

Use [lite](lite-team.md) for bounded changes with known ownership. Use [heavy](heavy-team.md) for independent implementation areas or cross-cutting contracts. File count alone does not justify another delegation layer. Reuse explicit customer authorization; ask only for missing decisions that prevent safe progress.

## Shared contracts

- Start at [the wiki](../docs/index.md), then [contributing](../docs/wiki/code/contributing.md) and [testing](../.opencode/skills/argos-testing/SKILL.md).
- One owner maintains `.planning/<date-slug>/task_plan.md`, findings and progress. These are local working memory; only curated design/decision/acceptance/handoff documents are eligible for Git. Follow [retention](../docs/wiki/code/planning-artifacts.md).
- Temporary artifacts belong in `.agent-scratch/`. Cargo artifacts are the deliberate exception: `python3 scripts/agent_cargo.py` owns `target/agents`. Never override it or create per-worker build caches.
- Follow [build cache hygiene](../docs/wiki/code/contributing.md#build-cache-hygiene): the build owner checks disk budgets before/between builds and at handoff, removes verified obsolete inactive agent artifacts, and retains the selected binary and reusable cache. Never wipe active or global caches.
- One build owner serializes Cargo in the integration tree, keeps the selected feature graph/profile stable and sets jobs from measured RAM/pressure. Worktrees are edit-only. Run a real-contract fixture smoke before long suites; docs-only work needs document/policy checks, not a full Rust build.
- Agents must not run Rust mutation tests or baselines locally until further notice. Execution belongs to GitHub Actions `mutation.yml`; report absent hosted evidence honestly.
- Preserve unrelated work, locks, servers, retained reusable caches and unresolved evidence; reclaim verified obsolete build caches under the linked hygiene policy. Remove only clean, integrated, task-owned worktrees; never force cleanup to make status look clean.
- Keep existing source conventions, React Compiler, accessible 3D topology/list twin, offline fixtures and host-native embedded constraints. Hosted runners own server/non-host verification.

## Continuation and workflow improvements

Inspect the host's agent status before waiting for more work. A message to a completed agent does not necessarily restart it; use the explicit supported follow-up/resume/wake operation, then verify running status. Preserve the existing task/evidence when resuming.

Suggest concrete workflow improvements with observed evidence and tradeoffs. Implement reversible process changes already authorized within owned paths without an approval pause; route changes outside the active role or authorization to the owner. Use early visual previews and incremental contract evidence under the canonical workflow; final Rust acceptance still needs the required native embedded gate. No arbitrary gate-attempt cap converts missing evidence into a pass.

## Handoff

Every assignment states task/phase, integration root and base revision, owned paths, read-only references, contract, acceptance checks, and existing evidence. Return changed paths, command/exit/test counts, skips, artifact locations, blockers and next action. A queued command or a generated screenshot is not verification. After interruption, inspect existing process/log/artifact state before starting another build.
