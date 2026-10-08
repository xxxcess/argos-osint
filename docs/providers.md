# Providers

[![Models and roles](diagrams/model-roles.svg)](diagrams/model-roles.html)

Models (internal id `Providers`) has four tabs: Defaults, OpenRouter, Google, Nvidia.

| Concern | Where |
| --- | --- |
| Credentials | `~/.argos/auth.json` (owner-only on Unix) |
| Role → provider + model | `config.toml` |

Saving or verifying a connection does not change a role. Old Writer config seeds Recon and Synthesis. Tool picker seeds OpenRouter `typesafe/jev-1.13` only when both fields are empty.

| Provider | Key env | Endpoint |
| --- | --- | --- |
| OpenRouter | `OPENROUTER_API_KEY` | optional HTTPS base |
| Google | `GEMINI_API_KEY`, then `GOOGLE_API_KEY` | `https://generativelanguage.googleapis.com/v1beta/openai` |
| Nvidia | `NVIDIA_API_KEY` | `https://integrate.api.nvidia.com/v1` |

Verification checks the current draft without saving. Legacy Grok/OpenAI in `auth.json` keep working until replaced. They have no new-setup tabs. Their keys are never sent to Google or Nvidia.

```sh
argos login
argos defaults show
argos defaults set recon --provider openrouter --model <model-id>
argos defaults set tool-picker --provider openrouter --model typesafe/jev-1.13
argos defaults set synthesis --provider openrouter --model <model-id>
argos models --role tool-picker
argos logout
```

| Role | Job |
| --- | --- |
| Recon | Directives and bindings |
| Tool picker | One next tool per request |
| Synthesis | Cited answers and claims |
| Classifier | Report mode from the prompt |
| Summarization | Graph/path summaries |
| Investigation harness | curator / resolver / assessor / controller |

Saving a role writes `config.toml` only. System log records `defaults.<role>: … -> …`. Provider list = accounts on this machine. Model list = what that account may call. A saved model id is kept if the live catalog is down.

Each role has an ordered `fallbacks` list of provider/account/model routes. Migration adds empty lists without changing the primary. Exact duplicates of the primary or of another fallback are rejected; the same model through another provider or account is distinct. Defaults shows priority, provider, and model; Add fallback opens a Google | Nvidia | OpenRouter catalog popup. Catalog browsing never issues inference. Help: `Tried top to bottom after primary retries`.

`argos osint user-agent` sets the public HTTP contact string. Blank → built-in `Argos OSINT/0.1 (…)`. Never send an empty User-Agent.

### Tool picker transport

`defaults show` reports `tool_picker: { provider, model, transport }` (`decisions` or `chat`). Role names: `tool-picker` and `tool_picker`. Pin `typesafe/jev-1.13`; do not pin `~typesafe/jev-latest`. On OpenRouter, the model list always offers `Jev 1.13 (decisions)` first.

| Transport | Models | Request |
| --- | --- | --- |
| Decisions | `typesafe/jev-*` or id contains `/jev` | `POST https://openrouter.ai/api/alpha/decisions`, one choice per remaining tool. Confidence &lt; 0.45 → deterministic fallback. |
| Chat | everything else | `{ "tool_id", "serves", "needs", "produces", "reason" }`. Validate + one repair. Same budget of 2 requests per turn. |

## OSINT data providers

[![OSINT catalog](diagrams/osint-providers.svg)](diagrams/osint-providers.html)

Primary: Firecrawl, SociaVault, Hunter. Saved key overrides env.

- Fallback field on the same row (`*_API_KEY_FALLBACK`) retries once after rate/quota, then is used first for later calls
- Atlas keeps a separate daily counter for the fallback account
- Keys go only to that provider’s host (Firecrawl bearer; SociaVault / Hunter `X-API-Key`)
- Hunter keys never go in the URL

Cache for the credit-reset interval: daily → 1 day; weekly → 1 week; monthly → 30 days.

- Firecrawl and Hunter reset monthly
- SociaVault prepaid never resets; public tools have no plan → 30 days
- NewsAPI, CourtListener, HackerTarget reset daily
- Failed results are never cached
- Live smoke tests are `#[ignore]`

| Provider | Tools | Notes |
|---|---|---|
| Firecrawl | search, scrape, map, batch scrape, crawl (off by default), extract | Batch scrape and crawl are polled jobs, charged per page. Extract uses a fixed schema (5 credits). |
| SociaVault | profile, search, search users, user content, Google search | 44 one-credit routes. No followers/following or single-post routes. Google search only after a weak Firecrawl search. |
| Hunter | domain finder, email count, domain search, email finder, email verifier, company enrichment, email insight, person enrichment, combined enrichment | Read endpoints only. Inputs from prompt, Firecrawl, SociaVault, or earlier Hunter. `hunter_tech_lookup` aliases company enrichment. |

## Typed provider diagnostics

Failed outbound requests are structured.

**Stages:** `configuration`, `admission`, `connect`, `first_response`, `response`, `stream`, `parse`, `validation`, `persistence`

**Categories:** `auth`, `permission`, `invalid_model`, `configuration`, `malformed_request`, `unsupported_transport`, `rate_limited`, `server`, `timeout`, `network`, `stream_interrupted`, `premature_eof`, `sse_error`, `malformed_payload`, `token_limit`, `refused`, `empty`, `invalid_result`, `persistence`, `cancelled`

Surfaces: Brain summary failure card, Jobs dashboard, `provider_verify` CLI. Events redact secrets.

## Budgeted executor

Shared chain (`provider_chain`): primary **4** attempts (waits 10s, 20s, 30s before retries); each fallback **3** (waits 10s, 20s). Maximum requests `4 + 3F`; all-failure base waits `60 + 30F` seconds. Exhaust the primary before fallback 1, then each fallback top to bottom. Snapshot routes at admission; persist attempt history.

- Every dispatched inference failure follows this policy, including 400/401/403/404, 429, 5xx, network/TLS, timeout, stream, and invalid output
- Missing credentials, invalid route, or definite capability incompatibility is a recorded skip (Blocked if nothing can dispatch)
- Admission wait does not consume an attempt
- Honor provider Retry-After / account cooldowns in addition to base waits
- `complete()` is one streaming request; `complete_one()` is one non-stream request. Hidden stream-to-nonstream retries were removed so the chain owns the budget
- Summarization uses the same primary budget of 4; a transport switch inside that budget still counts as a request
- Successful fallback is informational and does not mark a cycle Partial
- Provider 429 / 503 defer picker calls; deterministic order when no primaries remain

## Graph explanation jobs

- One job per memory per provider/model
- Cache key: memory text revision + graph brief + focus + provider + model + prompt version
- Failures persist with diagnostics. Memories stay. Atlas indexing continues.
- Stages: compact, evidence, synthesis

## News and legal keys

NewsAPI and CourtListener are keyed context providers, not primaries. Keys never appear in URL, tool input, cache key, plan, or stored body (echoed keys redacted). 0 credits; budgets are per-turn call caps. Fallback after rate/quota only. 401/403 do not switch accounts.

| Provider | Tools | Notes |
|---|---|---|
| NewsAPI | `/v2/everything`, `/v2/top-headlines` | Exact-phrase `q`, `pageSize` 10, no pagination. Key `NEWSAPI_API_KEY` as `X-Api-Key` to `newsapi.org`. Developer: 100/day, 24 h late, one month back. Cache 86400 s. |
| CourtListener | case law (`type=o`), dockets (`type=r`), judges (`type=p`) on `GET /api/rest/v4/search/` | Exact-phrase `q`, first page, ≤20, no highlight. Token `COURTLISTENER_API_TOKEN`. Space 12 s. Never retry 429. Cache 86400 s. |
| GNews | `gnews_search` `GET /api/v4/search` | `q` ≤200 chars, `max` 10. `GNEWS_API_KEY` as `X-Api-Key`. Atlas Search (needs `source.country`). Recon does not pick it. |
| NewsData.io | `newsdata_latest` `GET /api/1/latest` | Past 48 h. Do not send `timeframe` on free plan (HTTP 422). `NEWSDATA_API_KEY` as `apikey`. Atlas only. |
| Currents | `currents_latest` `GET /v1/latest-news` | `CURRENTS_API_KEY` as `Authorization: Bearer`. Atlas regional headlines. |
| Wikipedia (public) | `wikipedia_source_reliability` | MediaWiki `action=parse` on WP:RSP. No key. `gr`→B, `nc`→C, `gu`→D, deprecated/blacklist→E, unlisted→F. Needs identifying User-Agent. Index cached 30 days. |

Recon limits: `news_calls_per_turn` 2, `legal_calls_per_turn` 3. Spec defaults still to confirm: `sociavault_turn_credits_opening` 3, `sociavault_turn_credits_later` 8, `google_fallback_min_results` 3. `opening_sociavault_calls` in old files is unused.

## Verification and testing

- Unit tests for every failure category and stage
- Executor primary budget is 4 requests; admission wait consumes no attempt
- Cache invalidation when inputs change
- Graph explanation provider integration
