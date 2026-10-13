# Intel Brief — Sequential Preparation and Extraction — checklist

Spec: "Argos Intel Brief — Sequential Preparation and Extraction" (2026-10-13, audited main `bb0dc3a`).
Branch `feat/intel-brief-sequential-preparation` from origin/main `bb0dc3a857f0afc300d0ed1e0170a52ede1b73c7` (HEAD recheck: identical; main contains #39 and #40/#41).

Legend: **done** / **partial** / **deferred** / pending. Phases follow §7's order.

| § | Item | Status | Phase / notes |
|---|------|--------|---------------|
| 5 | Reaper race: selection + mutation in one immediate transaction; final CAS rechecks `julianday(lease_until)<=julianday(now)` + owner + epoch | done | 1 · `tasks::interrupt_expired_leases(_in)`; interleaving test |
| 5 | Renewal requires current unexpired ownership | done | 1 · `renew_lease` |
| 5 | Recovery advances epoch, closes matching attempt (`lease_expired`) | done | 1 |
| 5 | Recovery cancels the obsolete in-flight operation | partial | 1 · epoch fence rejects all stale writes/renewals; in-process request cancellation lands with stage executors (phase 3) |
| 5 | Generic and refresh-specific recovery consolidated | done | 1 · `recover_refreshes` calls the shared reaper (removed its non-atomic two-step update) |
| 5 | Worker recoveries tracked separately from provider attempts; bounded infrastructure policy; `max_attempts=1` not permanent on first expiry | done | 1 · `argos_tasks.recoveries`, `MAX_WORKER_RECOVERIES=3`, attempt refunded |
| 5 | `argos_task_dependencies(task_id,depends_on)` + brief manifest (`intel_brief_runs`, `intel_brief_stage_outputs`) | done | 1 · schema 35 (additive) |
| 5 | Ops `intel_brief_{body,claims,inferences,links}` routed to dedicated pool; unrelated/legacy drainers never claim them | done | 1 · `POOL_INTEL_BRIEF` |
| 5 | Dependency, `next_eligible_at` (julianday) and one-active-stage guard in claim SQL | done | 1 |
| 5 | Atomic admission of one chain per article generation; all four tasks persisted; only first claimable | done | 1 · `brief_pipeline::ensure_chain` |
| 5 | Stage commit: immutable output/hash, bind next input, acknowledge, unblock — one transaction, fenced | done | 1 · `commit_stage` |
| 5 | Failure fails only that stage; dependents `Queued · blocked by <stage>`; Retry resumes it only | done | 1 · `stage_views`, `retry_stage` |
| 5 | Reopening never resets an exhausted chain; unchanged completed chain issues no work | done | 1 |
| 5 | New input revisions supersede (epoch fencing); cancellation/deletion stops chain and publication | done | 1 · `supersede_run`, `cancel_chain` |
| 5 | Lease renewal from claim through publication/ack; lightweight SQLite conn + bounded busy retry; blocking DB work off current-thread loop; renewal-failure/heartbeat-age logging | pending | 3 (stage executors) |
| 5 | Visits/body-ready/recovery attach to the chain; remove duplicate `tracked::begin` body jobs | pending | 2–4 (`open_intel_briefing` wiring) |
| 5 | Final Stage-4 atomic publication + index outbox; index failure separate | pending | 3 |
| 2–4 | Four-stage contracts, cleanup order, prompts/wire, budgets, routing | pending | 2–3 |
| 4 | Related leads (`intel_inference_article_leads`) | pending | 4 |
| 6 | Shared stage-progress UI (brief Jobs, section headers, global Jobs, Logs); `artifact_steps` no longer copies parent terminal state | pending | 4 |
| 7 | Docs/diagrams | pending | 5 |
| 8.1 | One chain under repeated visits/reopen/recovery; one active stage; order 1→2→3→4; zero work on unchanged completed | partial | 1 · admission/claim level (`repeated_admission_*`, `overlapping_drainers_*`); model-request counts in phase 3 |
| 8.4 | Stage-3 failure keeps 1/2 completed, 3 failed, 4 blocked; Retry runs 3 only | partial | 1 · task/state level; all-surfaces UI in phase 4 |
| 8.5 | Expiry-selection→renewal→reaper interleaving; crash/reclaim; stale late output; cancellation; overlapping drainers | partial | 1 · done for those; >120s live processing, SQLite contention, publication delay, heartbeat-through-ack: phase 3 |
