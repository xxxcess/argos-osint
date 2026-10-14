# Model role routing, provider adapters and parallel brief preparation — checklist

Spec: "Argos — Model role routing, provider adapters, and parallel brief preparation" (2026-10-14, audited main `143d504`). Branch `feat/model-role-routing-provider-adapters`. HEAD comparison at start: the branch base equals the audited main `143d504`, so no listed file had changed since the audit.

Legend: **done** / **partial** / **deferred** / pending. Phase = the spec §9 patch order (1–5).

| § | Item | Status | Phase |
| --- | --- | --- | --- |
| 3 | `ProviderAdapter` (OpenRouter/Google/Nvidia + generic compatible) with pure `compile`/`normalize`; `ResolvedCapabilities` replacing provider-name heuristics in `resolve_adapter`/`request_policy::capabilities` | pending | 1 |
| 3 | Data-only `provider_profiles.json` (13 routes + conservative defaults); unknown/supported/unsupported features; semantic capability digest | pending | 1 |
| 3 | Capability cache in `app_state`, 24h background refresh, digest captured at stage admission | pending | 1–2 |
| 3 | OpenRouter `provider.require_parameters=true` for capabilities used | pending | 1 |
| 4 | `brief_contracts.rs` authoritative Claims/Inferences/Links schemas | pending | 1 |
| 4 | Structured chat preference: strict schema → forced inert `submit_result` tool → JSON mode → validated text; output-tool normalization | pending | 1–2 |
| 4 | Google native GenerateContent compile/normalize (systemInstruction, contents, generationConfig, thinking, usageMetadata, SSE) | pending | 1 |
| 4 | Jev: canonical DecisionContract compilation in BlockDecisions/FidelityDecisions; Score criteria fix; typed native non-2xx | pending | 1–2 |
| 5 | Answer target / reasoning policy / total ceiling separated; per-model effort maps; no unconditional temperature 0.2 | pending | 1–2 |
| 5 | Brief fast preset (`{"route_role":"summarization"}`), Apply/Reset | pending | 3 |
| 5 | `OperationOverride` unions, portable schema 2 (role references, route `account_id`), `check_role_compatibility` by contract | pending | 3 |
| 6 | `intel-brief-v2` dependencies, durable join, per-node reuse, two-slot drainer, StageContext, v1 cutover | pending | 4 |
| 7 | Contextual executor, operation wire budget (≤6), one HTTP request per transport call, typed recovery proposals, value-scoped learning | pending | 2 |
| 7 | Truthful status: `2 running · 0/4 completed`, blocked prerequisites, warnings | pending | 4 |
| 8 | Telemetry: real retry waits, per-dispatch admission, usage/cost/first-output timings, epoch-fenced attempt IDs, schema 41 | pending | 2–3 |
| 8 | Defaults UX: assignment source, response contract, adapter + evidence, editable overrides | pending | 3–4 |
| 9 | Acceptance 1–9, matrix, TUI captures, docs/diagrams/graphify, held-out comparison | pending | 5 |

## Phase log
