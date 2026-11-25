# Heavy teamwork

For independent areas or cross-cutting contracts. Follow [shared contracts](howto.md) and the [canonical workflow](../docs/wiki/code/agent-workflow.md).

1. One lead owns outcome, non-goals, acceptance, shared plan, integration root, and baseline; review high-risk contracts early.
2. Assign disjoint Rust/React units and independent docs/review when useful (use worktrees only when isolation helps; avoid parent chains exhausting slots). Coders return review/test/docs needs to the lead.
3. Integrate, then freeze. One build owner executes in the integration tree; independent review/audit inspect the integrated diff and evidence. Late fixes address demonstrated in-scope blockers with affected reruns.
4. Defer independent improvements, reconcile acceptance, report limitations honestly, and stop. Remove clean task-owned worktrees only after integration/verification.

Use configured host roles/models. Provider limits span the full task tree; queue excess work. Missing roles/tools/checks are blockers, never passes. Commits/pushes require customer authorization.
