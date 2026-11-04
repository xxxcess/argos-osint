# OpenCode agent team

[Code route](index.md) · [Wiki](../../index.md)

`coordinator` (Team Coordinator) is the default entry-point agent. All active catalog archetypes (`coordinator`, `orchestrator`, `architect`, `rust-coder`, `react-coder`, `reviewer`, `tester`, `auditor`, `scribe`, `utility`) use `mode: all`, so they can run as primary agents or children. Legacy `build` and `plan` agents are disabled and the old `general`/`explore` stubs were removed; ECC agent definitions are replaced. `/ecc-plan` selects `architect`, `/ecc-review` selects `reviewer`, `/ecc-verify` selects `tester`, and `/docs-review`, `/setup-wiki`, and `/refresh-wiki` select `scribe`.

| Caller | Permitted children | Configured model | Purpose |
| --- | --- | --- | --- |
| `coordinator` | `orchestrator`, `architect`, `rust-coder`, `react-coder`, `reviewer`, `tester`, `auditor`, `scribe` | `agents.<role>.model` | Default entry point: clarify material unknowns, preserve scope/constraints, and dispatch the smallest effective team |
| `orchestrator` | `architect`, `rust-coder`, `react-coder`, `reviewer`, `tester`, `auditor`, `scribe` | `agents.<role>.model` | Decompose multi-phase/multi-milestone work, own `.planning/`, coordinate tier-appropriate specialists, and drive iterative Scribe checkpoints while deferring edits, tests, builds, and substantive reviews to appropriate roles |
| `architect` | none | `agents.<role>.model` | Investigate code/docs directly, advise on contracts, and write `file:line` phase specs |
| `rust-coder` | none | `agents.<role>.model` | Investigate and implement assigned changes directly; return consultation, documentation and verification needs to the owning role; must consult architect before significant changes. **Exception:** this consult requirement does not apply to non-significant, conformant edits that do not alter architecture, behavior, or integration boundaries. |
| `react-coder` | none | `agents.<role>.model` | Investigate and implement assigned changes directly; return consultation, documentation and verification needs to the owning role; must consult architect before significant changes. **Exception:** this consult requirement does not apply to non-significant, conformant edits that do not alter architecture, behavior, or integration boundaries. |
| `reviewer` | `architect` | `agents.<role>.model` | Audit integrated diffs for correctness, concurrency/performance, `argos-rust-style`, and React UI compliance (`APPROVE \| REJECT`) |
| `tester` | `architect` | `agents.<role>.model` | Execute empirical host-native checks, TUI/UI screenshot verification, and adversarial stress tests in `INTEGRATION_ROOT` (`PASS \| FAIL \| BLOCKED \| PENDING`) |
| `auditor` | none | `agents.<role>.model` | Forensic integrity and anti-cheating leaf audit (`CLEAN \| INTEGRITY_VIOLATION` binary veto) |
| `utility` | none | `agents.<role>.model` | Quick general non-code edits (documentation, configuration, instructions); leaf role with no source/test/CI edit rights |
| `scribe` | none | `agents.<role>.model` | Review code deltas and maintain canonical documentation, tags, and diagrams |

## Concise Interview, Customer Translation & Dynamic Team Selection

Clear user requests and existing authorization are sufficient to proceed. The coordinator asks only for missing decisions that materially affect scope, risk or acceptance; an interview and second approval are not routine prerequisites. Use available ask tools, or a concise conversation question when none exist. Resolve routine team questions from existing contracts before escalating to the user.

Select the leanest topology by dependencies, risk and available capacity. File counts are rough indicators, not mandatory fan-out thresholds. Keep tightly coupled source and test changes in one unit. Run independent units concurrently only with disjoint ownership and enough slots; avoid idle parent chains that consume all provider capacity.

1. **Tier 1 — Direct Specialist (1 subagent)**:
   - Plan/architecture spec $\to$ `architect`; diff review $\to$ `reviewer`; UI/TUI verification $\to$ `tester`; wiki maintenance $\to$ `scribe`; authorized commit $\to$ `rust-coder`/`react-coder`.
2. **Tier 2 — Fast-Path Squad ($\le 3$ files, single crate/package)**:
   - `rust-coder`/`react-coder` implements and formats changes $\to$ `scribe` updates affected docs $\to$ verification gate (`reviewer` + `tester` + `auditor`) validates `INTEGRATION_ROOT`.
3. **Tier 3 — Full Project Orchestration ($> 3$ files, cross-crate, or multi-milestone)**:
   - `coordinator` dispatches `orchestrator`, which runs parallel `architect` (`AUDIT_UNIT`) explorers, dispatches `rust-coder`/`react-coder` (implementing disjoint assignments directly), schedules `scribe` at phase checkpoints, and gates each milestone on `reviewer: APPROVE`, `tester: PASS`, and `auditor: CLEAN`.

[Agent handoffs](../../diagrams/agent-team.html) follow one phase/milestone at a time. `orchestrator` owns shared plans and may call `architect` to investigate directly and produce a spec with exact files, contracts, performance/migration risks, and acceptance. `rust-coder`/`react-coder` investigates and implements assigned changes directly, formats them, and returns all verification, documentation, and CONSULT needs to the owning `orchestrator`/`coordinator` for dispatch. The owning role schedules `scribe`, `reviewer`, `tester`, and `auditor` at appropriate milestones. On any gate failure, `orchestrator`/`coordinator` synthesizes exact `file:line` remediation items and resumes `rust-coder`/`react-coder` for the next iteration. Plan-only requests stop before implementation. User exclusions override routine build/test defaults.

`architect` uses `AUDIT_UNIT` for exploration and `CONSULT` for decisions. It investigates directly and cannot spawn children in any mode. Stack coders investigate and implement their assigned work directly and never spawn children. They return unresolved decisions and verification needs to their owning role. `auditor` and `scribe` are strict leaf roles. Every handoff carries `PLAN_ID`, exact scope, references, and acceptance; only the owning `orchestrator` or `coordinator` updates shared plan files during delegated work.

## Models

`.opencode/opencode.json` (`agents.<role>.model`) is the source of truth for role models. Do not copy model IDs into role prompts or this guide: those copies drift when the customer changes configuration. Preserve configured choices unless a model change is explicitly requested.

Only `AUDIT_UNIT` exploration uses `.opencode/architect-worker.json`; stack coders use their configured role models. Tester, reviewer, auditor and scribe use their role models. Explicit user task overrides win. Pass the effective model on new/resumed calls where supported; unavailable models are reported without silent substitution.

### Task-wide provider concurrency guidance

- Across the entire task, cap concurrently active `architect`, `rust-coder`, and `react-coder` agents at two for `openai` and separately two for `nvidia`, including all such descendants across branches.
- Determine the provider from the prefix before the first slash of the effective model after overrides; an OpenRouter-hosted NVIDIA model counts as `openrouter`, not `nvidia`. Other providers are unaffected.
- Count active parents even while waiting for children, and count resumed invocations while active. Coordinate counts across branches; queue excess calls until a slot is free. Schedule nested calls so slots can be released and deadlocks are avoided.
- Exclude `coordinator`, `orchestrator`, `reviewer`, `tester`, `auditor`, and `scribe` themselves, but not their `architect` or coder descendants. This is operational guidance, not a runtime semaphore.

## Delegation and continuation

`experimental.subagent_depth: 4` supports nested specialist dispatch; `architect` and stack coders are leaf roles and cannot spawn children. Tester, reviewer, auditor, and architect CONSULT dispatch are owned by the parent; coders return those needs without spawning leaf roles. Leaf instructions prevent recursive workflow spawning.

A `sessionID` belongs to the calling parent: resume only a child returned by your own successful subagent call, using the same role. Never copy sibling, ancestor or previous-root child IDs into a new parent. Keep the parent/child mapping in its own local ledger; it is runtime state, not a durable spec.

If continuation returns unknown/foreign-session errors, confirm the old worker is no longer editing, then start a fresh child without `sessionID` and include the current spec, completed work and remaining scope. For model/permission/depth failures, report the exact diagnostic and stop that handoff for configuration repair. Do not claim success or fall back to direct edits/builds in the wrong role. Background children notify parents; avoid busy polling. One tester owns build execution and log collection per integration tree. Record process/session ownership, command, log path and completion status in the plan so a replacement can inspect an existing run instead of launching a duplicate. Never overlap edits with verification of those files. Rerun only checks invalidated by later changes; do not repeat a passed full gate for reporting or handoff alone.

## Permissions and tooling

All roles may read/search, use shell and web tools, and load relevant skills. `coordinator` holds coder-equivalent full write access (`edit *`); `orchestrator`, `architect`, `reviewer`, `tester`, and `auditor` edit tools allow only `.planning/**` evidence; `tester` additionally allows edit of scratch and fixture evidence under `.agent-scratch/**` and `**/.agent-scratch/**`; `rust-coder` and `react-coder` edit tools allow source edits (stack-split successors of `coder`). `scribe` edit tools permit `docs/**`, `README.md` files, and `.planning/**`; its role prohibits source/test/config/agent-instruction edits, builds, commits and delegation. It reports required changes outside documentation to the owning stack coder. `utility` permits quick non-code edits to `docs/**`, `README.md`, `.planning/**`, `.agent-scratch/**`, `.opencode/**` and `.agy/**` while denying source, tests (`.opencode/tests/**`), scripts and `.github/**`; it never spawns children. Shell access supports repo commands and is not a filesystem sandbox. Code Mode is enabled so configured MCP discovery tools and worktree/session utilities remain available; nested tools still enforce their own permissions. Browser tools remain denied. Delegation is restricted to the table above. `.opencode/AGENTS.md` is a concise shared guide loaded by `instructions` for all models, alongside root `AGENTS.md`. The project registers the installed `~/.agents/skills/planning-with-files` skill without changing global settings. The coordinator assigns edit tasks to coder agents only when the tasks require changes to Rust or React/JavaScript/TypeScript code; documentation and wiki work primarily goes to scribe, and quick non-code edits may go to `utility`. Assign instruction/config/tooling edits to an explicitly scoped coder or `utility`; workflow changes require Code Architect ownership. Scribe owns maintained documentation, not runtime configuration.

Use the installed `opencode debug config` and initialized `/api/agent` catalog to inspect effective models, prompts and permissions.

References: [official V2 agents](https://opencode.ai/v2/docs/agents), [config](https://opencode.ai/v2/docs/config), [tools](https://opencode.ai/v2/docs/tools). Navigate through [the wiki](../../index.md), tags and scoped source searches. `/docs-review`, `/setup-wiki` and `/refresh-wiki` select `scribe` and use the [maintenance workflow](../maintenance.md).

## Rust style

Any Rust edit or review under `crates/` loads `argos-rust-style` (Google's Rust Style Guide, with Argos overrides). `argos-osint-core` and `argos-osint-api` follow the library rules; `argos-osint-bin` and `argos-api-host` follow the application rules. Tokio async and the existing crate layout stay in force. `orchestrator` passes the skill into Rust phases; `architect` identifies it in the spec, `rust-coder` applies it directly, and `reviewer`/`tester` review diffs against it. The canonical file is `.opencode/skills/argos-rust-style/SKILL.md`; `.agents/skills/argos-rust-style` is the same skill. `cargo fmt` remains the formatter.

## UI/TUI visual verification and fix cycles

For UI/TUI visual verification and fix cycles, the owning coordinator or orchestrator tracks progress against the preceding iteration. After at most three iterations without meaningful progress, stop cycling and automatically defer the affected visual defect: record the concrete remaining defects and evidence as an open item, continue all other feasible implementation, and revisit only after implementation finishes; the coordinator then asks the customer about each open deferred defect. Deferral does not turn an unmet gate into a pass or waive accessibility, functionality, or integrity requirements. Report actual verification status and outstanding acceptance.

## React UI skills

React work uses [React UI rules](react-ui.md), including the [Argos look and feel](react-ui.md#argos-look--feel-reference). Stay on the Argos theme and design components creatively inside it. Topology graphs stay on the force-graph canvas. `argos-react-ui` covers design, accessibility and visual evidence; `argos-react-patterns` covers React Native composition, compiler-compatible hooks, Query state and rendering. All React roles retain the enabled Next Compiler and zero-warning Hooks/compiler lint baseline.

`/react-ui-review <scope>` selects `tester` for a read-only review (default: current UI delta). Frontend verification uses the pinned Node/pnpm, offline Playwright fixtures and inspected screenshots; shell Playwright does not require enabling the denied browser tool. Missing image/tool evidence remains pending.

## React test workflow

Use the same dependency-based topology for React tests as production changes. A small cohesive test change does not need a fresh architecture survey or nested coder layer. Keep real component/hook imports and network-boundary fixtures; update documentation only when its contract changes.

Use the canonical [React component testing contract](react-ui.md#react-component-testing-real-component-pattern) for component imports, isolated offline fixtures, React update handling, token assertions and browser screenshots. Run `pnpm test` in addition to typecheck, lint, compiled build and relevant Playwright checks. A unit-suite pass does not replace browser interaction or inspected visual evidence. Record failures and skips honestly in the named plan; do not institutionalize known flaky tests as accepted exceptions.

### Tester role

The `tester` role executes empirical host-native checks first (`--storage embedded`), then offline `pnpm test:e2e` cases. Tester reports `PASS / FAIL / BLOCKED / PENDING` with tested revision, dirty-diff identity, command exit codes, skipped checks, screenshot inspection and outstanding acceptance.

## Phase handoff and completion

Every dispatch carries this compact envelope; children start with fresh context:

```text
MODE: PHASE | EDIT_UNIT | AUDIT_UNIT | CONSULT | REVIEW | VERIFY | AUDIT | DOC_REVIEW
PLAN_ID / absolute PLAN_DIR / PHASE:
INTEGRATION_ROOT / absolute WORKTREE / BASE_REVISION:
OWNED_FILES: exact paths (read-only references listed separately)
SPEC: file:line refs, contracts, constraints, performance/migration risks
ACCEPTANCE: checks, expected results, evidence paths, user exclusions
RETURN: COMPLETE | NEEDS_DECISION | FAILED | APPROVE | REJECT | PASS | FAIL | BLOCKED | PENDING | CLEAN | INTEGRITY_VIOLATION; changed files and evidence
```

- `scribe` returns `COMPLETE / NEEDS_DECISION / BLOCKED` with reviewed baseline/dirty scope, doc changes, actual checks and gaps; a no-op still requires scope and rationale.
- `reviewer` returns `APPROVE / REJECT` with severity-ranked `file:line` findings.
- `tester` returns `PASS / FAIL / BLOCKED / PENDING` and reports tested revision plus dirty-diff identity, command exit codes, skipped checks, screenshot inspection and outstanding acceptance.
- `auditor` returns `CLEAN / INTEGRITY_VIOLATION` as a binary forensic integrity check on diffs, tests, and evidence.

## Worktree integration

1. The dispatching role records the main checkout as `INTEGRATION_ROOT`, its revision and existing dirty paths. Assign coders disjoint file ownership; shared contracts/lockfiles have one owner. Small sequential edits can use the integration tree; parallel feature units use task-owned worktrees under `.agent-scratch/worktrees/`.
2. Create each worktree from the agreed baseline (`git worktree add --detach <absolute-path> <base>`). Include any required uncommitted phase baseline with a scoped patch; never silently omit dirty prerequisites or copy unrelated edits.
3. Pass absolute `WORKTREE` and `PLAN_DIR` to each coder. Read the shared plan in the integration tree; never create a competing plan in a worker.
4. Each coder implements and formats its assigned changes directly, then returns owned paths, diff and unresolved decisions. The dispatching role waits for all coders and integrates only each unit's delta.
5. Have coders resolve integration conflicts; consult `architect` when contracts change, format the combined tree, and at scheduled documentation checkpoints give `scribe` exclusive ownership of the affected docs while it reviews the integrated delta. Wait for documentation updates before running the verification gate (`reviewer`, `tester`, `auditor`) in `INTEGRATION_ROOT`. Never test in feature worktrees.

Build owners follow [cache retention and disk budgets](contributing.md#build-cache-hygiene) before builds, between large checks and at handoff. Dispatchers include the retained cache and free-space budget in handoffs; only the execution owner performs safe cleanup of obsolete artifacts.

## Verification

Map each phase to [CI contracts](ci.md) and the affected `.github/workflows/` before choosing commands. Frontend phases include strict compiler/export and development browser checks; Rust phases start with targeted checks and use the embedded local gate when broad integration acceptance requires it. Respect Code Architect ownership for workflow edits and report hosted Actions separately from local evidence.

Rust: never run the all-platform or `server` stack from `.github/workflows/ci.yml` locally. Agents must stabilize the **local host machine platform** (`uname -ms`, `rustc -vV | grep '^host:'`) with the embedded defaults first (`ARGOS_EMBED` unset). Prefer targeted crate/binary commands during scoped phase checks (`python3 scripts/agent_cargo.py check|clippy|test --locked -p <crate> [--lib | --bin <bin> | --test <name>]`, and `ARGOS_SURREAL_MODE=embedded ARGOS_SURREAL_REQUIRED=1 python3 scripts/agent_cargo.py test --locked -p argos-osint-core --test storage_<name>` for individual storage contract suites; see [contributing.md](contributing.md#targeted-local-rust-builds-and-tests-fast-iteration)). Run the full `scripts/local_matrix.sh --storage embedded` gate in `INTEGRATION_ROOT` when broad workspace/storage verification is required, serializing Cargo and following the 1–2 full runs / maximum 3 cap. Config-only changes use `node --test .opencode/tests/*.test.mjs` plus runtime checks, without a Rust build.
