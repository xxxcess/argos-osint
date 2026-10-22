# Phase 1 design (draft): storage traits and the job system

Status: APPROVED by Code Architect (2026-10-22), with the decisions below. Base: `feat/surrealdb-storage` @ `00554d4`.
Sources: `/workspace/surreal-spec/impl-spec.md`, `storage-audit.md` §5, §6, §10, §14, `inventory.json`; `src/storage/error.rs` (61a4f9f).

## 1. Storage traits and repository boundaries

Everything lives in a new `argos-osint-core/src/storage/` module. It exposes async traits only, with no `Connection` escape hatch.

```
storage/
  client.rs     StorageClient { db: Surreal<Any>, mode: Server|Embedded, instance_id, reset_epoch }
  codec.rs      typed key codec (array record ids, round-trip + collision tests)
  tx.rs         run_tx(op_key, max_attempts, |tx| ...) -> Result<T, StoreError>  (§4)
  error.rs      (exists) is_conflict / normalize
  schema/*.surql ordered, checksummed; schema ledger record
  repo/{jobs,memory,recon,intel,artifacts,telemetry}.rs
```

| Trait | Owns (tables / invariants) | Callers from the audit §14 inventory |
|---|---|---|
| `JobRepo` (phase 1) | jobs, tasks, attempts, dependencies, retirements, scheduler lease, process registry, index outbox/change seq, counters | `tasks.rs`, `scheduler.rs`, `job_registry.rs`, `job_cleanup.rs`, `jobs_view.rs`, `atlas_work.rs`, `intel_recon/jobs.rs`, `intel_recon/worker.rs` (finalize), `intel_recon/brief_pipeline.rs` (chain/stage commit), `brain_neighborhood_brief_exec.rs` (run_claimed fence), `store/publication.rs::record_index_states`, TUI `app.rs` job panes, `cli.rs` job commands |
| `MemoryRepo` (graph, actors, vectors) | memories, brain_graph_nodes/edges, actors/aliases/redirects, memory_embeddings | `store.rs` (memory CRUD, delete_memory, write_then_index), `brain_*.rs` (actors, actor_dedup, backfill, browse, kinds, query, scope, topology_query), `related_*.rs`, `embed.rs`, `atlas_memory.rs`, `atlas_actor_index.rs`, `recon/brain_resources.rs`, `recon/graph.rs`, `recon/diversity.rs` |
| `ReconRepo` | threads, messages (per-owner counter), tool runs, credit holds | `recon.rs`, `recon/orchestrate.rs`, `recon/model_exec.rs`, `recon/tool_runner.rs`, `osint.rs`, `osint/wikipedia_rsp.rs`, `provider_capabilities.rs` |
| `IntelRepo` (Atlas, Intel, investigation) | atlas runs/articles/metadata/insights, intel reports/briefs/stages, investigations, insight publication | `atlas.rs`, `atlas_classify.rs`, `atlas_insights.rs`, `atlas_metadata.rs`, `intel_recon/{actor_review,body,brief_canonical,brief_stages,brief_timing,persist,replace_insights,synthesize}.rs`, `investigation/{insight_assessment,insight_persist,insights,persist}.rs`, `store/publication.rs::publish_atlas_insights`, `store.rs` atlas_* / article insight fns |
| `ArtifactRepo` | summaries, NLP cache, paginated artifact bodies | `nlp/mod.rs`, `summarization.rs`, `brain_neighborhood_brief_exec.rs` (cache write) |
| `TelemetryRepo` | telemetry, provider metrics, profile aggregates | `telemetry.rs`, `profile_stats.rs`, `provider_metrics.rs` |
| `ReadSnapshots` (read-only, single tx) | consistent bounded reads for UI | TUI `db_reads.rs`, `tracked.rs`, `ui.rs`, `app.rs` readers, `jobs_view.rs`, `profile_stats.rs` |

Rules:
- `Store` stays as a facade that holds `Arc<StorageClient>` plus one handle per repo. Its sync methods are converted module by module. Store-open and path sites (`cli.rs` 7, `tui/app.rs` 32, `recon.rs` 45 and others) call `StorageClient::connect(home)`, which runs the lifecycle from `design.md`.
- The TUI never awaits inside draw. Background workers own the runtime and publish immutable snapshots.
- Database work currently done in `Drop` moves to a bounded completion queue, with lease expiry as the fallback.
- Review gate: each phase PR adds `scripts/check_callers.py`, which diffs the §14 module list against `rg 'rusqlite|Connection|\.conn\b'` and fails if a converted module still references a raw connection.

Phase 1 delivers only `JobRepo`, `tx.rs`, `codec.rs` and the jobs schema. The other traits are signatures only (no impls), so later phases have a fixed target.

## 2. Job-system claim design (guard records, bounded retry)

SurrealDB uses optimistic snapshot isolation and has no partial index and no `UPDATE ... WHERE id=(SELECT ...) RETURNING` that is atomic over a subquery. SQLite's writer serialization is replaced by **guard records**. Every transaction that must serialize against another one UPDATEs the same guard record (`SET rev += 1`), so the two transactions write-write conflict at commit. One wins; the loser gets `TransactionConflict` (normalized by `storage::error`) and retries the whole transaction.

Guard tables (schemafull, one small record each):
- `job_guard:[job_id]`: job lifecycle (cancel, finalize, stage commit, chain changes).
- `pool_guard:[pool]`: claim serialization per pool.
- `lease_sweep_guard:singleton`: the reaper.
- `brief_chain_guard:[article_id]`: generation allocation per article.
- `cleanup_guard:singleton`: retention and history deletion.
- `scheduler_lease:singleton`: existing election record, conditional update.

### 2a. Claim (`tasks.rs::claim_with` :996, `claim_next_in` :960, `claim_task` :986)
These use UPDATE...RETURNING, not BEGIN IMMEDIATE, but they are the core. One transaction:
1. `UPDATE pool_guard:[pool] SET rev += 1` (first statement, so conflicts surface early).
2. Indexed candidate scan: `SELECT id, ... FROM task WHERE state IN ['queued','retry_scheduled'] AND pool = $pool AND next_eligible_at <= $now ORDER BY priority, created_at, id LIMIT $k` (k = 8). This uses index `task_claim_order ON task FIELDS state, pool, priority, created_at, id`, which replaces the SQLite partial index.
3. For each candidate in order, evaluate `CLAIM_ELIGIBLE` in SurrealQL inside the tx. Check the job is under 3 days old and not cancel-requested, there is no retirement, all deps are completed, and the intel_brief exclusivity rule holds (input_hash non-empty). Take the first eligible candidate.
4. `UPDATE $task SET state='running', lease_owner, lease_until, lease_epoch += 1, attempts += 1, ... WHERE state IN [...] RETURN AFTER`. Assert one row came back, else `THROW`.
5. `UPDATE job_guard:[job] SET rev += 1`. This makes the claim conflict with stop/finalize on the same job.

If none of the k candidates is eligible, widen the window by paging with a (priority, created_at, id) cursor. The cap is 4 pages. After that it returns `NoClaim` rather than spinning, which preserves the no-starvation property of the shared selector. `claim_task` skips steps 2 and 3 and checks the specific id.

### 2b. Per-site guards for the 7 job-system BEGIN IMMEDIATE sites

| # | Site | Invariant protected | Guard design |
|---|---|---|---|
| 1 | `crates/argos-osint-core/src/tasks.rs:1385` `interrupt_expired_leases` | Expired lease reaped once; renewed-between-select-and-update task never reaped; epoch advances | Touch `lease_sweep_guard`. Per task, `UPDATE task SET ... lease_epoch += 1 WHERE id=$id AND lease_epoch=$seen AND lease_until <= $now`. A renewal, which also bumps `task.rev`, makes this conflict or match zero rows. Then touch `job_guard:[job]`. Batch ≤ 64 tasks per tx. |
| 2 | `crates/argos-osint-core/src/job_cleanup.rs:113` `cleanup_batch` | 72 h retention from immutable `created_at`; never delete a job that has a running task or pending index work | Touch `cleanup_guard`, plus `job_guard:[id]` for every job deleted in the batch, so a concurrent claim (step 5 of 2a) conflicts. Re-check `eligible()` and OWNER_MISSING inside the tx. BATCH is unchanged. The "own tx only if autocommit" nesting goes away: callers pass a `&mut Tx` or call the standalone fn. |
| 3 | `crates/argos-osint-core/src/job_cleanup.rs:290` `stop_task_job` | Cancel the whole job subtree; epoch fence for in-flight work in other processes | Resolve the subtree, then touch `job_guard:[id]` for each id, in sorted order to keep conflict churn deterministic. Update attempts, tasks (`lease_epoch += 1`), jobs and brief runs in the same tx. Subtree size > 256: split into multiple txs, root first, and set `cancel_requested` first so claims stop immediately. |
| 4 | `crates/argos-osint-core/src/job_cleanup.rs:429` `clear_jobs_by_status` | Delete only terminal root trees with no running descendant or service | Same as #2: `cleanup_guard` plus `job_guard` per root and descendant. The recursive NOT EXISTS is evaluated in the tx after touching guards. The loop runs one tx per batch. |
| 5 | `crates/argos-osint-core/src/intel_recon/worker.rs:838` `finalize_job_state_fenced` | Finalize only with a live lease epoch and an unretired job | Touch `job_guard:[job]` and the finalize task record. Re-check `lease_epoch = $epoch AND status='running' AND lease_until > time::now()` and retirement inside the tx, then update the report job. The section and element counts are read before the tx today. Move them inside, or fence with `job_guard.rev` and re-read on retry. |
| 6 | `crates/argos-osint-core/src/intel_recon/brief_pipeline.rs:102` `ensure_chain` | One active chain per article; `MAX(generation)+1` unique; superseding cancels the old chain | `brief_chain_guard:[article_id]` holds the `generation` counter (replacing MAX+1). Touch it, read the active run, attach or supersede (touching the old `job_guard`), and create the new job and tasks. Add a unique index on `intel_brief_run(article_id, generation)` as a backstop. |
| 7 | `crates/argos-osint-core/src/intel_recon/brief_pipeline.rs:451` `commit_stage_with` | Stage output plus `complete_claimed` (epoch check) plus `bind_ready` of dependents commit atomically; a lost lease publishes nothing | Touch `job_guard:[job]` (this conflicts with stop and with claims of sibling stages, preserving the intel_brief exclusivity rule). `complete_claimed` becomes `UPDATE task ... WHERE lease_epoch=$e AND lease_owner=$o AND state='running'`, checking 1 row and otherwise `THROW 'lease_lost'` (mapped to `Ok(false)`, not retried). Stage output is UPSERTed under key `[task_id]`. The canonical fallback follows audit §6 "Savepoints" #4 (second tx with the same plan). |

Related site, outside the 7 but on the same fence pattern: `brain_neighborhood_brief_exec.rs:660` `run_claimed` does the same as #7 with `job_guard` plus a lease check, and is ported in phase 1 as well. The scheduler election (`scheduler.rs:157` `try_elect`) is a conditional `UPDATE scheduler_lease:singleton ... WHERE owner=$me OR lease_until < $now`, and conflicting losers read "not elected".

The 17 non-job production BEGIN IMMEDIATE sites (`store.rs` ×8, `store/publication.rs:295,776`, `atlas_memory.rs:700`, `atlas_metadata.rs:362,728`, `brief_canonical.rs:794`, `insight_persist.rs:282,1870`) are out of scope for phase 1. They get owner/memory/identity guards in phase 2.

## 3. Replacing the EXPLAIN QUERY PLAN test

`tasks.rs:2064 queued_claim_scan_uses_the_partial_index` becomes `claim_scan_uses_task_claim_order_index`:
1. Assert that the index exists: `INFO FOR TABLE task` → `indexes.task_claim_order` with the expected field list. This also guards the schema file against drift.
2. Assert the plan: run the **production** candidate query string (exported as `const CLAIM_CANDIDATES_SURQL`, not a copy) with `EXPLAIN FULL` appended. Assert that the plan contains an `Iterate Index` operation naming `task_claim_order`, and no `Iterate Table` over `task`. Seed 1k tasks first so the planner does not pick a table scan for an empty table.
3. Assert the ordering: whether the 3.3.4 planner satisfies ORDER BY from the index is checked in the spike. If not, the test asserts that the result order is correct over a fixture with ties, and the plan assertion is limited to index use plus `LIMIT` pushdown.

The same pattern replaces `brief_canonical_tests.rs:1046` and `brain_topology_tests.rs:1034`. Shared helper: `storage::testkit::assert_uses_index(db, query, index)`.

## 4. Server and embedded modes; conflict retry

`run_tx` is identical in both modes. The only mode-specific code is in `StorageClient::connect` (`ws://127.0.0.1:port` with the limited `argos` user vs `rocksdb://` with an exclusive home `flock`).

```
for attempt in 1..=MAX (default 6):
    r = db.query("BEGIN; ...guards first...; ...body...; COMMIT").await
    errs = r.take_errors()            // check every statement, not just transport
    match normalize(first_err):
      ok                          -> return
      is_conflict(e)              -> sleep(backoff: 5ms * 2^n, cap 200ms, full jitter); continue
      THROW 'lease_lost'|'no_claim' -> return domain outcome (no retry)
      transport/unknown-commit    -> resolve by op_key receipt (below), else Unavailable
return Err(StoreError::Contention{op, attempts})
```
- `normalize` (61a4f9f) maps the embedded engine's conflict to `Query/TransactionConflict`, so retry logic never branches on mode. A contract test covers both modes.
- Embedded mode is single-process, so conflicts come only from concurrent tasks within the process. The guard design is still needed (tokio workers race), and the same tests run against it.
- Unknown commit (server mode only: socket drop after COMMIT is sent): every job write carries an `op_key` (e.g. `claim:{owner}:{nonce}`) written into a `tx_receipt` record in the same tx. Before retrying, read `tx_receipt:[op_key]`. If present, return the stored result. Receipts are pruned by cleanup after 72 h.
- Service unavailable is never treated as a dead owner. Leases are not reaped while the reaper cannot reach the database, and epochs fence recovered work.
- No provider or model call happens inside `run_tx` closures. The retried closure is pure DB work (enforced by the closure type: no `async` external clients captured; reviewed).

## 5. Test plan

All tests run against a real RocksDB-backed server (`scripts/test_surreal.py`, unique database per test) and in embedded mode via `ARGOS_STORAGE=embedded`. The memory engine is never used for gates.
- **Contract (tx.rs):** a forced conflict retries and succeeds; exhaustion returns `Contention` after MAX attempts; a statement error inside the tx aborts everything (no partial commit); `THROW` maps to a domain outcome without retry; unknown-commit is resolved via receipt (fault injection: drop the connection after COMMIT).
- **Claim races:** N=8 clients × 200 tasks, with each task claimed exactly once per epoch. Port `overlapping_drainers_never_double_claim_a_stage`. Head-of-queue ineligible rows do not starve eligible ones. intel_brief exclusivity holds under races.
- **Per site (1–7):** for each, a two-client interleaving test using a `between` hook (as `interrupt_expired_leases_in` already does):
  1. A renewal racing the reaper never gets reaped.
  2. And 4. Cleanup racing a claim never deletes a job with a running task.
  3. Stop racing stage commit: the commit returns `lease_lost`, with no output.
  5. Finalize after a lost lease is refused.
  6. Concurrent `ensure_chain` for the same article yields one generation.
  7. Commit after an epoch bump publishes nothing.

  Port `failed_stage_blocks_dependents_preserves_predecessors_and_retry_resumes_only_it` and `crash_after_one_branch_commit_keeps_it_and_fences_the_stale_sibling`.
- **Scheduler:** two processes, one elected. Kill the leader and the follower takes over after 30 s. An unavailable DB does not count as a lost lease.
- **Process registry:** 5 s heartbeat; a stale process after 30 s has its tasks recovered with new epochs.
- **Index check:** §3 tests.
- **Codec:** round-trip and collision property tests for compound keys.
- **Matrix:** the existing 13-check local matrix, plus the storage suite in both modes. Do not push until Code Architect signs off on phase 0.

## Review decisions (Code Architect, 2026-10-22)
1. **Scope.** The 7 job-system sites in §2b, plus the claim at `tasks.rs:996`, plus `brain_neighborhood_brief_exec.rs:660`. From `scheduler.rs`, election at `scheduler.rs:157` `try_elect` is a read followed by a separate INSERT/UPDATE in autocommit, so it gets its own transaction on `scheduler_lease:singleton`. Task lease renewal (`tasks.rs:1356` `renew_lease`) is a single conditional update and is ported as one. The other 17 sites move to phase 2.
2. **Guard granularity.** One `pool_guard` per pool. The 8-client race records retries per claim and reports p50/p99/max. Split only if p99 > 3 retries or any claim exhausts its 6 attempts.
3. **Index test.** `INFO FOR TABLE` must list the index, and `EXPLAIN FULL` must show an index iterator with no table scan. Ordering is asserted on returned results, with a comment that this depends on 3.3.4 plan output.
4. **Receipts** go on claims and stage commits only. Every other job write is idempotent, and each site's interleaving test replays the operation and proves the replay is harmless.
5. **Large-tree stop.** First set `cancel_requested` on the root (one tx), then cancel descendants in bounded batches of about 100 per tx, resumable and idempotent. Workers check the flag before each stage. A crash-mid-stop-then-resume test is required.

## Resolved questions (kept for history)
1. **Which 7 sites?** The audit names no count, and production code has 24 BEGIN IMMEDIATE / `TransactionBehavior::Immediate` sites, plus 4 in tests. I took the 7 job-system ones (table above) and deferred 17 to phase 2. Should `brain_neighborhood_brief_exec.rs:660` count as the 7th instead of one of the cleanup sites?
2. **Guard granularity:** a single `pool_guard` per pool serializes all claims in that pool. That is fine at one worker per pool today, but should it be sharded (`pool_guard:[pool, n]`) now or later?
3. **EXPLAIN stability:** is it acceptable for the plan test to fall back to an order-correctness assertion if 3.3.4 does not report index-ordered scans?
4. **Unknown-commit receipts:** do they apply to all job writes (adds one record per tx) or only to claims and stage commits?
5. **Large subtree cancel (#3):** is a multi-tx split with `cancel_requested` set first acceptable, versus one big tx?
