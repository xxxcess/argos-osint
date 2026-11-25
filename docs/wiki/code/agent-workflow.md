# Efficient agent execution

[Code route](index.md) · [Contributor commands](contributing.md) · [Testing](../../../.opencode/skills/argos-testing/SKILL.md)

Canonical execution policy for repository agents. Host instructions and user authorization take precedence; role-specific workflows apply only when selected.

## Scope and finish line

- Pin outcome, non-goals, acceptance, and verification scope before substantial work (reuse existing plans; no ceremony for small edits).
- Deliver the smallest useful implementation. Defer optional refactors, adjacent bugs, extra benchmarks, and broad doc sweeps unless required for acceptance.
- Review risky contracts and run one realistic end-to-end check early. Finish scoped review and edits before expensive final checks, then freeze that revision.
- After freeze, reopen only for a demonstrated failure blocking agreed acceptance: record it, apply the smallest repair, and rerun affected checks. Do not start another general audit because work is nearly done.
- Plan one final verification pass; repeat a broad gate only when changed inputs or a diagnosed failure require it. A required failure never becomes a pass via retry cap or omission.
- Keep docs, plans, provenance, and cleanup proportional to the change.
- On user stop/wrap-up, start no new edits, audits, or checks ("no last fix"): stop only task-owned work, preserve evidence, and hand off exact completed, failed, and unverified items.

## Establish the critical path

- Confirm the intended PR base branch before edits (an explicit branch in the session counts) and record it in the named plan.
- In Nightly GSD, use GitHub Actions first for affected checks, tests, builds, and captures, reviewing exact-head hosted artifacts ([GSD verification policy](../../../.agents/skills/gsd/SKILL.md#github-ci-first)).
- Read the active plan, dirty-tree baseline, and relevant contracts; reuse existing authorization. Build realistic isolated fixtures via public services (never forge authority or relax guards) and smoke costly fixtures before repeating them.

## Coordinate work that can actually run in parallel

Use subagents early for useful independent implementation, review, or docs—assign concrete outcomes, disjoint writable paths, shared interfaces, and a return point. One owner integrates changes and owns the shared Cargo cache; delegation adds no approval gate.

Wake idle workers explicitly, integrate promptly, and reuse valid evidence. Never spawn redundant layers, edit a compiling unit, launch competing builds, or disturb user services/frontend locks.

## Keep one reusable build graph

Before compiling, inspect the existing cache and reuse the matching shared debug graph (record host, storage mode, features, target/profile, activating package/target set, and concurrency). Prefer scoped debug checks during iteration; make graph changes deliberate ([canonical graphs](contributing.md#verification-ownership-and-host-resources), [build recipes](contributing.md#targeted-local-rust-builds-and-tests-fast-iteration)). Zero-match companion targets are not passes.

Size concurrency to host memory (~2 build jobs starting point on 16 GiB) and serialize shared-state/timing fixtures. For slow active builds, inspect CPU/memory/throttling once before changing course—do not poll or restart useful compilation. Follow [cache hygiene](contributing.md#build-cache-hygiene); protect `target/debug` and the reusable agent cache.

## Diagnose before repeating

Start with the [formatting gate](contributing.md#build-and-test). Preserve the original error and bounded state, diagnose it, and rerun only the repaired contract. Reuse unaffected checks; an unchanged failure needs new evidence or an honest blocker, not identical retries. Stop irrecoverably failed task-owned checks when useful—never unrelated processes.

## Separate evidence by what it proves

- Record commands, exits, executed/skipped counts, and limitations. Queued, interrupted, skipped, unreached, zero-match, or generated-but-uninspected outputs are not passes. Keep implementation and verification status distinct.
- Bind performance evidence to corpus/model, workload, binary, and environment; do not re-measure or hash unrelated artifacts after prose/independent edits.
- Select Fleetworks checks via the [shared scope planner](ci.md#fleetworksyml--scoped-branch-checks). PRs to `dev`, `nightly`, and `main` run selected checks; Linux embedded compatibility runs for affected storage (direct dispatch keeps the 4-platform diagnostic). Trusted pushes publish commit-versioned [channel artifacts](versioning.md#release-automation). Dispatch mutation in CI only—never locally.
- For interactive native review, a passing hosted Mac app build at exact head is enough to start review before broader Linux/storage gates finish (rebuild locally only if the hosted artifact cannot support an authorized check); keep unfinished/failing gates visible.
- For UI deltas, dispatch [UI visual review](native-verification.md#dispatchable-ui-visual-review) at frozen PR head for affected scenes/surfaces (or use existing CI captures until the workflow lands on the default branch): inspect captures, attach selected images inline, retain the full artifact, and record durable doc/release refs.
- For embedded/compatibility changes, run the [native embedded gate](contributing.md#full-local-embedded-gate--github-ci-monitoring) after focused checks pass, with storage-mode vars unset so the required storage phase executes once.

## Communicate and improve the workflow

Report completed outcomes, current blockers, and the next decisive check; base estimates on observed timings and cut optional work rather than moving the finish line. Keep guidance concise (one canonical rule, links elsewhere, role deltas only), apply reversible fixes within scope, and deliver honest final handoffs.
