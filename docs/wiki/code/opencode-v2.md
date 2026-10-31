# OpenCode agent team

[Code route](index.md) · [Wiki](../../index.md)

`coordinator` (Team Coordinator) is the default entry-point agent (`opencode-go/glm-5.3-flash#high`). All active catalog archetypes (`coordinator`, `orchestrator`, `architect`, `rust-coder`, `react-coder`, `reviewer`, `tester`, `auditor`, `scribe`) use `mode: all`, so they can run as primary agents or children. Legacy `build` and `plan` agents are disabled and the old `general`/`explore` stubs were removed; ECC agent definitions are replaced. `/ecc-plan` selects `architect`, `/ecc-review` selects `reviewer`, `/ecc-verify` selects `tester`, and `/docs-review`, `/setup-wiki`, and `/refresh-wiki` select `scribe`.

| Caller | Permitted children | Configured model | Purpose |
| --- | --- | --- | --- |
| `coordinator` | `orchestrator`, `architect`, `rust-coder`, `react-coder`, `reviewer`, `tester`, `auditor`, `scribe` | `opencode-go/glm-5.3-flash#high` | Default entry point: conduct a concise task interview, lock scope/constraints, and dispatch the most efficient team from the catalog |
| `orchestrator` | `architect`, `rust-coder`, `react-coder`, `reviewer`, `tester`, `auditor`, `scribe` | `opencode-go/glm-5.3-flash#high` | Decompose multi-phase/multi-milestone work, own `.planning/`, coordinate tier-appropriate specialists, and drive iterative Scribe checkpoints while deferring edits, tests, builds, and substantive reviews to appropriate roles |
| `architect` | `architect`, `rust-coder`, `react-coder`, `reviewer`, `tester`, `auditor`, `scribe` | `opencode-go/kimi-k3.1-pro#high` | Explore code/docs (`AUDIT_UNIT`), advise on contracts (`CONSULT`), write `file:line` phase specs, or coordinate single-phase work |
| `rust-coder` | `architect`, `rust-coder`, `react-coder`, `reviewer`, `tester`, `auditor`, `scribe` | `opencode-go/glm-5.3#high` | Delegate disjoint stack-split `EDIT_UNIT` edits (operational policy: only EDIT_UNIT children spawned; verification/CONSULT returned to owning orchestrator/architect/coordinator). Integrate worktree deltas |
| `react-coder` | `architect`, `rust-coder`, `react-coder`, `reviewer`, `tester`, `auditor`, `scribe` | `opencode-go/glm-5.3#high` | Delegate disjoint stack-split `EDIT_UNIT` edits (operational policy: only EDIT_UNIT children spawned; verification/CONSULT returned to owning orchestrator/architect/coordinator). Integrate worktree deltas |
| `reviewer` | `architect` | `openrouter/nvidia/nemotron-3-ultra-550b-a55b:free#high` | Audit integrated diffs for correctness, concurrency/performance, `argos-rust-style`, and React UI compliance (`APPROVE \| REJECT`) |
| `tester` | `architect` | `openrouter/nemotron-3.5-super-VL-free#max` | Execute empirical host-native checks, TUI/UI screenshot verification, and adversarial stress tests in `INTEGRATION_ROOT` (`PASS \| FAIL \| BLOCKED \| PENDING`) |
| `auditor` | none | `openrouter/nvidia/nemotron-3-ultra-550b-a55b:free#high` | Forensic integrity and anti-cheating leaf audit (`CLEAN \| INTEGRITY_VIOLATION` binary veto) |
| `scribe` | none | `openrouter/nvidia/nemotron-3-ultra-550b-a55b:free#high` | Review code deltas and maintain canonical documentation, tags, and diagrams |

## Concise Interview, Customer Translation & Dynamic Team Selection

Before spinning up subagents (unless `[skip-interview]` is passed or the prompt is a trivial one-step query/commit), `coordinator` uses the interactive user ask tool (`question`) to run a concise 1–3 question interview with `[Default]` recommendations and `(OPEN)` options confirming scope/deliverables, restrictions/exclusions, and verification depth. Whenever subagents return `NEEDS_DECISION` or blockers during execution, `coordinator` translates the team's technical needs into a concise customer-facing question via the user ask tool. Once the task prompt is approved, `coordinator` selects the leanest catalog topology:

1. **Tier 1 — Direct Specialist (1 subagent)**:
   - Plan/architecture spec $\to$ `architect`; diff review $\to$ `reviewer`; UI/TUI verification $\to$ `tester`; wiki maintenance $\to$ `scribe`; authorized commit $\to$ `rust-coder`/`react-coder`.
2. **Tier 2 — Fast-Path Squad ($\le 3$ files, single crate/package)**:
   - `rust-coder`/`react-coder` implements and formats changes $\to$ `scribe` updates affected docs $\to$ verification gate (`reviewer` + `tester` + `auditor`) validates `INTEGRATION_ROOT`.
3. **Tier 3 — Full Project Orchestration ($> 3$ files, cross-crate, or multi-milestone)**:
   - `coordinator` dispatches `orchestrator`, which runs parallel `architect` (`AUDIT_UNIT`) explorers, dispatches `rust-coder`/`react-coder` (managing disjoint `EDIT_UNIT` worktrees), schedules `scribe` at phase checkpoints, and gates each milestone on `reviewer: APPROVE`, `tester: PASS`, and `auditor: CLEAN`.

[Agent handoffs](../../diagrams/agent-team.html) follow one phase/milestone at a time. `orchestrator` or `architect` uses `planning-with-files`, the wiki, and current source to write the spec with exact files, contracts, performance/migration risks, and acceptance, and owns shared planning files. `rust-coder`/`react-coder` assigns disjoint `EDIT_UNIT` children, waits for formatted edits, and returns all verification, documentation, and CONSULT needs to the owning `orchestrator`/`architect`/`coordinator` for dispatch. The owning role schedules `scribe`, `reviewer`, `tester`, and `auditor` at appropriate milestones. On any gate failure, `orchestrator`/`architect` synthesizes exact `file:line` remediation items and resumes `rust-coder`/`react-coder` for the next iteration. Plan-only requests stop before implementation. User exclusions override routine build/test defaults.

`architect` exploration children use `AUDIT_UNIT` mode; decision calls use `CONSULT`. Both are read-only leaf modes. `EDIT_UNIT` children dispatched by stack coders edit/format only their assigned files and never delegate/build/test. `auditor` and `scribe` are strict leaf roles. Every handoff carries `PLAN_ID`, exact scope, references, and acceptance; only the owning `orchestrator` or `architect` updates shared plan files.

## Models

Role models and reasoning variants live in `.opencode/opencode.json` at `agents.<role>.model`:

- **Default Team Coordinator (`coordinator`)**: `opencode-go/glm-5.3-flash#high` for high-reasoning interactive interviewing and triage.
- **Architecture (`architect`)**: `opencode-go/kimi-k3.1-pro#high` for architectural specifications and contract consultations.
- **Orchestration (`orchestrator`)**: `opencode-go/glm-5.3-flash#high`.
- **Implementation coordination (`rust-coder`, `react-coder`)**: `opencode-go/glm-5.3#high`.
- **Review, Forensic Audit & Documentation (`reviewer`, `auditor`, `scribe`)**: `openrouter/nvidia/nemotron-3-ultra-550b-a55b:free#high`. **Caveat**: repo evidence (`docs/archive/brief-request-mode-test-report.md`) shows the OpenRouter `:free` endpoint returning 404s for this model — treat as a known availability risk.
`opencode-go/glm-5.3-flash` (like `glm-5.3`) exposes effort tiers `low | high | max` only — there is no `medium`; requests for GLM 'medium' map to `#high`, the middle tier.
- **Verification (`tester`)**: `openrouter/nemotron-3.5-super-VL-free#max`.
- **Fast `EDIT_UNIT` & Worker Overrides**: `opencode-go/glm-5.3-flash#high` in `.opencode/coder-rust-worker.json` and `.opencode/coder-react-worker.json`; `opencode/big-pickle` (no variant) in `.opencode/architect-worker.json`.

Set exploration and editing child defaults independently:

- `.opencode/architect-worker.json`: `architect`'s and `orchestrator`'s `AUDIT_UNIT` exploration children (`opencode/big-pickle`).
- `.opencode/coder-rust-worker.json` and `.opencode/coder-react-worker.json`: `EDIT_UNIT` coders spawned by each stack coder (both `opencode-go/glm-5.3-flash#high`; verbatim ID); excludes `scribe`, `reviewer`, and `auditor`. Stack coders do not spawn `tester`, `reviewer`, `auditor`, or `architect` CONSULT children; those needs are returned to the owning orchestrator/architect/coordinator. Rust/react-coder spawned `EDIT_UNIT` children use the `glm-5.3-flash#high` worker overrides from these two files.

Each file contains `{"model": "provider/model#variant"}` (omit the `#variant` suffix for models without variants, such as `opencode/big-pickle`). The caller reads it and supplies `subagent.model` on every new/resumed worker call. An explicit user task override wins. For `scribe`, `reviewer`, and `auditor`, callers read and explicitly pass `agents.<role>.model` from `.opencode/opencode.json`. The child keeps its role prompt/permissions; only its model changes. Unavailable models/variants are reported without silent substitution.

### Task-wide provider concurrency guidance

- Across the entire task, cap concurrently active `architect`, `rust-coder`, and `react-coder` agents at two for `openai` and separately two for `nvidia`, including all such descendants across branches.
- Determine the provider from the prefix before the first slash of the effective model after overrides; `openrouter/nvidia/...` is `openrouter`, not `nvidia`. Other providers are unaffected.
- Count active parents even while waiting for children, and count resumed invocations while active. Coordinate counts across branches; queue excess calls until a slot is free. Schedule nested calls so slots can be released and deadlocks are avoided.
- Exclude `coordinator`, `orchestrator`, `reviewer`, `tester`, `auditor`, and `scribe` themselves, but not their `architect` or coder descendants. This is operational guidance, not a runtime semaphore.

## Delegation and continuation

`experimental.subagent_depth: 4` supports `coordinator` $\to$ `orchestrator` $\to$ `rust-coder`/`react-coder` $\to$ `EDIT_UNIT` children. Tester, reviewer, auditor, and architect CONSULT dispatch are owned by the parent; coders return those needs without spawning leaf roles. Leaf instructions prevent recursive workflow spawning.

A `sessionID` belongs to the calling parent: resume only a child returned by your own successful subagent call, using the same role. Never copy sibling, ancestor or previous-root child IDs into a new parent. Keep the parent/child mapping in its own local ledger; it is runtime state, not a durable spec.

If continuation returns unknown/foreign-session errors, confirm the old worker is no longer editing, then start a fresh child without `sessionID` and include the current spec, completed work and remaining scope. For model/permission/depth failures, report the exact diagnostic and stop that handoff for configuration repair. Do not claim success or fall back to direct edits/builds in the wrong role. Background children notify parents; do not poll or overlap owned edits with verification.

## Permissions and tooling

All roles may read/search, use shell and web tools, and load relevant skills. `coordinator` holds coder-equivalent full write access (`edit *`); `orchestrator`, `architect`, `reviewer`, `tester`, and `auditor` edit tools allow only `.planning/**` evidence; `tester` additionally allows edit of scratch and fixture evidence under `.agent-scratch/**` and `**/.agent-scratch/**`; `rust-coder` and `react-coder` edit tools allow source edits (stack-split successors of `coder`). `scribe` edit tools permit `docs/**`, `README.md` files, and `.planning/**`; its role prohibits source/test/config/agent-instruction edits, builds, commits and delegation. It reports required changes outside documentation to the owning stack coder. Shell access supports repo commands and is not a filesystem sandbox. Code Mode is enabled so configured MCP discovery tools and worktree/session utilities remain available; nested tools still enforce their own permissions. Browser tools remain denied. Delegation is restricted to the table above. `.opencode/AGENTS.md` is a concise shared guide loaded by `instructions` for all models, alongside root `AGENTS.md`. The project registers the installed `~/.agents/skills/planning-with-files` skill without changing global settings. The coordinator assigns edit tasks to coder agents only when the tasks require changes to Rust or React/JavaScript/TypeScript code; documentation and wiki work primarily goes to scribe. If an instruction/config edit is outside scribe permissions, report the blocker rather than assigning a coder.

Use the installed `opencode debug config` and initialized `/api/agent` catalog to inspect effective models, prompts and permissions.

References: [official V2 agents](https://opencode.ai/v2/docs/agents), [config](https://opencode.ai/v2/docs/config), [tools](https://opencode.ai/v2/docs/tools). Navigate through [the wiki](../../index.md), tags and scoped source searches. `/docs-review`, `/setup-wiki` and `/refresh-wiki` select `scribe` and use the [maintenance workflow](../maintenance.md).

## Rust style

Any Rust edit or review under `crates/` loads `argos-rust-style` (Google's Rust Style Guide, with Argos overrides). `argos-osint-core` and `argos-osint-api` follow the library rules; `argos-osint-bin` and `argos-api-host` follow the application rules. Tokio async and the existing crate layout stay in force. `orchestrator`/`architect` pass the skill into Rust phases, `rust-coder` passes it to Rust `EDIT_UNIT`s, and `reviewer`/`tester` review diffs against it. The canonical file is `.opencode/skills/argos-rust-style/SKILL.md`; `.agents/skills/argos-rust-style` is the same skill. `cargo fmt` remains the formatter.

## React UI skills

React work uses [React UI rules](react-ui.md), including the [Argos look and feel](react-ui.md#argos-look--feel-reference). Stay on the Argos theme and design components creatively inside it. Topology graphs stay on the force-graph canvas. `argos-react-ui` covers design, accessibility and visual evidence; `argos-react-patterns` covers React Native composition, compiler-compatible hooks, Query state and rendering. All React roles retain the enabled Next Compiler and zero-warning Hooks/compiler lint baseline.

`/react-ui-review <scope>` selects `tester` for a read-only review (default: current UI delta). Frontend verification uses the pinned Node/pnpm, offline Playwright fixtures and inspected screenshots; shell Playwright does not require enabling the denied browser tool. Missing image/tool evidence remains pending.

## React test workflow

This project uses Vitest with React Testing Library and MSW for unit testing shared React UI components across `packages/ui`, `packages/intel`, and `packages/api-client`. The test workflow is:

- **Tier 1 — Direct Specialist**: Single test file or small contract change. Coder implements test patterns, Scribe updates affected docs, verification gate (`reviewer: APPROVE`, `tester: PASS`, `auditor: CLEAN`).
- **Tier 2 — Fast-Path Squad**: ≤3 test files in a single package. React coder writes real-component tests importing from `@argos/ui`, `@argos/intel`, or `@argos/api-client`. Scribe documents testing patterns, gate validates INTEGRATION_ROOT.
- **Tier 3 — Full Project Orchestration**: Multi-package or cross-crate test changes. Orchestrator dispatches parallel architects, coders manage disjoint EDIT_UNIT worktrees, Scribe updates affected docs at checkpoints, and gates each milestone on `reviewer: APPROVE`, `tester: PASS`, and `auditor: CLEAN`.

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

1. The owning stack coder (`rust-coder` or `react-coder`) records the main checkout as `INTEGRATION_ROOT`, its revision and existing dirty paths. Assign disjoint file ownership; shared contracts/lockfiles have one owner. Small sequential edits can use the integration tree; parallel feature units use task-owned worktrees under `.agent-scratch/worktrees/`.
2. Create each worktree from the agreed baseline (`git worktree add --detach <absolute-path> <base>`). Include any required uncommitted phase baseline with a scoped patch; never silently omit dirty prerequisites or copy unrelated edits.
3. Pass absolute `WORKTREE` and `PLAN_DIR` to the child. Read the shared plan in the integration tree; never create a competing plan in a worker.
4. `EDIT_UNIT` children implement and format only. Return owned paths, diff and unresolved decisions. The owning stack coder waits for all editors, inspects changes, and integrates only each unit's delta.
5. Resolve integration conflicts through `architect` when contracts change, format the combined tree, and at scheduled documentation checkpoints give `scribe` exclusive ownership of the affected docs while it reviews the integrated delta. Wait for documentation updates before running the verification gate (`reviewer`, `tester`, `auditor`) in `INTEGRATION_ROOT`. Never test in feature worktrees.

## Verification

Map each phase to [CI contracts](ci.md) and the affected `.github/workflows/` before choosing commands. Frontend phases include strict compiler/export and development browser checks; Rust phases use the embedded local gate. Respect Code Architect ownership for workflow edits and report hosted Actions separately from local evidence.

Rust: never run the all-platform or `server` stack from `.github/workflows/ci.yml` locally. Agents must stabilize the **local host machine platform** (`uname -ms`, `rustc -vV | grep '^host:'`) with the embedded defaults first (`ARGOS_EMBED` unset). Prefer targeted crate/binary commands during scoped phase checks (`python3 scripts/agent_cargo.py check|clippy|test --locked -p <crate> [--lib | --bin <bin> | --test <name>]`, and `ARGOS_SURREAL_MODE=embedded ARGOS_SURREAL_REQUIRED=1 python3 scripts/agent_cargo.py test --locked -p argos-osint-core --test storage_<name>` for individual storage contract suites; see [contributing.md](contributing.md#targeted-local-rust-builds-and-tests-fast-iteration)). Run the full `scripts/local_matrix.sh --storage embedded` gate in `INTEGRATION_ROOT` when broad workspace/storage verification is required, serializing Cargo and following the 1–2 full runs / maximum 3 cap. Config-only changes use `node --test .opencode/tests/*.test.mjs` plus runtime checks, without a Rust build.
