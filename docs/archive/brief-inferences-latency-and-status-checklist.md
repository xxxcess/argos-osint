# Brief Inferences latency and status — checklist

> Maintenance note (2026-10-24): retired code-index tooling notes were removed; other recorded results are unchanged.

> Historical record. For current behavior, start at the [wiki](../index.md).

Audit of main `1d3eb59` (Jobs detail of a partial `intel_brief`: Inferences timed out after 7m51s, categorised `invalid_result`, attempt history showing `#1` three times). Branch `fix/brief-inferences-latency-and-status`, draft PR. Fixes are root-cause and apply to every brief stage (Claims, Inferences, Links, cleanup), not to Inferences alone.

Standing rules: Inferences **always** uses thinking models. There is no faster non-thinking primary. Latency is bounded by skipping to the next route on a stall or timeout, by a per-model reasoning effort wherever the model supports one, and by deadlines that are generous for thinking but bounded by a wall-clock cap.

## Audit claims re-verified against the code

| # | Audit claim | Verdict |
| --- | --- | --- |
| 1 | `StageBudget::policy` sends no token cap or reasoning effort (`request_policy.rs`), so `INFERENCES_BUDGET` is decoration | **confirmed** for the effort (`request_policy.rs` policy returned `Default`; the old test `automatic_operations_have_no_output_or_reasoning_budget` pinned it). Fixed in B. Token caps remain deliberately unsent. |
| 2 | Stream deadlines are 60s headers / 180s first output / 90s inactivity (`provider_attempt.rs`) | **confirmed**, and brief stages stream via `execute_chat(stream=true)`. **Partly wrong:** a 60s *header* wait is not a thinking problem on a stream (SSE headers arrive before thinking); it means the provider was queueing. Only first-output/inactivity are thinking-sensitive, and those are what B resizes. |
| 3 | `summary()` drops the specific message and prints `(HTTP 200)` on a timeout | **confirmed**, fixed in A |
| 4 | `brief_stages.rs` hard-codes `ErrorCategory::InvalidResult` | **confirmed**, fixed in A |
| 5 | The attempt history lists every task's attempt without a name; `Attempts N` is the sum of task starts, not wire requests | **confirmed**, fixed in A |
| 6 | Retry wait is 0 because chain backoff never reaches `retry_wait_ms` | **confirmed**, fixed in A |
| 7 | A timeout is `RetryRoute` and the interactive budget allows one identical primary resend (`provider_chain.rs` `from_failure`, `WireBudget::INTERACTIVE`) | **confirmed**, fixed in C. The HTTP 200 on a timeout means headers arrived and the stream then stalled; Phase A relabelled it. |
| 8 | Any failing per-claim request aborts the whole Inferences stage | pending (phase D) |

## Phases

| Phase | Scope | State |
| --- | --- | --- |
| A | Typed error category, specific timeout text, task names in attempt history, labelled counts, real retry wait | see log |
| B | Per-model thinking effort for Inferences (never off, no token caps); thinking-sized deadlines | see log |
| C | Skip to the next route on a stall (no non-thinking primary); operation wall-clock cap that includes pacing and queue time | see log |
| D | Per-claim checkpoints, re-run only failed claims, complete with warnings so Links can run | see log |
| E | Jobs `r` retry parity; docs, diagrams, final matrix | **done** |

## Phase log

### Phase A — status truthfulness

- [x] `ProviderFailure::summary()` keeps the specific local message ("no model output within 180s") and shows an HTTP status only for >=400; a 2xx that later stalls reads "after HTTP 200 headers" (`provider_diag.rs`).
- [x] `stage_error_category` reads `ModelOperationError.category`; `InvalidResult` remains only for real parse/validation failures (`brief_stages.rs`).
- [x] `ErrorCategory::from_provider` maps provider categories to task categories (`tasks.rs`).
- [x] Attempt rows carry `task_label`; the detail shows `Inferences #1`, `Claims #1`, `Full article #1`, plus `[category]` per attempt (`jobs_view.rs`, `tui/jobs.rs`).
- [x] Counts are labelled `Task attempts 3 · provider requests 4` (`wire_requests` from `recon_model_attempts`).
- [x] Provider backoff credited to `retry_wait_ms`, fenced by lease epoch (`brief_timing::record_retry_wait`).
- Tests: `summary_keeps_the_specific_timeout_and_never_shows_2xx_as_the_failure`, `timeout_failure_keeps_its_category_and_attempt_rows_name_their_task`, `provider_backoff_is_credited_to_retry_wait_and_fenced_by_epoch`, `jobs_detail_names_tasks_labels_counts_and_keeps_the_real_error_category`.
- Screens: `docs/screens/brief-latency/before-jobs-detail-160x90.png`, `after-A-jobs-detail-160x90.png`.

### Phase B — thinking effort and deadlines
- [x] `StageBudget::policy` sends the stage's reasoning effort when it thinks; `Off`/`Default` stages are unchanged (`request_policy.rs`). `ReasoningEffort::thinks()`, `StageBudget::thinks()`.
- [x] `Efficient` resolves per model to a thinking-on value (profiles unchanged: Gemini `low`, Super `low`, Ultra `medium`, Gemma `high`, lightning `enable_thinking`). OpenRouter catalog evidence now confirms `reasoning.effort` low/medium/high with `low` as efficient (`provider_capabilities.rs`). Unsupported models omit the field and report it.
- [x] No output/reasoning ceilings are sent for any stage (unchanged decision).
- [x] Thinking-sized deadlines selected by `OperationScope::deadlines` (`provider_attempt.rs`, `recon/model_exec.rs`): stream first output 300s, inactivity 120s, total 600s; non-stream headers 540s. Other stages keep their previous deadlines.
- [ ] Moving to the next route on a stall, and the wall-clock cap: phase C.
- Tests: `inferences_send_a_per_model_thinking_effort_and_never_turn_thinking_off`, `automatic_stages_leave_output_and_reasoning_limits_to_the_provider` (amended), `automatic_operations_have_no_output_caps_and_only_thinking_stages_send_effort`, `thinking_stages_get_thinking_sized_deadlines_and_others_keep_theirs`, `inferences_send_their_per_model_effort_on_the_wire_and_claims_do_not` (mock server, request body inspected).

### Phase C — fallback on stall and wall-clock cap
- Verified: route 0 repeated because `Category::Timeout` is `RetryRoute` and `WireBudget::INTERACTIVE` grants route 0 one transient retry after a 10s backoff; two ~180s first-output stalls plus a third route account for the ~7m51s. Provider pacing (`provider_rate_limit::wait`, before the attempt deadline starts) and the 50ms admission poll loop were unbounded; a 429/503 cooldown could add 10-60s per send. Claim 4 (`invalid_result`) was already fixed and re-confirmed in A.
- [x] `WireBudget.skip_stalled_route`: a timed-out route falls through to the next dispatchable route with no backoff (`provider_chain.rs`). Inferences keeps thinking models; there is no faster primary.
- [x] `WireBudget.wall_clock` (600s; 900s for thinking stages via `INTERACTIVE_THINKING`) bounds queue, pacing, backoff and dispatch; `DispatchContext.remaining` bounds each attempt (`model_exec::bounded_by_cap`), a long backoff is skipped, expiry reports `timeout`.
- [x] `ChainReport.wall_clock_exceeded` and the "operation wall-clock cap reached" termination text.
- Tests: `a_stalled_route_falls_through_instead_of_repeating`, `transient_failures_keep_their_retry_and_the_last_route_retries_a_stall`, `the_wall_clock_cap_ends_the_operation_with_a_timeout`, `a_backoff_longer_than_the_remaining_cap_moves_on`, `interactive_operations_have_a_wall_clock_cap_sized_for_thinking`, `an_attempt_that_outlives_the_cap_becomes_a_typed_timeout`.

### Phase D — degrade instead of fail
- Verified claim 7: Links blocked by a *failed* Inferences is correct (nothing to link). The defect was that one claim's failure failed the whole stage (claim 8, confirmed), so Links never saw the claims that did succeed.
- [x] Each claim's Inferences result is checkpointed (`brief_pipeline::checkpoint_put/get`, keyed by job, stage, claim and a hash of prompt + request + resolved route, stored in `app_state`). Reused on retry, cleared on a clean commit.
- [x] A failing claim is recorded (`claim_failures`, `claim_analysis[].analysis_failed` with its provider category); siblings keep running; cancellation still aborts. All claims failing still fails the stage with the provider category (timeout).
- [x] Some claims failed: the stage commits `warning: N of M claims could not be analyzed (categories)`, shows "Completed with warnings", Links runs on the analyzed claims.
- [x] `retry_stage` also reopens a completed-with-warnings stage and its descendants; checkpoints mean only the failed claims are asked again; Links reruns on the full set. Warning outputs are never adopted by cross-generation reuse.
- [x] Jobs detail shows `Warning · Inferences: …` and the partial-result hint (`JobDetail.warnings`).
- Not done: fidelity/dedup failures after all claims succeed still fail the stage (claim checkpoints survive, so a retry re-runs only those steps); Links itself does not degrade on its own failure.
- Tests: `one_failed_claim_completes_with_warnings_and_retry_asks_only_that_claim`, `every_claim_failing_fails_inferences_with_the_provider_category`, `jobs_detail_shows_partial_completion_with_warnings_and_links_completed`.
- Screens: `docs/screens/brief-latency/before-jobs-detail-160x90.png` (main: Inferences failed, Links blocked), `after-A-…` (status fixed, still failed), `after-D-jobs-partial-160x90.png` (completed with warnings, Links completed).

### Phase E — parity, docs and final matrix
- [x] Jobs panel `r` now uses `retry_stage` for brief chains (small and safe: `jobs_view::retry_failed_tasks` delegates for chains with an `intel_brief_runs` row, and `JobDetail.retryable` counts failed branches plus a warning stage). Test: `jobs_retry_resumes_failed_and_warning_brief_stages`.
- [x] Diagrams updated: `intel-brief-chain` (per-claim checkpoints, warning completion, Retry), `intel-job-states` (Retry resumes, warnings), `model-request-scheduling` (stall falls through, wall-clock cap). `docs/wiki/code/architecture.md` and `docs/wiki/concepts/providers.md` updated.
- [x] Final matrix (all with `--locked`, clippy `-D warnings`): default features (LanceDB), `--no-default-features`, `--features nlp` — results in the PR body.

### Phase F — stage-wide request budget and route health (user's imported profile)
Root cause of 84 requests (offline repro with the imported chain, `brief_user_chain_tests.rs`): classifier work is one operation per ≤8-sentence batch (3 claims x ~5 batches = 15 sequential operations on a 37-sentence article), and each operation re-walked the chain from route 0. With the first routes down (Jev/OpenRouter 503 or SSE `A Timeout Occurred`, Gemini 503) each operation cost 4 requests: 60 of 80 requests were classifier, none of them able to learn the route was dead. `WireBudget` and the wall-clock cap were per operation, so nothing bounded the stage; the `SseError` timeout was retried on the same route.
- [x] `StageLedger` (provider_chain): one per stage execution, shared through `StageContext`/`OperationScope`. Route health: a route that failed unwell (timeout, 5xx, 429, network, stream, auth) is passed over for 120/300/600s by sibling operations when a later route is available; the last dispatchable route is never skipped; success clears it.
- [x] Stage request budget: dispatch ceiling (default 24; Inferences declares `2 x (batches + claims + 2) + 8`). Exhaustion stops dispatch with `stage request budget reached` (timeout category) instead of cycling providers.
- [x] SSE error events whose text/code say timeout (`timeout`, `timed out`, 408/504/524) are `Category::Timeout`: skip to the next route, no same-route resend.
- [x] Classifier batches across claims run with the same 3 slots as generation (`classify_context` future owns its state). Evidence policy unchanged: every candidate is still classified.
- Result on the repro: healthy 20 requests (unchanged); first routes down 80 -> 32. Inferences still use thinking routes with fallback; no model or effort change.
- Tests: `dead_first_routes_do_not_multiply_across_operations`, `a_route_that_just_stalled_is_passed_over_by_sibling_operations`, `the_last_dispatchable_route_is_never_passed_over`, `the_stage_budget_stops_dispatch_across_operations`.
- Not done: a classifier failure still fails the whole stage (not per claim); no change to batch size (Jev question limit).

## Audit claims: final verdicts
| # | Claim | Verdict |
| --- | --- | --- |
| 1 | No effort/caps sent; INFERENCES_BUDGET decorative | confirmed (effort); fixed B |
| 2 | Deadlines too short for thinking | partly wrong (60s header wait is provider queueing on a stream); first-output/inactivity resized in B |
| 3 | `(HTTP 200)` on a timeout / error text | confirmed; fixed A |
| 4 | `invalid_result` hard-coded | confirmed; fixed A |
| 5 | Attempt rows unnamed; counts conflated | confirmed; fixed A |
| 6 | Retry wait 0 | confirmed; fixed A |
| 7 | Timeout repeats route 0 / pacing unbounded | confirmed; fixed C |
| 8 | One claim failure fails Inferences; Links blocked | confirmed; fixed D |
