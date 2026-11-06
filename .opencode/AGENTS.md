# Argos — OpenCode Quick Guide

Follow root [AGENTS.md](../AGENTS.md) and the canonical [agent workflow](../docs/wiki/code/agent-workflow.md); user instructions take precedence. Preserve unrelated changes.

## Host and profile scope

The role/edit/model restrictions below apply when the configured OpenCode team profile is explicitly in use. Other hosts keep their own active role, tool and delegation rules; reading an OpenCode skill does not turn a Codex or Antigravity agent into an OpenCode orchestrator. Use the host's actual catalog and configured model assignments.

The canonical workflow owns build scheduling, resource budgets, fail-fast fixtures, evidence reuse, visual previews and process improvements. Suggest concrete improvements with observed evidence and tradeoffs; apply reversible improvements already authorized within your edit scope without another approval pause. Route changes outside an explicitly assigned role to the appropriate owner.

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

Orchestrators own plans and delegate edits, tests and substantive reviews. Architects and coders work directly and never spawn children; coders return documentation, consultation and verification needs to the parent. Shell must not bypass role boundaries. The parent owns acceptance and schedules docs as behavior stabilizes. Run authorized focused smoke checks and previews when the relevant contract is runnable; documentation or a three-way milestone ceremony must not delay that feedback.

### Capacity and recovery

Follow [provider limits and continuation](../docs/wiki/code/opencode-v2.md#task-wide-provider-concurrency-guidance). Disjoint ownership; reuse evidence; one tester owns builds/logs per integration tree. Models come from JSON config.

**Usage-limit escalation:** when any dispatched agent (architect, coders, reviewer, tester, auditor, scribe, utility) cannot respond due to provider usage/limit exhaustion, the team must not change that agent's configured model target on its own. Preserve the unit's completed work per the repo recovery rules, stop the unit cleanly, and escalate to the coordinator so the customer decides how to proceed. No implicit fallback to another provider or model ID.

Completed-agent follow-up is host-specific: a plain message may not restart a completed worker. Inspect status, use the host's explicit follow-up/resume/wake operation, and verify that the worker is running before relying on it. Never poll or wait indefinitely for an idle completed child.

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
- **Disk/cache:** Follow [build cache hygiene](../docs/wiki/code/contributing.md#build-cache-hygiene) before builds, between large checks and at handoff. The build owner removes verified obsolete agent artifacts, preserves the selected reusable cache and reports free space; roles without execution ownership return cleanup needs to that owner.
- **CI:** Inspect [contracts](../docs/wiki/code/ci.md) and affected `.github/workflows/`; stabilize host-native embedded build first; monitor Actions for other platforms
- **Plan:** Pin one named task; use [planning format](../docs/wiki/code/planning-artifacts.md#working-format); update phase checkboxes with evidence and Next Step
- **Test:** Load [Argos testing guidelines](skills/argos-testing/SKILL.md) for Rust/React test rules
- **Rust:** Load `argos-rust-style` for `crates/`; follow async conventions (Tokio); ignore Chromium/Fuchsia bans
- **React UI:** Load `argos-react-ui` + `argos-react-patterns`; follow [UI rules](../docs/wiki/code/react-ui.md) and [look & feel](../docs/wiki/code/react-ui.md#argos-look--feel-reference); keep React Compiler/Hooks lint enabled
- **Maintain:** [Setup/refresh wiki](../docs/wiki/maintenance.md) updates routes, tags, contracts, diagrams in the same change; scratch in `.agent-scratch/`; commits require authorization and exact staging

---

## Design contract

Follow the root design contract and [React UI guide](../docs/wiki/code/react-ui.md): terminal desk/HUD tokens, explicit density, accessible focus/selection and `ForceCanvas` (`react-force-graph-3d`) with list twin. TUI retains simpler chrome. Inspect affected visual fixtures before approval.

For UI/TUI work, capture affected fixtures early as source-labelled previews, then inspect actual PNGs. Repeated failures without new evidence require a different diagnostic or implementation approach; record the defect and continue independent work while resolving it. Escalate only a concrete blocker or material user decision. Final visual approval still requires matching source/fixture/renderer provenance and the required passing embedded gate; unmet accessibility, functionality or integrity checks remain open.

## Rust mutation execution restriction

Until further notice, run Rust mutations and baselines only on GitHub Actions (`mutation.yml`), never locally. Report local mutation execution as intentionally skipped; ordinary targeted checks and required embedded gates still apply.
