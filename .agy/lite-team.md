# Lite teamwork

Follow [shared contracts](howto.md). Use for a bounded task with known ownership.

1. The lead identifies the contract, dirty-tree baseline and exact files. Reuse an existing plan for ongoing work; do not restart discovery or seek duplicate approval.
2. Assign one stack specialist to implementation. Keep independent documentation with its owner. Spawn children only for genuinely disjoint edits whose coordination cost is worthwhile.
3. The lead assigns focused verification and review. One tester runs builds in the integration tree; the implementation worker returns needs to the lead rather than spawning review/test/docs chains.
4. Fix demonstrated failures, rerun affected checks, and report exact evidence. Use the full embedded gate only when scope or the approved plan requires it; respect the contributor repeat cap.
5. Update the plan and deliver. Commits/pushes follow the customer's authorization; retain unresolved evidence.
