# Argos — OpenCode Quick Guide

Follow root [AGENTS.md](../AGENTS.md); user instructions take precedence. Preserve unrelated changes.

## Agent Operation Chain

**coordinator / orchestrator → architect or rust-coder/react-coder**; the owner delegates implementation, docs and the verification gate.

Choose the smallest team; existing authorization bypasses repeat intake:

| Agent | Role | Key Constraints |
|-------|------|-----------------|
| **coordinator** | Decompose, provide context, select team, record constraints | No source edits; owns `.planning/` intake |
| **orchestrator** | Split units, select tier, hand off to architects | Only `.planning/` writes; delegate implementation, docs, tests/builds, substantive review and forensic audit |
| **architect** | Investigate scope, gaps, perf/security | No children or source edits; return decisions |
| **rust-coder** / **react-coder** | Implement; return contract, docs and verification needs | No children; parent owns gate |
| **tester** | Host-native embedded checks first (`--storage embedded`) | No full platform CI locally |
| **auditor** | Binary veto (`INTEGRITY_VIOLATION` blocks completion) | Forensic integrity, no facade code |
| **reviewer** | Standards review | `APPROVE` required |
| **scribe** | Docs/wiki at parent checkpoints before verification | Keeps tags/contracts/diagrams current |
| **utility** | Quick non-code edits (docs, config, instructions) | No source/tests/CI edits; no children |

Orchestrators own plans and delegate edits, tests and substantive reviews. Architects and coders work directly and never spawn children; coders return documentation, consultation and verification needs to the parent. Shell must not bypass role boundaries. The parent owns acceptance and schedules docs after behavior changes.

### Capacity and recovery

Follow [provider limits and continuation](../docs/wiki/code/opencode-v2.md#task-wide-provider-concurrency-guidance). Disjoint ownership; reuse evidence; one tester owns builds/logs per integration tree. Models come from JSON config.

**Gate:** `reviewer: APPROVE + tester: PASS + auditor: CLEAN`

---

## Tier Selection (`.opencode/opencode.json`)

| Tier | Scope | Team |
|------|-------|------|
| **Tier 1** | ≤1 file, single crate/package, read-only or docs | Direct specialist (architect/reviewer/tester/scribe/rust-coder/react-coder); quick non-code edit → `utility` |
| **Tier 2** | ≤3 files, single crate/package, well-defined | architect (if targets unclear) → rust-coder/react-coder → scribe (if docs) → gate |
| **Tier 3** | >3 files, multi-crate, mixed Rust+React | orchestrator → parallel architect → worktree-isolated coders → scribe → gate |

---

## Quick Reference

- **Navigate:** [wiki](../docs/index.md) → concepts/code → source; use [tags](../docs/wiki/tags.md) and scoped search
- **CI:** Inspect [contracts](../docs/wiki/code/ci.md) and affected `.github/workflows/`; stabilize host-native embedded build first; monitor Actions for other platforms
- **Plan:** Pin one named task; use [planning format](../docs/wiki/code/planning-artifacts.md#working-format); update phase checkboxes with evidence and Next Step
- **Test:** Load [Argos testing guidelines](skills/argos-testing/SKILL.md) for Rust/React test rules
- **Rust:** Load `argos-rust-style` for `crates/`; follow async conventions (Tokio); ignore Chromium/Fuchsia bans
- **React UI:** Load `argos-react-ui` + `argos-react-patterns`; follow [UI rules](../docs/wiki/code/react-ui.md) and [look & feel](../docs/wiki/code/react-ui.md#argos-look--feel-reference); keep React Compiler/Hooks lint enabled
- **Maintain:** [Setup/refresh wiki](../docs/wiki/maintenance.md) updates routes, tags, contracts, diagrams in the same change; scratch in `.agent-scratch/`; commits require authorization and exact staging

---

## Design contract

Follow the root design contract and [React UI guide](../docs/wiki/code/react-ui.md): terminal desk/HUD tokens, explicit density, accessible focus/selection and `ForceCanvas` (`react-force-graph-3d`) with list twin. TUI retains simpler chrome. Inspect affected visual fixtures before approval.

For UI/TUI visual verification/fix cycles, the owner tracks progress against the prior iteration. After at most three iterations without meaningful progress, stop and automatically defer the defect: record it with evidence as an open item, continue other implementation, and revisit only after implementation ends; the coordinator then asks the customer about each open deferred defect. Deferral never turns an unmet gate into a pass or waives accessibility, functionality or integrity.

## Rust mutation execution restriction

Until further notice, run Rust mutations and baselines only on GitHub Actions (`mutation.yml`), never locally. Report local mutation execution as intentionally skipped; ordinary targeted checks and required embedded gates still apply.
