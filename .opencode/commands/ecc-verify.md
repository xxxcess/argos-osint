---
description: Verify affected contracts with scoped checks and required integration gates
agent: tester
---

Follow [scope/finish-line](../../docs/wiki/code/agent-workflow.md#scope-and-finish-line) and role permissions. Verify $ARGUMENTS with `ecc-verify` and agreed scope, following [contributor gates](../../docs/wiki/code/contributing.md#full-local-embedded-gate--github-ci-monitoring). Reuse unaffected evidence, honor exclusions/required gates, and report executed/skipped/pending results. No Rust build for docs/config-only work.
