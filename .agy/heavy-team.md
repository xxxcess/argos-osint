# Heavy teamwork

Follow [shared contracts](howto.md). Use for independent implementation areas or cross-cutting contracts.

1. One lead pins the plan, integration root, base revision and acceptance criteria. Record existing edits and active processes before resuming work.
2. Map dependencies and assign disjoint edit units to Rust/React specialists. Create worktrees only where isolation helps. Coders may delegate edit units, but return review/test/docs needs to the lead. Do not reserve all execution slots with parents waiting on children.
3. The plan owner integrates completed changes and schedules documentation checkpoints. Workers write separate evidence files; only the owner edits shared status.
4. One tester owns serialized Cargo verification. Run scoped checks first; schedule the full embedded gate after integration stabilizes. Run browser suites serially when they share server ports or artifact directories. Reuse passing evidence unless relevant source/config changes invalidate it.
5. Independent review and integrity audit assess the integrated diff and execution evidence. Failures return to the relevant owner; only affected checks rerun before a justified final gate. Never mark skipped/unexecuted checks passed.
6. Reconcile acceptance checkboxes and report limitations. Preserve unresolved evidence and unrelated work. Remove clean task-owned worktrees only after integration and verification.

Use the host's configured role/model catalog. Concurrency limits apply to the whole task, including descendants; queue work rather than repeatedly retrying provider-limit failures. A missing role/tool is a concrete blocker to report, not permission to invent a successful gate.
