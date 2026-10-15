# Intel Brief — Sequential Preparation and Extraction — checklist

Spec: "Argos Intel Brief — Sequential Preparation and Extraction" (2026-10-13, audited main `bb0dc3a`).
Branch `feat/intel-brief-sequential-preparation` from origin/main `bb0dc3a857f0afc300d0ed1e0170a52ede1b73c7`. Rechecked at phase 5: main has not moved, so no merge was needed.

Legend: **done** / **partial** / **deferred**. Phases follow §7's order (1 tasks, 2 cleanup, 3 stages, 4a wiring + publication, 4b UI + fidelity, 5 canonical + anchors + §8 + docs).

## §2–§5 Orchestration, cleanup, stages, publication

| § | Item | Status | Phase / evidence |
|---|------|--------|------------------|
| 5 | Reaper race: selection + mutation in one immediate transaction; final CAS rechecks expiry + owner + epoch | done | 1 · `tasks::interrupt_expired_leases(_in)`; interleaving test |
| 5 | Renewal requires current unexpired ownership | done | 1 · `renew_lease` |
| 5 | Recovery advances epoch, closes matching attempt, cancels the obsolete in-flight operation | done | 1 + 3 · epoch fence; `tokio::select` drops the request |
| 5 | Generic and refresh-specific recovery consolidated | done | 1 |
| 5 | Worker recoveries tracked separately from provider attempts; bounded policy; attempt refunded | done | 1 · `recoveries`, `MAX_WORKER_RECOVERIES=3` |
| 5 | Dependencies + brief manifest (`argos_task_dependencies`, `intel_brief_runs`, `intel_brief_stage_outputs`) | done | 1 · schema 35 |
| 5 | Dedicated `intel_brief` pool; dependency / `next_eligible_at` / one-active-stage guard in claim SQL | done | 1 |
| 5 | Atomic admission of one chain per article generation; fenced stage commit binds the next input | done | 1 · `ensure_chain`, `commit_stage` |
| 5 | Failure fails only that stage; dependents `Queued · blocked by <stage>`; Retry resumes it only; exhausted chain never reset | done | 1 + 3 |
| 5 | New inputs supersede; cancellation / deletion stops chain and publication | done | 1 |
| 5 | Heartbeat from claim through commit/ack on a light connection with bounded busy retry; heartbeat age and renewal failures recorded | done | 3 · `Store::open_light`, `intel-brief-heartbeat:*` |
| 5 | Visits / body-ready / recovery / Reload attach to the chain; no duplicate body jobs | done | 4a + 4b · `ensure_intel_brief_pipeline`, `reload_intel_brief_pipeline` |
| 5 | Final Stage-4 publication: brief snapshot + leads in the fenced commit; previous kept; older generation never overwrites newer | done | 4a · schema 36 |
| 5 | Canonical Brain derivations + index outbox from the brief through the **shared** publisher (no duplicate path); index failure separate | done | 5 · `insight_persist::publish_snapshot_on` (lane-independent) + `write_derivations`; insight-refresh `publish_snapshot` = lease fence around it; `brief_canonical::{prepare,publish_on,footer}`; savepoint rollback; report precedence |
| 3 | Cleanup order: baseline → NLP → conditional Classifier (≤12, 512 tokens) → verbatim removals → revalidation + span maps + hash | done | 2 + 4b |
| 3 | NLP mentions as keep-anchors | done | 5 · `clean_body::nlp_anchors` (body mentions named in title/description + title/description tokens), wired in `body.rs` |
| 3 | No Synthesis rewrite; display and extraction use one `intel-clean-body-v1` artifact; Full article pane renders it | done | 2 + 3 + 4b |
| 3 | Stage 1 fails on a missing/truncated/unvalidated story | done | 2–3 |
| 4 | Spec prompts, strict JSON, short IDs, Rust-resolved UTF-8 spans, local field/count limits | done | 3 |
| 4 | Budgets 2048 / 3072 / 1536; cleanup 512; fidelity 1024 | done | 3 + 4b (fallback ceiling test in 5) |
| 4 | Role routing and Jev capability skips via the shared executor | done (reused) | 3 |
| 4 | Long bodies: independent ~12KB body chunks, up to three in flight; exact + semantic dedup, ≤5 final inferences | done | 3 |
| 4 | Stage 4 local shortlist (self excluded, URL + syndication dedupe, cap 12); local finish when there are no inferences or no candidates | done | 3 |
| 4 | Lead gate: inference-specific overlap with the cited metadata + a named verification gap; country alone is not an overlap | done | 5 · `filter_leads` |
| 4 | Claim-assessor fidelity batches in Stages 2/3, cached receipts, unfaithful items withheld | done | 4b |
| 4 | `intel_inference_article_leads`; "Potential corroboration" rendering | done | 4a + 4b |
| 2 | Lazy mode classification; Synthesis rewrite removed | done | 3 + 4a |

## §6 UI

| Item | Status | Evidence |
|------|--------|----------|
| Step k/4, per-stage rows, one spinner, activity labels, blocker/failure notes | done | 4b · `brief_stage_lines` |
| Publication / Index footer (Pending / Published · Index pending / Indexed / Not indexed / Index failed / report precedence) | done | 5 · `brief_canonical::footer`, polled every tick |
| Global Jobs `stage_label`s identical to the brief rows; `intel_brief.stage` log events | done | 4b |
| Content panes show checkpoints immediately; a parent failure is not copied (artifacts show "Validated · not published") | done | 4b |

## §8 Acceptance

| § | Check | Status | Evidence |
|---|-------|--------|----------|
| 8.1 | One chain under repeated visits/reopen/recovery; one active stage; 1→2→3→4; zero requests on unchanged reopen | done | `one_chain_ordered_dispatch_*`, `repeated_admission_*`, UI `7-cached` frame |
| 8.2 | Scoped inputs; rejection of unknown IDs / invalid UTF-8 / hallucinated sources / overlong / duplicate / sixth inference | done | `validators_reject_*`, `sentence_spans_*` |
| 8.3 | Cleanup preserves byline/attribution/negation/corrections/quotes/chronology; Unicode; missing NLP/classifier; incomplete acquisition; long-body coverage; a body detail supports an inference without becoming a claim | done | `clean_body_tests.rs` (7 incl. `nlp_mentions_named_in_the_headline_become_keep_anchors`), `long_body_*`, `one_chain_*` |
| 8.4 | Stage-3 failure: 1/2 completed, 3 failed, 4 blocked in all surfaces; Retry runs 3 only; empty arrays complete; no-inference Stage 4 uses no model | done | `stage_three_failure_*`, `empty_inferences_*`, `intel_sequential_stage_screens` |
| 8.5 | Reaper interleaving; processing beyond the lease; SQLite contention; publication delay; crash/reclaim; stale late output; cancellation; deletion; overlapping drainers; heartbeat through ack; attempts survive recovery | done (processing beyond lease is scaled: 1 s lease / 2.5 s work, not a literal 120 s) | `renewal_between_*`, `heartbeat_covers_*`, `sqlite_contention_and_publication_delay_*`, `overlapping_live_drainers_*`, `crash_recovery_*`, `stale_worker_output_*`, `cancellation_*` |
| 8.6 | Jev primary with chat fallback, all-Jev chain, native decision cleanup, token-limit repair, server errors, malformed/unfinished streams, unsupported options; no repeated successful calls | done (shared executor tests reused, plus brief-specific `brief_fidelity_falls_back_to_chat_with_the_stage_budget`) | `model_exec_routing_tests`, `model_exec_format_tests`, `provider_attempt` |
| 8.7 | Lead explanation names the overlap and the verification gap; shared country alone yields no lead; syndicated copies add nothing; report precedence and canonical provenance/indexing survive | done | `shared_country_alone_yields_no_lead_*`, `shortlist_*`, `brief_canonical_tests.rs` (4) |
| 8.8 | Brief and Jobs buffers for queued / running / completed / failed / empty / cached states at 160×50, 120×40, 100×32, 80×24, 60×18 and too-small (40×12) | partial | Captured and inspected. At 60×18 the jobs pane shows the Step summary and the stage rows need scrolling. New rows are text; Retry/Reload reuse the existing button geometry, so keyboard/mouse navigation was not separately re-verified in this branch. |

## Not done / out of scope

- Real-provider latency measurements and live-terminal checks: not run (offline mocks only; the OpenRouter key was not used).
- Runs used `cargo build` and `ARGOS_EMBED=0 cargo test --workspace` (the user's rule). The spec's `scripts/agent_cargo.py --locked --no-default-features` and `--features nlp` matrix was not run separately.
