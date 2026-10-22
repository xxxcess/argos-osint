# OpenCode agent team

`architect` is the default. Architect, coder and tester all use `mode: all`, so they can run as primary agents or children. Legacy build/plan/general/explore agents are disabled; ECC agent definitions are replaced. `/ecc-plan` and `/ecc-review` select architect; `/ecc-verify` selects tester.

| Caller | Permitted children | Purpose |
| --- | --- | --- |
| architect | architect, coder, tester | Explore code/docs, dispatch a phase, review evidence |
| coder | coder, architect, tester | Delegate edits, ask decisions, verify a phase |
| tester | architect | Ask a specific verification/design question |

[Agent handoffs](diagrams/agent-team.html) follow one phase at a time. Architect uses planning-with-files, graphify and current source to write the spec with exact files, contracts, performance/migration risks and acceptance. It owns shared planning files. Coder assigns disjoint EDIT_UNIT children, including tests/docs/fixtures when needed, waits for formatted edits, and invokes tester. Tester returns evidence to coder, which relays it to architect. Architect reviews actual diagnostics/source and performance, updates the plan and provides corrections before accepting or advancing a phase. Plan-only requests stop before implementation. User exclusions override routine build/test defaults.

Architect exploration children use AUDIT_UNIT mode; decision calls use CONSULT. Both are read-only leaf modes. EDIT_UNIT coder children edit/format only their assigned files and never delegate/build/test. Consultations return decisions to the caller rather than starting another workflow. Every handoff carries PLAN_ID, exact scope, references and acceptance; only architect updates shared plan files.

## Models

Role models and maximum-effort requests live in `.opencode/opencode.json` at `agents.<role>.model` and `request.body`. Architect/tester default to available `opencode/glm-5.3-flash-free#max`; coder retains `opencode/mimo-v2.6-flash-free#max`. The previous `opencode/step-5-preview-free` model was absent from the installed catalog. Provider/model support determines actual reasoning behavior.

Set exploration and editing child defaults independently:

- `.opencode/architect-worker.json`: architect's AUDIT_UNIT children.
- `.opencode/coder-worker.json`: coder's EDIT_UNIT children.

Each file contains `{"model": "provider/model#variant"}`. The caller reads it and supplies `subagent.model` on every new/resumed worker call. An explicit user task override wins. Calls to a different role use that role's configured model unless the user overrides it. These files are workflow data, not native OpenCode config keys. Unavailable models/variants are reported without silent substitution.

## Delegation and continuation

`experimental.subagent_depth: 3` supports architect → coder → tester → architect CONSULT. The former top-level `subagent_depth` does not configure the installed V2 runtime. Delegation permissions must allow the selected role, and each called role must support child mode. Leaf instructions prevent recursive workflow spawning.

A `sessionID` belongs to the calling parent: resume only a child returned by your own successful subagent call, using the same role. Never copy sibling, ancestor or previous-root child IDs into a new parent. Keep the parent/child mapping in its own local ledger; it is runtime state, not a durable spec.

If continuation returns unknown/foreign-session errors, confirm the old worker is no longer editing, then start a fresh child without sessionID and include the current spec, completed work and remaining scope. For model/permission/depth failures, report the exact diagnostic and stop that handoff for configuration repair. Do not claim success or fall back to direct edits/builds in the wrong role. Background children notify parents; do not poll or overlap owned edits with verification.

The supplied screenshot shows a failed continued call followed by direct fallback, but omits the diagnostic. The exact original cause is unconfirmed; the configuration and instructions address the runtime's depth, role-mode and parent-ownership requirements.

## Permissions and tooling

All roles may read/search, use shell and web tools, and load relevant skills. Architect/tester edit tools allow only planning evidence; coder may edit source. Shell access supports repo commands and is not a filesystem sandbox. Code Mode and browser tools remain denied. Delegation is restricted to the table above. Follow AGENTS.md for isolated Cargo commands, TUI capture/image/interaction/metric checks, documentation, scratch, compatibility and commits. Tester marks unavailable image/live evidence pending.

Use the installed `opencode debug config` and initialized `/api/agent` catalog to inspect effective models, prompts and permissions. Initial catalog loads can be empty while initialization completes. Existing sessions retain selected agents/models; start a new architect session or explicitly switch. Retired sessions do not acquire new parent/child relationships.

References: [official V2 agents](https://opencode.ai/v2/docs/agents), [config](https://opencode.ai/v2/docs/config), [tools](https://opencode.ai/v2/docs/tools). Existing [graph-first discovery](diagrams/agent-graphify.html) still applies.
