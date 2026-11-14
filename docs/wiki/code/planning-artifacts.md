# Planning artifacts

[Code route](index.md) · [Wiki](../../index.md) · [Agent workflow](agent-workflow.md)

## Working format

Use `.planning/<YYYY-MM-DD-slug>/` for multi-phase work; small edits need no new plan. Pin the exact task on resume. Use the maintained [task plan](templates/planning/task_plan.md), [findings](templates/planning/findings.md) and [progress](templates/planning/progress.md) templates; do not rewrite historical plans.

- `task_plan.md`: requested outcome, non-goals, finite acceptance/checks, owned phases and one Next Step. Phase status: pending, in_progress, blocked or complete.
- `findings.md`: relevant evidence and decisions; defer independent discoveries rather than adding requirements.
- `progress.md`: concise completed changes, actual checks/results, limitations and handoff.

One owner updates checkboxes/status at phase transitions and handoff; workers report evidence to that owner. Keep implementation and verification separate. Unverified/blocked items stay unchecked. Apply the [scope/freeze rule](agent-workflow.md#scope-and-finish-line): late plan changes need a demonstrated in-scope blocker or user direction. Record the reason and affected checks; never silently expand scope or restart general review.

## Retention in Git

Keep a planning artifact when another contributor needs it to implement, review,
resume a shared phase or understand a lasting decision. Creating a file during
an agent session does not make it a repository deliverable.

| Artifact | Git policy |
| --- | --- |
| Maintained requirements, architecture, shared roadmap or design | Commit relevant changes in `docs/` when they express agreed scope, dependencies or contracts. Avoid duplicate project onboarding/state files. |
| Named task `design.md`, `decisions.md`, `acceptance.md`, `handoff.md` | Eligible after curation. Commit only if they retain useful information beyond the session and belong to the change. |
| `task_plan.md`, `findings.md`, `progress.md`, worker ledgers, raw research, intermediate validation notes | Local working memory; ignored. Promote useful conclusions into maintained docs or a curated deliverable. |
| Raw simulation JSON, tool output, diagnostic logs, generated cells/PNGs/galleries and duplicate capture runs | Local evidence; ignored in named task directories. Promote only a small, referenced fixture selection to `docs/screenshots/` when it demonstrates a distinct contract. |
| `.planning/.active_plan`, host handoffs, session IDs, caches and environments | Local runtime state. OpenCode automatic handoffs stay in plugin storage. |

For a cutover, record a reviewed disposition for each shared plan or archive before moving it. Keep revision-bound evidence and open obligations. Link superseded instructions to the maintained contract in the disposition record. The [Fleetworks dispositions](../../../.planning/2026-11-09-fleetworks-overhaul/decisions.md) identify the retained, superseded and pending surfaces. The already-tracked Fleetworks plan remains the shared handoff for this cutover.

When an ad hoc task needs planning artifacts, use `.planning/<YYYY-MM-DD-slug>/`. Git ignores their contents by
default, with exceptions for the curated Markdown names above and numbered
phase designs at the task directory root. An exception makes a file eligible, not mandatory. Existing
non-dated shared planning records require individual review; inspect their
contents rather than adding the whole `.planning/` tree. New non-dated task
folders must follow the same policy even if Git does not ignore them automatically.

Before a commit, classify planning changes and stage exact paths or hunks.
Do not force-add ignored files just to clear status. Ignore rules do not untrack
existing files: when an existing tracked artifact is local runtime state, remove
it from the index with `git rm --cached <exact-path>` while preserving its local
copy. Do not bulk-untrack shared planning records. GSD scaffolding is not required;
use maintained docs and named task plans without adding a milestone framework.

Curated designs/decisions explain the final contract, rationale and remaining
tradeoffs. Acceptance records identify the change/revision, checks actually run,
results, regeneration commands and limitations. A durable handoff contains agreed
remaining scope and dependencies that another contributor needs, without host
session identifiers. Avoid links to ignored evidence in tracked records; preserve
reproducible commands and results instead. Do not present planned checks as passed.

At completion, consolidate useful knowledge before any cleanup. Retain active
plans and unresolved evidence locally. Remove only task-owned obsolete artifacts
when safe, and repair references first. Never delete unrelated plans or unresolved evidence.
TUI evidence follows [tui-verification.md](tui-verification.md).

For the initial semantic-versioning setup, retain the curated
[acceptance record](../../../.planning/2026-10-11-semantic-versioning/acceptance.md).
Its session plan, research, progress, intermediate `validation.md` and raw
`release-simulation.json` remain local; the maintained policy is
[versioning.md](versioning.md).
