# Seeded actor catalog

[Core](core.md#brain) · [Storage](storage.md) · [Launcher](interfaces.md)

`@tag:brain-memory`

Brain actors come from the local `datasets/argos_actors.csv` catalog after its first seed. The CSV supplies stable `actor_id`, `actor_name`, `name_normalized`, JSON `aliases`, actor type and provenance columns. Catalog identity is a deterministic hash of the source actor ID; equal names never collapse different source IDs. All CSV columns remain in `brain_actor.provenance_json` and `catalog_json`. Actor memories and graph nodes use the same IDs. Source catalog membership is an identity reference, not evidence of involvement, current sanctions, recognition or wrongdoing.

[![Catalog ingestion and matching](../../diagrams/actor-catalog.svg)](../../diagrams/actor-catalog.html)

## Venture and commands

Venture probes `argos storage actor-seed-status` in a short-lived child before drawing its menu. It shows **Seed DB** when the completion receipt is missing/incomplete **or** the catalog actor table is empty. A failed/locked status query is reported as unavailable and never treated as an empty database. The parent owns no datastore between sessions.

Selecting Seed DB runs `argos storage seed-actors` in a child process. New homes initialize embedded storage first; existing homes retain their configured storage mode. The dataset resolves from `--file`, `ARGOS_ACTORS_CSV`, the working directory's `datasets/argos_actors.csv`, the Argos home's dataset folder, then the source checkout. Installed binaries must have a local CSV or an explicit path; the 140 MB CSV is not embedded in the executable. No network or model is used for seeding.

```sh
argos storage actor-seed-status
argos storage seed-actors --file datasets/argos_actors.csv
```

The complete CSV is parsed and validated before writes. Empty inputs, missing columns, duplicate IDs, malformed CSV and malformed alias arrays fail rather than marking a partial import complete. Ingestion commits batches of 128 actors, aliases, typed memories, graph nodes and progress together. Stable record keys and insert-if-absent writes make a retry idempotent. A leased `brain_actor_seed:catalog` receipt prevents competing seed writers; interrupted processes become retryable after the five-minute lease. The receipt becomes `complete` only after all batches commit. This is an initial static catalog import; updating existing catalog identities requires an explicit reset/reseed, not a silent overwrite of analyst state.

The progress screen reuses the Profile gauge and terminal theme. It shows committed actor rows / validated total rows, alias rows, elapsed seconds and completion. Validation has an unknown denominator. Esc/Ctrl+C requests cancellation after the current committed batch; retry retains those batches. Piped input/output prints progress lines. The launcher returns after the child exits, releasing embedded storage.

## Article and claim association

A repository caches one deterministic multi-pattern matcher. Matcher initialization runs in an owning worker so an HTTP timeout cannot discard a partially loaded cache and restart it on the next request. Per-article reads check the catalog completion receipt directly; full actor counts remain part of explicit catalog status/setup, not every brief read. Title and description are compared against canonical names, `name_normalized` and strong aliases with normalized Unicode letters/digits, collapsed punctuation/spacing and word boundaries. Matching retains byte spans into the original text. Short keys under three characters and `weak_aliases` are excluded. Aliases with multiple catalog owners stay ambiguous and do not pick a person or organization. Dataset-specific actor types map to Brain person/place/organization/unknown; their original values remain in provenance.

Once activated, phase 6 uses catalog matches even when NLP metadata is absent. Mention receipts and `mentioned_in` links point to the seeded actor, so different articles reuse its identity. Intel API/TUI actor lists and brief context use the same matcher; extraction-only names do not add actors. Claim publication keeps claims whose subjects do not resolve, omitting those actor references rather than creating provisional identities. Existing classification, extraction, source revisions, claim/inference publication and relationship evidence rules remain in place. Catalog actors remain active without article support and are excluded from automatic identity normalization/deduplication. Explicit merge operations cannot merge catalog IDs.

Historical homes preserve their prior source-scoped behavior until the first seed activates catalog mode. An interrupted seed never falls back to creating source-scoped actors. A reset removes both catalog data and its receipt and makes Seed DB available again. Default recall remains Claim/Inference; seeding does not schedule 95,000 model embedding requests.

## Verification

`actor_catalog` unit tests cover boundaries, Unicode spans, ambiguous aliases and invalid CSV. `storage_actor_catalog` verifies partial seed/retry, idempotency, completion/empty-table readiness, canonical matching without NLP and omission of unknown identities using the isolated storage harness. Launcher tests cover conditional menu dispatch and progress rendering. Run targeted commands through the [contributor workflow](contributing.md), with `ARGOS_SURREAL_MODE=embedded ARGOS_SURREAL_REQUIRED=1` for storage contracts.
