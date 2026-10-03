# Providers

Providers has Grok, OpenAI, OpenRouter, and Defaults tabs. Connections supply credentials; Defaults independently selects a provider and model for three roles: Recon (questions and bindings), Tool picker (tool order), and Synthesis (answers). Changing or checking a connection does not change any role. Existing Writer configuration initializes Recon and Synthesis during migration. The Tool picker defaults to OpenRouter with `typesafe/jev-1.13`, seeded only when its provider and model are both empty.

Grok uses Grok Build subscription sign-in through `grok login --oauth`. OpenAI uses ChatGPT subscription sign-in through Codex CLI device authorization. OpenRouter uses an API key or `OPENROUTER_API_KEY`; its optional HTTPS endpoint is configurable. Verification checks the current draft without saving it. Account credentials are stored in `~/.argos/auth.json` with owner only permissions on Unix. Provider authentication and model usage follow the provider's own terms.

```sh
argos login
argos defaults show
argos defaults set recon --provider openrouter --model <model-id>
argos defaults set tool-picker --provider openrouter --model typesafe/jev-1.13
argos defaults set synthesis --provider openrouter --model <model-id>
argos models --role tool-picker
argos models --role synthesis
argos logout
```

The Recon role derives the turn's three questions and extracts input bindings from observations. The Tool picker role picks one tool per request until the ordered list is complete. Jev decisions models (`typesafe/jev-1.13`, `~typesafe/jev-latest`, or an id containing `/jev`) use `POST https://openrouter.ai/api/alpha/decisions` with the OpenRouter key; any other model uses the chat transport with a JSON reply. `defaults show` reports `tool_picker: { provider, model, transport }` with `transport` set to `decisions` or `chat`. `tool-picker` and `tool_picker` both name the role. Pin the concrete Jev id rather than the `~typesafe/jev-latest` alias so a later release cannot change picker behavior without a Defaults change. The Synthesis role writes cited answers and extracts evidence-backed claims. On Defaults, a three-way selector (Recon, Tool picker, Synthesis) chooses the role, and Provider and Model open a list for that role only. Saving a role writes `config.toml` only, never credentials, and the System event log records the change (for example `defaults.tool_picker: … -> …`). For the Tool picker on OpenRouter, the model list always offers `Jev 1.13 (decisions)` first, even when `GET /api/v1/models` omits it; `argos models --role tool-picker` does the same. The provider list is the accounts connected on this machine (Grok subscription, ChatGPT sign-in, an OpenRouter key, and Local). The model list is what that account is allowed to call. ChatGPT subscription selects the Codex default. A saved model identifier is retained if a live catalog is unavailable. `argos osint user-agent` sets the identifying contact string for public HTTP services separately from model defaults. When it is unset or blank, every OSINT request sends the built-in `Argos OSINT/0.1 (…)` User-Agent; a request never goes out with an empty User-Agent.

## OSINT data providers

Firecrawl, SociaVault, and Hunter are the primary OSINT providers. Each has one API key: `FIRECRAWL_API_KEY`, `SOCIAVAULT_API_KEY`, and `HUNTER_API_KEY`, or the key field shown on any of that provider's tools in OSINT. A saved key overrides the environment variable. Keys are sent only to the provider's own API host: a bearer token for Firecrawl, and an `X-API-Key` header for SociaVault and Hunter. Hunter keys never go in the URL. A completed result is cached for the provider's credit-reset interval: one day when the plan resets daily, one week when it resets weekly, and 30 days when it resets monthly. Firecrawl and Hunter reset monthly. SociaVault's prepaid credits never reset, and the public tools have no credit plan, so both cache for 30 days. NewsAPI, CourtListener, and HackerTarget reset daily. Failed results are never cached. Live smoke tests for these providers are `#[ignore]` and need real keys.

| Provider | Tools | Notes |
|---|---|---|
| Firecrawl | search, scrape, map, batch scrape, crawl (off by default), extract | Batch scrape and crawl are polled jobs that are charged per page. Extract uses a fixed schema and costs 5 credits. |
| SociaVault | profile, search, search users, user content, Google search | 44 one-credit routes. Followers/following and single-post routes are excluded. Google search is only a fallback after a weak Firecrawl search. |
| Hunter | domain finder, email count, domain search, email finder, email verifier, company enrichment, email insight, person enrichment, combined enrichment | Read endpoints only. Inputs come from the prompt, Firecrawl, SociaVault, or earlier Hunter calls. `hunter_tech_lookup` is an alias for company enrichment. |

NewsAPI and CourtListener (issue #29) are keyed context providers, not primary providers. NewsAPI uses `newsapi_api_key` or `NEWSAPI_API_KEY`, sent only as the `X-Api-Key` header to `newsapi.org`. CourtListener uses `courtlistener_api_token` or `COURTLISTENER_API_TOKEN`, sent only as `Authorization: Token <token>` (with `Accept: application/json`) to `www.courtlistener.com`. Neither key ever appears in a URL, tool input, cache key, plan, or stored body (a body that echoes the key is stored redacted). Both are listed at 0 credits; their budgets are per-turn call caps.

| Provider | Tools | Notes |
|---|---|---|
| NewsAPI | article search (`/v2/everything`), top headlines (`/v2/top-headlines`) | Exact-phrase `q`, `pageSize` 10 (search and headlines), no pagination. Optional `from`/`to`, `language`, `sort_by`, `domains` (search) and `country`, `category` (headlines). Developer plan: 100 requests a day, articles 24 h late, one month back. Cache 86400 s, the daily reset. `status: error` bodies (`apiKeyInvalid`, `apiKeyMissing`, `rateLimited`, …) become failed results with readable messages. |
| CourtListener | case law (`type=o`), federal dockets (`type=r`), judges (`type=p`) on `GET /api/rest/v4/search/` | Exact-phrase `q`, first page only, at most 20 results, no `highlight`, never semantic or POST search. Optional `court`, `filed_after`, `filed_before` (case law). Free tier 5/min, 50/hour, 125/day: requests are spaced 12 s apart and 429s are not retried. Cache 86400 s, the daily reset. 401/403 become failed results with readable messages. |
| GNews | `gnews_search` on `GET /api/v4/search` | Keyword `q` of at most 200 characters, `lang`, `max` 10, optional `from`. Key `GNEWS_API_KEY` as `X-Api-Key`. Free tier 100 requests/day. Atlas uses Search because `source.country` is not on top headlines. Recon does not pick it. |
| NewsData.io | `newsdata_latest` on `GET /api/1/latest` | Keyword `q`, `language`, and `size` 10. The latest endpoint is the past 48 hours. `timeframe` is optional and paid; the free plan returns HTTP 422 if it is sent. Key `NEWSDATA_API_KEY` as `apikey`, redacted from the stored URL. Free tier 200 credits/day. Atlas only. |
| Currents | `currents_latest` on `GET /v1/latest-news` | `language`, uppercase `country`, `page_size` 20. Key `CURRENTS_API_KEY` as `Authorization: Bearer`. Free tier 250 requests/day. Atlas regional headlines only. |

Recon limits add `news_calls_per_turn` (default 2) and `legal_calls_per_turn` (default 3). Failed and rate-limited results are never cached.

Recon limits add `sociavault_turn_credits_opening` (default 3), `sociavault_turn_credits_later` (default 8), and `google_fallback_min_results` (default 3). These are spec defaults that still need confirmation. `opening_sociavault_calls` is still read from older settings files but is no longer used.

