---
description: Verify affected contracts with scoped checks and required integration gates
agent: tester
---

Verify $ARGUMENTS with ecc-verify. Keep ARGOS_EMBED unset; Select checks by affected contracts; no Rust build for docs/config-only changes. Broad Rust gate: `scripts/local_matrix.sh --storage embedded` in the combined tree. Honor exclusions/retry caps; report actual results and pending evidence.

Apply [build cache hygiene](../../docs/wiki/code/contributing.md#build-cache-hygiene) before checks, between large build phases and at handoff; keep the current reusable cache and remove verified obsolete inactive artifacts.
