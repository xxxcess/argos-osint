# Antigravity Lite Agent Team (`lite-team.md`)

[How-To Guide](./howto.md) · [Heavy Team](./heavy-team.md)

Fast-path 2-agent workflow (`coder` + `tester`) for scoped tasks ($\le 3$ files, single module, no breaking contract/schema migrations). `coder` acts as the primary coordinator (`experimental.subagent_depth: 2`).

| Caller | Permitted Children | Assigned `subagent.model` | Purpose | 
| ----- | ----- | ----- | ----- | 
| `coder` (Rust/System) | `coder`, `tester`, `architect` (`CONSULT`) | `google/gemini-3.8-flash#high` | Implement & format Rust/script changes (`crates/**`, `scripts/**`, `Cargo.*`), invoke `tester` (`google/argon#low`) | 
| `coder` (React Native) | `coder`, `tester`, `architect` (`CONSULT`) | `anthropic/claude-sonnet-5.5#low` | Implement & format React/React Native changes (`apps/**`, `packages/**`, `ui/**`, `**/*.tsx`, `**/*.ts`), invoke `tester` (`anthropic/claude-sonnet-5.5#low`) | 
| `tester` (Rust/System) | `architect` (`CONSULT`) | `google/argon#low` | Run host-native targeted Rust/system verification gates in `INTEGRATION_ROOT` and return `PASS / FAIL / BLOCKED / PENDING` | 
| `tester` (React Native UI) | `architect` (`CONSULT`) | `anthropic/claude-sonnet-5.5#low` | Run React Native UI verification gates in `INTEGRATION_ROOT`, enforce repo UI design standards and conventions (`docs/wiki/code/react-ui.md`, `docs/wiki/code/conventions.md`, `theme.rs` HUD/glass palette, React Compiler/Hooks lint, Playwright fixture screenshots), and return `PASS / FAIL / BLOCKED / PENDING` | 
| `architect` (`CONSULT`) | `none` | `google/argon#low` | Optional read-only leaf consultation if `coder` or `tester` hits an unexpected contract ambiguity | 

## Fast-Path Execution Sequence

1. **Scope Check & Model Selection:**
   * If the task touches **Rust/Backend/Scripts**, `coder` runs on `google/gemini-3.8-flash#high` and pairs with `tester` on `google/argon#low`.
   * If the task touches **React/React Native UI**, both `coder` and its dedicated `tester` agent run on `anthropic/claude-sonnet-5.5#low`, loading `.agents/skills/argos-react-ui` and `.agents/skills/argos-react-patterns` and enforcing [React UI rules](../docs/wiki/code/react-ui.md) and [repo conventions](../docs/wiki/code/conventions.md).
   * If scope expands beyond $3$ files, alters shared contracts, or requires full multi-phase `.planning/` lifecycle management, stop and escalate to `./heavy-team.md`.

2. **Implementation & Storage Boundaries:**
   * `coder` records `INTEGRATION_ROOT` and baseline dirty paths.
   * Sequential edits across $\le 3$ files execute directly in `INTEGRATION_ROOT` or in a single isolated worktree under `.agent-scratch/worktrees/<task-id>`.
   * All temporary files, scratch directories, and intermediate outputs **must** use `.agent-scratch/` or `.agent-scratch/worktrees/`.
   * If the task requires recording durable planning notes or spec updates intended for long-term commit persistence to GitHub, store them in the appropriate `.planning/` directory (never in `.agent-scratch/`).

3. **Verification (`VERIFY`):** `coder` formats all modified files and dispatches the scope-appropriate `tester` (`google/argon#low` for Rust/system work; a separate `tester` on `anthropic/claude-sonnet-5.5#low` for React Native UI work). For React Native UI scope, `tester` validates strict adherence to repository UI design standards and conventions (`docs/wiki/code/react-ui.md`, `theme.rs` HUD/glass palette, shared React Native primitives, zero-warning React Compiler/Hooks lint, and Playwright fixture screenshot inspection). `tester` executes the targeted verification gate in `INTEGRATION_ROOT`, writes any temporary verification logs to `.agent-scratch/`, and returns exit codes and diagnostics.

4. **Fix Loop & Cleanup:** If `tester` reports `FAIL`, `coder` applies targeted fixes and resumes `tester` via its parent-owned `sessionID`. Once `PASS` is achieved, `coder` cleans up `.agent-scratch/` and `.agent-scratch/worktrees/`.

## Handoff Envelope (`coder` → `tester` / `CONSULT`)

```text
MODE: VERIFY | EDIT_UNIT | CONSULT
SUBAGENT_MODEL: google/argon#low | google/gemini-3.8-flash#high | anthropic/claude-sonnet-5.5#low
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
* **Embedded Storage Mode & Host-Native Platform Builds (See `.github/workflows/`):** All local test and build work **must** run in **embedded storage mode** (`--storage embedded`, default features `embedded,nlp` with `ARGOS_EMBED` unset) and for the **local host machine's platform type** (do not run the CI-only `server` storage mode or cross-compile for non-host platforms). Reference `.github/workflows/ci.yml`, `.github/workflows/intel-web.yml`, `.github/workflows/commit-policy.yml`, and `docs/wiki/code/ci.md` to match the local check to the host platform leg. Use bash shell commands (`uname -ms`, `rustc -vV | grep host`, `sysctl -n hw.ncpu 2>/dev/null || nproc`) to inspect the host platform and machine specs before running builds.
* **Targeted Rust Gate (Embedded Storage Only):** `python3 scripts/agent_cargo.py check --locked` and scoped host-native tests (`python3 scripts/test_surreal.py --mode embedded` when storage is touched), or `scripts/local_matrix.sh --storage embedded` in `INTEGRATION_ROOT` (`ARGOS_EMBED` unset, `CARGO_BUILD_JOBS=2`).
* **Targeted React Native / UI Gate:** Reference `.github/workflows/intel-web.yml` — pinned Node/pnpm compiler/lint/build check (`pnpm typecheck`, `pnpm lint`, `pnpm check:web-build` with zero warnings) and relevant offline Playwright fixture.
* **Post-Task Cleanup:** Follow root `AGENTS.md` cleanup rules. If a temporary worktree under `.agent-scratch/worktrees/` was used, remove it cleanly (`git worktree remove`) and run `git worktree prune` after integrating deltas. Delete temporary scratch files and retain only concise verification evidence in `.agent-scratch/`.
