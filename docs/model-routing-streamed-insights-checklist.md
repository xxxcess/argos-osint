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
| 3.5/3.6 | Every-role capability selection, bidirectional recompile, repair policy | partial | chat executor skips Jev for all generative roles; per-role audit of direct callers is phase 2 |
| 6.1 | Insight stages stream; observer/progress threaded `execute_stage → execute_chat → attempt_insight → attempt_full → read_stream` | done | phase 1 (attempt_insight no longer drops observer) |
| 6.2/6.3 | Typed `AttemptProgress` (dispatched/headers/first output/receiving/heartbeat/complete) + `ModelExecEvent::Progress`; reasoning counted, never text; ~1s throttle; no per-token SQLite write | done (core) | durable `insight_refresh_execution.activity` (schema 33, additive/idempotent); UI display is phase 3 |
| 6.4 | Byte-buffered SSE (split UTF-8), prompt `[DONE]`, length/interrupted/EOF never accepted | done | stream_tests |
| 6.5/6.6 | Policies: ordinary unchanged (10/60/30/120); insight stream 10/60 + first output 180 / inactivity 90 / total 600; heartbeats cannot extend; native decisions & decision chat fallback keep ordinary | done | paused-clock tests |
