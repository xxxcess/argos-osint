---
description: Verify affected contracts with scoped checks and required integration gates
agent: tester
---

Follow the canonical [agent workflow](../../docs/wiki/code/agent-workflow.md) for execution, evidence reuse and improvement suggestions. Preserve this command's configured OpenCode role when selected; other hosts retain their active instructions.

Verify $ARGUMENTS with ecc-verify. Keep ARGOS_EMBED unset; select checks by affected contracts; no Rust build for docs/config-only changes. Broad Rust gate: `scripts/local_matrix.sh --storage embedded` in the combined tree. Smoke the real affected fixture first, retain the selected Cargo feature graph and choose jobs from measured RAM/pressure. Honor user exclusions and the required final gate; rerun only invalidated checks, without an arbitrary attempt cap. Report actual executed/skipped results and pending evidence.

Apply [build cache hygiene](../../docs/wiki/code/contributing.md#build-cache-hygiene) before checks, between large build phases and at handoff; keep the current reusable cache and remove verified obsolete inactive artifacts.
