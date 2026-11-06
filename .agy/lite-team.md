# Lite teamwork

Follow [shared contracts](howto.md), the canonical [agent workflow](../docs/wiki/code/agent-workflow.md) and proactive [build cache hygiene](../docs/wiki/code/contributing.md#build-cache-hygiene). Use for a bounded task with known ownership.

1. The lead identifies the contract, dirty-tree baseline and exact files. Reuse an existing plan for ongoing work; do not restart discovery or seek duplicate approval.
2. Assign one stack specialist to implementation. Keep independent documentation with its owner. Spawn children only for genuinely disjoint edits whose coordination cost is worthwhile.
3. The lead assigns focused verification and review. One tester runs builds in the integration tree; the implementation worker returns needs to the lead rather than spawning review/test/docs chains.
4. Run a real-contract fixture smoke as soon as the unit is runnable; inspect early visual previews when relevant. Fix demonstrated failures and rerun the affected closure with a stable feature graph. Complete the required final embedded gate after relevant Rust integration stabilizes; rerun broad checks only when later changes invalidate them, without an arbitrary attempt cap.
5. Update the plan and deliver. Commits/pushes follow the customer's authorization; retain unresolved evidence.
