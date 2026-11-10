# Efficient agent execution

[Code route](index.md) · [Contributor commands](contributing.md) · [Testing](../../../.opencode/skills/argos-testing/SKILL.md)

Canonical execution policy for repository agents. Host instructions and user authorization take precedence. Role-specific workflows apply only when selected.

## Scope and finish line

- Pin the requested outcome, non-goals, acceptance and verification scope before substantial work. Use an existing plan; do not create ceremony for a small edit.
- Complete the smallest useful implementation. Optional refactors, adjacent bugs, extra benchmarks and documentation sweeps are follow-ups unless required for the requested acceptance. Finding an issue does not authorize expanding the task.
- Review risky contracts and exercise one realistic end-to-end case early. Finish the scoped review and edits before expensive final checks; freeze that revision.
- After freeze, reopen only for a demonstrated failure that blocks the agreed acceptance. Record the failure, make the smallest repair and rerun affected checks. Defer independent findings. Do not start another general audit or review cycle because the work is nearly finished.
- Plan one final verification pass. Repeat a broad gate only when changed inputs or a diagnosed failure require it; identify why affected checks are insufficient. A required failure stays failed, never becomes a pass through a retry limit or omission.
- Keep documentation, plans, provenance and cleanup proportional to the change. They support delivery; they are not independent improvement projects.
- When the user asks to stop or wrap up, start no new edits, audits or checks. Stop only task-owned work when appropriate, preserve evidence, and hand off exact completed, failed and unverified items. Do not make a “last fix” after that request.

## Establish the critical path

Read the active plan, dirty-tree baseline and relevant contracts. Reuse existing authorization. Identify dependencies, the earliest decisive check and the remaining required work. Build realistic isolated fixtures through public services; never forge authority or relax guards to make tests pass. Smoke a costly fixture before repetitions.

## Coordinate work that can actually run in parallel

Use available subagents early for useful independent implementation, review or documentation. Assign concrete outcomes, disjoint writable paths, shared interfaces and a return point. One owner integrates changes and owns the shared Cargo cache. Delegation creates no new approval gate.

Wake completed workers explicitly, integrate results promptly, and reuse their valid evidence. Do not create workers, layers or duplicated reviews merely to keep agents busy. Finish edits before compilation; never edit a compiling unit or launch competing builds. Preserve user services and frontend locks.

## Keep one reusable build graph

Record and reuse host, storage mode, features, target/profile and concurrency through the [build recipes](contributing.md#targeted-local-rust-builds-and-tests-fast-iteration). Dependency feature unification matters: workspace bin tests enable core fixtures; an isolated package can trigger another large build. Zero-match companion targets are not test passes.

Choose build/test concurrency from memory and observed pressure; two build jobs are a starting point on this 16 GiB host. Serialize shared-state or timing-sensitive fixtures. For slow active builds, inspect CPU time, memory and host throttling once before changing course; do not repeatedly poll diagnostics or restart useful compilation. Follow [cache hygiene](contributing.md#build-cache-hygiene); preserve `target/debug` and the reusable agent cache.

## Diagnose before repeating

Preserve the original error and bounded relevant state, diagnose it, then rerun the repaired contract. Reuse checks whose relevant inputs did not change. An unchanged failure needs new evidence or an honest blocker, not identical retries or an unrelated repair campaign. Stop irrecoverably failed task-owned checks when useful; never stop unrelated processes.

Terminal checks must reconstruct current cells/state, not concatenate ANSI history. Use deliberate input and wait for its transition; repeated keys can cross child-process boundaries.

## Separate evidence by what it proves

Record actual commands, exits, executed/skipped counts and limitations. Queued/interrupted runs, zero matches and generated-but-uninspected screenshots are not passes. Keep implementation and verification status separate.

Bind performance evidence to the relevant corpus/model, workload, binary and environment. Do not repeat measurements or hash large unrelated artifacts merely because prose or another independent file changed. Capture affected UI states early; reuse inspected evidence only when relevant inputs match.

Select Fleetworks checks with the [shared scope planner](ci.md#fleetworksyml--scoped-branch-checks). Run affected local checks in the integration checkout and record selected hosted results separately. Use an isolated pinned server or broker fixture for an authorized targeted network contract. Full server, all-platform and mutation matrices run in hosted CI.

For changes to embedded or compatibility behavior, run the applicable [native embedded gate](contributing.md#full-local-embedded-gate--github-ci-monitoring) after the focused check passes. Leave storage-mode variables unset so its required storage phase runs once. Inspect companion target contracts because some native tests also execute in the workspace phase.

## Communicate and improve the workflow

Report the completed outcome, current blocker and next decisive check. Base estimates on observed complete command times and remaining stages; do not repeatedly promise “almost done.” If work overruns, explain the cause and reduce optional work instead of moving the finish line.

Keep guidance concise: one canonical rule, links from other entrypoints, and only role-specific differences. Apply small reversible workflow fixes within existing scope; defer broader improvements. Final handoffs state what changed, what was verified and what remains.
