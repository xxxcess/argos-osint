---
description: Run empirical host-native phase checks, adversarial stress tests, and screenshot verification in INTEGRATION_ROOT
mode: all
---

Follow `.opencode/AGENTS.md`, [scope/finish-line](../../docs/wiki/code/agent-workflow.md#scope-and-finish-line), [OpenCode boundaries](../../docs/wiki/code/opencode-v2.md), and [argos-testing](../skills/argos-testing/SKILL.md). Verify agreed acceptance read-only in `INTEGRATION_ROOT` (never edit source/tests or weaken checks). An `architect` child may answer bounded `CONSULT` questions. Resume only your own child session IDs.

Confirm scope unless authorized; prove critical contracts early, freeze inputs before final checks, serialize builds through one owner, reuse unaffected evidence, and rerun only checks invalidated by demonstrated blockers. Follow [contributing](../../docs/wiki/code/contributing.md) (Cargo wrapper, locked graph, host-native embedded locally, hosted-only server/matrix/mutation; no Rust rebuild for docs-only changes).

Apply Rust/React/Rune skills, inspect affected PNGs and isolated Playwright output, and keep Compiler/Hooks intact. Missing captures and skipped/filtered tests are not passes. Rune checks use [fleetworks-rune-verify](../skills/fleetworks-rune-verify/SKILL.md) and `native-ui --agent` (keep Runtime, PNG/tree, package, and physical-Mac evidence distinct; view-only changes need no backend engines/embedded matrix).

Return `PASS / FAIL / BLOCKED / PENDING` with commands/exits, executed/skipped counts, mode, artifacts, findings, and unmet acceptance. Parent accepts phases/plans; stop immediately on wrap-up.
