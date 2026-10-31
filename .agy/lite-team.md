# Antigravity Lite Agent Team (`lite-team.md`)

[How-To Guide](./howto.md) · [Heavy Team](./heavy-team.md)

Fast-path stack-split coder workflow (`rust-coder` / `react-coder` + `tester`) for scoped tasks ($\le 3$ files, single module, no breaking contract/schema migrations). The owning stack coder acts as the primary coordinator (`experimental.subagent_depth: 2`).

| Caller | Permitted Children | Assigned `subagent.model` | Purpose | 
| ----- | ----- | ----- | ----- | 
| `coordinator` / `rust-coder` / `react-coder` (Orchestration) | `rust-coder`, `react-coder`, `tester`, `architect` (`CONSULT`) | `google/gemini-4-argon#low` *(alias: `google/argon#low`)* | General team orchestration for efficient work completion, dispatching agents, and tracking scoped task progress (`experimental.subagent_depth: 2`) | 
| `rust-coder` (Rust Implementation & Admin) | `tester`, `architect` (`CONSULT`) | Coder: `opencode-go/kimi-k2.7-code`; spawned `EDIT_UNIT` children: `opencode-go/kimi-k2.7-code` (no reasoning variants; verbatim ID) | Precise changes in `crates/**`, `scripts/**`, Rust formatting, and general administrative work not requiring heavy reasoning | 
| `react-coder` (React Implementation & Admin) | `tester`, `architect` (`CONSULT`) | Coder: `opencode-go/kimi-k2.7-code`; spawned `EDIT_UNIT` children: `opencode-go/kimi-k2.7-code` (no reasoning variants; verbatim ID) | Precise UI changes in `apps/**`, `packages/**`, and general administrative work not requiring heavy reasoning | 
| `tester` (Verification) | `architect` (`CONSULT`) | `google/gemini-4-argon#low` *(alias: `google/argon#low`)* | Run host-native targeted Rust/system and UI verification gates in `INTEGRATION_ROOT` and return `PASS / FAIL / BLOCKED / PENDING` | 
| `architect` (`CONSULT`) | `none` | `anthropic/claude-sonnet-5.5#low` | Consult team member for code planning, code review, architecture, and analysis (e.g. contract design, UI design standards/glass palette review, type safety) | 

## Fast-Path Execution Sequence

1. **Scope Check & Model Selection:**
   * **General Team Orchestration & Plan Tracking:** Use `google/gemini-4-argon#low` (alias: `google/argon#low`) for general team orchestration for efficient work completion and tracking scoped plan progress.
   * **Precise File Edits & Administrative Work:** `rust-coder` and `react-coder` run on `opencode-go/kimi-k2.7-code` for precise changes in their stack's repo files or worktrees (Rust/scripts and React Native UI, respectively) and general administrative work not requiring heavy reasoning. Their spawned `EDIT_UNIT` children run on `opencode-go/kimi-k2.7-code` (no reasoning variants; verbatim ID).
   * **Code Planning, Review, Architecture & Analysis Consultation:** Consult a team member with `anthropic/claude-sonnet-5.5#low` (`architect` / `CONSULT`) whenever code planning, architecture guidance, technical analysis, or code review (such as auditing diffs or enforcing [React UI rules](../docs/wiki/code/react-ui.md), the [Argos look and feel](../docs/wiki/code/react-ui.md#argos-look--feel-reference), [repo conventions](../docs/wiki/code/conventions.md), and React Compiler rules) is required. Component design may be original inside the Argos theme.
   * **Host-Native Verification:** `tester` runs on `google/gemini-4-argon#low` to execute targeted verification gates in `INTEGRATION_ROOT`.
   * If scope expands beyond $3$ files, alters shared contracts, or requires full multi-phase `.planning/` lifecycle management, stop and escalate to `./heavy-team.md`.

2. **Implementation & Storage Boundaries:**
   * The owning stack coder (`rust-coder` or `react-coder`) records `INTEGRATION_ROOT` and baseline dirty paths.
   * Sequential edits across $\le 3$ files execute directly in `INTEGRATION_ROOT` or in a single isolated worktree under `.agent-scratch/worktrees/<task-id>`. Rust edits load `argos-rust-style`; React UI edits load `argos-react-ui` + `argos-react-patterns`.
   * All temporary files, scratch directories, and intermediate outputs **must** use `.agent-scratch/` or `.agent-scratch/worktrees/`.
   * If the task requires recording durable planning notes or spec updates intended for long-term commit persistence to GitHub, store them in the appropriate `.planning/` directory (never in `.agent-scratch/`).

3. **Verification (`VERIFY`):** The owning stack coder formats all modified files and dispatches `tester` (`google/gemini-4-argon#low`). For code review, design inspection, or React Native UI verification against repo UI design standards and conventions (`docs/wiki/code/react-ui.md` Argos look and feel, creative component design inside the theme, shared React Native primitives, zero-warning React Compiler/Hooks lint, and Playwright fixture screenshot inspection), consult team member on `anthropic/claude-sonnet-5.5#low`. `tester` executes the targeted verification gate in `INTEGRATION_ROOT`, writes any temporary verification logs to `.agent-scratch/`, and returns exit codes and diagnostics.

4. **Fix Loop & Cleanup:** If `tester` reports `FAIL`, the owning stack coder applies targeted fixes and resumes `tester` via its parent-owned `sessionID`. Once `PASS` is achieved, the owning stack coder cleans up `.agent-scratch/` and `.agent-scratch/worktrees/`.

## Handoff Envelope (`rust-coder`/`react-coder` → `tester` / `CONSULT`)

```text
MODE: VERIFY | EDIT_UNIT | CONSULT
SUBAGENT_MODEL: google/gemini-4-argon#low | opencode-go/kimi-k2.7-code (coder) | opencode-go/kimi-k2.7-code (EDIT_UNIT child; verbatim ID) | anthropic/claude-sonnet-5.5#low
INTEGRATION_ROOT / BASE_REVISION:
WORKTREE / SCRATCH_DIR: .agent-scratch/ or .agent-scratch/worktrees/<task-id>
PLAN_DIR (if persistent GitHub commit required): .planning/
OWNED_FILES: exact modified paths
SPEC: concise task goal, constraints, and file:line refs
ACCEPTANCE: host-native targeted check commands and expected results
RETURN: PASS | FAIL | BLOCKED | PENDING; command exit codes and evidence paths in .agent-scratch/
```

## Verification, Storage & Cleanup Conventions

* **Temporary Files & Worktrees (`.agent-scratch/` & `.agent-scratch/worktrees/`):** Strictly confine all temporary files, scratch directories, build caches, and isolated worktrees to `.agent-scratch/` or `.agent-scratch/worktrees/`. Never commit `.agent-scratch/` contents to GitHub.
* **Long-Term GitHub Persistence (`.planning/`):** Use the appropriate `.planning/` directory for any persistent planning or specification artifacts meant to be committed to GitHub.
* **Local Host Stabilization First & GitHub CI Monitoring (See `.github/workflows/ci.yml`):** Never run the all-platform or `server` storage stack from `.github/workflows/ci.yml` locally. Agents must primarily focus on getting the **local host machine's platform build and embedded storage tests (`--storage embedded`, default features `embedded,nlp` with `ARGOS_EMBED` unset) to a stable state first**. Use bash shell commands (`uname -ms`, `rustc -vV | grep '^host:'`, `sysctl -n hw.ncpu 2>/dev/null || nproc`) to inspect the host platform and machine specs before running Rust builds, and monitor GitHub Actions checks (`gh pr checks`, `gh run list --branch "$(git branch --show-current)"`, `gh run view <run-id> --log-failed`) when available for issues on other platforms once pushed.
* **Targeted Rust Gate (Embedded Storage Only):** Prefer scoped crate and test commands matching the modified files (`CARGO_BUILD_JOBS=2`, `ARGOS_EMBED` unset):
  * `python3 scripts/agent_cargo.py check --locked -p <crate>` and `python3 scripts/agent_cargo.py clippy --locked -p <crate> --all-targets -- -D warnings`
  * `python3 scripts/agent_cargo.py test --locked -p <crate> --lib [filter]` or `python3 scripts/agent_cargo.py test --locked -p <crate> --test <name> [filter]`
  * Single embedded storage test binary when a specific storage module is touched: `ARGOS_SURREAL_MODE=embedded ARGOS_SURREAL_REQUIRED=1 python3 scripts/agent_cargo.py test --locked -p argos-osint-core --test storage_<name> [filter] -- --nocapture`
  * Full embedded gate only when cross-cutting verification is needed: `scripts/local_matrix.sh --storage embedded` in `INTEGRATION_ROOT`.
* **Targeted React Native / UI Gate:** Reference `.github/workflows/intel-web.yml` — pinned Node/pnpm compiler/lint/build check (`pnpm typecheck`, `pnpm lint`, `pnpm check:web-build` with zero warnings) and relevant offline Playwright fixture.
* **Post-Task Cleanup:** Follow root `AGENTS.md` cleanup rules. If a temporary worktree under `.agent-scratch/worktrees/` was used, remove it cleanly (`git worktree remove`) and run `git worktree prune` after integrating deltas. Delete temporary scratch files and retain only concise verification evidence in `.agent-scratch/`.
