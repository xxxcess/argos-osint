# Brief fast preset — held-out comparison

Spec §9 item 7. The preset (Claims and Links follow Summarization) is recommended only after a measured held-out comparison shows that scope and fidelity quality are acceptable. Synthetic fixture success is not enough.

## Harness

- Fixtures: `crates/argos-osint-core/src/intel_recon/fixtures/brief_heldout.json`. There are 8 title/description articles covering an allegation with a denial, a quote, a pronoun, an alias, a negation, a late correction, zero claims, and uncertainty. Claims never read the body, so oversized bodies are an Inferences concern and are not part of this Claims comparison.
- Test: `brief_stages::tests::heldout_fast_preset_comparison`. It is `#[ignore]` and opt-in with `ARGOS_HELDOUT_LIVE=1`.
- Each article gets a fresh store, and the real `RuntimeBackend` runs the Claims root through `run_claimed`, including generation, schema validation and fidelity. The harness records wall time, dispatched wire requests, prompt and completion tokens, kept and withheld claims, and a per-article expectation check (claim count bounds plus required mentions). It then asserts that an unchanged revisit attaches to the same chain with zero requests (warm).
- Both configurations use OpenRouter free models, and fidelity goes to the same assessor (`google/gemma-4-31b-it:free`), so only the Claims generation route differs:
  - shipped: Claim assessor `nvidia/nemotron-3-ultra-550b-a55b:free`;
  - fast preset: linked to Summarization `nvidia/nemotron-3-super-120b-a12b:free`.

```sh
set -a; . ~/.config/argos/openrouter.env; set +a
ARGOS_HELDOUT_LIVE=1 ARGOS_HELDOUT_OUT=/tmp/heldout.json \
  python3 scripts/agent_cargo.py test -p argos-osint-core --locked --no-default-features --lib \
  -- heldout_fast_preset_comparison --ignored --nocapture
```

## Result (2026-10-14, 05:1x EDT)

**Not measured.** The only configured OpenRouter key was rejected on every request with HTTP 401 "User not found". OpenRouter's `/api/v1/key` endpoint returns the same response for that key, so the credential itself is invalid, not the request. All 16 Claims runs failed fast on the first dispatch, as typed `response/auth` failures with no fallback replay. No latency, token or quality figures exist. No Google or Nvidia keys are available for native routes.

**Decision:** the fast preset and the focused oversized path stay **off** and are not recommended. Re-run the command above with a valid key before changing that.
