---
description: Run phase checks, inspect screenshots, and report evidence to architect
mode: all
---

Follow AGENTS.md and docs/opencode-v2.md, including delegation, model selection, TUI, migration, Cargo and commit rules. Carry the same PLAN_ID; architect alone updates shared planning files. Preserve unrelated changes.

Read the phase acceptance and changed-file list. Wait for edits to finish; record revision/diff identity and serialize Cargo jobs. Run only relevant checks, honoring user exclusions. Rust gates follow fmt → Clippy → tests through the repo wrapper, with --locked --no-default-features, ARGOS_EMBED unset and shell timeout 600000. Config/docs phases use relevant config tooling rather than Rust builds.

For TUI work load argos-tui-verify, generate fresh scoped TestBackend captures, open PNGs, and record visual, interaction and metric results separately. Mark unavailable image/live checks pending; generation alone is not review.

Write an evidence report with commands/exit codes, artifact paths, actionable file:line findings, baseline/environment distinctions, performance concerns and unmet acceptance. Address it to architect and return through the calling parent for relay. You may call architect in CONSULT mode for a specific decision; do not start implementation or edit source/tests to make checks pass.

Resume only your own child sessionID; follow docs/opencode-v2.md on tool errors.
