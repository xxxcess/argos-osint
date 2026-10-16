# Intel brief provider reliability

Baseline: `xxxcess/argos-osint`, main `1d3eb59`, PR #45 `9ca9547`, audited 2026-10-16. The reproduction uses the supplied portable profile in an isolated temporary database; no saved credentials or user data are changed.

## Reproduced cause

The profile routes Synthesis through OpenRouter Nemotron Ultra, Gemini Flash, then NVIDIA Nemotron Ultra. OpenRouter's endpoint catalog advertises NVIDIA as the free Ultra upstream, so the first and last routes do not establish independent model capacity.

| Live probe | Result |
| --- | --- |
| OpenRouter Ultra, existing output tool | HTTP 200 headers, no answer or reasoning for 210 seconds |
| OpenRouter Ultra, explicit supported `medium` effort | Still no model output for 210 seconds |
| OpenRouter Ultra, `medium`, validated text instead of a forced output tool | Still no model output for 210 seconds |
| Google Gemini 3.8 Flash, native schema stream | HTTP 503, high demand |
| NVIDIA Ultra, existing 65-second probe timeout | No response headers; timeout |
| NVIDIA Ultra, extended 180-second probe timeout | Still no response headers; timeout |
| NVIDIA Super, thinking `low`, same claim/evidence | Two JSON inferences, 34.25 seconds |
| OpenRouter Super, thinking `low`, strict schema, same claim/evidence | Two JSON inferences, 42.78 seconds |

These measurements establish an availability problem with the configured generation chain during this run. Longer waits and truthful status labels cannot make an unavailable model produce an answer. The forced output tool was not the sole cause. They do not establish future provider uptime or article-wide inference quality.

## Pipeline fixes

- `provider_profiles.json` supplies the exact OpenRouter Super/Ultra thinking vocabulary on a cold dispatch. Super uses `low`; Ultra uses `medium`, not unsupported `low`. Automatic operations retain provider output limits.
- `provider_capabilities::{parse_openrouter_reasoning, fetch_evidence, apply_catalog}` reads `reasoning.supported_efforts` from the model catalog during background refresh. Endpoint parameter support alone never invents effort levels. Current catalog evidence can narrow a profile; an unknown control is omitted and diagnosed. Model swaps retain their own contracts.
- `brief_stages::classify_context_cached` persists validated context batches against original body spans, claim, policy/prompt and captured Classifier chain. Retry reuses them, including when synthesis or later fidelity fails.
- A failed classifier batch withholds its claim's analysis and records `phase=context`; independent claims continue. Generated checkpoints are revalidated, including mandatory counterevidence, and receipts distinguish reused work from newly sent requests. A clean commit clears the checkpoints.
- All claims failing remains a failed stage. Fidelity and final-selection failures still block unvalidated publication. Links receives only accepted inferences; legitimate empty results keep their existing reasons. Claims/body roots, stage publication/indexing and lease fencing retain their contracts.

## Route the supplied profile through working configured models

For this profile, the existing Summarization chain is OpenRouter Super → Google Flash → NVIDIA Super. Link the two brief generation operations to that chain. Their logical role remains Synthesis, and its defaults for other work remain unchanged. Subsequent Summarization model swaps are followed by the references.

Merge these entries into the existing `config.toml`, replacing these two entries if already present:

```toml
[defaults.operation_overrides."intel_brief_inferences.generate"]
route_role = "summarization"

[defaults.operation_overrides."intel_brief_links.generate"]
route_role = "summarization"
```

Alternatively import the supplied credential-free schema-2 routing update in Profile → System → Configs. Its provider/tool/quota arrays are empty merge inputs, and its role assignments match the supplied profile. Reopen the brief: the changed operation revisions rerun Inferences and Links while compatible Claims and the full body remain reusable. No global preset or model default is changed by the code patch.

## Re-run the configured integration check

The ignored `configured_profile_runs_claims_inferences_and_links` test retains a fixed fictional article through the Store/NLP receipt boundary and seeds related metadata, then exercises real Claims generation, extraction fidelity, context classification, Synthesis, inference support, semantic selection, Links and canonical publication. It requires nonempty claims, inferences and links, active canonical memories of each kind, no warning completion, and reuse on an unchanged revisit. The routing-update merge asserts that provider credentials, unrelated settings/tool credentials and quotas survive. Vector embedding/index draining is outside this check.

The 2026-10-16 live run using the supplied profile plus the routing update produced four claims, five inferences and two links in 285.96 seconds. All 13 model requests succeeded, and active canonical Claim, Inference and Related memories were asserted. An earlier generation-only run produced three claims, five inferences and two links in 261.96 seconds. A separate intervening run failed at Claims fidelity on a transient connection error; the successful runs establish execution of the full model path, not a provider uptime guarantee.

```sh
ARGOS_TEST_PROFILE=/absolute/path/arogs-config.json \
ARGOS_TEST_BRIEF_PATCH=/absolute/path/argos-brief-routing-update.json \
ARGOS_TEST_BRIEF_REPORT=/absolute/path/brief-live-results.json \
python3 scripts/agent_cargo.py test -p argos-osint-core --locked --no-default-features \
  configured_profile_runs_claims_inferences_and_links -- --ignored --nocapture
```

Omit `ARGOS_TEST_BRIEF_PATCH` to reproduce the original route chain. This test calls the configured providers; it never contacts article sites, uses tool credentials, prints keys or alters the supplied configuration.

For the optional local NLP path, add `--features nlp`. A managed test environment with its own trusted proxy CA can additionally enable `reqwest/rustls-tls-native-roots` and supply its CA through `SSL_CERT_FILE`; certificate verification must remain enabled. No application TLS change is needed.

Provider contract reference: [OpenRouter per-model reasoning options](https://openrouter.ai/docs/guides/best-practices/reasoning-tokens#discovering-per-model-reasoning-options). Availability observations above are direct authenticated probes, not inferred from the catalog.
