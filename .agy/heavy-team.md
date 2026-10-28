# Antigravity Heavy Agent Team (`heavy-team.md`)

[How-To Guide](./howto.md) · [Lite Team](./lite-team.md)

`architect` is the default primary coordinator (`experimental.subagent_depth: 3`). `architect`, `coder`, `tester`, and `scribe` can run as primary agents or children (`mode: all`).

| Caller | Permitted Children | Assigned `subagent.model` | Purpose |
| --- | --- | --- | --- |
| `orchestrator` / `architect` (Team Lead) | `architect` (`CONSULT`), `coder`, `tester`, `scribe` | `google/gemini-4-argon#low` *(alias: `google/argon#low`)* | Long-term team coordination, plan tracking, and general team orchestration for efficient work completion (`experimental.subagent_depth: 3`) |
| `architect` / `consultant` (`CONSULT` / Analysis) | `none` | `anthropic/claude-sonnet-5.5#low` | Consult team member for code planning, code review, architecture, and analysis (explore code/docs, define technical spec/contracts, audit diffs, review UI conventions) |
| `coder` (Implementation & Admin) | `coder`, `architect` (`CONSULT`), `tester`, `scribe` | `google/gemini-3.8-flash#medium` | Precise changes in repo files or worktrees (`crates/**`, `apps/**`, `packages/**`, `scripts/**`), manage `.agent-scratch/worktrees/`, format files, and handle general administrative tasks |
| `tester` (Verification) | `architect` (`CONSULT`) | `google/gemini-4-argon#low` *(alias: `google/argon#low`)* | Run host-native Rust/CI and UI verification gates in `INTEGRATION_ROOT`, consult `anthropic/claude-sonnet-5.5#low` for code review/analysis |
| `scribe` (Plan Tracking & Docs) | `none` | `google/gemini-4-argon#low` *(alias: `google/argon#low`)* / `google/gemini-3.8-flash#medium` | Long-term plan tracking in `.planning/` (`google/gemini-4-argon#low`); general administrative doc formatting and chore updates (`google/gemini-3.8-flash#medium`) |

## Phase Sequence & Handoffs

1. **Phase 0 Bootstrap (Team Coordination, Code Planning & Plan Tracking):**
   * The team orchestrator (`google/gemini-4-argon#low`) establishes long-term team coordination and orchestrates the phase lifecycle.
   * For code planning, architecture, and analysis, the team consults a team member with `anthropic/claude-sonnet-5.5#low` (`architect` / `CONSULT` / `AUDIT_UNIT`) to explore the repo, define exact file targets, verify contract boundaries, and produce the technical specification.
   * The orchestrator dispatches `scribe` (`DOC_REVIEW` on `google/gemini-4-argon#low`) to initialize and track the durable `.planning/` docs framework via `planning-with-files` for long-term GitHub persistence.
2. **Scoped Implementation (`EDIT_UNIT` — Precise Changes & Administrative Work):**
   * `coder` runs on `google/gemini-3.8-flash#medium` to handle precise changes in repo files or worktrees and general administrative work.
   * `coder` records `INTEGRATION_ROOT` and assigns disjoint `EDIT_UNIT` children (`subagent.model: google/gemini-3.8-flash#medium`) using task-owned worktrees under `.agent-scratch/worktrees/` for precise source edits across crates, packages, and UI files.
   * If code planning, review, or architecture questions arise during implementation, consult team member on `anthropic/claude-sonnet-5.5#low`.
   * `EDIT_UNIT` children edit and format only their assigned files, use `.agent-scratch/` for any temporary data, and never delegate, build, or test.
3. **Documentation & `.planning/` Checkpoint (`DOC_REVIEW`):**
   * Once deltas are integrated and formatted in `INTEGRATION_ROOT`, `coder` dispatches `scribe`.
   * `scribe` uses `google/gemini-4-argon#low` for long-term plan tracking in `.planning/`, and `google/gemini-3.8-flash#medium` for general administrative documentation updates, wiki docs, diagrams, and READMEs, returning `COMPLETE | NEEDS_DECISION | BLOCKED`.
4. **Verification Gate (`VERIFY`):**
   * `coder` invokes `tester` (`subagent.model: google/gemini-4-argon#low`) to execute host-native verification gates in `INTEGRATION_ROOT` (Rust builds, clippy, and embedded storage mode tests referencing `.github/workflows/ci.yml` and `docs/wiki/code/ci.md`).
   * For code review, architecture analysis, and React Native UI compliance review (`react-ui.md`, `theme.rs` HUD/glass palette, shared React Native primitives, zero-warning React Compiler/Hooks lint, and offline Playwright fixture screenshots), consult team member on `anthropic/claude-sonnet-5.5#low`.
   * Both verification results and review notes are saved to `.agent-scratch/` and returned (`PASS | FAIL | BLOCKED | PENDING`) to `coder`, which relays them to the orchestrator.
5. **Acceptance & Cleanup:**
   * The orchestrator (`google/gemini-4-argon#low`) reviews diagnostics, verifies long-term plan tracking ledgers in `.planning/` for GitHub persistence, and instructs `coder` to execute `.agent-scratch/` and `.agent-scratch/worktrees/` cleanup.

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
SUBAGENT_MODEL: google/gemini-4-argon#low | google/gemini-3.8-flash#medium | anthropic/claude-sonnet-5.5#low
PLAN_ID / absolute PLAN_DIR (.planning) / PHASE:
INTEGRATION_ROOT / absolute WORKTREE (.agent-scratch/worktrees/...) / BASE_REVISION:
OWNED_FILES: exact paths (read-only references listed separately)
SPEC: file:line refs, contracts, constraints, performance/migration risks
ACCEPTANCE: host-native checks, expected results, scratch evidence paths (.agent-scratch/...), user exclusions
RETURN: COMPLETE | NEEDS_DECISION | FAILED | BLOCKED | PASS | FAIL | PENDING; changed files and evidence
```

## Verification Commands, Rust Embedded Storage & Host Platform Rules

* **Local Host Stabilization First (Never Run All-Platform CI Stack Locally):** Never run the multi-platform or `server` storage stack from `.github/workflows/ci.yml` on the local machine. Agents must primarily get the **local host machine's native platform build and embedded storage tests (`--storage embedded`, default features `embedded,nlp` with `ARGOS_EMBED` unset) to a stable, passing state first**. Use bash shell commands (`uname -ms`, `rustc -vV | grep '^host:'`, `sysctl -n hw.ncpu 2>/dev/null || nproc`) to detect the host platform and CPU specs before running Rust builds.
* **Targeted Rust Commands (Scoped Phase Verification in Embedded Mode):**
  * Crate check & Clippy: `python3 scripts/agent_cargo.py check --locked -p <crate>` and `python3 scripts/agent_cargo.py clippy --locked -p <crate> --all-targets -- -D warnings`
  * Unit & binary tests: `python3 scripts/agent_cargo.py test --locked -p <crate> --lib [filter]` (or `--bin argos [filter]`)
  * Integration test binary: `python3 scripts/agent_cargo.py test --locked -p <crate> --test <name> [filter]` (e.g., `--test api_contract`, `--test stack_cli`)
  * Single embedded storage contract test: `ARGOS_SURREAL_MODE=embedded ARGOS_SURREAL_REQUIRED=1 python3 scripts/agent_cargo.py test --locked -p argos-osint-core --test storage_<name> [filter] -- --nocapture`
* **Full Local Rust Gate (Embedded Storage Only):** `scripts/local_matrix.sh --storage embedded` in `INTEGRATION_ROOT` (`ARGOS_EMBED` unset) or `python3 scripts/test_surreal.py --mode embedded --no-fail-fast` for the detected host platform (respecting the 1–2 runs / max 3 cap).
* **GitHub CI Monitoring for Other Platforms:** Once local builds are stable and pushed, monitor GitHub Actions checks/runs for issues on non-host platforms (`ubuntu-24.04`, `ubuntu-24.04-arm`, `macos-14`, `macos-15-intel`) or `server` storage mode when git/GitHub CLI commands are available (`gh pr checks`, `gh run list --branch "$(git branch --show-current)"`, `gh run view <run-id> --log-failed`).
* **React Native / UI:** Reference `.github/workflows/intel-web.yml` — pinned Node/pnpm compiler/lint checks (`pnpm typecheck`, `pnpm lint`, `pnpm build`, `pnpm check:web-build` with zero-warning Hooks/Next compiler baseline) and offline Playwright fixtures via shell.
