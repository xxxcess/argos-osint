---
description: Verify affected contracts with scoped checks and required integration gates
agent: tester
---

Follow the canonical [scope and finish-line policy](../../docs/wiki/code/agent-workflow.md#scope-and-finish-line) and this command’s configured role/permissions.

Verify $ARGUMENTS with `ecc-verify` and the agreed test scope. Follow the [contributor gates](../../docs/wiki/code/contributing.md#full-local-embedded-gate--github-ci-monitoring) for applicable integration checks. Preserve existing authorization/exclusions and required gates; use affected checks, reuse unaffected evidence, and report executed/skipped/pending results. No Rust build for docs/config-only work.
