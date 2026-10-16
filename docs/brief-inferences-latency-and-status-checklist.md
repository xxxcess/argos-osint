# Brief Inferences latency and status — checklist

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
| 7 | A timeout is `RetryRoute` and the interactive budget allows one identical primary resend | pending (phase C) |
| 8 | Any failing per-claim request aborts the whole Inferences stage | pending (phase D) |

## Phases

| Phase | Scope | State |
| --- | --- | --- |
| A | Typed error category, specific timeout text, task names in attempt history, labelled counts, real retry wait | see log |
| B | Per-model thinking effort for Inferences (never off, no token caps); thinking-sized deadlines | see log |
| C | Skip to the next route on a first-output or inactivity stall; operation wall-clock cap that includes pacing and queue time | pending |
| D | Per-claim checkpoints, re-run only failed claims, complete with warnings so Links can run | pending |
| E | Offline reproductions for the whole path; docs, diagrams, matrix | pending |

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
