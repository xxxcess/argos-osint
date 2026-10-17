# Brief Inferences: request-mode failures, Atlas test and Defaults popups — test report

Baseline: main `c7984b8` (PR #45 + PR #46). Dates 2026-10-16 (America/New_York).

## Real versus mocked
| Item | Real | Mocked |
| --- | --- | --- |
| Per-route probes (smallest request: plain, forced tool, forced tool + `require_parameters`) with the keys in the user's exported profile | yes | |
| Atlas news-cycle run (`atlas::run_live`, Fresh, same call as the TUI) | yes, 3 runs, 194 / 238 / 235 articles, scratch `ARGOS_HOME` | |
| Brief pipeline (Full article, Claims, Inferences, Links) on 8 Atlas articles, real `RuntimeBackend` and real providers | yes (ignored test `live_atlas_then_briefs`) | |
| OpenRouter public endpoint metadata | yes | |
| Chain, ladder and recovery regression tests | | request-aware mock server (deterministic, regression only) |
| Defaults popups | TestBackend render + cell screenshots | |

Keys: the OpenRouter, Google and NVIDIA keys inside the user's profile were usable. The standalone `~/.config/argos/openrouter.env` key on the box is invalid and was not used. The Grok (xAI) key is out of credits and is not in any Inferences chain, so Grok routes could not be verified live. The Google key hit quota (429) during runs 2 and 3. No keys were printed or committed.

## Root cause of the screenshot's 404s (live-proven)
Probe of `nvidia/nemotron-3-ultra-550b-a55b` (OpenRouter `:free` and direct NVIDIA), all three request modes:
`404 Function id '948fe171-…' version 'null': Specified function in account '…' is not found` in every mode.
`nemotron-3-super-120b-a12b` returns 200 in all three modes on both routes (forced tool works).

So Ultra has no deployment for this account; OpenRouter's `:free` route is the same NVIDIA upstream. This is **not** a request-mode problem, so no mode ladder can fix it. Previously the app labelled it "endpoint does not support this request mode", learned nothing, and each sibling operation re-hit the same dead model on every route. Combined with Gemini 503/429, that produced the screenshot.

## Fix
- `provider_diag`: a deployment-missing 404 is `InvalidModel` ("the configured model was not found"), with upstream detail shown and tenant account IDs masked.
- `provider_chain::StageLedger`: hard rejects (auth, permission, invalid model) are remembered per stage by model family (`:suffix` and `provider/` stripped), so the same model through another gateway is skipped ("same model already rejected in this stage"); 1800 s route breaker for hard rejects.
- Last live route gets 2 extra overload (503/429) retries. A cooling route is still tried when all later routes are dead (bug found live, regression test added).
- Mode ladder kept for real gateway rejections: a 404 "No endpoints found that support …" steps forced tool → json → validated text (test `gateway_404_rejecting_the_forced_tool_steps_the_mode_down_to_validated_text`). Learned capabilities remain in-process only (not persisted; open).
- Classifier: PR #46 already withholds only the failing claim (`phase=context`); all claims failing still fails the stage.
- Defaults tab: every role's provider and model field now opens the popup card (`is_picker_field` = any role field; ui cursor match likewise). Test `every_defaults_role_opens_its_provider_and_model_popups` covers all 9 roles.

## Live results (run 3, final code, 3 articles; runs 1–2 earlier)
| Article | Full article | Claims | Inferences | Links |
| --- | --- | --- | --- | --- |
| run1 eurekalert: mRNA vaccines compared head-to-head | ok | ok (3) | FAILED (old classification: invalid_model) | queued |
| run1 abc.net.au: El Niño wettest on record | ok | ok (2) | ok (5) | queued |
| run2 thestar.com.my: Golf-Asian Tour postpone Saudi International | ok | ok (1) | ok | ok |
| run2 thestar.com.my: Martinez available for Leeds | ok | ok (3) | ok | FAILED Gemini 429 quota |
| run2 thestar.com.my: Sze Fei-Izzuddin bow out | ok | ok (3) | ok with warnings | ok |
| run3 tradewindsnews.com: Danaos / Oslo bulkers IPO | ok | ok (2) | FAILED: Ultra 404 + Gemini 429 quota (+ one 60 s no-headers timeout) | blocked |
| run3 thescottishsun.co.uk: Haaland property | FAILED: bot wall ("Verifying Device"), acquisition incomplete | ok | blocked | blocked |
| run3 southernmarylandchronicle.com: SMCM Oslo Peace Congress | ok | ok (3) | queued when the test's drain window ended | queued |

Atlas: `outcome=ok`, state `partial` phase 1 only because of "acquisition failures 1" (one source fetch failed; not investigated).

Per-route errors seen live:
| Route | Result |
| --- | --- |
| OpenRouter `nemotron-3-super…:free` | 200, all modes |
| NVIDIA `nemotron-3-super…` | 200, all modes |
| OpenRouter `nemotron-3-ultra…:free` | 404 Function not found (every mode) |
| NVIDIA `nemotron-3-ultra…` | 404 Function not found (every mode) |
| Google `gemini-3.8-flash` | works when quota is available; 429 quota exceeded in runs 2–3, one 503, one 60 s header timeout |

Interpretation: with Ultra undeployed, the user's Inferences chain effectively depends on Gemini alone, so Gemini quota exhaustion fails the stage. Failures are now clearly labelled and cost far fewer requests, but no code can conjure a working route.
**Recommendation:** replace Ultra with Super in the Synthesis chain (works live on both gateways) or add Super as a fallback; use the `route_role=summarization` overrides in `docs/brief-provider-reliability.md`; check Gemini quota/billing.

## Defaults popup before/after
Before: provider field of Claim assessor / Investigation controller was a plain "Paste API key" text field.
After: the provider popup card lists the providers.
Screenshots: `docs/screens/request-mode/before/` and `docs/screens/request-mode/after/` (`defaults-claim_assessor-provider-popup.png`, `defaults-investigation_controller-provider-popup.png`).

## Matrix
Run on the code of `5cd45cf` (later commits are docs only). `cargo fmt --all --check` clean.
| Configuration | clippy `-D warnings` | tests (core / bin / other) |
| --- | --- | --- |
| default features | clean | 1148 / 272 / 3 passed, 0 failed (11 ignored live/dump tests) |
| `--locked --no-default-features` | clean | 1118 / 272 / 3 passed, 0 failed |
| `--features nlp` | clean | 1148 / 272 / 3 passed, 0 failed |

## Follow-up (2026-10-16 evening): acquisition failure, Ultra history, Grok removal

### 1. Atlas "acquisition failures 1" — LIVE + OFFLINE
- **What counts as one:** a news-feed call (phase 1 latest-news, phase 2 per-country headlines for GNews, NewsData, NewsAPI, Currents) that returned a transport error, a non-2xx status, or an unreadable body. Each increments `coverage.acquisition_failures`, which alone makes the run state `partial`.
- **What was wrong:** the status text never said which feed or why; the HTTP client followed no redirects and never retried, so one timeout, 5xx or 3xx marked the whole cycle partial; and a provider quota/429 (an expected daily condition) counted as a failure.
- **Live evidence:** runs 1–3 each ended `partial` with failures 1. Three further live Atlas runs later the same day (no model calls for two of them) ended with failures 0 (`completed` phase 6; and `blocked` at classification only because the classifier was deliberately disabled for those runs). The single failure therefore was transient or quota-related and could not be re-observed, so the exact feed and message of the original three cannot be quoted; the code did not record it. This is stated as unresolved-by-observation, resolved-by-design.
- **Fix (offline tests):** `dispatch` retries a transport error, 408 or 5xx once with backoff, follows up to 3 same-host https redirects (a foreign-host redirect is reported as such), names timeouts. Quota/rate-limit faults are counted as `quota_skipped` ("quota-limited calls N") and do not make the run partial. Real failures are listed by provider and cause in the status text, e.g. `acquisition failures 1 (newsdata malformed JSON: …)`. Tests: `acquisition_tests::*` (retry, transport, redirects, quota vs failure).
- A single failed fetch still marks the run `partial`, deliberately: coverage really was reduced and the resume cursor retries only that call.

### 2. Ultra history and request shape — OFFLINE (git) + LIVE (one earlier probe set)
- `git log -S"nemotron-3-ultra"`: Ultra first appears in code in `7e72832` (2026-10-14, provider adapters, PR #43 follow-up). Before that no source, test or default named it, so any earlier success was via a user-configured model id with the pre-adapter request (`provider.rs`: plain chat, `tool_choice: "auto"` at most, no `reasoning_effort`, no forced tool, no `response_format`).
- Since `7e72832` the adapters send to Ultra: forced single output tool, no `response_format`, `reasoning_effort` from its own vocabulary (none/medium/high), `provider.require_parameters=true` on OpenRouter when options are sent, streaming. Brief Inferences (`339eca5`, `592385b`, `d81684d`) added the effort/deadline sizing; they did not change the model id or routing.
- In the user's profile the **recon** role uses `nemotron-3-super…`, not Ultra. Ultra is only in **synthesis** (primary on OpenRouter `:free`, fallback Nvidia) and as a claim-assessor fallback. So recon never exercised Ultra in this profile; the "recon works" observation is Super working.
- Live probes (earlier today, same keys): Ultra returned `404 Function id … Specified function in account … is not found` on both OpenRouter `:free` and Nvidia direct for the plain request (no tools), the forced-tool request, and forced tool + `require_parameters`. A request with no tools at all fails identically, so no request shape (including a recon-style one) avoids it. Both catalogs (`/models`) list the model id, so the id is advertised but not deployed for this account at the time of the probes.
- Late re-check: a recon-style request (tools + `tool_choice: auto`, non-streaming) to both routes did not 404; it timed out at 60 s on each. That differs from the earlier 0.2 s 404 and suggests a cold or flapping deployment rather than a permanent absence; it is not conclusive. A second, tighter comparison probe was blocked by the sandbox's command review and was not run, so this stays open. No Inferences request change was justified by the evidence: the request is not what differs.
- Conclusion: deployment-side. Mitigation already shipped (classification, stage-wide dead-model memory, breaker); the user is adding a Super fallback to Synthesis.

### 3. Grok (xAI) removal — OFFLINE tests
Removed: provider preset and `GrokModel` catalog, `grok_oauth` module and its bearer/login paths, `grok-subscription` kind and its error text, `x.ai` host detection, CLI `login` branch, TUI choices and "Grok (legacy)" label, portable-schema `grok` kind (v1 and v2 enums), docs and diagrams. Default text provider with nothing saved is now OpenRouter.
Migration (no crash, clear warning):
- `auth.json`: Grok accounts / text / voice slots are ignored on load (file untouched until the next save); TUI status says "Grok (xAI) is no longer supported; its saved routes were ignored".
- `config.toml`: every role, the decision fallback and operation overrides lose Grok routes on load; a removed primary is replaced by the first remaining fallback or cleared (inherits); `writer_provider` cleared; the file is rewritten without them.
- Portable import: the Grok provider entry, its routes and rate limits are dropped, with one warning (`Grok (xAI) is no longer supported and was not imported: …`); the user's real profile fixture, which still carries a Grok provider, imports cleanly.
Tests: `secrets::loading_old_auth_drops_removed_grok_connections_and_keeps_the_rest`, `provider::removed_provider_tests::*`, `config_transfer_tests::an_old_profile_with_grok_still_imports_and_drops_every_grok_route_with_one_warning`.
Kept deliberately: the key-redaction regex still masks `xai-…` strings in logs.

## Open
- Learned request-mode capabilities are not persisted across restarts.
- The mock header-stall repro shapes remain mock-only.
- Atlas "acquisition failures 1" and the Links "Queued" after the test's drain window were not root-caused.
- Grok routes not verified live (no credits).
