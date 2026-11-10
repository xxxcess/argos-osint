# Fleetworks OpenCode quick guide

Follow root [AGENTS.md](../AGENTS.md) and the [canonical agent workflow](../docs/wiki/code/agent-workflow.md). User instructions take precedence. These role, model and edit boundaries apply to the selected OpenCode team profile. Other hosts use their own roles and tools.

## Agent operation chain

Choose the smallest team. A clear request or existing contract permits work. Ask only about material unknowns and reuse existing authorization. Use a direct specialist for a cohesive task, an architect for unresolved contracts, or an orchestrator for independent work.

| Role | Ownership |
| --- | --- |
| Coordinator | User intake and specialist dispatch. Owns `.planning/` intake. No source edits. |
| Orchestrator | Writes shared plans only. Owns disjoint units and acceptance. Delegates implementation, documentation, verification, review and audit. |
| Architect | Direct investigation and consultation. No source edits or children. |
| Rust and React coders | Implement owned code and fixtures. No children. Return review, docs and verification needs. |
| Tester | Owns authorized checks and the integration build cache. |
| Reviewer and auditor | Review and integrity. Audit has a binary veto. |
| Scribe | Maintains affected docs, tags and diagram pairs before final verification. |
| Utility | Quick non-code edits. No source, test or CI edits and no children. |

The parent owns shared plans, assigns disjoint paths and appoints one build owner. Shell access follows the same edit boundaries. Pass acceptance, non-goals and verification with each handoff. Follow the [role catalog and continuation rules](../docs/wiki/code/opencode-v2.md) for child sessions, permissions and models. On provider exhaustion preserve completed work and escalate to the coordinator. Model substitution requires user direction.

**Final code gate** requires `reviewer: APPROVE`, `tester: PASS` and `auditor: CLEAN`. Review risks early, reuse unaffected evidence and freeze before final checks. Reopen only for demonstrated blockers to agreed acceptance. On wrap-up, stop new work and report gaps.

## Quick reference

- **Build and CI:** Follow [contributor commands](../docs/wiki/code/contributing.md) and [cache hygiene](../docs/wiki/code/contributing.md#build-cache-hygiene). Use affected [Fleetworks scopes](../docs/wiki/code/ci.md#fleetworksyml--scoped-branch-checks) and the embedded gate for affected compatibility paths. Local Rust uses the host and embedded storage. Dispatch `embedded.yml` for hosted platform compatibility and `mutation.yml` for mutation evidence; do not run mutation locally.
- **Testing and design:** Load [repository testing guidelines](skills/argos-testing/SKILL.md) and the applicable Rust, React and TUI skills. Preserve Compiler and Hooks checks, shared UI primitives and `ForceCanvas` with its list twin. Inspect actual affected figures and [visual fixtures](../docs/wiki/code/tui-verification.md).
- **Plan and handoff:** Use the [named planning format](../docs/wiki/code/planning-artifacts.md#working-format). Record checks and Next Step. Scratch and worktrees belong in `.agent-scratch/`. Commit only with authorization and exact staging.
- **Navigate and maintain:** Start at the [wiki](../docs/index.md) and [tags](../docs/wiki/tags.md). Use [wiki maintenance](../docs/wiki/maintenance.md) for affected routes, contracts and figure pairs.
