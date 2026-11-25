# OpenCode agent team

[Code route](index.md) · [Wiki](../../index.md) · [Canonical agent workflow](agent-workflow.md)

Applies only when the OpenCode team profile is selected; other hosts use the canonical workflow with their own roles and tools.

`coordinator` is the default entry point. All 10 active archetypes run with `mode: all` (primary or child). Legacy `build`/`plan` are disabled; `general`/`explore` and old ECC agents are removed. Slash routing: `/ecc-plan` → `architect`, `/ecc-review` → `reviewer`, `/ecc-verify` → `tester`, `/docs-review`/`/setup-wiki`/`/refresh-wiki` → `scribe`, `/react-ui-review` → `tester`.

| Role | Permitted children | Model | Purpose |
| --- | --- | --- | --- |
| `coordinator` | `orchestrator`, `architect`, `rust-coder`, `react-coder`, `reviewer`, `tester`, `auditor`, `scribe` | `agents.<role>.model` | Intake, clarify material unknowns, preserve scope, dispatch smallest team |
| `orchestrator` | `architect`, `rust-coder`, `react-coder`, `reviewer`, `tester`, `auditor`, `scribe` | `agents.<role>.model` | Multi-phase decomposition, `.planning/` owner, coordinates specialists/Scribe; no code/test/review |
| `architect` | none | `agents.<role>.model` | Direct investigation (`AUDIT_UNIT`), contract advice (`CONSULT`), `file:line` phase specs |
| `rust-coder`, `react-coder` | none | `agents.<role>.model` | Implement assigned stack changes directly; consult `architect` only for material unresolved contracts; return docs/review/verification needs |
| `reviewer` | `architect` | `agents.<role>.model` | Audit integrated diffs for correctness, concurrency/perf, Rust/React/Rune rules (`APPROVE \| REJECT`) |
| `tester` | `architect` | `agents.<role>.model` | Host-native checks, visual/browser verification, stress tests in `INTEGRATION_ROOT` (`PASS \| FAIL \| BLOCKED \| PENDING`) |
| `auditor` | none | `agents.<role>.model` | Forensic anti-cheating leaf audit (`CLEAN \| INTEGRITY_VIOLATION` binary veto) |
| `utility` | none | `agents.<role>.model` | Quick non-code edits (docs/config/instructions); no source/test/CI edits |
| `scribe` | none | `agents.<role>.model` | Maintain canonical docs, tags, and paired diagrams |

## Scope and team selection

Follow [scope and finish line](agent-workflow.md#scope-and-finish-line) and [Agent handoffs](../../diagrams/agent-team.html). Pick the smallest team: direct specialist for cohesive work, `architect` for unresolved contracts, `orchestrator` + disjoint specialists for independent areas. Delegate early with one build owner; only the plan owner edits shared status. Final code gate: `reviewer: APPROVE` + `tester: PASS` + `auditor: CLEAN`. Plan-only stops before implementation; user exclusions override routine defaults.

## Models

`.opencode/opencode.json` (`agents.<role>.model`) is the single source of truth—never copy model IDs into prompts or docs. `AUDIT_UNIT` uses `.opencode/architect-worker.json`; user task overrides win. Pass effective models on calls where supported; never silently substitute unavailable models.

### Task-wide provider concurrency guidance

- Cap concurrently active `architect`, `rust-coder`, and `react-coder` agents (including descendants and waiting/resumed calls) at **2** for `openai` and **2** for `nvidia` (provider = prefix before first `/`; OpenRouter-hosted NVIDIA counts as `openrouter`).
- Exclude top-level `coordinator`, `orchestrator`, `reviewer`, `tester`, `auditor`, and `scribe` from counts (not their `architect`/coder descendants). Queue excess work to avoid deadlocks.
- On provider limit exhaustion, keep the configured model, preserve completed work, stop the unit cleanly, and escalate to `coordinator`—no implicit fallback.

## Delegation and continuation

`experimental.subagent_depth: 4` supports the child matrix above while keeping leaf restrictions. A `sessionID` belongs strictly to its calling parent and role (track in a local ledger; never copy sibling/ancestor/prior-root IDs). On a foreign-session error, confirm the prior worker stopped editing and start a fresh child with current scope/evidence. Report model/permission/depth failures honestly.

## Permissions and tooling

All roles may read/search, use shell/web tools, and load skills; browser tools stay denied; Code Mode enables MCP discovery/session utilities while enforcing nested permissions. Shell access is not a filesystem sandbox—respect role boundaries:

- **Full edit (`edit *`):** `coordinator`, `rust-coder`, `react-coder` (assign code edits only to stack coders).
- **`.planning/**` only:** `orchestrator`, `architect`, `reviewer`, `tester`, `auditor` (`tester` also edits `.agent-scratch/**` and `**/.agent-scratch/**`).
- **Docs only (`scribe`):** `docs/**`, `README.md`, `.planning/**` (no source/test/config/instruction edits, builds, commits, or delegation).
- **Quick non-code (`utility`):** `docs/**`, `README.md`, `.planning/**`, `.agent-scratch/**`, `.opencode/**`, `.agy/**`; denies source, `.opencode/tests/**`, `scripts/**`, `.github/**` (`.github` is Code Architect-owned).

`.opencode/AGENTS.md` loads via `instructions` alongside root `AGENTS.md`, and `~/.agents/skills/planning-with-files` is registered locally. Inspect effective settings with `opencode debug config` and `/api/agent`. Refs: [V2 agents](https://opencode.ai/v2/docs/agents), [config](https://opencode.ai/v2/docs/config), [tools](https://opencode.ai/v2/docs/tools), [wiki](../../index.md), [maintenance](../maintenance.md).

## Stack skills and visual verification

- **Rust style:** Every edit/review under `crates/` loads `argos-rust-style` (`.opencode/skills/argos-rust-style/SKILL.md` / `.agents/skills/argos-rust-style`): library rules for `argos-osint-core`/`argos-osint-api`, app rules for `fleetworks-desktop`/`fleetworks-search-index`/`argos-api-host`, Tokio async, existing crate layout, and `cargo fmt`.
- **Native visuals:** Follow [agent-workflow](agent-workflow.md) and [native verification](native-verification.md); inspect actual captures with matching provenance.

## React UI skills

Follow [React UI rules](react-ui.md) ([Fleetworks look and feel](react-ui.md#fleetworks-look-and-feel)) and keep `ForceCanvas` + its accessible list twin for topology. Load `argos-react-ui` (design/a11y/visuals) and `argos-react-patterns` (RN composition, compiler-safe hooks, Query, perf), preserving Next Compiler and zero-warning lint. `/react-ui-review <scope>` runs `tester` read-only with pinned Node/pnpm and offline Playwright (shell Playwright works with the browser tool denied).

Follow the [React component testing contract](react-ui.md#react-component-testing-real-component-pattern): run `pnpm test`, typecheck, lint, compiled build, and relevant Playwright checks (unit tests never replace browser/visual inspection; never institutionalize flaky skips).

## Phase handoff and completion

Every dispatch uses this compact envelope (resume existing workers/evidence where supported):

```text
MODE: PHASE | EDIT_UNIT | AUDIT_UNIT | CONSULT | REVIEW | VERIFY | AUDIT | DOC_REVIEW
PLAN_ID / absolute PLAN_DIR / PHASE:
INTEGRATION_ROOT / absolute WORKTREE / BASE_REVISION:
OWNED_FILES: exact paths (read-only references listed separately)
SPEC: file:line refs, contracts, constraints, performance/migration risks
ACCEPTANCE: checks, expected results, evidence paths, user exclusions
RETURN: COMPLETE | NEEDS_DECISION | FAILED | APPROVE | REJECT | PASS | FAIL | BLOCKED | PENDING | CLEAN | INTEGRITY_VIOLATION; changed files and evidence
```

- `scribe`: `COMPLETE / NEEDS_DECISION / BLOCKED` + reviewed scope, doc changes, checks, gaps (no-op still names scope and rationale).
- `reviewer`: `APPROVE / REJECT` + severity-ranked `file:line` findings.
- `tester`: `PASS / FAIL / BLOCKED / PENDING` + tested revision, dirty-diff identity, exit codes, skips, screenshot inspection, and open acceptance (host-native `--storage embedded` first, then offline `pnpm test:e2e`).
- `auditor`: `CLEAN / INTEGRITY_VIOLATION` binary integrity check.

## Worktree integration and verification

Record `INTEGRATION_ROOT`, base revision, and dirty prerequisites before dispatch. Give coders disjoint ownership; place task worktrees under `.agent-scratch/worktrees/` only when isolation helps, carry prerequisites via scoped patches, and keep worktrees edit-only. Final review, test, and audit run in `INTEGRATION_ROOT` under one build owner per [CI contracts](ci.md), [targeted recipes](contributing.md#targeted-local-rust-builds-and-tests-fast-iteration), and [cache hygiene](contributing.md#build-cache-hygiene).

Local Rust runs host-native embedded storage (`ARGOS_EMBED` unset); run `scripts/local_matrix.sh --storage embedded` after inputs stabilize when broad integration acceptance applies (fix failures with focused reruns; server, all-platform, and mutation runs stay hosted-only). Config-only changes run `node --test .opencode/tests/*.test.mjs` and runtime checks without a Rust rebuild; guidance-only changes use scoped link/policy checks.
