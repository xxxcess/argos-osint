# Model routing, streamed insights and reliable Brain publication — checklist

Spec: "Argos — model routing, streamed insights and reliable Brain publication" (2026-10-13, audited main 322c528).
Branch `feat/model-routing-streamed-insights`, started from origin/main `322c52842110f360ecd889b31a126f36e1ad76c3` (HEAD recheck: identical to audited SHA).

Status legend: **done** / **partial** / **deferred** / pending.

| § | Item | Status | Notes |
|---|------|--------|-------|
| 1 | Preserve baseline improvements | pending | |
| 3.1 | Central Jev family detection (`provider::is_jev_family`; exact `typesafe/jev`, `-<version>`, `~…-latest`; wire ID preserved; no `contains("/jev")`) | done | phase 1; `routing_tests::jev_family_detection_is_exact` |
| 3.2 | Capability separate from endpoint (`native_decision_capability`); wrong provider/missing key = explicit skip, never chat | done | phase 1; decision executor marks skip before dispatch |
| 3.3/3.4 | Claim Assessor extraction picks first non-Jev chat route; zero-request skips keep route indices; all-Jev → "Claim Assessor extraction requires a non-Jev chat fallback" | done | phase 1; Jev/Jev/Super/Ultra, missing-key, wrong-provider, all-Jev tests |
| 2 | `OperationFormat` (bounded_decision / generated_json / prose) beside `OperationScope`; `OperationFormat::supports` per actual route | done | phase 2 |
| 3.5 | Capability selection for every role; generative ops with Jev primary pick first compatible chat fallback; no cross-role borrowing | done | phase 2: `select_role_secret` walks the complete resolved chain (inheritance + fallbacks) |
| 3.6 | Recompile per fallback (Jev↔chat), one bounded chat repair then declared fallback | done | executor already recompiles per route; covered by mode-classification test (native → chat w/ repair) |
| 6.1 | Insight stages stream; observer/progress threaded `execute_stage → execute_chat → attempt_insight → attempt_full → read_stream` | done | phase 1 (attempt_insight no longer drops observer) |
| 6.2/6.3 | Typed `AttemptProgress` (dispatched/headers/first output/receiving/heartbeat/complete) + `ModelExecEvent::Progress`; reasoning counted, never text; ~1s throttle; no per-token SQLite write | done | phase 1 core + phase 3 UI |
| 6.4 | Byte-buffered SSE (split UTF-8), prompt `[DONE]`, length/interrupted/EOF never accepted | done | stream_tests |
| 6.5/6.6 | Policies: ordinary unchanged (10/60/30/120); insight stream 10/60 + first output 180 / inactivity 90 / total 600; heartbeats cannot extend; native decisions & decision chat fallback keep ordinary | done | paused-clock tests |
| 4 | Mode classification (article + prompt) via complete Classifier chain, native `questions` object keyed by stable `mode` ID, strict labels | done | phase 2; `classify_*_mode_for_role`; legacy single-secret helpers keep inactive status but never send an incapable Jev to chat |
| 4 | Page cleanup as keep/remove decisions over known block IDs (Classifier); Rust computes ranges; verbatim retention | done | phase 2; `BlockDecisions` adapter (native + chat); runtime path uses the captured chain; legacy batch path uses native Jev decisions |
| 4 | Memory predicate/time relevance → Evidence Curator; identity/memory identity → Entity Resolver; batches partitioned by role (≤8) | done | phase 2; `role_for_kind`/`role_batch_end`; checkpoint key includes the role's resolved assignment |
| 4 | Single-secret callers (Intel body/report synthesis+classifier, summary flush, summary route key, actor review, CLI Atlas classifier) → capability selection | done | phase 2 (first compatible route in full chain; full multi-route failover only where the shared executor is used) |
| 4 | Graph explanations use typed source-claim/inference kind | done | phase 2; `Store::memory_insight_kind` (canonical `insight_derivations`), heuristic only for untyped memories |
| 4 | Atlas collection classifier-only, no automatic Brain publication | done (unchanged) | verified no new traffic added |
| 6 (UI) | Intel/Jobs status sequence Queued → Waiting for output → Receiving output → Validating → Published → Indexed; "Waiting for decision"; explicit stall/retry wait/failure; no fake %/token counts; headers never "model activated" | done | phase 3; `insight_execution::status_step`, `Progress.activity`/`activity_age_secs`, Intel `Insights · <step>`, Jobs detail `Activity:` line |
| 6 (Logs) | Activity transitions (dispatch, first output, stream complete, attempt outcome, retry wait) logged as `insight.activity` events, never per delta | done | phase 3 |
| 8.4 | Progress reaches Intel/Jobs/Logs during requests; attempt change resets provisional counters; stage change resets activity | done | `progress_reaches_jobs_and_logs_*`, `status_steps_*`, TUI `streamed_insight_progress_screens` |
