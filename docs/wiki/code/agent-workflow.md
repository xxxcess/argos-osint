# Efficient agent execution

[Code route](index.md) · [Contributor workflow](contributing.md) · [Testing guidelines](../../../.opencode/skills/argos-testing/SKILL.md)

This is the shared execution policy for repository agents. Host instructions and the user's current authorization take precedence. OpenCode and Antigravity role profiles apply when those workflows are explicitly selected; do not impose their delegation hierarchy on another host.

## Establish the critical path

Read the active plan, dirty-tree baseline and relevant contracts before rediscovering work. Preserve unrelated changes and reuse explicit authorization. Ask only for information or approval that is actually missing. Adding a focused regression within an already approved test scope does not require another intake round.

Identify the smallest end-to-end case that can invalidate the design. Exercise it early, before a large matrix or repeated measurements. For admitted Atlas data, a realistic fixture may need a real saved classifier, current admission, current metadata, retained body and native NLP receipts, and an actor catalog. Set up these prerequisites through existing services. Modern-looking tags, manually forged completion fields and relaxed production guards do not establish authority.

Run the most failure-prone benchmark or launcher case once before its repetitions. Keep the workload, quality assertions and publication checks intact. Use diagnostic runs during development; record accepted timings only after inputs and binaries are stable.

## Coordinate work that can actually run in parallel

Use authorized delegation for independent work whose benefit exceeds its coordination cost. Give each worker explicit owned paths, acceptance, existing evidence and a concrete next action. One owner coordinates shared files and one owner runs Cargo commands in the integration cache. Implementation, documentation, read-only review and independent process checks can proceed together when they do not interfere with the check being measured.

A handoff must start the recipient. On hosts exposing separate operations, sending a message to a completed agent does not restart it: use the follow-up/wake operation and verify its running status. Check completion and blockers at message boundaries. Do not wait indefinitely for an idle worker or add another delegation layer to repair a missed handoff.

Finish edits to the units being compiled before handing them to the build owner. Report any later edit immediately. Do not claim a running check covers a file revision it did not read. Serialize Cargo builds; do not interrupt the user's services or contend for their frontend locks.

## Keep one reusable build graph

Record host, storage mode, features, selected targets, profile, target directory and build jobs. Reuse that recipe through focused tests, fixtures and benchmarks. Selecting a different crate or dropping an activating test target can change dependency features and trigger another large compile even when the source is unchanged. Inspect Cargo artifact features and freshness when a supposedly small check rebuilds core dependencies.

Choose build concurrency from RAM and observed memory use, rather than logical CPU count. Two build jobs are a sensible starting point on this repository's 16 GiB host. Reduce concurrency when swapping begins; increase it only with evidence. Test concurrency is separate: use isolated parallel cases where safe and serialize fixtures that share state, measure latency or enforce startup/shutdown deadlines. Separate temporary homes do not remove contention for host CPU, memory and embedded-storage initialization; do not stack several heavy process checks and then raise deadlines to hide the resulting timeout.

When a CPU-active build is unusually slow, compare process CPU time with wall time and check host limits before changing code or restarting Cargo. On macOS, `pmset -g therm` reports `CPU_Speed_Limit`; record its value and timestamp alongside memory pressure and active workload. A low speed limit makes cache-based time estimates unreliable. Preserve user applications and the current useful build; do not treat wall time alone as a compiler hang or start a competing diagnostic build. Bound profiler symbolization separately from sampling so a short sample cannot become another prolonged CPU-heavy task.

Use the wrapper and the shared `target/agents` cache. Follow [cache hygiene](contributing.md#build-cache-hygiene), retain the useful feature graph and remove only proven obsolete artifacts. Estimate additional disk growth from a recent matching build plus a reserve. Fixed planning budgets are fallbacks when that evidence is absent. Cleanup proof should fit the risk: exact ownership, path, identity and holder checks normally suffice for task-owned artifacts; repeatedly hashing many GiB of unrelated binaries is not a default prerequisite for deletion.

## Diagnose before repeating

On the first meaningful failure, preserve the command, original error and relevant bounded state. Follow the error through wrappers to the underlying transaction or service failure. Generic messages such as “inputs unavailable” must not replace a known publication failure code. Prefer existing diagnostics; avoid successive guesses at prerequisites.

Stop a test process that is already irrecoverably failed when doing so saves work and preserves useful diagnostics. Verify that it is the task-owned process, retain its log, and report the interruption honestly. Never stop unrelated builds, services or frontend processes.

Fix the demonstrated cause and rerun the affected contract. Reuse passing checks whose relevant source, configuration and fixtures did not change. Do not rerun an entire library, benchmark cohort or matrix simply because another independent test or document changed. A repeated failure without new evidence calls for a better diagnostic or smaller reproduction, rather than another identical attempt.

Terminal assertions must reconstruct the visible screen or use deterministic cells. Concatenating ANSI redraw fragments loses retained characters and can match stale history. Assert the intended current state and fresh transition, while preserving navigation, restored values and terminal cleanup. Send one deliberate input gesture and wait for its state transition; repeated quit/confirm keys can cross a child-process boundary and act on the returned launcher.

## Separate evidence by what it proves

Record commands, exits, executed/ignored/skipped counts, relevant source and configuration identities, and limitations. A zero-match filter, missing harness, generated image, queued command or interrupted run is not a pass.

Use provenance proportional to the evidence. Performance requires immutable corpus/model identity, actual workload counts, binary identity and a stable measured environment. An unrelated prose edit does not invalidate Rust runtime evidence. An error-message or fixture change may require a focused rerun; it does not automatically require repeating every previously measured case. Keep failed attempts and superseded records distinguishable.

Capture affected UI states early enough to discover layout defects before the final gate. Inspect actual images. Carry a review only when the relevant rendering code, fixture, renderer and buffer/image identities justify it; keep visual, interaction and backend evidence distinct. See [TUI verification](tui-verification.md).

After targeted stability, run the required native embedded gate. Its parent must leave `ARGOS_SURREAL_MODE`, `ARGOS_SURREAL_REQUIRED` and `ARGOS_SURREAL_BIN` unset: the standard workspace phase intentionally skips mode-selected storage, and the dedicated storage phase runs it once with embedded mode required. Targeted storage commands set both mode and required explicitly. Preserve the [local/hosted restrictions](ci.md): no local server/all-platform matrix or Rust mutation execution.

Inspect each target's mode contract before estimating gate time. An unset storage mode skips fixtures that use the mode-aware harness; native-only tests that directly open `Target::Embedded` still execute. Currently `storage_classifier_admission`, `storage_admission_maintenance` and `storage_classifier_calibration` run 16 tests in both gate phases, including 15 native database cases. Account for this repeated work and shared fixture locks. A future runner improvement should explicitly partition targets so each behavior executes once with the required backend evidence and the same feature graph; preserve standalone storage-runner coverage and do not silently drop tests.

Run a final gate again only when a material change or failure warrants it. Record the reason. A numerical retry cap must not turn incomplete required verification into a pass, and unchanged full-suite retries are not useful evidence.

## Communicate and improve the workflow

Report completed outcomes, the current blocker and the next decisive check. Derive time estimates from observed compile/test durations, remaining stages and cache state; state uncertainty and whether the final gate has started. Use complete command wall times, including process startup, schema setup and teardown; in-process test or benchmark timers alone can understate the remaining time. Avoid “almost done” when a long required gate remains.

Agents are encouraged to suggest workflow improvements during implementation. Record the concrete problem, supporting evidence, proposed change, expected benefit, tradeoffs and affected guidance. Continue independent authorized work while assessing the proposal. Reversible improvements within the user's scope can be applied and verified without another approval pause. Changes to product behavior, verification requirements, protected paths, model/provider assignments or external actions still follow their applicable authority.

Suggestions are useful input, not a new gate. Prefer one maintained policy with links from entrypoints over conflicting copies. Verify changed guidance links and scan for contradictory instructions before handoff. Keep the final report clear about implementation, verification, outstanding work and any adopted process improvements.
