---
name: gsd
description: >-
  Babysit open pull requests (CI checks and Cursor bot security reviews) and
  autonomously implement specifications from PRs titled "spec implementation:
  <feature name>" by dispatching per-PR subagents on a ~1-hour scheduled loop
  until all triggered implementations, security fixes, and checks complete.
---

# GSD — PR Babysitter, Security Responder & Spec Implementer

Run a bounded (~1 hour) autonomous monitoring and delivery loop in the active conversation thread:
1. **Babysit open PRs** in `xxxcess/argos-osint` — track hosted CI runs, inspect failed logs, dispatch subagents to implement and push targeted fixes for genuine regressions (or rerun transient runner failures), address **Cursor bot (`cursor[bot]`) security review findings**, and leave merging to the user.
2. **Implement spec PRs via dedicated per-PR subagents** — watch for open PRs titled `spec implementation: <feature name>` (case-insensitive), spawn a subagent per PR (which may break independent modules/docs into disjoint child workers) inside an isolated worktree, extract the spec from the PR body/comments and changed spec files, build performant, secure, and scalable implementations on that PR's branch, update affected docs/diagrams per [conventions](../../../docs/wiki/code/conventions.md), run scoped checks, and commit + push back to the PR branch.
3. **Continue on a timer** for ~60 minutes until the time window expires and all triggered implementations, security fixes, and CI checks settle.

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
   - Inspect `gh pr view <num> -R xxxcess/argos-osint --json title,body,comments,reviews,files,headRefName,baseRefName`.
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
5. **Commit & push regularly as verified slices progress**:
   - Do not hold all edits local until the very end of a multi-part spec: commit and push verified milestone slices (`git add <paths...>`, never `git add .`, push to `origin HEAD:<headRefName>`) regularly as packages/tests pass so PR reviewers, CI, and Cursor see steady incremental progress.
   - Return each pushed `headRefOid` (`pushed_sha`), changed file list, and exact test results so the parent records `status: "ci_pending"` in `.agent-scratch/gsd-state.json` and monitors the resulting CI run.

### 3. Resolve Cursor bot security findings & well-scoped PR implementation comments

On every cycle, inspect `cursor_security_findings` **and** `pr_feedback_comments` reported by `gsd_poll.py` (which scans active, non-stale inline review comments, PR reviews, and issue comments):

1. For each PR with unaddressed Cursor security findings (`unaddressed_cursor_count > 0`) or unaddressed reviewer/implementation feedback comments (`unaddressed_feedback_count > 0`), dispatch or resume that PR's subagent (`invoke_subagent` or `send_message`) in `.agent-scratch/worktrees/pr-<num>`:
   - Inspect every cited `path`, `line`, security explanation, and reviewer request in detail.
   - For security findings, verify the exploit/vulnerability path in the actual codebase — never blindly suppress warnings or weaken validation.
   - For reviewer/implementation feedback, resolve all well-scoped actionable items cleanly within PR boundaries (and explicitly note out-of-scope suggestions rather than expanding scope recklessly).
   - Implement clean root-cause fixes preserving performance and scalability, and add a focused regression test proving each fix where feasible.
2. Validate affected tests + commit/version policy, commit only exact changed files, and push to `origin HEAD:<headRefName>`.
3. Record resolved IDs in `addressed_cursor_comments` / `addressed_feedback_comments` in `.agent-scratch/gsd-state.json` (and reply concisely on the PR/comment referencing the fix commit SHA when appropriate). Wait for Cursor/CI on the new commit to verify no new findings remain.

### 4. Babysit open PR CI checks & backend container logs

For each tracked open PR (both regular open PRs and pushed `spec implementation:` PRs):
1. Check `statusCheckRollup` and latest workflow runs for `headRefOid`.
2. **Pending / queued / in-progress**: leave alone and check on the next cycle.
3. **Failed check or container error/semantic warning**:
   - Fetch failed job logs via `gh run view <run_id> -R xxxcess/argos-osint --log-failed` (and inspect image/Compose smoke service logs for container `ERROR`s or semantically harmful `WARN`s).
   - Distinguish **transient runner/infra issues** (e.g. runner lost communication, macOS capacity timeout, GitHub API 502/rate limit) from **real code/lint/test/service-log failures**.
   - For transient failures: rerun failed jobs with `gh run rerun <run_id> -R xxxcess/argos-osint --failed` (at most twice per commit SHA before flagging as blocked).
   - For real failures or backend container errors / actionable semantic warnings: dispatch or wake that PR's subagent in `.agent-scratch/worktrees/pr-<num>` to fix the root cause cleanly, verify with the affected local test, validate commit message & versions, and push back to `origin HEAD:<headRefName>`.
4. **All checks green, `unaddressed_cursor_count == 0`, `unaddressed_feedback_count == 0`, and no unresolved container errors/actionable warnings**:
   - Mark the PR's current `headRefOid` as `"complete"` in `.agent-scratch/gsd-state.json`.
   - **Do NOT merge the PR.** Leave merging to the user and note that the PR is green and ready for review/merge.

### 5. Schedule next wakeup & exit rule

- **Keep polling** using the `schedule` tool (e.g. `DurationSeconds: 240` for a 4-minute cadence, or `120` when a CI run or Cursor review is near completion).
- **Stop condition**: End the scheduled loop once **both**:
  1. Current time >= `end_at` (~1 hour after `started_at`), **and**
  2. All triggered `spec implementation:` implementations, active `cursor[bot]` security findings, active PR subagents, and their latest CI checks have finished (or are explicitly reported as blocked/failing after retry).
- If `end_at` is reached while a triggered PR subagent, security fix, or its final CI verification is still in flight, finish that item before stopping.
- Provide a concise status table on exit (and whenever new specs complete, security fixes land, or checks turn green/blocked), with clickable `file://` and PR links.
