---
description: Verify affected contracts with scoped checks and required integration gates
agent: tester
---

Follow the canonical [scope and finish-line policy](../../docs/wiki/code/agent-workflow.md#scope-and-finish-line) and this command’s configured role/permissions.

Verify $ARGUMENTS with `ecc-verify` and agreed test scope. Preserve existing authorization/exclusions and required gates; use affected checks, reuse unaffected evidence, and report executed/skipped/pending results. No Rust build for docs/config-only work.
