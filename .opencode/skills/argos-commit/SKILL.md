---
name: argos-commit
description: Prepare or create an authorized Argos commit with Conventional Commits, release impact and scoped staging.
---

Use the canonical [agent workflow](../../../docs/wiki/code/agent-workflow.md) with the active host's roles and existing authorization. OpenCode-only restrictions apply when that profile is explicitly selected.

Apply existing commit authorization; this skill grants no push/merge/release permission. When an OpenCode role is explicitly selected, preserve its configured commit/edit boundaries and use the assigned committer. Other hosts follow their active role and existing user authorization.

Follow `docs/wiki/code/versioning.md`: inspect status/staged diff, preserve unrelated work, stage exact paths, and review the full staged diff. Use Conventional Commits with appropriate release impact and breaking/migration notes. Never hand-bump versions, invent Release-As or rewrite commits without authorization.

Keep the message in `.agent-scratch/`. Run `node scripts/release_check.mjs message --file <path>`, `node scripts/release_check.mjs versions`, `git diff --cached --check`, and relevant task checks; commit with `git commit --file <path>`. Report SHA and validation.

Use `docs/wiki/code/planning-artifacts.md` to exclude local plans/logs/session IDs/raw evidence. Architecture/contract changes include consistent docs/diagrams per `docs/wiki/code/conventions.md`.
