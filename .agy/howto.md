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
| **Team Orchestration & Coordination** | Fast-path team orchestration for efficient work completion (`google/gemini-4-argon#low` / `google/argon#low`) | Long-term team coordination and general team orchestration (`architect` on `google/gemini-4-argon#low` / `google/argon#low`) |
| **Precise Edits & Administrative Work** | `coder` (`google/gemini-3.8-flash#medium`) for precise changes in repo files or worktrees & general admin | `coder` (`google/gemini-3.8-flash#medium`) for precise changes in repo files or worktrees & general admin |
| **Code Planning, Review & Architecture** | Consult team member with `anthropic/claude-sonnet-5.5#low` (`architect` / `CONSULT`) | Consult team member with `anthropic/claude-sonnet-5.5#low` (`architect` / `CONSULT`) |
| **Verification (`tester`)** | Host-native verification gate (`google/gemini-4-argon#low`); consult `anthropic/claude-sonnet-5.5#low` for code/UI review | Host-native verification gate (`google/gemini-4-argon#low`); consult `anthropic/claude-sonnet-5.5#low` for code/UI review |

---

## 2. Model Selection Matrix for Antigravity Teamwork Teams

Every agent in `/teamwork` must be configured with one of the three designated models according to its role and cognitive requirement:

| Model | Subagent Model Identifier | Intended Responsibilities & Selection Rules |
| --- | --- | --- |
| **Gemini 4 Argon Low** | `google/gemini-4-argon#low`<br>*(alias: `google/argon#low`)* | **Long-term team coordination, plan tracking, and general team orchestration for efficient work completion.**<br>• **Team Orchestration:** Primary coordinator / orchestrator (`architect` in heavy team, fast-path coordinator in lite team) driving efficient team completion and subagent workflows.<br>• **Long-Term Team Coordination:** Managing phase handoffs between team members and supervising multi-phase tasks.<br>• **Plan Tracking:** Durable roadmap and phase progress tracking within the `.planning/` framework for GitHub persistence.<br>• **Host Verification:** Driving host-native test and build verification gates (`tester`). |
| **Gemini 3.8 Flash Medium** | `google/gemini-3.8-flash#medium` | **General administrative work & precise changes in repo files or worktrees.**<br>• **Precise Code & File Changes:** Assigned to `coder` and `EDIT_UNIT` workers making exact, targeted modifications in repository files or isolated worktrees (`.agent-scratch/worktrees/`).<br>• **General Administrative Work:** Executing routine administrative chores, non-reasoning file movements, code formatting, and mechanical documentation upkeep that does not require heavy reasoning. |
| **Claude Sonnet 5.5 Low** | `anthropic/claude-sonnet-5.5#low` | **Code planning, review, architecture, and analysis.**<br>• **Consultation Specialist:** Consult team member with this model whenever deep reasoning or analytical rigor is required.<br>• **Code Planning & Architecture:** Defining pre-implementation technical specifications, trait/type design, cross-crate contract boundaries, and system architectures.<br>• **Code Review:** Auditing code diffs, pull requests, and worktree deltas against repository conventions, type contracts, and React Native UI design standards ([Argos look and feel](../docs/wiki/code/react-ui.md#argos-look--feel-reference): theme tokens, creative component design inside the theme, React Compiler compliance).<br>• **Technical Analysis:** Investigating complex contract ambiguities, architectural refactoring, and root-cause diagnostic analysis. |

---

## 3. Directory, Persistence & Platform Build Conventions (Mandatory for Both Teams)

1. **Temporary Files & Directories (`.agent-scratch/` & `.agent-scratch/worktrees/`):**
   * Every temporary file, scratch patch, diagnostic log, isolated Cargo target cache (`CARGO_TARGET_DIR`), or temporary directory **must** be created strictly inside `.agent-scratch/` or `.agent-scratch/worktrees/`.
   * Never create ad-hoc temporary files in the repository root or source directories, and never commit `.agent-scratch/` contents to GitHub.
   * Clean, integrated worktrees under `.agent-scratch/worktrees/` and stale temporary files in `.agent-scratch/` must be pruned before completing the workflow.
2. **Long-Term Commit Persistence (`.planning/`):**
   * All durable planning artifacts, architectural specifications, phase roadmaps, and decision records intended for long-term persistence and GitHub commits **must** be stored in the appropriate `.planning/` directory (initialized and maintained via `~/.agents/skills/planning-with-files`).
3. **Rust Builds & Tests — Local Host Stabilization First & GitHub CI Monitoring (See `.github/workflows/ci.yml`):**
   * **Never run the all-platform or `server` storage matrix from `.github/workflows/ci.yml` locally.** Agents must primarily focus on getting **local host-platform builds and embedded storage tests (`--storage embedded`, default features `embedded,nlp` with `ARGOS_EMBED` unset) to a stable, passing state first**.
   * Reference `.github/workflows/ci.yml`, [docs/wiki/code/ci.md](../docs/wiki/code/ci.md), and [docs/wiki/code/contributing.md](../docs/wiki/code/contributing.md#targeted-local-rust-builds-and-tests-fast-iteration) to match local Rust verification to the single embedded leg for the detected host platform (`macos-14` arm64, `macos-15-intel` x86_64, `ubuntu-24.04` x86_64, `ubuntu-24.04-arm` aarch64), and `.github/workflows/intel-web.yml` / `.github/workflows/commit-policy.yml` when web or policy checks apply.
   * Detect the host machine platform, OS/architecture, CPU cores, and native Rust target triple via bash shell commands (`uname -ms`, `rustc -vV | grep '^host:'`, `sysctl -n hw.ncpu 2>/dev/null || nproc`).
   * **Use targeted Rust build & test commands for scoped iteration** before running the full embedded gate:
     * Scoped compile & lint by crate: `python3 scripts/agent_cargo.py check --locked -p <crate>` and `python3 scripts/agent_cargo.py clippy --locked -p <crate> --all-targets -- -D warnings` (`argos-osint-core`, `argos-osint-bin`, `argos-osint-api`, `argos-api-host`).
     * Scoped unit/binary tests: `python3 scripts/agent_cargo.py test --locked -p <crate> --lib [filter]` or `--bin argos [filter]`.
     * Scoped integration test binary: `python3 scripts/agent_cargo.py test --locked -p <crate> --test <name> [filter]` (e.g., `--test api_contract`, `--test stack_cli`).
     * Scoped embedded storage contract test (single binary instead of all 60+ storage binaries): `ARGOS_SURREAL_MODE=embedded ARGOS_SURREAL_REQUIRED=1 python3 scripts/agent_cargo.py test --locked -p argos-osint-core --test storage_<name> [filter] -- --nocapture`.
     * Full embedded storage suite / full local matrix gate: `python3 scripts/test_surreal.py --mode embedded --no-fail-fast` or `scripts/local_matrix.sh --storage embedded`.
   * **Monitor GitHub CI for other platforms:** Once local host builds are stable and changes are pushed, agents should monitor GitHub Actions build/test checks for issues on non-host platforms or `server` storage mode if git/GitHub CLI commands are available (`gh pr checks`, `gh run list --branch "$(git branch --show-current)"`, `gh run view <run-id> --log-failed`).

---
