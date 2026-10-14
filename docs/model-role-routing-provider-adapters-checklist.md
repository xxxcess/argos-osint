# Model role routing, provider adapters and parallel brief preparation — checklist

Spec: "Argos — Model role routing, provider adapters, and parallel brief preparation" (2026-10-14, audited main `143d504`). Branch `feat/model-role-routing-provider-adapters`. HEAD comparison at start: the branch base equals the audited main `143d504`, so no listed file had changed since the audit.

Legend: **done** / **partial** / **deferred** / pending. Phase = the spec §9 patch order (1–5).

| § | Item | Status | Phase |
| --- | --- | --- | --- |
| 3 | `ProviderAdapter` (OpenRouter/Google/Nvidia + generic compatible) with pure `compile`/`normalize`; `ResolvedCapabilities` replacing provider-name heuristics in `resolve_adapter`/`request_policy::capabilities` | **done** — `provider_adapters/{mod,openrouter,google,nvidia}.rs`. `compile` is pure; `normalize_final` and `StreamDecoder` handle chat and Google SSE. `request_policy::{capabilities,apply}` were removed. `resolve_adapter` is now capability-driven (`adapter_for_caps`). `provider::{complete,complete_one,decide}` and `provider_attempt` compile through the adapters | 1 |
| 3 | Data-only `provider_profiles.json` (13 routes + conservative defaults); unknown/supported/unsupported features; semantic capability digest | **done** — `provider_capabilities.rs`: tri-state `Support`, `ReasoningCaps`, exact-ID profiles, OpenRouter/Google catalog parsers, value-scoped observed rejections, and a `digest()` that excludes timestamps | 1 |
| 3 | Capability cache in `app_state`, 24h background refresh, digest captured at stage admission | **done** — `app_state` cache (`provider_caps:<hash>`), 24h freshness, coalesced `refresh_in_background`. `provider_capabilities::admit` triggers the refresh and resolves one snapshot per route at stage admission. Digests go in `declaration_json.capability_digests`; recovery merges only observed rejections | 1–2 |
| 3 | OpenRouter `provider.require_parameters=true` for capabilities used | **done** — set whenever an optional option is sent | 1 |
| 4 | `brief_contracts.rs` authoritative Claims/Inferences/Links schemas | **done** — schema builders, limit text and contracts. Stage prompts carry the limits sentence, and each `StageRequest` sends its contract (sorted ID enums) through `model_exec` | 1–2 |
| 4 | Structured chat preference: strict schema → forced inert `submit_result` tool → JSON mode → validated text; output-tool normalization | **done** — compile preference order and `finalize_channel` (exactly one `submit_result` call; rejects unknown or multiple calls, refusal, invalid or truncated output). Brief stages use it through `model_exec`, which validates tool arguments like content. `ContractAdapter::parse_chat_as` reports the actual structured mode | 1–2 |
| 4 | Google native GenerateContent compile/normalize (systemInstruction, contents, generationConfig, thinking, usageMetadata, SSE) | **done** — `google.rs`: exactly one `models/` prefix, `x-goog-api-key`, `responseFormat.text` schema for confirmed models, thinking levels, `thought` parts kept apart from the answer, `finishReason` mapping, `streamGenerateContent?alt=sse` | 1 |
| 4 | Jev: canonical DecisionContract compilation in BlockDecisions/FidelityDecisions; Score criteria fix; typed native non-2xx | **done** — both adapters compile and parse through `DecisionContract`/`DecisionState` with minimal state. Score is 2–10 ordered levels with an application-scale mapping and a fractional `score_value`. Noul "no" = 1−p. The winning probability is never replaced by confidence. Contracts are validated before network access; native non-2xx is typed | 1–2 |
| 5 | Answer target / reasoning policy / total ceiling separated; per-model effort maps; no unconditional temperature 0.2 | **done** — `RequestPolicy` plus the `StageBudget` table (cleanup, claims, fidelity, inferences with an enforced-budget 4096 ceiling, support, links) wired into cleanup, fidelity and generation scopes | 1–2 |
| 5 | Brief fast preset (`{"route_role":"summarization"}`), Apply/Reset | **done** — `RoleDefaults::{apply_fast_preset,reset_fast_preset,fast_preset_applied}`; Defaults button *Apply/Reset fast preset*; the Brief operations panel shows `→ summarization (linked)`. Ships unset. Phase 5: the held-out comparison harness exists, but the live run could not be measured because the OpenRouter key on file is rejected (401). The preset therefore stays off and unrecommended ([benchmark](benchmarks/brief-fast-preset-heldout.md)) | 3, 5 |
| 5 | `OperationOverride` unions, portable schema 2 (role references, route `account_id`), `check_role_compatibility` by contract | **done** — internal union with mixed/unknown/operation-reference rejection at parse time; `PortableOverride`; schema 2 (`profile-config-v2.schema.json`) chosen on export only when used, with schema-1 rejection; account kind validation and account-map key preservation (`AuthFile::set_account_keyed`); contract-based compatibility now covers fallbacks; TUI fallback picker aligned | 3 |
| 6 | `intel-brief-v2` dependencies, durable join, per-node reuse, two-slot drainer, StageContext, v1 cutover | **done** (reuse partial) — explicit dependency rows; roots bound at admission, descendants bound once by compare-and-set to both prerequisite task IDs and output hashes; `drain_parallel` two-slot drainer in the scheduler; immutable `StageContext` through StageRequest/prepare_body/assess/generate and the Body `JobRuntime`; epoch-scoped operation IDs with the job generation; v1 chains fenced and cut over. Reuse: roots by node revision, descendants by prerequisite hashes + revision. *Partial*: Links is never reused (its candidate set is read live), and reusing body acquisition bytes with a local re-clean is deferred | 4 |
| 7 | Contextual executor, operation wire budget (≤6), one HTTP request per transport call, typed recovery proposals, value-scoped learning | **done** — `execute_with_context` + `DispatchContext`, `WireBudget::INTERACTIVE` (reserved initial dispatch per route, ≤2 per route, one primary transient retry, shared +2). The hidden resend was removed: `AttemptReport.recovery` is re-dispatched by the chain without backoff | 2 |
| 7 | Truthful status: `2 running · 0/4 completed`, blocked prerequisites, warnings | **done** — `stage_views` from the dependency rows (`Queued · after Full article, Claims`, transitive `Queued · blocked by X`, `Completed with warnings`); `chain_summary` heads the brief panel (`Brief · 2 running · 0/4 completed`); Jobs/Logs share the same views; `refresh_job_state` does not count blocked tasks as queued | 4 |
| 8 | Telemetry: real retry waits, per-dispatch admission, usage/cost/first-output timings, epoch-fenced attempt IDs, schema 41 | **done** — actual backoff in `wait_ms`, per-dispatch queue time, and migration 41 (`provider_execution_schema`). Attempt rows now carry reported reasoning/visible tokens, USD cost, header/first-reasoning/first-output/stream-completion timings and policy diagnostics. Operations and stage timings carry `task_epoch`; the epoch is also the attempt generation, so attempt IDs are epoch-fenced; timing aggregates the current epoch only. Phase 4: the epoch moved into the operation ID and the attempt generation is now the job generation (`StageContext`) | 2–4 |
| 8 | Defaults UX: assignment source, response contract, adapter + evidence, editable overrides | **done** — Defaults shows each Brief operation's source (inherited / explicit override / linked), response contract and adapter with capability evidence (strict schema, JSON mode, tools, provenance chain and profile note). `Op ▸` selects an operation; *Save for op* writes the Provider/Model fields as its override; *Reset op* restores inheritance; *Apply/Reset fast preset* unchanged | 3–4 |
| 9 | Acceptance 1–9, matrix, TUI captures, docs/diagrams/graphify, held-out comparison | **done, except the live measurement** — regression fixtures for items 1–6 and 8 exist (phases 1–5); phase 5 adds metadata-change / Links-only refresh and crash-after-one-branch tests. The full matrix passes (default features with LanceDB, `--no-default-features`, `--features nlp`; clippy `-D warnings` on each). Before/after TUI captures and the Defaults evidence fix are done, and docs, diagrams and graphify are updated. Item 7: the harness and fixtures exist, but the live comparison was **not measured** (invalid OpenRouter key; no Google or Nvidia keys) | 5 |

## Phase log

### Phase 1 — capabilities, adapters, schema builders, typed native errors

- HEAD comparison: the branch base equals the audited `143d504`, so no listed file had changed.
- The hidden unsupported-option resend in `provider_attempt::run` remains for this phase, but it is now value-scoped and recompiles from the same contract. Phase 2 replaces it with an explicit recovery proposal under the operation wire budget.
- Behaviour change: an OpenRouter, x.ai or OpenAI model without a profile or catalog evidence no longer gets `response_format` just because of its provider. It uses validated text until a catalog refresh confirms JSON mode.

### Phase 2 — contextual executor, bounded recovery, canonical decisions, stage budgets

- The hidden resend in `provider_attempt::run` was removed. A rejected named option is learned (field and value) and returned as `AttemptReport.recovery`. The executor sends the recompiled dispatch inside the operation budget with no backoff. An unknown 400 gets no proposal.
- `WireBudget` applies to interactive scopes only (brief generation, brief-owned cleanup, fidelity). Background callers keep the 4/3 policy.
- Behaviour change: Claims and Links now run with thinking off (previously `low`), and Inferences with `Efficient` (previously provider default), as the spec §5 budget table requires.
- Behaviour change: `DecisionAnswer::choice_probability` no longer falls back to confidence.
- The background catalog refresh is disabled under `cfg(test)` (`set_background_refresh`), so mock servers only see operation requests.
- Persisting reasoning/visible tokens, cost and first-output timings in their own columns needs migration 41 (phase 3). For now they go to telemetry only.

### Phase 3 — configuration unions, portable schema 2, migration 41, epoch snapshots

- Migration 41 is additive. Pre-41 rows keep NULL for the new counters and epochs. The store test upgrades a populated schema-40 DB, reopens it twice and compares the columns to a fresh store.
- Behaviour change: `check_role_compatibility` previously rejected chat models on Tool Picker and the decision roles, and checked primaries only. It now checks the whole chain against its contract, so a decisions-only *fallback chain* on a generation role is caught.
- Behaviour change: the TUI fallback picker now accepts Jev fallbacks on mixed roles (Evidence curator, Entity resolver, Claim assessor, Controller), matching import.
- Behaviour change: importing a provider entry whose `id` differs from its kind (e.g. `openrouter-work`) now keeps that account key instead of overwriting the default account of that kind.
- The task epoch is read from `argos_tasks.lease_epoch` when an operation is dispatched. In phase 4, `StageContext` will carry it directly.

### Phase 4 — dependency graph, two-slot drainer, StageContext, Defaults editing

- Behaviour change: Full article and Claims now run together; Claims read the immutable metadata snapshot taken at stage start. Inferences waits for both and is bound once to both output hashes.
- Behaviour change: a failed Full article no longer blocks Claims. Retry re-queues every failed task of the chain, leaving completed siblings in place.
- Behaviour change: the attempt `generation` is the job generation again; the lease epoch is in the operation ID (`{task_id}:e{epoch}:{key}:{payload_hash}`) and `task_epoch`. `with_task_epoch` no longer sets the generation. Stage timing groups by task and epoch.
- Behaviour change: fidelity receipts are keyed by the resolved assessor route (`intel-fidelity:{receipt}:{route}`), so a route change re-assesses.
- Node revisions are independent per node; descendants see upstream changes through the prerequisite output hashes bound at the join.
- Deferred: reusing body acquisition bytes with a local re-clean; Links reuse (it reads its candidate set live).
- No live provider checks were needed for this phase; all tests are offline mocks.

### Phase 5 — regression fixtures, TUI review, held-out comparison

- Defaults: the operation evidence (`Source · Contract`, `Adapter`, `Evidence`) now fits at 160x50 without scrolling. The footnote takes one row when it fits, and Source and Contract share a line.
- New regression tests: `changed_metadata_reruns_claims_branch_and_links_only_refresh_reruns_links` and `crash_after_one_branch_commit_keeps_it_and_fences_the_stale_sibling`.
- Held-out harness: `heldout_fast_preset_comparison` with `fixtures/brief_heldout.json`. Live result: **not measured**, because the configured OpenRouter key returns 401 "User not found" (confirmed on `/api/v1/key`). No Google or Nvidia keys are available, so no native-route check ran. The preset and focused path stay off.
- Under `cfg(test)` the background catalog refresh is disabled, so a live harness run resolves capabilities from profiles and conservative defaults only. A production run would also use the catalog evidence.
- Not covered live, all for lack of valid credentials: authenticated capability/availability checks, cold/warm p50/p95 latency, token usage, and quality.
