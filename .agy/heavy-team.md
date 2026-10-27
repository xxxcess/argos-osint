# Antigravity Heavy Agent Team (`heavy-team.md`)

[How-To Guide](./howto.md) · [Lite Team](./lite-team.md)

`architect` is the default primary coordinator (`experimental.subagent_depth: 3`). `architect`, `coder`, `tester`, and `scribe` can run as primary agents or children (`mode: all`).

| Caller | Permitted Children | Assigned `subagent.model` | Purpose |
| --- | --- | --- | --- |
| `architect` | `architect`, `coder`, `tester`, `scribe` | `google/argon#low` | Explore code/docs (`AUDIT_UNIT`), define spec, trigger `scribe` `.planning/` bootstrap, review phase evidence |
| `coder` (Rust/System) | `coder`, `architect`, `tester`, `scribe` | `google/gemini-3.8-flash#high` | Delegate Rust/repo `EDIT_UNIT` edits, manage `.agent-scratch/worktrees/`, call `scribe` & `tester` |
| `coder` (React Native) | `coder`, `architect`, `tester`, `scribe` | `anthropic/claude-sonnet-5.5#low` | Delegate React/React Native UI `EDIT_UNIT` edits, enforce Next/React Compiler rules, call `scribe` & `tester` |
| `tester` (Rust/System) | `architect` | `google/argon#low` | Run host-native Rust/CI verification gates in `INTEGRATION_ROOT`, ask `architect` design questions (`CONSULT`) |
| `tester` (React Native UI) | `architect` | `anthropic/claude-sonnet-5.5#low` | Run React Native UI verification gates, enforce repo UI design standards/conventions (`react-ui.md`, `theme.rs` HUD/glass palette, React Compiler/Hooks lint, Playwright fixture screenshots), ask `architect` (`CONSULT`) |
| `scribe` | `none` | `google/argon#low` | Exclusively own `.planning/` framework (`planning-with-files`) for GitHub commit persistence, review code deltas, maintain wiki/READMEs |

## Phase Sequence & Handoffs

1. **Phase 0 Bootstrap (`.planning/` Setup for Long-Term Persistence):** `architect` explores the repo (spawning `AUDIT_UNIT` children on `google/argon#low` if needed) and writes the technical specification with exact files, contracts, performance/migration risks, and acceptance criteria. Before spawning `coder`, `architect` dispatches `scribe` (`DOC_REVIEW`) to initialize and structure the `.planning/` docs framework via `~/.agents/skills/planning-with-files` so durable planning artifacts are tracked for long-term GitHub commit persistence.
2. **Scoped Implementation (`EDIT_UNIT`):** `coder` records `INTEGRATION_ROOT` and assigns disjoint `EDIT_UNIT` children using task-owned worktrees under `.agent-scratch/worktrees/`:
   * **Rust / System Scope (`crates/**`, `scripts/**`, `Cargo.*`):** Spawn `coder` with `subagent.model: google/gemini-3.8-flash#high`.
   * **React / React Native Scope (`apps/**`, `packages/**`, `ui/**`, `**/*.tsx`, `**/*.ts`):** Spawn `coder` with `subagent.model: anthropic/claude-sonnet-5.5#low` and load `.agents/skills/argos-react-ui` and `.agents/skills/argos-react-patterns`.
   * `EDIT_UNIT` children edit and format only their assigned files, use `.agent-scratch/` for any temporary data, and never delegate, build, or test.
3. **Documentation & `.planning/` Checkpoint (`DOC_REVIEW`):** Once deltas are integrated and formatted in `INTEGRATION_ROOT`, `coder` dispatches `scribe` (`google/argon#low`). `scribe` updates the persistent `.planning/` framework files, wiki docs, diagrams, and READMEs for GitHub commit persistence, returning `COMPLETE | NEEDS_DECISION | BLOCKED`.
4. **Verification Gate (`VERIFY`):** `coder` invokes `tester` on the integrated tree using a separate model/role configuration by scope:
   * **Rust / System Verification:** Spawn `tester` with `subagent.model: google/argon#low` to run host-native Rust builds, clippy, and storage/runtime tests.
   * **React Native UI Verification:** Spawn a separate `tester` agent with `subagent.model: anthropic/claude-sonnet-5.5#low` for work touching React Native UI (`apps/web-desktop`, `packages/{ui,intel,api-client}`). This agent loads `.agents/skills/argos-react-ui` and `.agents/skills/argos-react-patterns`, strictly adheres to repository design standards and conventions (`docs/wiki/code/react-ui.md`, ` docs/wiki/code/conventions.md`, `theme.rs` palette with HUD/glass treatment, shared React Native primitives, zero-warning React Compiler/Hooks lint, and visual inspection of offline Playwright fixture screenshots), and executes `/react-ui-review` checks.
   * Both `tester` agents write temporary test logs to `.agent-scratch/` and return `PASS | FAIL | BLOCKED | PENDING` evidence to `coder`, which relays it unchanged to `architect`.
5. **Acceptance & Cleanup:** `architect` reviews diagnostics and performance, instructs `scribe` to finalize phase state in `.planning/` for GitHub persistence, and instructs `coder` to execute `.agent-scratch/` and `.agent-scratch/worktrees/` cleanup.

## Storage, Persistence & Cleanup Conventions

* **Temporary Files & Directories (`.agent-scratch/` & `.agent-scratch/worktrees/`):** All temporary files, scratch directories, patch files, runtime diagnostics, and isolated feature worktrees **must** use either `.agent-scratch/` or `.agent-scratch/worktrees/`. Never write temporary files outside `.agent-scratch/`, and never stage or commit `.agent-scratch/` paths to GitHub.
* **Long-Term GitHub Commit Persistence (`.planning/`):** All durable planning documents, phase specifications, architectural decisions, and status ledgers intended for long-term repository persistence and GitHub commits **must** be placed in the appropriate `.planning/` directory owned by `scribe` (`planning-with-files`).
* **Write Boundaries:** `architect` and `tester` edit only local planning/verification evidence in `.agent-scratch/`. `coder` edits source, tests, and fixtures. `scribe` edits only `.planning/` docs, repo docs, and READMEs (never source, tests, or configs). Browser tools are denied.
* **Worktree Isolation:** Create parallel feature worktrees from the agreed baseline (`git worktree add --detach .agent-scratch/worktrees/<unit-id> <base>`). Isolate parallel Rust checks with `CARGO_TARGET_DIR=.agent-scratch/target-<unit-id>` and `CARGO_BUILD_JOBS=2`. Never run `tester` inside feature worktrees.
* **Workflow Cleanup:** Retain task worktrees under `.agent-scratch/worktrees/` only until verified and integrated. Upon phase completion, remove clean integrated worktrees without force (`git worktree remove`), run `git worktree prune`, delete temporary build/patch artifacts, and keep only essential diagnostic logs in `.agent-scratch/`.

## Handoff Envelope

Every subagent call must include this compact envelope:

```text
MODE: PHASE | EDIT_UNIT | AUDIT_UNIT | CONSULT | VERIFY | DOC_REVIEW
SUBAGENT_MODEL: google/argon#low | google/gemini-3.8-flash#high | anthropic/claude-sonnet-5.5#low
PLAN_ID / absolute PLAN_DIR (.planning) / PHASE:
INTEGRATION_ROOT / absolute WORKTREE (.agent-scratch/worktrees/...) / BASE_REVISION:
OWNED_FILES: exact paths (read-only references listed separately)
SPEC: file:line refs, contracts, constraints, performance/migration risks
ACCEPTANCE: host-native checks, expected results, scratch evidence paths (.agent-scratch/...), user exclusions
RETURN: COMPLETE | NEEDS_DECISION | FAILED | BLOCKED | PASS | FAIL | PENDING; changed files and evidence
```

## Verification Commands & Host Platform Rules

* **Host-Machine Target Only:** Agents only need to run Rust tests and builds based on the local machine they are working on (skip cross-compilation or non-host targets). Use the shell (`uname -ms`, `rustc -vV`, `sysctl`/`lscpu`) to inspect the machine specs and host platform target if needed before running builds.
* **Rust:** `scripts/local_matrix.sh --storage embedded` in `INTEGRATION_ROOT` (`ARGOS_EMBED` unset) or targeted `python3 scripts/agent_cargo.py … --locked` for the detected host platform. Storage checks via `test_surreal.py`.
* **React Native / UI:** Pinned Node/pnpm compiler/lint checks (zero-warning Hooks/Next compiler baseline) and offline Playwright fixtures via shell.
