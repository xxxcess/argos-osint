# OpenCode agent team

[Code route](index.md) · [Wiki](../../index.md) · [Canonical agent workflow](agent-workflow.md)

This page describes the explicitly selected OpenCode team profile. Its role/model/edit restrictions do not replace another host's active instructions. Codex, Antigravity and other hosts use the canonical workflow with their own available roles and tools. Keep OpenCode's configured role boundaries when that profile is invoked.

`coordinator` (Team Coordinator) is the default entry-point agent. All active catalog archetypes (`coordinator`, `orchestrator`, `architect`, `rust-coder`, `react-coder`, `reviewer`, `tester`, `auditor`, `scribe`, `utility`) use `mode: all`, so they can run as primary agents or children. Legacy `build` and `plan` agents are disabled and the old `general`/`explore` stubs were removed; ECC agent definitions are replaced. `/ecc-plan` selects `architect`, `/ecc-review` selects `reviewer`, `/ecc-verify` selects `tester`, and `/docs-review`, `/setup-wiki`, and `/refresh-wiki` select `scribe`.

| Caller | Permitted children | Configured model | Purpose |
| --- | --- | --- | --- |
| `coordinator` | `orchestrator`, `architect`, `rust-coder`, `react-coder`, `reviewer`, `tester`, `auditor`, `scribe` | `agents.<role>.model` | Default entry point: clarify material unknowns, preserve scope/constraints, and dispatch the smallest effective team |
| `orchestrator` | `architect`, `rust-coder`, `react-coder`, `reviewer`, `tester`, `auditor`, `scribe` | `agents.<role>.model` | Decompose multi-phase/multi-milestone work, own `.planning/`, coordinate tier-appropriate specialists, and drive iterative Scribe checkpoints while deferring edits, tests, builds, and substantive reviews to appropriate roles |
| `architect` | none | `agents.<role>.model` | Investigate code/docs directly, advise on contracts, and write `file:line` phase specs |
| `rust-coder` | none | `agents.<role>.model` | Investigate and implement assigned changes directly; return consultation, documentation and verification needs to the owning role; consult architect only for material unresolved contracts or architecture decisions; existing grounded scope is sufficient to proceed. |
| `react-coder` | none | `agents.<role>.model` | Investigate and implement assigned changes directly; return consultation, documentation and verification needs to the owning role; consult architect only for material unresolved contracts or architecture decisions; existing grounded scope is sufficient to proceed. |
| `reviewer` | `architect` | `agents.<role>.model` | Audit integrated diffs for correctness, concurrency/performance, `argos-rust-style`, and React UI compliance (`APPROVE \| REJECT`) |
| `tester` | `architect` | `agents.<role>.model` | Execute empirical host-native checks, TUI/UI screenshot verification, and adversarial stress tests in `INTEGRATION_ROOT` (`PASS \| FAIL \| BLOCKED \| PENDING`) |
| `auditor` | none | `agents.<role>.model` | Forensic integrity and anti-cheating leaf audit (`CLEAN \| INTEGRITY_VIOLATION` binary veto) |
| `utility` | none | `agents.<role>.model` | Quick general non-code edits (documentation, configuration, instructions); leaf role with no source/test/CI edit rights |
| `scribe` | none | `agents.<role>.model` | Review code deltas and maintain canonical documentation, tags, and diagrams |

## Scope and team selection

Follow the canonical [scope and finish line](agent-workflow.md#scope-and-finish-line); existing authorization is sufficient. Ask only for missing material decisions, defer independent improvements and stop new work at handoff.

Choose the smallest useful team: direct specialist for a coherent task; focused architect consultation for an unresolved contract; orchestrator with disjoint specialists for independent areas. Delegate early when it shortens the critical path, with one build owner. Do not add a new approval ceremony or repeat the role chain for routine fixes.

[Agent handoffs](../../diagrams/agent-team.html) retain the profile's responsibilities: architects investigate directly using `AUDIT_UNIT` or advise through `CONSULT`; stack coders implement and return docs/review/verification needs without children. The owning coordinator/orchestrator integrates and dispatches scribe/reviewer/tester/auditor. Only the plan owner edits shared status during delegation. Final code acceptance remains `reviewer: APPROVE + tester: PASS + auditor: CLEAN`; demonstrated failures return to their owner. Plan-only requests stop before implementation; user exclusions override routine verification defaults.

## Models

`.opencode/opencode.json` (`agents.<role>.model`) is the source of truth for role models. Do not copy model IDs into role prompts or this guide: those copies drift when the customer changes configuration. Preserve configured choices unless a model change is explicitly requested.

Only `AUDIT_UNIT` exploration uses `.opencode/architect-worker.json`; stack coders use their configured role models. Tester, reviewer, auditor and scribe use their role models. Explicit user task overrides win. Pass the effective model on new/resumed calls where supported; unavailable models are reported without silent substitution.

### Task-wide provider concurrency guidance

- Across the entire task, cap concurrently active `architect`, `rust-coder`, and `react-coder` agents at two for `openai` and separately two for `nvidia`, including all such descendants across branches.
- Determine the provider from the prefix before the first slash of the effective model after overrides; an OpenRouter-hosted NVIDIA model counts as `openrouter`, not `nvidia`. Other providers are unaffected.
- Count active parents even while waiting for children, and count resumed invocations while active. Coordinate counts across branches; queue excess calls until a slot is free. Schedule nested calls so slots can be released and deadlocks are avoided.
- Exclude `coordinator`, `orchestrator`, `reviewer`, `tester`, `auditor`, and `scribe` themselves, but not their `architect` or coder descendants. This is operational guidance, not a runtime semaphore.
- When any dispatched agent cannot respond due to provider usage/limit exhaustion, do not change that agent's configured model target. Preserve completed work per repo recovery rules, stop the unit cleanly, and escalate to the coordinator so the customer decides how to proceed. No implicit fallback to another provider or model ID.

## Delegation and continuation

`experimental.subagent_depth: 4` supports nested dispatch within the table above; architects, coders, auditors and scribes retain their leaf restrictions. Use the [canonical workflow](agent-workflow.md) for ownership, useful parallel work, explicit worker wakeups, one build owner and unaffected evidence reuse.

A `sessionID` belongs to the calling parent: resume only a child returned by that parent's successful call, using the same role. Never copy sibling, ancestor or previous-root IDs. Keep the mapping in a local runtime ledger. For an unknown/foreign-session error, confirm the former worker stopped editing, then start a fresh child with current scope/evidence. Report model/permission/depth failures; do not claim success, substitute a model or bypass role boundaries through direct edits/builds.

## Permissions and tooling

All roles may read/search, use shell and web tools, and load relevant skills. `coordinator` holds coder-equivalent full write access (`edit *`); `orchestrator`, `architect`, `reviewer`, `tester`, and `auditor` edit tools allow only `.planning/**` evidence; `tester` additionally allows edit of scratch and fixture evidence under `.agent-scratch/**` and `**/.agent-scratch/**`; `rust-coder` and `react-coder` edit tools allow source edits (stack-split successors of `coder`). `scribe` edit tools permit `docs/**`, `README.md` files, and `.planning/**`; its role prohibits source/test/config/agent-instruction edits, builds, commits and delegation. It reports required changes outside documentation to the owning stack coder. `utility` permits quick non-code edits to `docs/**`, `README.md`, `.planning/**`, `.agent-scratch/**`, `.opencode/**` and `.agy/**` while denying source, tests (`.opencode/tests/**`), scripts and `.github/**`; it never spawns children. Shell access supports repo commands and is not a filesystem sandbox. Code Mode is enabled so configured MCP discovery tools and worktree/session utilities remain available; nested tools still enforce their own permissions. Browser tools remain denied. Delegation is restricted to the table above. `.opencode/AGENTS.md` is a concise shared guide loaded by `instructions` for all models, alongside root `AGENTS.md`. The project registers the installed `~/.agents/skills/planning-with-files` skill without changing global settings. The coordinator assigns edit tasks to coder agents only when the tasks require changes to Rust or React/JavaScript/TypeScript code; documentation and wiki work primarily goes to scribe, and quick non-code edits may go to `utility`. Assign instruction/config/tooling edits to an explicitly scoped coder or `utility`; `.github` CI workflow edits remain Code Architect-owned. Reversible guidance/tooling improvements already authorized by the user may proceed through the assigned role within its scope; no extra approval ceremony is needed. Scribe owns maintained documentation, not runtime configuration.

Use the installed `opencode debug config` and initialized `/api/agent` catalog to inspect effective models, prompts and permissions.

References: [official V2 agents](https://opencode.ai/v2/docs/agents), [config](https://opencode.ai/v2/docs/config), [tools](https://opencode.ai/v2/docs/tools). Navigate through [the wiki](../../index.md), tags and scoped source searches. `/docs-review`, `/setup-wiki` and `/refresh-wiki` select `scribe` and use the [maintenance workflow](../maintenance.md).

## Rust style

Any Rust edit or review under `crates/` loads `argos-rust-style` (Google's Rust Style Guide, with repository overrides). `argos-osint-core` and `argos-osint-api` follow the library rules; `argos-osint-bin` and `argos-api-host` follow the application rules. Tokio async and the existing crate layout stay in force. `orchestrator` passes the skill into Rust phases; `architect` identifies it in the spec, `rust-coder` applies it directly, and `reviewer`/`tester` review diffs against it. The canonical file is `.opencode/skills/argos-rust-style/SKILL.md`; `.agents/skills/argos-rust-style` is the same skill. `cargo fmt` remains the formatter.

## UI/TUI visual verification

Follow the [canonical evidence workflow](agent-workflow.md) and [TUI verification](tui-verification.md): inspect actual previews and retain matching provenance; missing visual, accessibility, functionality or integration evidence remains open.

## React UI skills

React work uses [React UI rules](react-ui.md), including the [Fleetworks look and feel](react-ui.md#fleetworks-look-and-feel). Stay on the Fleetworks theme and design components creatively inside it. Topology graphs stay on the force-graph canvas. `argos-react-ui` covers design, accessibility and visual evidence; `argos-react-patterns` covers React Native composition, compiler-compatible hooks, Query state and rendering. All React roles retain the enabled Next Compiler and zero-warning Hooks/compiler lint baseline.

`/react-ui-review <scope>` selects `tester` for a read-only review (default: current UI delta). Frontend verification uses the pinned Node/pnpm, offline Playwright fixtures and inspected screenshots; shell Playwright does not require enabling the denied browser tool. Missing image/tool evidence remains pending.

## React test workflow

Use the canonical [React component testing contract](react-ui.md#react-component-testing-real-component-pattern) for component imports, isolated offline fixtures, React update handling, token assertions and browser screenshots. Run `pnpm test` in addition to typecheck, lint, compiled build and relevant Playwright checks. A unit-suite pass does not replace browser interaction or inspected visual evidence. Record failures and skips honestly in the named plan; do not institutionalize known flaky tests as accepted exceptions.

### Tester role

The `tester` role executes empirical host-native checks first (`--storage embedded`), then offline `pnpm test:e2e` cases. Tester reports `PASS / FAIL / BLOCKED / PENDING` with tested revision, dirty-diff identity, command exit codes, skipped checks, screenshot inspection and outstanding acceptance.

## Phase handoff and completion

Every dispatch carries this compact envelope; resume existing workers/evidence when supported rather than rediscovering the task:

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

Record `INTEGRATION_ROOT`, base revision and dirty prerequisites before dispatch. Give coders disjoint ownership; use task-owned worktrees under `.agent-scratch/worktrees/` only when isolation helps. Carry required uncommitted prerequisites through a scoped patch, preserve unrelated edits and integrate only the assigned delta. Coders resolve conflicts; consult architect for changed contracts. Keep one shared-plan owner and one build owner. Worktrees are edit-only; final review/test/audit runs in `INTEGRATION_ROOT` under the [canonical workflow](agent-workflow.md) and [cache policy](contributing.md#build-cache-hygiene).

## Verification

Map each phase to [CI contracts](ci.md) and the affected `.github/workflows/` before choosing commands. Frontend phases include strict compiler/export and development browser checks; Rust phases start with targeted checks and use the embedded local gate when broad integration acceptance requires it. Respect Code Architect ownership for workflow edits and report hosted Actions separately from local evidence.

Follow the [canonical workflow](agent-workflow.md) and [contributor commands](contributing.md#targeted-local-rust-builds-and-tests-fast-iteration) for the stable build graph, resource limits, focused fixtures and evidence reuse. One build owner records the execution recipe; missing or skipped checks are not passes.

Rust verification targets the native host and embedded storage with `ARGOS_EMBED` unset. The required final `scripts/local_matrix.sh --storage embedded` gate runs in `INTEGRATION_ROOT` after relevant inputs stabilize. Fix demonstrated failures with focused reruns before another broad run; an arbitrary attempt cap never waives required acceptance. Local mutation tests/baselines, server storage and all-platform matrices remain excluded; hosted evidence is reported separately.

Config-only changes use the relevant `node --test .opencode/tests/*.test.mjs` and runtime checks without a Rust rebuild. Guidance-only changes use scoped link/policy checks. Keep improvements within the agreed scope and role ownership.
