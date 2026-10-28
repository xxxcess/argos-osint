# OpenCode agent team

[Code route](index.md) · [Wiki](../../index.md)

`architect` is the default. Architect, coder, tester and Scribe all use `mode: all`, so they can run as primary agents or children. Legacy build/plan/general/explore agents are disabled; ECC agent definitions are replaced. `/ecc-plan` and `/ecc-review` select architect; `/ecc-verify` selects tester.

| Caller | Permitted children | Purpose |
| --- | --- | --- |
| architect | architect, coder, tester, scribe | Explore code/docs, dispatch a phase, review evidence |
| coder | coder, architect, tester, scribe | Delegate edits, ask decisions, verify a phase |
| tester | architect | Ask a specific verification/design question |
| scribe | none | Review code deltas and maintain canonical documentation |

[Agent handoffs](../../diagrams/agent-team.html) follow one phase at a time. Architect uses planning-with-files, the wiki and current source to write the spec with exact files, contracts, performance/migration risks and acceptance. It owns shared planning files. Coder assigns disjoint EDIT_UNIT children, including tests/docs/fixtures when needed, waits for formatted edits, runs Scribe on accumulated changes at scheduled phase checkpoints, then invokes tester. Scribe updates owned docs/diagrams and returns review evidence; it reports source-tag or agent-guidance fixes to coder. Architect schedules meaningful phase checkpoints, may batch small internal-only phases with tracked uncovered scope, and requires remaining changes reviewed before final delivery. Later fixes that change documented behavior trigger another scoped Scribe pass. Tester returns evidence to coder, which relays it to architect. Architect reviews actual diagnostics/source and performance, updates the plan and provides corrections before accepting or advancing a phase. Plan-only requests stop before implementation. User exclusions override routine build/test defaults.

Architect exploration children use AUDIT_UNIT mode; decision calls use CONSULT. Both are read-only leaf modes. EDIT_UNIT coder children edit/format only their assigned files and never delegate/build/test. Consultations return decisions to the caller rather than starting another workflow. Every handoff carries PLAN_ID, exact scope, references and acceptance; only architect updates shared plan files.

## Models

Role models and reasoning variants live in `.opencode/opencode.json` at `agents.<role>.model`. Architect and coder coordinators default to `opencode/glm-5.3-flash-free#max`, and tester/Scribe to the vision-capable `opencode/nemotron-3.5-super-vl-free#xhigh`. Coder-spawned children other than Scribe use `opencode/mimo-v2.6-flash-free` (provider defaults; no advertised reasoning variant). All three base IDs are present in the installed free OpenCode catalog. GLM advertises `max`; Nemotron advertises `xhigh` as its highest variant. Unsupported `#max` suffixes and raw reasoning-body overlays are not sent. Provider/model support determines actual reasoning behavior.

Set exploration and editing child defaults independently:

- `.opencode/architect-worker.json`: architect's AUDIT_UNIT children.
- `.opencode/coder-worker.json`: EDIT_UNIT coders, tester and architect CONSULT spawned by coder; excludes Scribe.

Each file contains `{"model": "provider/model#variant"}`. The caller reads it and supplies `subagent.model` on every new/resumed worker call. An explicit user task override wins. Coder passes its worker override for editor/tester/CONSULT children. For Scribe, all callers read and explicitly pass `agents.scribe.model` from `.opencode/opencode.json`, keeping it equal to tester’s role default; other callers use the target role's configured model except architect's AUDIT_UNIT override. The child keeps its role prompt/permissions; only its model changes. These files are workflow data, not native OpenCode config keys. Unavailable models/variants are reported without silent substitution.

## Delegation and continuation

`experimental.subagent_depth: 3` supports architect → coder → tester → architect CONSULT. The former top-level `subagent_depth` does not configure the installed V2 runtime. Delegation permissions must allow the selected role, and each called role must support child mode. Leaf instructions prevent recursive workflow spawning.

A `sessionID` belongs to the calling parent: resume only a child returned by your own successful subagent call, using the same role. Never copy sibling, ancestor or previous-root child IDs into a new parent. Keep the parent/child mapping in its own local ledger; it is runtime state, not a durable spec.

If continuation returns unknown/foreign-session errors, confirm the old worker is no longer editing, then start a fresh child without sessionID and include the current spec, completed work and remaining scope. For model/permission/depth failures, report the exact diagnostic and stop that handoff for configuration repair. Do not claim success or fall back to direct edits/builds in the wrong role. Background children notify parents; do not poll or overlap owned edits with verification.

## Permissions and tooling

All roles may read/search, use shell and web tools, and load relevant skills. Architect/tester edit tools allow only planning evidence; coder may edit source. Scribe edit tools permit docs, README files and local planning evidence; its role prohibits source/test/config/agent-instruction edits, builds, commits and delegation. It reports required changes outside documentation to coder. Shell access supports repo commands and is not a filesystem sandbox. Code Mode is enabled so configured MCP discovery tools and worktree/session utilities remain available; nested tools still enforce their own permissions. Browser tools remain denied. Delegation is restricted to the table above. `.opencode/AGENTS.md` is a concise shared guide loaded by `instructions` for all models, alongside root AGENTS.md (OpenCode combines these files; this is not an override). The project registers the installed `~/.agents/skills/planning-with-files` skill without changing global settings. Follow AGENTS.md for isolated Cargo commands, TUI capture/image/interaction/metric checks, documentation, scratch, compatibility and commits. Tester marks unavailable image/live evidence pending.

Use the installed `opencode debug config` and initialized `/api/agent` catalog to inspect effective models, prompts and permissions. Initial catalog loads can be empty while initialization completes. Existing sessions retain selected agents/models; start a new architect session or explicitly switch. Retired sessions do not acquire new parent/child relationships.

References: [official V2 agents](https://opencode.ai/v2/docs/agents), [config](https://opencode.ai/v2/docs/config), [tools](https://opencode.ai/v2/docs/tools). Navigate through [the wiki](../../index.md), tags and scoped source searches. `/docs-review`, `/setup-wiki` and `/refresh-wiki` select Scribe and use the [maintenance workflow](../maintenance.md). The handoff plugin retains bounded idle/compaction summaries; source search has no graph gate.

## Rust style

Any Rust edit or review under `crates/` loads `argos-rust-style` (Google's Rust Style Guide, with Argos overrides). `argos-osint-core` and `argos-osint-api` follow the library rules; `argos-osint-bin` and `argos-api-host` follow the application rules. Tokio async and the existing crate layout stay in force. Architect passes the skill into Rust phases, coder passes it to Rust `EDIT_UNIT`s, and tester reviews diffs against it. The canonical file is `.opencode/skills/argos-rust-style/SKILL.md`; `.agents/skills/argos-rust-style` is the same skill. `cargo fmt` remains the formatter. Roles, models, and delegation permissions are unchanged.

## React UI skills

React work uses [React UI rules](react-ui.md), including the [Argos look and feel](react-ui.md#argos-look--feel-reference). Stay on the Argos theme and design components creatively inside it. Topology graphs stay on the force-graph canvas. `argos-react-ui` covers design, accessibility and visual evidence; `argos-react-patterns` covers React Native composition, compiler-compatible hooks, Query state and rendering. All React roles retain the enabled Next Compiler and zero-warning Hooks/compiler lint baseline; platform persistence/request IDs stay behind the app adapter. Canonical files live in `.opencode/skills/`; `.agents/skills/` symlinks expose the same skills to other hosts. Existing plan/implementation skills route through the role prompts, which select these UI skills only for relevant work.

`/react-ui-review <scope>` selects tester for a read-only review (default: current UI delta). Frontend verification uses the pinned Node/pnpm, offline Playwright fixtures and inspected screenshots; shell Playwright does not require enabling the denied browser tool. Missing image/tool evidence remains pending. Roles, models and delegation permissions are unchanged.

## Phase handoff and completion

Every dispatch carries this compact envelope; children start with fresh context:

```text
MODE: PHASE | EDIT_UNIT | AUDIT_UNIT | CONSULT | VERIFY | DOC_REVIEW
PLAN_ID / absolute PLAN_DIR / PHASE:
INTEGRATION_ROOT / absolute WORKTREE / BASE_REVISION:
OWNED_FILES: exact paths (read-only references listed separately)
SPEC: file:line refs, contracts, constraints, performance/migration risks
ACCEPTANCE: checks, expected results, evidence paths, user exclusions
RETURN: COMPLETE | NEEDS_DECISION | FAILED; changed files and evidence
```

Scribe returns COMPLETE / NEEDS_DECISION / BLOCKED with reviewed baseline/dirty scope, doc changes, actual checks and gaps; a no-op still requires scope and rationale. It is a leaf role and cannot accept shared phases. Standalone Scribe maintains its own named plan.

Tester uses PASS / FAIL / BLOCKED / PENDING and reports tested revision plus dirty-diff identity, command exit codes, skipped checks, screenshot inspection and outstanding acceptance. Its report is addressed to architect but returns through its calling coder. Coder relays it unchanged with integration notes. A CONSULT child returns an advisory decision; it cannot update the owning architect's plan or accept a phase. Questions affecting scope return to the owning architect.

The owning architect updates the spec and planning files, then resumes its own coder with exact corrections. Coder resumes its own editors/tester as needed. No sibling/ancestor messaging or shared session IDs are assumed. Await foreground calls or background completion notifications before verification. Implementation requests continue phase by phase without routine user confirmation; plan-only requests stop after the spec. Completion requires all phase acceptance and final review, including performance risks; blocked/pending evidence remains explicit.

## Worktree integration

1. Coder records the main checkout as INTEGRATION_ROOT, its revision and existing dirty paths. Assign disjoint file ownership; shared contracts/lockfiles have one owner. Small sequential edits can use the integration tree; parallel feature units use task-owned worktrees under `.agent-scratch/worktrees/`.
2. Create each worktree from the agreed baseline (`git worktree add --detach <absolute-path> <base>`). Include any required uncommitted phase baseline with a scoped patch; never silently omit dirty prerequisites or copy unrelated edits. Record that baseline so it is excluded from the worker's returned delta.
3. Pass absolute WORKTREE and PLAN_DIR to the child. Subagent creation inherits location; it does not implicitly enter a worktree. Use absolute edit paths and shell `workdir`, or move only the child session with Code Mode and wait for that move to take effect. Read the shared plan in the integration tree; never create a competing plan in a worker.
4. EDIT_UNIT children implement and format only. Return owned paths, diff and unresolved decisions. Coder waits for all editors, inspects changes (including new/untracked files), and integrates only each unit's delta. Without commit authorization use reviewed patches and `git apply --check` before applying; with authorization coordinator may commit exact paths and cherry-pick. Never blindly overwrite files or apply a baseline twice.
5. Resolve integration conflicts through architect when contracts change, format the combined tree, at scheduled documentation checkpoints give Scribe exclusive ownership of the affected docs while it reviews the integrated delta. Wait for documentation updates before giving tester the final tree and diff identity. Never test in feature worktrees. Retain task-owned worktrees until changes are verified/integrated; remove only clean, fully integrated task worktrees without force.

## Verification

Map each phase to [CI contracts](ci.md) and the affected `.github/workflows/` before choosing commands. Frontend phases include strict compiler/export and development browser checks; Rust phases use the embedded local gate. Respect Code Architect ownership for workflow edits and report hosted Actions separately from local evidence.

Rust: never run the all-platform or `server` stack from `.github/workflows/ci.yml` locally. Agents must stabilize the **local host machine platform** (`uname -ms`, `rustc -vV | grep '^host:'`) with the embedded defaults first (`ARGOS_EMBED` unset). Prefer targeted crate/binary commands during scoped phase checks (`python3 scripts/agent_cargo.py check|clippy|test --locked -p <crate> [--lib | --bin <bin> | --test <name>]`, and `ARGOS_SURREAL_MODE=embedded ARGOS_SURREAL_REQUIRED=1 python3 scripts/agent_cargo.py test --locked -p argos-osint-core --test storage_<name>` for individual storage contract suites; see [contributing.md](contributing.md#targeted-local-rust-builds-and-tests-fast-iteration)). Run the full `scripts/local_matrix.sh --storage embedded` gate (or `test_surreal.py --mode embedded`) in `INTEGRATION_ROOT` when broad workspace/storage verification is required, serializing Cargo and following the 1–2 full runs / maximum 3 cap. Monitor GitHub Actions build/test checks (`gh pr checks`, `gh run list --branch "$(git branch --show-current)"`, `gh run view <run-id> --log-failed`) when available to catch issues on other platforms or `server` mode. Config-only changes use `node --test .opencode/tests/*.test.mjs` plus runtime checks, without a Rust build.

Inspect `opencode debug config`, `opencode models`, and the initialized `/api/agent` catalog. Test actual handoffs in a fresh read-only session: architect → coder → EDIT_UNIT coder, coder → Scribe documentation review and coder → tester → CONSULT architect, then architect resumes its own coder via sessionID. A valid config alone is not proof of delivery. Keep raw runtime evidence in `.agent-scratch/`; never report a model/network failure as a passed handoff. Model variants and request overlays are preferences, not proof of provider-side reasoning; when switching models select its supported high/max variant and check the catalog rather than inventing one.

Continuous documentation is defined by the [Scribe workflow](../maintenance.md#continuous-documentation-with-scribe); it requires a phase-triggered review, not a background watcher.
