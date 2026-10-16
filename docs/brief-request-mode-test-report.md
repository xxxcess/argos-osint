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
MATRIX_PLACEHOLDER

## Open
- Learned request-mode capabilities are not persisted across restarts.
- The mock header-stall repro shapes remain mock-only.
- Atlas "acquisition failures 1" and the Links "Queued" after the test's drain window were not root-caused.
- Grok routes not verified live (no credits).
