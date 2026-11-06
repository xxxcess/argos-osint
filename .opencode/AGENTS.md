# Argos — OpenCode Quick Guide

Follow root [AGENTS.md](../AGENTS.md) and the canonical [agent workflow](../docs/wiki/code/agent-workflow.md); user instructions take precedence. Preserve unrelated changes.

## Host and profile scope

The role/edit/model restrictions below apply when the configured OpenCode team profile is explicitly in use. Other hosts keep their own active role, tool and delegation rules; reading an OpenCode skill does not turn a Codex or Antigravity agent into an OpenCode orchestrator. Use the host's actual catalog and configured model assignments.

Follow the canonical [scope and finish line](../docs/wiki/code/agent-workflow.md#scope-and-finish-line): agree outcome/non-goals/acceptance, review risk early, freeze after required changes, limit late work to demonstrated in-scope blockers and defer independent improvements. Wrap-up stops new work with an honest handoff; no new approval gate.

## Agent Operation Chain

**coordinator / orchestrator → appropriate specialist**; the owner delegates implementation, docs and final verification within this profile. Consult the architect for unresolved material contracts or architecture decisions. A clear user request, grounded existing contract or completed consultation is sufficient to implement; a new architecture interview or formal spec is not a routine prerequisite.

Choose the smallest team; existing authorization bypasses repeat intake:

| Agent | Role | Key Constraints |
|-------|------|-----------------|
| **coordinator** | Decompose, provide context, select team, record constraints | No source edits; owns `.planning/` intake |
| **orchestrator** | Split units, select tier, hand off to architects | Only `.planning/` writes; delegate implementation, docs, tests/builds, substantive review and forensic audit |
| **architect** | Investigate scope, gaps, perf/security | No children or source edits; return decisions |
| **rust-coder** / **react-coder** | Implement; return contract, docs and verification needs | No children; escalate unresolved material contracts; parent owns final gate |
| **tester** | Host-native embedded checks first (`--storage embedded`) | No full platform CI locally |
| **auditor** | Binary veto (`INTEGRITY_VIOLATION` blocks completion) | Forensic integrity, no facade code |
| **reviewer** | Standards review | `APPROVE` required |
| **scribe** | Docs/wiki as contracts stabilize; before final delivery | Keeps tags/contracts/diagrams current |
| **utility** | Quick non-code edits (docs, config, instructions) | No source/tests/CI edits; no children |

Orchestrators own plans and acceptance, delegating implementation, docs and verification within the table above. Architects and coders remain leaf roles. Shell access must not bypass role boundaries. Use independent delegation early when useful; keep one build owner and follow the canonical workflow.

### Capacity and recovery

Follow [provider limits and continuation](../docs/wiki/code/opencode-v2.md#task-wide-provider-concurrency-guidance). Disjoint ownership; reuse evidence; one tester owns builds/logs per integration tree. Models come from JSON config.

**Usage-limit escalation:** when any dispatched agent (architect, coders, reviewer, tester, auditor, scribe, utility) cannot respond due to provider usage/limit exhaustion, the team must not change that agent's configured model target on its own. Preserve the unit's completed work per the repo recovery rules, stop the unit cleanly, and escalate to the coordinator so the customer decides how to proceed. No implicit fallback to another provider or model ID.

Use the canonical workflow for explicit worker wakeups, continuation and evidence reuse.

**Final code gate for this profile:** `reviewer: APPROVE + tester: PASS + auditor: CLEAN`. Build evidence follows the canonical workflow; no arbitrary gate-attempt cap waives required verification.

---

## Team selection (`.opencode/opencode.json`)

| Tier | Scope | Team |
|------|-------|------|
| **Tier 1** | One cohesive, well-understood unit | Direct specialist; quick non-code edit → `utility` |
| **Tier 2** | A bounded unit with a material contract question | Focused architect consult when needed → stack specialist; docs/review/verification owned separately |
| **Tier 3** | Independent areas or cross-cutting integration contracts | Orchestrator assigns disjoint specialists and one build owner; worktrees only when useful |

---

## Quick Reference

- **Navigate:** [wiki](../docs/index.md) → concepts/code → source; use [tags](../docs/wiki/tags.md) and scoped search
- **Disk/cache:** The build owner follows [cache hygiene](../docs/wiki/code/contributing.md#build-cache-hygiene); other roles return cleanup needs to that owner.
- **CI:** Inspect [contracts](../docs/wiki/code/ci.md) and affected `.github/workflows/`; stabilize host-native embedded build first; monitor Actions for other platforms
- **Plan:** Pin one named task; use [planning format](../docs/wiki/code/planning-artifacts.md#working-format); update phase checkboxes with evidence and Next Step
- **Test:** Load [Argos testing guidelines](skills/argos-testing/SKILL.md) for Rust/React test rules
- **Rust:** Load `argos-rust-style` for `crates/`; follow async conventions (Tokio); ignore Chromium/Fuchsia bans
- **React UI:** Load `argos-react-ui` + `argos-react-patterns`; follow [UI rules](../docs/wiki/code/react-ui.md) and [look & feel](../docs/wiki/code/react-ui.md#argos-look--feel-reference); keep React Compiler/Hooks lint enabled
- **Maintain:** [Setup/refresh wiki](../docs/wiki/maintenance.md) updates routes, tags, contracts, diagrams in the same change; scratch in `.agent-scratch/`; commits require authorization and exact staging

---

## Design contract

Follow the root design contract and [React UI guide](../docs/wiki/code/react-ui.md): terminal desk/HUD tokens, explicit density, accessible focus/selection and `ForceCanvas` (`react-force-graph-3d`) with list twin. TUI retains simpler chrome. Inspect affected visual fixtures before approval.

Use the canonical workflow and [TUI verification](../docs/wiki/code/tui-verification.md) for early previews, inspected images and provenance. Final visual approval requires the applicable passing gate; accessibility, functionality and integrity failures remain open.

## Rust mutation execution restriction

Until further notice, run Rust mutations and baselines only on GitHub Actions (`mutation.yml`), never locally. Report local mutation execution as intentionally skipped; ordinary targeted checks and required embedded gates still apply.
