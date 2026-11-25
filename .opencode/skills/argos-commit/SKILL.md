---
name: argos-commit
description: Prepare or create an authorized Argos commit with Conventional Commits, release impact and scoped staging.
---

Follow the [agent workflow](../../../docs/wiki/code/agent-workflow.md) and active host permissions (OpenCode boundaries apply only when selected). Grants no push, merge, or release permission; requires existing commit authorization.

- Follow `docs/wiki/code/versioning.md`: inspect status/diff, preserve unrelated work, stage exact paths, review full staged diff, and use Conventional Commits with accurate release impact and breaking/migration notes. Never hand-bump versions, invent `Release-As`, or rewrite commits without authorization.
- Write the message in `.agent-scratch/`, run `node scripts/release_check.mjs message --file <path>`, `node scripts/release_check.mjs versions`, `git diff --cached --check`, and relevant task checks, then commit via `git commit --file <path>` and report SHA + validation.
- Exclude local plans/logs/session IDs/raw evidence per `docs/wiki/code/planning-artifacts.md`; keep affected docs/diagrams consistent per `docs/wiki/code/conventions.md`.
