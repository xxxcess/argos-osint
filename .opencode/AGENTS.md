# Argos — OpenCode Quick Guide

Follow root [AGENTS.md](../AGENTS.md); user instructions take precedence. Preserve unrelated changes.

## Agent Operation Chain

**coordinator → orchestrator → architect → rust-coder/react-coder → tester/auditor/reviewer → scribe**

Unless user specifies otherwise, work propagates through this chain:

| Agent | Role | Key Constraints |
|-------|------|-----------------|
| **coordinator** | Decompose request, provide context, suggest team, record constraints | Never edits source; owns `.planning/` intake |
| **orchestrator** | Break into units, select tier, hand off to architects | No direct edits; only `.planning/` writes |
| **architect** | Analyze scope, anticipate gaps, push back on perf/security, delegate to coders | No source edits; consults user via `NEEDS_DECISION` |
| **rust-coder** / **react-coder** | Implement exactly, consult architect on complexity | Must wait for verification gate |
| **tester** | Host-native embedded checks first (`--storage embedded`) | Never run full platform CI locally |
| **auditor** | Binary veto (`INTEGRITY_VIOLATION` blocks completion) | Forensic integrity, no facade code |
| **reviewer** | Standards/conventions review | `APPROVE` required |
| **scribe** | Docs/wiki updates after gate passes | Keeps tags/contracts/diagrams current |

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