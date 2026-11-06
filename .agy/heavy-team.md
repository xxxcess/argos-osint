# Heavy teamwork

Follow [shared contracts](howto.md), the canonical [agent workflow](../docs/wiki/code/agent-workflow.md) and proactive [build cache hygiene](../docs/wiki/code/contributing.md#build-cache-hygiene). Use for independent implementation areas or cross-cutting contracts.

1. One lead pins the plan, integration root, base revision and acceptance criteria. Record existing edits and active processes before resuming work.
2. Map dependencies and assign disjoint edit units to Rust/React specialists. Create worktrees only where isolation helps. Coders may delegate edit units, but return review/test/docs needs to the lead. Do not reserve all execution slots with parents waiting on children.
3. The plan owner integrates completed changes and schedules documentation checkpoints. Workers write separate evidence files; only the owner edits shared status.
4. One build owner serializes Cargo, preserves the gate-compatible feature graph and chooses jobs from measured RAM/pressure. Smoke the real integrated fixture before long suites; use early source-labelled visual previews. Schedule the required final embedded gate after relevant Rust integration stabilizes. Run browser suites serially when they share server ports or artifact directories. Reuse passing evidence unless relevant source/config/fixture/renderer changes invalidate it; track those boundaries explicitly rather than globally resetting every result.
5. Independent review and integrity audit assess the integrated diff and execution evidence. Failures return to the relevant owner; only affected checks rerun before a justified final gate. Never mark skipped/unexecuted checks passed.
6. Reconcile acceptance checkboxes and report limitations. Preserve unresolved evidence and unrelated work. Remove clean task-owned worktrees only after integration and verification.

Use the host's configured role/model catalog. Concurrency limits apply to the whole task, including descendants; queue work rather than repeatedly retrying provider-limit failures. A missing role/tool is a concrete blocker to report, not permission to invent a successful gate.
