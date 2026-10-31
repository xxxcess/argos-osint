# Argos — OpenCode Quick Guide

Follow root [AGENTS.md](../AGENTS.md); user instructions take precedence. Preserve unrelated changes.

## Agent Operation Chain

**coordinator → orchestrator → architect → rust-coder/react-coder**; the owning parent delegates documentation and the verification gate.

Unless user specifies otherwise, work propagates through this chain:

| Agent | Role | Key Constraints |
|-------|------|-----------------|
| **coordinator** | Decompose request, provide context, suggest team, record constraints | Never edits source; owns `.planning/` intake |
| **orchestrator** | Break into units, select tier, hand off to architects | Only `.planning/` writes; delegate implementation, docs, tests/builds, substantive review, and forensic audit |
| **architect** | Analyze scope, anticipate gaps, push back on perf/security, delegate to coders | No source edits; consults user via `NEEDS_DECISION` |
| **rust-coder** / **react-coder** | Implement exactly; return contract, documentation and verification needs to parent | Spawn children only for assigned code-edit `EDIT_UNIT` work; parent owns verification gate |
| **tester** | Host-native embedded checks first (`--storage embedded`) | Never run full platform CI locally |
| **auditor** | Binary veto (`INTEGRITY_VIOLATION` blocks completion) | Forensic integrity, no facade code |
| **reviewer** | Standards/conventions review | `APPROVE` required |
| **scribe** | Docs/wiki updates at parent-owned checkpoints before verification | Keeps tags/contracts/diagrams current |

The orchestrator may perform brief reads, coordination, report interpretation, and normal planning, but must not directly edit source, tests, docs, or config, run tests or builds, or conduct potentially lengthy or substantive reviews. Delegate implementation to coders, documentation to `scribe`, tests/builds to `tester`, substantive review to `reviewer`, and forensic integrity audit to `auditor`; shell commands or other workarounds must not bypass these boundaries.

Rust and React coders may spawn children only for assigned code-edit `EDIT_UNIT` work. They never spawn `tester`, `reviewer`, `auditor`, `scribe`, or `architect` consultation children; they return those needs to the owning `coordinator`, `orchestrator`, or `architect` for delegation. The parent schedules documentation at checkpoints and again after behavior-affecting fixes, and owns the required verification gate.

### Task-wide provider concurrency guidance

- Across the entire task, cap concurrently active `architect`, `rust-coder`, and `react-coder` agents at two for `openai` and separately two for `nvidia`, including all such descendants across branches.
- Determine the provider from the prefix before the first slash of the effective model after overrides; `openrouter/nvidia/...` is `openrouter`, not `nvidia`. Other providers are unaffected.
- Count active parents even while waiting for children, and count resumed invocations while active. Coordinate counts across branches; queue excess calls until a slot is free. Schedule nested calls so slots can be released and deadlocks are avoided.
- Exclude `coordinator`, `orchestrator`, `reviewer`, `tester`, `auditor`, and `scribe` themselves, but not their `architect` or coder descendants. This is operational guidance, not a runtime semaphore.

**Verification gate (required before completion):**
```
reviewer: APPROVE  +  tester: PASS  +  auditor: NO_VETO
```

---

## Tier Selection (from `.opencode/opencode.json` catalog)

| Tier | Scope | Team |
|------|-------|------|
| **Tier 1** | ≤1 file, single crate/package, read-only or docs | Direct specialist (architect/reviewer/tester/scribe/rust-coder/react-coder) |
| **Tier 2** | ≤3 files, single crate/package, well-defined | architect (if targets unclear) → rust-coder/react-coder → scribe (if docs) → gate |
| **Tier 3** | >3 files, multi-crate, mixed Rust+React | orchestrator → parallel architect → worktree-isolated coders → scribe → gate |

---

## Quick Reference

- **Navigate:** [wiki](../docs/index.md) → concepts/code → source; use [tags](../docs/wiki/tags.md) and scoped searches
- **CI:** Inspect [workflow contracts](../docs/wiki/code/ci.md) and affected `.github/workflows/`; stabilize host-native embedded build first; monitor GitHub Actions for other platforms
- **Plan:** Pin one named task; use [planning format](../docs/wiki/code/planning-artifacts.md#working-format); update phase checkboxes with evidence and Next Step
- **Test:** Load [Argos testing guidelines](skills/argos-testing/SKILL.md) for Rust/React test rules
- **Rust:** Load `argos-rust-style` for `crates/`; follow async conventions (Tokio); ignore Chromium/Fuchsia bans
- **React UI:** Load `argos-react-ui` + `argos-react-patterns`; follow [UI rules](../docs/wiki/code/react-ui.md) and [look & feel](../docs/wiki/code/react-ui.md#argos-look--feel-reference); keep React Compiler/Hooks lint enabled
- **Maintain:** [Setup/refresh wiki](../docs/wiki/maintenance.md) updates routes, tags, contracts, diagrams in same change; scratch in `.agent-scratch/`; commits require authorization and exact staging

---

## Design Contract (React Native First, TUI Secondary)

- Primary target: React Native. Design inside Argos theme (terminal desk + HUD/glass).
- TUI: Simpler — no new chrome, no extra density modes, no decorative elements.
- Tokens: Accent = titles/focus/primary actions; Select = current selection (distinct from focus).
- Density: Default spaced; compact only for launchers, palettes, headline lists.
- Focus: Must never hide other panels/content.
- Topology: `ForceCanvas` (`react-force-graph-2d`) + accessible list twin. No second graph renderer.
- For visual changes, state: component type, density (default/compact), tokens used.

## Rust mutation execution restriction

Until further notice, agents must not run Rust mutation tests or mutation baselines locally, including `cargo mutants` and cargo-mutants runner invocations. Use the GitHub Actions `mutation.yml` workflow for execution and inspect its hosted results. Local mutation execution is deliberately skipped, not passed; ordinary targeted checks and the embedded gate remain required.
