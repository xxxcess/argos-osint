---
name: gsd
description: >-
  Babysit open pull requests (CI checks and Cursor bot security reviews) and
  autonomously implement specifications from PRs titled "spec implementation:
  <feature name>" by dispatching per-PR subagents on a ~1-hour scheduled loop
  until all triggered implementations, security fixes, and checks complete.
  Every nightly run keeps backend and artifact jobs green first, then builds a
  fully featured Linux x86_64 native app with its Linux CI jobs green, and only
  then fixes the other platform app builds.
---

# GSD — PR Babysitter, Security Responder & Spec Implementer

Run a bounded (~1 hour) autonomous monitoring and delivery loop in the active conversation thread:
1. **Babysit open PRs** in `xxxcess/argos-osint` — track hosted CI runs, inspect failed logs, dispatch subagents to implement and push targeted fixes for genuine regressions (or rerun transient runner failures), address **Cursor bot (`cursor[bot]`) security review findings**, and leave merging to the user.
2. **Implement spec PRs via dedicated per-PR subagents** — watch for open PRs titled `spec implementation: <feature name>` (case-insensitive), spawn a subagent per PR (which may break independent modules/docs into disjoint child workers) inside an isolated worktree, extract the spec from the PR body/comments and changed spec files, build performant, secure, and scalable implementations on that PR's branch, update affected docs/diagrams per [conventions](../../../docs/wiki/code/conventions.md), run scoped checks, and commit + push back to the PR branch.
3. **Continue on a timer** for ~60 minutes until the time window expires and all triggered implementations, security fixes, and CI checks settle.

---

## Nightly workflow (canonical)

**Priority order on every nightly run:**

1. **Keep the backend suite and its artifacts stable at all times.** Backend checks, embedded storage, release images (Docker), search index, server timings and any other backend artifact jobs must stay green. A backend or artifact failure is fixed first, never deferred.
2. **Then get a fully featured Linux x86_64 native app with its Linux app CI jobs green.**
3. **Only then fix the other platform app builds** (Mac aarch64, Mac x86_64 and other OS targets). Scoped web and other remaining checks are fixed after step 2 as well.

CI enforces the same order in [fleetworks.yml](../../../.github/workflows/fleetworks.yml). Backend, embedded storage, image, search-index, server-timing and Linux x86_64 app jobs block `Fleetworks source checks`, `Fleetworks artifact checks` and **Fleetworks required**. The Mac app jobs wait for the Linux app job (`needs:`), still run and report (a red job stays red, with a warning annotation), and do not block those gates except on a trusted publication run, which attaches the macOS ZIPs. Job names are unchanged.

The **backend gate** is every backend and artifact job on the PR head: `Backend suite / Linux amd64` and `/ Linux arm64`, `Affected embedded storage` (x86_64 and aarch64), `Image checks / Linux amd64` and `/ Linux arm64`, and the Atlas BERT runtime jobs when selected ([CI guide](../../../docs/wiki/code/ci.md#fleetworksyml--scoped-branch-checks)). The **Linux gate** is the hosted job `Linux app / x86_64-unknown-linux-gnu / Build, verify and package Linux native (debug)` ([fleetworks-native-linux.yml](../../../.github/workflows/fleetworks-native-linux.yml), scope `desktop_linux`) passing every step: engine-free check, `cargo fmt --check`, scoped unit tests for `fleetworks-desktop`, `fleetworks-workstation` and `fleetworks-contracts`, native UI captures, `build-native` with the companion `argos-api-host`, and the Docker-free local-backend smoke ([native app evidence](../../../docs/wiki/code/ci.md#native-app-review-evidence)). "Fully featured" means every spec item that touches the native app is present in that build, not stubbed. A failure shared by a Linux job and a Mac job (the same test, the same format diff) counts as a Linux failure.

Run these steps in order. The loop sections below keep their detail.

1. **Poll.** Run `gsd_poll.py` ([Routine Loop](#routine-loop-per-poll-cycle) step 1). Skip release-bot and fork (`isCrossRepository`) PRs. Never merge.
2. **Work in the PR worktree.** For each actionable PR use `.agent-scratch/worktrees/pr-<n>`, checked out to the PR head. Do not touch the main checkout, `main` or `nightly`.
3. **Read owner comments as requirements.** New owner comments on the PR (and in the session) are new requirements. Track each one in `.agent-scratch/gsd-state.json` under `prs.<n>.requirements` with its comment id and `open | done`; mark one done only with the pushed sha that satisfies it.
4. **Build from the spec in priority order.** Never leave a backend or artifact job red to work on the Linux app, and never spend a push on a Mac or other-OS failure while the backend gate or the Linux gate is red, unless the same fix clears them.
5. **Cheap local checks only.** CI is the test runner. Run locally only: `rustfmt --check --edition 2021 apps/fleetworks-desktop/src/lib.rs` (it follows the module tree) or `cargo fmt --check`, `prettier --check` on changed web and Markdown files, `python3 scripts/fleetworks_doc_links.py` (0 broken), and `node scripts/release_check.mjs message --file <msg>` with `node scripts/release_check.mjs versions`. No full builds, no full test suites and no mutation runs ([root rules](../../../AGENTS.md)).
6. **Commit and push.** Stage exact paths, use Conventional Commits and one small commit per slice. Before the first push of a run, check `git merge-tree --write-tree origin/<base> HEAD`. Push with the full branch name and a literal sha: `git push origin <sha>:refs/heads/<headRefName>`, with no shell variables, substitutions or brace expansion. Never force-push.
7. **Read failed CI once.** After a push, read each failed job log once (`gh api --allow-escape-sequences repos/xxxcess/argos-osint/actions/jobs/<id>/logs`) instead of waiting in a loop; the owner or the next scheduled wake reports CI events. Plain `gh pr view` can fail, so read PRs through `gh api repos/xxxcess/argos-osint/pulls/<n>`.
8. **Fix in priority order.**

   | Order | Failing jobs | Action |
   | --- | --- | --- |
   | 1 | Backend suite, embedded storage, release images, search index, server timings, other backend artifact jobs | Fix first, push, re-read |
   | 2 | `Linux app / x86_64-unknown-linux-gnu` | Fix once the backend gate is green on the head |
   | 3 | Mac app aarch64 and x86_64, other OS targets, Scoped web | Fix last |

   A runner-capacity or toolchain failure (lost runner, no capacity, GitHub 5xx, a C build such as `blake3`) is not our code: rerun the failed jobs at most twice per commit with `gh api -X POST repos/xxxcess/argos-osint/actions/runs/<id>/rerun-failed-jobs` once the run has completed, then report it without changing code. Fix real failures at the root cause. Refresh visual-regression baselines only from CI capture artifacts, never from local renders.
9. **Keep docs and design in sync.** Update affected docs, tags and diagrams in the same change ([conventions](../../../docs/wiki/code/conventions.md)). Keep [docs/design/matte-canvas](../../../docs/design/matte-canvas/README.md) consistent with the code and add reproduction steps for anything generated or derived. Do not touch work the owner put on hold.
10. **Update state.** After each push and each CI read, write `head_sha`, `status`, `backend_gate` and `linux_gate` (each `pending | failing | green`), `requirements`, `addressed_cursor_comments`, `worktree` and `notes` to `.agent-scratch/gsd-state.json`.
11. **One status comment.** Post or update a single PR comment that starts with `<!-- gsd-status -->`: head sha, backend gate, Linux gate, other job results, open requirements. Create it with `gh api repos/xxxcess/argos-osint/issues/<n>/comments -f body='...'` and update it with `gh api -X PATCH repos/xxxcess/argos-osint/issues/comments/<id> -f body='...'`. Always pass the body inline, never read from a file.
12. **Leave visual verification to reviewers.** Other agents and reviewers inspect screenshots; a nightly run only refreshes baselines from CI artifacts. Report and end the turn; do not wait on CI.

---

## Performance, Security & Scalability Bar

Every implementation and fix produced by GSD or its subagents must satisfy all three axes without shortcuts:

- **Security**:
  - Enforce authentication, origin/pairing binding, and capability/role boundaries strictly; never weaken checks or fall back to unauthenticated/insecure paths.
  - Validate and bound all external, HTTP, IPC, and workflow inputs (exact SHA/path pinning, parameterized SurrealQL/SQL queries instead of string interpolation, path traversal & SSRF guards, minimal `permissions:` in GitHub Actions, no secret leakage in logs/errors).
- **Performance**:
  - Avoid unnecessary allocations, clones, and O(N²) loops in hot paths; eliminate N+1 storage/API round-trips in favor of batched/indexed queries or parallel bounded requests.
  - Never block async Tokio executors with sync CPU/file I/O (`spawn_blocking` / async primitives when needed); stream or cap large payloads and reuse existing connections/caches.
- **Scalability**:
  - Enforce explicit upper bounds on queues, concurrency (`Semaphore` / bounded channels), batch sizes, cursor/keyset pagination, memory/CPU cgroup budgets, and retry backoff with idempotency keys so throughput holds as datasets and active jobs grow.
- **Backend Container Log Hygiene & Semantic Warning Triage**:
  - Inspect backend service container logs (from CI image/Compose smoke logs and any authorized isolated Compose stack under verification) for `ERROR`, `PANIC`, `FATAL`, crash restarts, and `WARN` entries.
  - **Resolve all container `ERROR` logs** from first-party backend services (`api`, `brain`, `brief-worker`, `worker`, `native-worker`, `control`, `execution`, `tools`, `jobs`, `search-index`) with a root-cause fix and regression test before considering work complete.
  - **Evaluate every `WARN` log semantically** rather than ignoring or blindly muting it:
    - Fix warnings whose semantics indicate silent fallback, lost leases/locks, dropped batches/messages, resource/cgroup saturation, unbounded queues, broken config/wiring, or security exposure.
    - Verify explicitly when a warning reflects normal idempotent startup or recoverable backpressure (e.g. `429 write_paced` defer + intact replay), and address client pacing/log noise if retries are avoidable.
- **Simulated Backend Log Verification (Native App Checks)**:
  - Keep `apps/fleetworks-desktop` engine-free (no Docker/backend dependency inside native Cargo tests) while verifying log-consuming, readiness, job-streaming, and backpressure/error UI behaviors against **simulated backend log streams**:
    1. **Loopback mock gateway (`TcpListener` + `fleetworks_contracts::operational_logs` over `127.0.0.1:0` driving `rune::Runtime`)**: Mirror the `TcpListener` + delayed-response seams in [maintenance tests](../../../apps/fleetworks-desktop/src/maintenance/tests.rs) and [jobs query tests](../../../apps/fleetworks-desktop/src/jobs_query_tests.rs) alongside [log tests](../../../apps/fleetworks-desktop/src/log_tests.rs) so actual native HTTP/JSON transport, follow cursors, stale-epoch/cancel races, and Rune UI state/tree transitions run end-to-end offline.
    2. **Semantic log scenario replay**: Drive sequences of synthetic `info`, recoverable `warn` (e.g. `status=429` `write_paced`, `correction_required=false`), fatal `error` (`correction_required=true`, truncated/malformed pages, HTTP `503`), and high-volume `DISPLAY_LIMIT` bursts; assert bounded memory/table size, intact cursor continuation, secret/token redaction, and accurate UI surfacing.
    3. **Review & test gate**: When implementing or reviewing changes affecting operational logs, jobs/cycle progress, readiness diagnostics, or retry/error handling, require simulated backend-log `Runtime` tests in `cargo test --locked -p fleetworks-desktop` (which executes in CI via [fleetworks-native.yml](../../../.github/workflows/fleetworks-native.yml)).

---

## Subagent Delegation Architecture (Per PR)

Keep the root GSD loop thin so it can monitor multiple PRs concurrently without losing responsiveness or colliding on files:

1. **One PR Lead Subagent per actionable PR** (`invoke_subagent` with `TypeName: "self"` and an explicit isolated worktree `.agent-scratch/worktrees/pr-<number>`):
   - Spawn one PR lead subagent whenever a `spec implementation: <feature name>` PR needs implementation, or an open PR needs CI-failure / `cursor[bot]` security fixes.
   - Assign **disjoint ownership**: each PR lead operates strictly inside `.agent-scratch/worktrees/pr-<number>` (checked out to `origin/<headRefName>`), keeping the main checkout's dirty files and `dist/` untouched.
   - Store `{ "conversation_id": "...", "head_sha": "...", "status": "..." }` in `.agent-scratch/gsd-state.json` under `prs.<num>.subagent`. If follow-up failures or Cursor comments arrive on that PR while its lead subagent is still alive, wake it with `send_message` instead of spawning a second conflicting worker on the same branch.
2. **Work decomposition inside a PR**:
   - For multi-part specs or cross-stack changes (e.g., Rust core/API + React web + docs/diagrams), the PR lead breaks the spec into concrete units and spawns targeted subagents with **disjoint file lists**:
     - Use `TypeName: "research"` subagents in parallel to inspect impacted symbols, existing helpers, and security/performance constraints.
     - Use `TypeName: "self"` subagents (or sequential phases) for disjoint code/test/doc slices inside `.agent-scratch/worktrees/pr-<number>`, passing exact spec acceptance, non-goals, and owned paths.
   - Keep **one integration/build owner** per PR worktree (and follow [cache hygiene](../../../docs/wiki/code/contributing.md#build-cache-hygiene): use `python3 scripts/agent_cargo.py ... --locked` with shared `target/agents/debug`; never run two concurrent Cargo builds against the same target cache).
3. **Reactive wakeups (no polling loops)**:
   - Never busy-poll subagents; let `invoke_subagent` / `send_message` notifications and the `schedule` timer wake the coordinator naturally.

---

## State & Paths

- **State file** (unstaged scratch): `.agent-scratch/gsd-state.json`
  Tracks `started_at`, `end_at` (`started_at + 3600s`), `last_poll_at`, and per-PR state (`head_sha`, `kind`: `babysit | spec_impl`, `status`: `queued | implementing | verifying | pushed | ci_pending | complete | blocked`, `addressed_cursor_comments`, `subagent`, `worktree`, `notes`).
- **Isolated worktrees**: `.agent-scratch/worktrees/pr-<number>`
  One worktree per PR branch so concurrent PR subagents never clash with each other or with unrelated local edits.
- **Tool PATH**: Always prepend `/opt/homebrew/bin:/usr/local/bin` before invoking `gh`, `node`, or `git`:
  ```sh
  export PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"
  ```

---

## Routine Loop (Per Poll Cycle)

### 1. Poll open PRs, CI status, and Cursor security findings

Run the helper script:

```sh
export PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"
python3 .agents/skills/gsd/scripts/gsd_poll.py --repo xxxcess/argos-osint --state .agent-scratch/gsd-state.json
```

Pass `--reset` only when starting a fresh ~1-hour GSD session. Skip external fork PRs (`isCrossRepository: true`) for direct branch pushes and skip automated dependency/release-bot PRs (e.g. `release-please--branches--main*`) unless explicitly requested.

### 2. Dispatch per-PR subagents for `spec implementation: <feature name>` PRs

Match any open PR whose title starts with `spec implementation:` (case-insensitive) and whose latest `headRefOid` has not already been implemented and pushed by GSD (check `.agent-scratch/gsd-state.json` + recent commits on the PR branch).

For each new or updated `spec implementation: <feature name>` PR, spawn a dedicated PR Lead subagent (`invoke_subagent`, `TypeName: "self"`) instructed to:
1. **Read the full spec**:
   - Read the PR with `gh api repos/xxxcess/argos-osint/pulls/<num>` plus its issue comments and reviews (plain `gh pr view` can fail).
   - Read any spec/plan/design markdown files added or modified in the PR.
2. **Create/sync the PR's isolated worktree**:
   - `git fetch origin <headRefName>` and ensure `.agent-scratch/worktrees/pr-<num>` points to `origin/<headRefName>`.
   - Respect repository rules (`AGENTS.md`, `argos-rust-style`, `argos-react-ui`, `argos-react-patterns`, `fleetworks-rune-verify` as applicable) and enforce the **Performance, Security & Scalability Bar**.
3. **Decompose, implement & document**:
   - Break multi-component specs into parallel research/implementation subagents with disjoint `OWNED_FILES` inside `.agent-scratch/worktrees/pr-<num>`, then integrate cleanly.
   - Update affected docs under `docs/wiki/` and paired `docs/diagrams/*.{html,svg}` if behavior, APIs, or architecture changed, and verify with `python3 scripts/fleetworks_doc_links.py`.
4. **Scoped local checks & policy validation**:
   - Run targeted unit/integration checks, linters, container log checks (when a runtime stack is exercised), simulated backend-log `Runtime` tests (when native log/readiness/jobs UI or contracts change), and performance/security edge-case tests covering the changed stack. Leave full hosted matrices to GitHub CI.
   - Validate Conventional Commit formatting and shared versions before committing:
     ```sh
     node scripts/release_check.mjs message --file <msg_file>
     node scripts/release_check.mjs versions
     ```
5. **Commit, push & report back**:
   - Stage only the exact changed files (`git add <paths...>`, never `git add .`).
   - Commit and push with the full branch name and a literal sha (`git push origin <sha>:refs/heads/<headRefName>`), never force-push.
   - Return the new `headRefOid` (`pushed_sha`), changed file list, and exact test results so the parent records `status: "ci_pending"` in `.agent-scratch/gsd-state.json` and monitors the resulting CI run.

### 3. Dispatch fixes for Cursor bot (`cursor[bot]`) security findings

On every cycle, inspect `cursor_security_findings` reported by `gsd_poll.py` (which scans active, non-stale inline review comments, PR reviews, and issue comments from `cursor[bot]`):

1. For each PR with unaddressed findings (`unaddressed: true`), dispatch or resume that PR's subagent (`invoke_subagent` or `send_message`) in `.agent-scratch/worktrees/pr-<num>`:
   - Inspect every cited `path`, `line`, and security explanation in detail.
   - Verify the exploit/vulnerability path in the actual codebase — never blindly suppress warnings or weaken validation.
   - Implement a root-cause security fix preserving performance and scalability, and add a focused regression test proving the fix where feasible.
2. Validate affected tests + commit/version policy, commit only exact changed files, and push with the full branch name and a literal sha (`git push origin <sha>:refs/heads/<headRefName>`).
3. Record each resolved finding's `uid` (e.g., `review_comment:4573016625`) in `addressed_cursor_comments` in `.agent-scratch/gsd-state.json` (and reply concisely on the PR/comment referencing the fix commit SHA if helpful). Wait for Cursor's next review pass on the new commit to verify no new findings remain.

### 4. Babysit open PR CI checks & backend container logs

For each tracked open PR (both regular open PRs and pushed `spec implementation:` PRs):
1. Check `statusCheckRollup` and latest workflow runs for `headRefOid`.
2. **Pending / queued / in-progress**: leave alone and check on the next cycle.
3. **Failed check or container error/semantic warning**:
   - Fetch failed job logs via `gh run view <run_id> -R xxxcess/argos-osint --log-failed` (and inspect image/Compose smoke service logs for container `ERROR`s or semantically harmful `WARN`s).
   - Distinguish **transient runner/infra issues** (e.g. runner lost communication, macOS capacity timeout, GitHub API 502/rate limit) from **real code/lint/test/service-log failures**.
   - For transient failures: rerun failed jobs with `gh api -X POST repos/xxxcess/argos-osint/actions/runs/<run_id>/rerun-failed-jobs` after the run completes (at most twice per commit SHA before flagging as blocked; do not change code for a runner or toolchain problem).
   - For real failures or backend container errors / actionable semantic warnings: dispatch or wake that PR's subagent in `.agent-scratch/worktrees/pr-<num>` to fix the root cause cleanly, verify with the affected local test, validate commit message & versions, and push back by full branch name and literal sha. Fix in the [priority order](#nightly-workflow-canonical).
4. **All checks green and `unaddressed_cursor_count == 0` and no unresolved container errors/actionable warnings**:
   - Mark the PR's current `headRefOid` as `"complete"` in `.agent-scratch/gsd-state.json`.
   - **Do NOT merge the PR.** Leave merging to the user and note that the PR is green and ready for review/merge.

### 5. Schedule next wakeup & exit rule

- **Wake on events, not in-turn loops.** Wake on the `schedule` timer or when the owner reports a CI event; after a push, report and end the turn instead of waiting on CI.
- **Stop condition**: End the scheduled loop once **both**:
  1. Current time >= `end_at` (~1 hour after `started_at`), **and**
  2. All triggered `spec implementation:` implementations, active `cursor[bot]` security findings, active PR subagents, and their latest CI checks have finished (or are explicitly reported as blocked/failing after retry).
- If `end_at` is reached while a triggered PR subagent, security fix, or its final CI verification is still in flight, finish that item before stopping.
- Provide a concise status table on exit (and whenever new specs complete, security fixes land, or checks turn green/blocked), with clickable `file://` and PR links.
