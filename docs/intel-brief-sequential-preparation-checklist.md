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
| 3 | Cleanup order: local baseline → NLP annotation → conditional Classifier (ambiguous blocks only, ≤12/batch, neighbour context, default keep) → local verbatim removals → revalidation + span maps + content hash | done (`intel_recon/clean_body.rs`) | 2 |
| 3 | Spec cleanup prompt used for Classifier block decisions | done (`BLOCK_SYSTEM`, `CLEANUP_PROMPT`) | 2 |
| 3 | No Synthesis rewrite; display and extraction consume one `intel-clean-body-v1` artifact | done (`commit_refined_body`, `article_input`) | 2 |
| 3 | NLP mentions as keep-anchors | partial: title anchors used; NLP annotation still runs/caches but its mentions are not yet fed to the ambiguity heuristic | 2 |
| 3 | Stage 1 fails on missing/truncated/unvalidated story | done: executor enforces `stage_one_admissible` with the acquisition receipt (`acquisition_complete`) | 2–3 |
| 4 | Spec prompts for Claims / Inferences / Links, strict JSON, short IDs, Rust-resolved UTF-8 spans | done (`intel_recon/brief_stages.rs`) | 3 |
| 4 | Local field/count limits (6 claims ≤35w; 5 inferences ≤35w/why ≤45w/≤2 limitations ≤20w; 5 leads ≤40w), unknown-ID / duplicate / fence / links-in-inference rejection | done | 3 |
| 4 | Budgets 2048/3072/1536 through `OperationScope::with_completion_tokens` → `attempt_insight` (replaces fixed 8,192 in this lane) | done; cleanup batches 512 (`CLEANUP_TOKENS`, 4b) | 3 |
| 4 | Role routing: Claims → claim_assessor, Inferences/Links → synthesis; Jev skip / all-Jev failure via existing `execute_chat` | done (reused) | 3 |
| 4 | Long bodies: ordered windows within Stage 3 carrying ≤5 proposals, later windows revise, five total | done | 3 |
| 4 | Stage 4 local shortlist (self excluded, canonical URL + syndicated-title dedupe, cap 12); no-inference / no-candidate finishes locally | done | 3 |
| 4 | Batched Claim-assessor fidelity decisions inside Stages 2/3 | done (`brief_fidelity.rs`: ≤12/batch, 1024 tokens, strict label check, receipts cached `intel-fidelity:{hash}`, unfaithful items withheld) | 4b |
| 4 | `intel_inference_article_leads` persistence | done (schema 36, written in the Stage-4 publication) | 4a |
| 4 | "Potential corroboration" rendering in Links | done (metadata leads, not confirmed support) | 4b |
| 5 | Heartbeat from claim through commit/ack on a light connection with bounded busy retry; renewal failures + heartbeat age recorded | done (`Store::open_light`, `intel-brief-heartbeat:*`) | 3 |
| 5 | Cancellation / deletion / lost lease drop the in-flight request; nothing is written | done | 3 |
| 5 | Stage failure fails only that stage (model chain is the single retry owner); Retry resumes it | done | 3 |
| 5 | Brief-owned body retrieval does not schedule a competing article refresh | done (`commit_refined_body`) | 3 |
| 5 | Final Stage-4 publication: combined snapshot (`intel_brief_publications`) + leads in the fenced Stage-4 commit; previous kept until success; older generation never overwrites newer; separate from report snapshots | done | 4a |
| 5 | Canonical Brain derivations + index outbox from the brief snapshot | deferred: fidelity now available; canonical `publish_snapshot` is bound to insight-refresh leases/PipelineOutput types | 5 |
| 2 | `ensure_intel_brief_pipeline` wired into `open_intel_briefing`; no separate refresh/body job on visit; dedicated `argos-intel-brief` runner (multi-thread runtime) in `WorkerPool::spawn_default`; UI polls stage views each tick and reloads on change; Retry resumes the failed stage | done | 4a |
| 2 | Mode classification lazy (Recon configuration) | done | 4a |
| 2 | Reload (force refresh) through the chain instead of the legacy body job | done (`reload_intel_brief_pipeline`, `force_body`; legacy `ensure_article_body_fetch` + `WorkEvent::IntelBody` removed) | 4b |
| 3 | Synthesis body-rewrite functions removed | done | 3 |
| 8.1 | One chain, 1→2→3→4 order, scoped inputs, zero requests on unchanged reopen | done (`brief_stages_tests.rs`) | 3 |
| 8.2 | Validator rejections + UTF-8 spans + shortlist dedupe | done | 3 |
| 8.4 | Stage-3 failure isolation, Retry runs Stage 3 only, empty arrays complete, no-inference Stage 4 uses no model | done | 3 |
| 8.5 | Heartbeat beyond lease with concurrent reapers, stale output fenced after epoch change, cancellation/parent cancel drop in-flight request, attempts preserved | done; SQLite contention / publication delay / overlapping drainers partially (reaper thread only) | 3 |
| 8.3 | Cleanup regressions (byline/quote/negation/correction/Unicode verbatim, span maps, classifier missing/failing keeps all, ≤12 sequential batches, degrading removals rejected, incomplete story fails) | done (`clean_body_tests.rs`, 6 tests) | 2 |
| 4 | Related leads (`intel_inference_article_leads`) | done | 4a |
| 6 | Shared stage-progress UI (brief Jobs, section headers, global Jobs, Logs); `artifact_steps` no longer copies parent terminal state | done (Step k/4, per-stage rows, one spinner, activity labels, global Jobs `stage_label`, `intel_brief.stage` log events; screens in `docs/screens/intel-sequential/`) | 4b |
| 7 | Docs/diagrams | pending | 5 |
| 8.1 | One chain under repeated visits/reopen/recovery; one active stage; order 1→2→3→4; zero work on unchanged completed | partial | 1 · admission/claim level (`repeated_admission_*`, `overlapping_drainers_*`); model-request counts in phase 3 |
| 8.4 | Stage-3 failure keeps 1/2 completed, 3 failed, 4 blocked; Retry runs 3 only | done | 1 + 4b (`intel_sequential_stage_screens` asserts brief Jobs, panes, global Jobs) |
| 8.5 | Expiry-selection→renewal→reaper interleaving; crash/reclaim; stale late output; cancellation; overlapping drainers | partial | 1 · done for those; >120s live processing, SQLite contention, publication delay, heartbeat-through-ack: phase 3 |
