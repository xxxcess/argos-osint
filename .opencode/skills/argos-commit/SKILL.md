---
name: argos-commit
description: Prepare or create an authorized Argos commit with Conventional Commits, release impact and scoped staging.
---

Read `docs/versioning.md` for the maintained policy. Apply existing task authorization; loading this skill does not authorize a commit, push, merge or release. On the planning-only `architect` agent, propose the message and release impact only.

For a commit task, inspect status and the staged diff before staging. Preserve unrelated and pre-staged work; stage exact task paths or hunks, then inspect the entire staged diff. Choose the type from the final behavior: `feat` adds capability, `fix` corrects behavior, `perf` improves efficiency; maintenance types do not request a release. Split unrelated changes. An incompatible public change needs `!` plus a `BREAKING CHANGE:` footer explaining impact and migration. An editor reports compatibility impact to its parent and does not commit.

State the problem/result and actual validation in the commit body when useful. Keep message files in task-owned `.agent-scratch/` and use `git commit --file <path>`. Before committing, run `node scripts/release_check.mjs message --file <path>`, `node scripts/release_check.mjs versions`, `git diff --cached --check` and the checks appropriate to the change. These checks do not replace Rust/TUI verification.

Leave workspace versions, `version.txt`, the release manifest and generated changelog to Release Please unless the task explicitly concerns a release/baseline repair. Never invent `Release-As`, manually publish a tag, or amend/rewrite existing commits without task authorization. PR titles follow the same convention; retain breaking-change footers in the squash message. Report the resulting commit SHA and relevant checks. Preserve models and unrelated permissions.

Classify planning changes with `docs/planning-artifacts.md`. Stage relevant curated durable deliverables by exact path; session plans/logs, raw evidence and `.planning/.active_plan` remain local. Do not force-add ignored artifacts or add the entire `.planning/` tree. Report intentionally retained local evidence without treating it as unfinished implementation.

For new or significant architectural/conceptual code shifts, check that relevant maintained docs/diagrams are included and consistent before committing; follow `docs/conventions.md#architecture-and-concepts`. Resolve gaps within authorized scope or report required missing work.
