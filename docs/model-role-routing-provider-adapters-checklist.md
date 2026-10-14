# Model role routing, provider adapters and parallel brief preparation — checklist

Spec: "Argos — Model role routing, provider adapters, and parallel brief preparation" (2026-10-14, audited main `143d504`). Branch `feat/model-role-routing-provider-adapters`. HEAD comparison at start: the branch base equals the audited main `143d504`, so no listed file had changed since the audit.

Legend: **done** / **partial** / **deferred** / pending. Phase = the spec §9 patch order (1–5).

| § | Item | Status | Phase |
| --- | --- | --- | --- |
| 3 | `ProviderAdapter` (OpenRouter/Google/Nvidia + generic compatible) with pure `compile`/`normalize`; `ResolvedCapabilities` replacing provider-name heuristics in `resolve_adapter`/`request_policy::capabilities` | **done** — `provider_adapters/{mod,openrouter,google,nvidia}.rs`. `compile` is pure; `normalize_final` and `StreamDecoder` handle chat and Google SSE. `request_policy::{capabilities,apply}` were removed. `resolve_adapter` is now capability-driven (`adapter_for_caps`). `provider::{complete,complete_one,decide}` and `provider_attempt` compile through the adapters | 1 |
| 3 | Data-only `provider_profiles.json` (13 routes + conservative defaults); unknown/supported/unsupported features; semantic capability digest | **done** — `provider_capabilities.rs`: tri-state `Support`, `ReasoningCaps`, exact-ID profiles, OpenRouter/Google catalog parsers, value-scoped observed rejections, and a `digest()` that excludes timestamps | 1 |
| 3 | Capability cache in `app_state`, 24h background refresh, digest captured at stage admission | **partial** — `app_state` cache (`provider_caps:<hash>`), 24h freshness, coalesced `refresh_in_background`, `fetch_evidence`. Pending (phase 2): trigger the refresh and capture the digest at stage admission | 1–2 |
| 3 | OpenRouter `provider.require_parameters=true` for capabilities used | **done** — set whenever an optional option is sent | 1 |
| 4 | `brief_contracts.rs` authoritative Claims/Inferences/Links schemas | **done** — schema builders, limit text and contracts; validators use the same constants. Pending (phase 2): wire them into the stage prompts and executor | 1 |
| 4 | Structured chat preference: strict schema → forced inert `submit_result` tool → JSON mode → validated text; output-tool normalization | **partial** — compile preference order and `finalize_channel` (exactly one `submit_result` call; rejects unknown or multiple calls, refusal, invalid or truncated output). Fragmented SSE arguments are assembled and count as output. Pending (phase 2): use through `model_exec`; `ContractAdapter::parse_chat` reporting the actual kind | 1–2 |
| 4 | Google native GenerateContent compile/normalize (systemInstruction, contents, generationConfig, thinking, usageMetadata, SSE) | **done** — `google.rs`: exactly one `models/` prefix, `x-goog-api-key`, `responseFormat.text` schema for confirmed models, thinking levels, `thought` parts kept apart from the answer, `finishReason` mapping, `streamGenerateContent?alt=sse` | 1 |
| 4 | Jev: canonical DecisionContract compilation in BlockDecisions/FidelityDecisions; Score criteria fix; typed native non-2xx | **partial** — `decide` builds through `compile_decisions` and returns typed `http_failure` (status, code, request ID, Retry-After). Pending (phase 2): `DecisionContract` compilation for BlockDecisions/FidelityDecisions and the Score criteria fix | 1–2 |
| 5 | Answer target / reasoning policy / total ceiling separated; per-model effort maps; no unconditional temperature 0.2 | **partial** — `RequestPolicy{max_output_tokens, thinking_ceiling, reasoning_budget}` with no hidden reserve; `Efficient` intent; per-model effort maps; budget labelled hard/advisory; temperature only from profiles. Pending (phase 2): stage budget table | 1–2 |
| 5 | Brief fast preset (`{"route_role":"summarization"}`), Apply/Reset | pending | 3 |
| 5 | `OperationOverride` unions, portable schema 2 (role references, route `account_id`), `check_role_compatibility` by contract | pending | 3 |
| 6 | `intel-brief-v2` dependencies, durable join, per-node reuse, two-slot drainer, StageContext, v1 cutover | pending | 4 |
| 7 | Contextual executor, operation wire budget (≤6), one HTTP request per transport call, typed recovery proposals, value-scoped learning | pending | 2 |
| 7 | Truthful status: `2 running · 0/4 completed`, blocked prerequisites, warnings | pending | 4 |
| 8 | Telemetry: real retry waits, per-dispatch admission, usage/cost/first-output timings, epoch-fenced attempt IDs, schema 41 | pending | 2–3 |
| 8 | Defaults UX: assignment source, response contract, adapter + evidence, editable overrides | pending | 3–4 |
| 9 | Acceptance 1–9, matrix, TUI captures, docs/diagrams/graphify, held-out comparison | pending | 5 |

## Phase log

### Phase 1 — capabilities, adapters, schema builders, typed native errors

- HEAD comparison: the branch base equals the audited `143d504`, so no listed file had changed.
- The hidden unsupported-option resend in `provider_attempt::run` remains for this phase, but it is now value-scoped and recompiles from the same contract. Phase 2 replaces it with an explicit recovery proposal under the operation wire budget.
- Behaviour change: an OpenRouter, x.ai or OpenAI model without a profile or catalog evidence no longer gets `response_format` just because of its provider. It uses validated text until a catalog refresh confirms JSON mode.
