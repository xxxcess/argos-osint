# Brief Inferences: request-mode failures, Atlas test and Defaults popups — test report

Status: work in progress (draft PR). Baseline: main `c7984b8` (PR #45 + the author's PR #46 changes).

## Real versus mocked
| Item | Real | Mocked |
| --- | --- | --- |
| OpenRouter public model/endpoint metadata (`/api/v1/models`, `/models/{id}/endpoints`) | yes (fetched 2026-10-16) | |
| NVIDIA public documentation / forum evidence | yes (web) | |
| Provider inference calls (OpenRouter, Google, NVIDIA) | no (OpenRouter key invalid, no Google/NVIDIA keys on the box) | request-aware mock server playing all providers |
| Atlas news-cycle fetch | (filled in below) | |

## Findings so far (root cause of the screenshot's 404s)
- OpenRouter `nvidia/nemotron-3-ultra-550b-a55b:free`: public endpoint metadata shows the only upstream (Nvidia) with `status -5`, 0% uptime over 30 min and 9.6% over 1 day. It does advertise `tools` and forced `tool_choice` (function) but not `response_format`/`structured_outputs`; the capability profile matches. The 404 is the upstream being down behind OpenRouter's generic "Provider returned error", not an unsupported forced tool.
- NVIDIA `nvidia/nemotron-3-ultra-550b-a55b`: NVIDIA documents forced `tool_choice` for hosted NIM; the 404 `Function '…': Not found for account '…'` is an account-scope (Public API Endpoints) deployment gap that is permanent for that key. It is not a request-mode problem.
- App behaviour: `provider_diag::classify_status` maps every 404 whose text does not mention "model" to `Unsupported` ("the endpoint does not support this request mode"), moves to the next route with no mode ladder, and other operations of the stage re-try the same route. The label is misleading for both cases above.

(Sections for the Atlas run, per-route errors, ladder results, Defaults popup before/after and matrix are completed as the work lands.)
