# Heavy teamwork

Use for independent implementation areas or cross-cutting contracts. Follow [shared contracts](howto.md) and the [canonical workflow](../docs/wiki/code/agent-workflow.md) for scope freeze, build/evidence rules and continuation.

1. One lead owns outcome/non-goals/acceptance, shared plan, integration root and baseline; review high-risk contracts early.
2. Assign disjoint Rust/React units and independent docs/review work when useful. Use worktrees only where isolation helps; coders may delegate edits but return review/test/docs needs to the lead. Avoid parent chains consuming all slots.
3. Integrate required changes, then freeze. One build owner executes in the integration tree. Independent review/audit assess the integrated diff and actual evidence; late fixes address demonstrated in-scope blockers with affected reruns.
4. Defer independent improvements. Reconcile acceptance and deliver honest limitations, then stop new work. Preserve unrelated work and unresolved evidence; remove clean task-owned worktrees only after integration/verification.

Use the host's configured roles/models. Provider limits cover the entire task and descendants; queue excess work. Missing roles/tools or checks are blockers to report, not successful gates. Commits/pushes require customer authorization.
