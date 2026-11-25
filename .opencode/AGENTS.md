# Fleetworks OpenCode quick guide

Follow root [AGENTS.md](../AGENTS.md) and the [canonical agent workflow](../docs/wiki/code/agent-workflow.md). User instructions take precedence. These boundaries apply only to the selected OpenCode team profile.

## Agent operation chain

Pick the smallest team: direct specialist for cohesive work, `architect` for unresolved contracts, `orchestrator` for independent parallel areas. Ask only for material unknowns; reuse existing authorization.

| Role | Ownership |
| --- | --- |
| `coordinator` | User intake, `.planning/` intake, specialist dispatch. No source edits. |
| `orchestrator` | Shared `.planning/` only; owns disjoint units and delegates code, docs, test, review, audit. |
| `architect` | Direct investigation/consultation. No source edits or children. |
| `rust-coder`, `react-coder` | Owned code/fixtures. No children. Return review, docs, and verification needs. |
| `tester` | Authorized checks and integration build cache. |
| `reviewer`, `auditor` | Diff review (`APPROVE`/`REJECT`) and forensic integrity (`CLEAN`/`INTEGRITY_VIOLATION` veto). |
| `scribe` | Affected docs, tags, and diagram pairs before final verification. |
| `utility` | Quick non-code edits. No source/test/CI edits or children. |

Parents own shared plans, disjoint paths, and one build owner; shell access respects the same edit boundaries. Pass acceptance, non-goals, and verification with each handoff. Follow the [role catalog](../docs/wiki/code/opencode-v2.md) for sessions, permissions, and models. On provider exhaustion, preserve completed work and escalate to `coordinator`; never silently substitute models.

**Final code gate:** `reviewer: APPROVE`, `tester: PASS`, `auditor: CLEAN`. Review risks early, reuse unaffected evidence, freeze before final checks, and reopen only for demonstrated acceptance blockers. On wrap-up, stop new work and report gaps.

## Quick reference

- **Visual authority:** [Glass Canvas guide](../docs/design/matte-canvas/README.md), native control allowlist, and shared React guidance (opaque fallbacks, no pager chrome or resting animation wake loops).
- **Build & CI:** Nightly GSD runs supported checks/builds/captures in [GitHub Actions](../.agents/skills/gsd/SKILL.md#github-ci-first). Follow [contributing](../docs/wiki/code/contributing.md), [cache hygiene](../docs/wiki/code/contributing.md#build-cache-hygiene), and affected [Fleetworks scopes](../docs/wiki/code/ci.md#fleetworksyml--scoped-branch-checks). Local Rust uses host-native embedded storage; dispatch `embedded.yml` and `mutation.yml` in CI (no local mutation runs).
- **Testing & UI:** Load [argos-testing](skills/argos-testing/SKILL.md). Native Rune uses [fleetworks-rune-verify](skills/fleetworks-rune-verify/SKILL.md) and [native verification](../docs/wiki/code/native-verification.md); terminal TUI is retired. Preserve Rust style, React Compiler/Hooks, shared primitives, and `ForceCanvas` + list twin. Inspect affected figures/fixtures; native views own neither backend lifecycle nor topology.
- **Plan & wiki:** Use the [named plan format](../docs/wiki/code/planning-artifacts.md#working-format) (checks + Next Step), keep scratch/worktrees in `.agent-scratch/`, and commit only with authorization and exact staging. Navigate via [wiki](../docs/index.md), [tags](../docs/wiki/tags.md), and [maintenance](../docs/wiki/maintenance.md).
