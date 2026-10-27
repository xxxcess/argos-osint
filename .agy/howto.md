# Antigravity `/teamwork` Launch Guide: Lite vs. Heavy Team

This directory contains two agent workflow specifications designed exclusively for the Google Antigravity CLI `/teamwork` command:

* **Heavy Team (`./heavy-team.md`):** Full 4-agent orchestration (`architect`, `coder`, `tester`, `scribe`) with Phase 0 `.planning/` framework management for long-term GitHub persistence, isolated parallel worktrees under `.agent-scratch/worktrees/`, and full verification gates.
* **Lite Team (`./lite-team.md`):** Fast-path 2-agent workflow (`coder`, `tester`) for scoped edits, bug fixes, and single-module updates ($\le 3$ files) using `.agent-scratch/` for all temporary outputs without multi-worktree overhead.

---

## 1. Scope Evaluation & Team Selection Matrix

When given a task item or specification, evaluate its scope against this matrix before launching `/teamwork`:

| Criteria | Lite Team (`./lite-team.md`) | Heavy Team (`./heavy-team.md`) |
| --- | --- | --- |
| **File Scope** | $\le 3$ files modified within a single crate or UI directory | $> 3$ files, cross-crate changes, or mixed Rust + React Native changes |
| **Contracts & Schemas** | No breaking API, trait, or SurrealDB schema changes | New traits, wire budgets, database migrations, or cross-module contracts |
| **Long-Term GitHub Persistence (`.planning/`)** | Minimal or no new `.planning/` scaffolding required (uses existing `.planning/` context if applicable) | Requires durable `.planning/` tracking (`planning-with-files` owned by `scribe`) for long-term GitHub commit persistence |
| **Temporary Files & Worktrees (`.agent-scratch/`)** | All temporary files/dirs in `.agent-scratch/` or a single worktree in `.agent-scratch/worktrees/` | Disjoint parallel `EDIT_UNIT` worktrees in `.agent-scratch/worktrees/` and scratch files in `.agent-scratch/` |
| **Primary Coordinator** | `coder` (`google/gemini-3.8-flash#high` or `anthropic/claude-sonnet-5.5#low`) | `architect` (`google/argon#low`) |
| **Verification (`tester`)** | Separate `tester` by scope: `google/argon#low` (Rust/System) or `anthropic/claude-sonnet-5.5#low` (React Native UI, enforcing repo UI design standards & conventions) | Separate `tester` by scope: `google/argon#low` (Rust/System) and `anthropic/claude-sonnet-5.5#low` (React Native UI, enforcing `react-ui.md`, `theme.rs` HUD/glass palette, & conventions) |

---

## 2. Directory, Persistence & Platform Build Conventions (Mandatory for Both Teams)

1. **Temporary Files & Directories (`.agent-scratch/` & `.agent-scratch/worktrees/`):**
   * Every temporary file, scratch patch, diagnostic log, isolated Cargo target cache (`CARGO_TARGET_DIR`), or temporary directory **must** be created strictly inside `.agent-scratch/` or `.agent-scratch/worktrees/`.
   * Never create ad-hoc temporary files in the repository root or source directories, and never commit `.agent-scratch/` contents to GitHub.
   * Clean, integrated worktrees under `.agent-scratch/worktrees/` and stale temporary files in `.agent-scratch/` must be pruned before completing the workflow.
2. **Long-Term Commit Persistence (`.planning/`):**
   * All durable planning artifacts, architectural specifications, phase roadmaps, and decision records intended for long-term persistence and GitHub commits **must** be stored in the appropriate `.planning/` directory (initialized and maintained via `~/.agents/skills/planning-with-files`).
3. **Host-Native Rust Builds & Machine Specs:**
   * Agents only need to run Rust builds and tests for the local machine and platform they are currently working on (do not attempt cross-compilation or non-native platform targets).
   * Use the shell (e.g., `uname -ms`, `rustc -vV`, `sysctl -n hw.ncpu` or `nproc`) to inspect the local machine specs, OS/architecture, and native target triple if needed to know which platform to build and test for.

---
