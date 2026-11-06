# Local classifier

[Code route](index.md) · [Taxonomy](../concepts/strategic-domains-taxonomy.md) · [Wiki](../../index.md)

@tag:classifier

Atlas uses the local exact-two classifier and current-taxonomy readers. Historical admission and provider monitoring have separate durable jobs; Focus Brief and generated neighborhood summaries run on demand. The synthetic seed is development evidence, not independent production-quality approval. Verification results and empirical timing are reported separately from these source contracts.

## Ownership and public entry points

The feature-independent [taxonomy](../../../crates/argos-osint-core/src/atlas_taxonomy.rs) owns the six stable IDs and official names. The optional `classifier` Cargo feature adds QuickJS independently of `nlp`; supported default builds enable `embedded,nlp,classifier`. Management commands remain parseable without the feature and report unavailable rather than initializing an engine.

| Source | Responsibility |
| --- | --- |
| [classifier/mod.rs](../../../crates/argos-osint-core/src/classifier/mod.rs) | Typed errors, receipts, snapshots, inference facade, worker controls and compiled identity |
| [engine.rs](../../../crates/argos-osint-core/src/classifier/engine.rs) | Worker-owned QuickJS runtime, fixed-code integrity checks, bounded queues, cancellation and restore |
| [artifact.rs](../../../crates/argos-osint-core/src/classifier/artifact.rs) | Immutable model files, active pointer, writer lease, checksum validation and activation history |
| [training.rs](../../../crates/argos-osint-core/src/classifier/training.rs) | Corpus validation, canonical hashes, deterministic family representatives and candidate fitting |
| [calibration.rs](../../../crates/argos-osint-core/src/classifier/calibration.rs) | Independent isotonic gates, family evaluation, promotion evidence and activation authorization |
| [provider_labels.rs](../../../crates/argos-osint-core/src/classifier/provider_labels.rs) | Raw provider-label contracts, text evidence and masked partial membership |
| [provider_capture.rs](../../../crates/argos-osint-core/src/classifier/provider_capture.rs) | Exact, bounded provider metadata capture before filtering; AI tags remain separate |
| [atlas_admission_maintenance.rs](../../../crates/argos-osint-core/src/atlas_admission_maintenance.rs) | Durable retained-occurrence reclassification with 64-row activations and original provenance |
| [storage/classifier_calibration.rs](../../../crates/argos-osint-core/src/storage/classifier_calibration.rs) | Observation origins, terminal-cycle outbox and independent bounded monitoring |
| [atlas_calibration.rs](../../../crates/argos-osint-core/src/atlas_calibration.rs) | Bounded story-family discovery and permanent partitions |
| [data_paths.rs](../../../crates/argos-osint-core/src/data_paths.rs) | Saved local dataset/artifact locations shared by Profile and Venture |
| [dataset.rs](../../../crates/argos-osint-core/src/classifier/dataset.rs) | Read-only runtime resolution and bounded synthetic CSV validation |
| [readiness.rs](../../../crates/argos-osint-core/src/classifier/readiness.rs) | Shared home probe/cache, manual jobs, pin/evaluate/rollback and cycle-preflight permit |

The neutral [runtime budget](../../../crates/argos-osint-core/src/js_runtime_budget.rs) enforces classifier reservations without importing NLP policy. No JavaScript values cross worker threads. Runtime errors, queue contention, cancellation and file failures remain operational faults; they never become an article `unk` label or a fabricated successful rejection.

## Model, bundle and receipt contract

Six independent Natural binary Bayes models classify `present` versus `absent`, one model per domain. The JavaScript bridge tokenizes title/description once and shares that token array across models. Features use binary token presence, not token counts or TF-IDF. `Unknown` membership omits a provider example from that binary model; missing provider labels do not establish negatives. Vocabulary fitting uses training representatives only.

Each model persists its own append-only feature order and independent token-to-index bindings. Restore never reconstructs it from object-key enumeration; numeric-looking tokens must not reorder earlier feature indices. Log-space present/absent scores produce per-domain raw log odds. These are relevance signals, not probabilities, and are calibrated independently rather than normalized across domains.

The generated [bundle](../../../crates/argos-osint-core/assets/classifier/natural-classifier.js) exposes `globalThis.argosNaturalBayes`; the fixed [bridge](../../../crates/argos-osint-core/src/classifier/bridge.js) exposes `globalThis.argosClassifier`. All five operations (`trainBatch`, `finishTraining`, `classifyBatch`, `serializeModel`, `restoreModel`) exchange data-only JSON strings. Model artifacts cannot supply code, filesystem bindings or runtime Node imports.

Pinned upstream is Natural 8.2.6 and apparatus 0.0.10. Vendored files, licenses, adaptation patch, source hashes and emitted protocol details live in the [asset README](../../../crates/argos-osint-core/assets/classifier/README.md) and [source manifest](../../../crates/argos-osint-core/assets/classifier/source-manifest.json). Adaptations remove Node persistence/events/parallel helpers and the unrelated top-level apparatus closure, preserve token semantics and stop words, maintain explicit feature order, and expose stable log-space scores. Build and parity tooling use Node; the shipped engine does not.

Runtime limits begin at 128 MiB heap and 1 MiB stack per worker, with a 512 MiB process reservation ceiling. Inference batches have at most 64 items. Candidate fitting uses a separate cancellable worker and one OS writer per home. The ordinary bridge queue is bounded to 16 MiB. Immutable restore has a separate owned startup lane for payloads up to the 64 MiB artifact limit; completed candidates close their fitting worker before terminal restore, reusing its runtime slot. Replay responses are bounded separately from QuickJS heap/stack reservations. These controls do not assert a 512 MiB process RSS limit. Queue count/bytes, queue timeout, bridge-step deadline and overall candidate deadline are separate bounds; the default overall training deadline is 30 minutes, not a renewed per-batch allowance. Limits are implementation controls, not measured throughput claims.

A receipt retains six scores/raw diagnostics and publishes exactly the top two independently qualifying domains. Informative-feature and domain thresholds still apply to a manually activated seed. Blank, unsupported-language, OOV and insufficient-domain inputs have explicit rejection reasons; model failures propagate as errors. Receipt schema, taxonomy, preprocessing, model and admission-policy identities are validated when persisted results are consumed.

## Immutable storage and activation

Files live in the configured `[data_paths].classifier_dir` directory, defaulting to `<ARGOS_HOME>/classifier/`, independently of SurrealDB:

- `models/<model_id>/manifest.json`, `model.json`, `metrics.json`: immutable candidate inputs, learned parameters and actual evaluation report.
- `active.json`: atomic active pointer with immutable manifest/payload/report hashes and checked activation generation.
- `writer.lock`: stable OS lock shared by manual and future background fitting; never unlink it to clear contention.
- `activations/<generation>.json` and `activated-models/<model_id>.json`: bounded committed history proving named rollback eligibility.
- `manual-jobs/<job_id>/`: immutable request, bounded progress/result and cancellation records. Per-job ownership is checked alongside the shared writer; a PID or timestamp alone is not authority.

Validation binds all compiled identities, the six domains and binary label support, finite parameters, report/payload consistency and actual immutable bytes. Activation and rollback revalidate and restore before replacing the pointer. A candidate alone is not an activation capability. Named rollback is limited to models proven previously activated; an arbitrary synthetic candidate cannot bypass consent through rollback. A failure after pointer replacement is reported as uncertain and re-read under ownership rather than claiming the previous model certainly survived.

Synthetic installs retain candidates by default. Explicit synthetic activation has visible **Synthetic baseline** provenance and does not grant production approval. Automatic promotion needs independent reviewed-real calibration/development families, sufficient accepted family counts, complete pair evidence, nonregression and measured runtime budgets. Seed row counts or provider/model agreement cannot satisfy those requirements.

## Dataset resolution and validation

Resolution order is explicit `--file` → captured `ARGOS_CLASSIFIER_CSV` → saved `[data_paths].article_csv` → runtime repository `dataset/` → repository `datasets/` → `<ARGOS_HOME>/dataset/`, using the exact filename `argos_atlas_strategic_domains.csv`. Explicit/env/saved selections are authoritative, including an explicitly empty invalid value. Only an absent automatic candidate permits fallback; a malformed selected CSV never silently selects a different corpus.

The resolver recognizes a workspace through bounded Cargo markers while walking captured cwd ancestors, then executable-parent ancestors. Installed builds have no embedded developer-path fallback and do not need `.git`. Resolution probes regular-file metadata and readability without parsing CSV contents. Parsing checks the same bounded open stream, exact-byte hash and before/after file identity; file changes abort the operation.

The parser accepts RFC quoted commas/quotes/newlines, LF/CRLF and one leading UTF-8 BOM. The header is the exact unique 18-field set in the [dataset contract](../concepts/strategic-domains-taxonomy.md), in any column order. Limits include 128 MiB input, 128 KiB raw records, 16 KiB metadata fields, 32 KiB title/description per row, 64 MiB total feature text and 50,000 rows. No truncation, lossy decoding or label/notes-as-feature behavior is permitted.

Every CLI CSV row is complete synthetic English annotation. The seed version additionally requires 1,700 rows, 220 families, 1,130/285/285 split rows, 142/39/39 split families, focused/pair/negative family cardinalities and the prescribed domain/pair counts. Other synthetic versions retain strict schema, uniqueness and family-leakage checks but do not claim delivered-release validation. Reviewed-real evidence uses the typed reviewed pipeline; a CSV string cannot assert review approval. Canonical split/family hashes reuse the training module's framing.

The seed's 55 positive calibration rows per domain represent eight independent positive families, not 55 independent stories. This does not meet the 20-accepted-family production gate.

## Shared management and launchers

```sh
argos classifier status --json
argos classifier train --file /runtime/path/argos_atlas_strategic_domains.csv
argos classifier train --activate-synthetic-baseline
argos classifier evaluate --file /runtime/path/argos_atlas_strategic_domains.csv
argos classifier rollback --model previously-activated-model
argos classifier reclassify --request-id history-2026-11 --run-id retained-run
argos classifier history-job --job-id returned-job-id --action status
argos classifier history-job --job-id returned-job-id --action pause
argos classifier history-job --job-id returned-job-id --action resume
argos classifier history-job --job-id returned-job-id --action cancel
```

Status uses one `classifier-readiness-v1` DTO with `missing`, `ready`, `training`, `invalid`, `incompatible`, `unavailable`. Dataset state, integrity and active-model state are orthogonal. Training may retain a usable prior model; missing CSV does not invalidate an existing one. A cold or changed artifact is unavailable/verification-pending until its real integrity check completes. Probes do not parse CSV, initialize QuickJS, train, open storage or scan all models. Dataset hashes are cached by actual file fingerprints; explicit train/evaluate and pin revalidate independently.

Manual training persists a bounded request before returning acceptance. Replaying the same ID/body returns the original operation, even if the CSV later changes or disappears; changed consent/file identity conflicts. Candidate-only completion is successful and distinct from activation, including when an existing pointer is invalid or incompatible. Explicit synthetic repair requires a strictly verified prior activation identity and preserves its history/generation; unreadable or malformed lineage is never treated as an absent prior. A durable terminal result precedes final owner/writer teardown. Readiness reports retryable `writer_busy` and disables new training until the existing writer is free; status observes that transition without creating a lock. Starting the next operation reaps its exact terminal monitor through a bounded retained join. Cancellation and shutdown retain owned worker handles, release only owned leases and preserve the prior model before commit. Evaluation pins once, scores deterministic Test-family representatives with frozen gates and does not fit, calibrate or activate.

Venture checks the saved classifier directory before presenting service choices. When no usable saved classifier exists, stable key **5**, **Train and save classifier**, prompts for the article training CSV. `auto` uses the configured/default dataset. Selecting this initial setup action explicitly trains, saves and activates a synthetic baseline, identified as development evidence. A successful return triggers a fresh integrity check before TUI, API or Web becomes available. A failed or cancelled setup leaves the services unavailable. Actor seeding remains optional key **4** and also prompts for its CSV path. Explicit CLI training retains its candidate-only default.

TUI Profile → Configs → Dataset paths edits the actor CSV, article training CSV and classifier save directory in `config.toml`. Blank values select automatic defaults; relative saved paths use `ARGOS_HOME`, and `~/` uses the operating-system home. Explicit `--file` and dataset environment variables take precedence over saved CSV paths. A configured missing/invalid CSV never falls back silently. Changing the save directory selects a different artifact store; it does not move or delete existing models. All model, pointer, writer-lock and journal operations use that store. Saved Natural classifier JSON restores without the training CSV. These local machine paths remain separate from portable credential export/import.

The classifier progress panel shares terminal restoration/cancellation mechanics with actor seeding and writes progress to stderr, leaving final JSON on stdout. Classifier setup has no React panel or HTTP management routes.

The `preflight_cycle` API supplies a private permit retaining a real model pin. Fresh acquisition pins active; resumed acquisition/admission requires the exact saved snapshot; proven post-admission-only work needs no classifier. Atlas consumes this permit before run mutation, inventory or acquisition. Its run-scoped adapter batches local computation at 64 observations and persists occurrence/source/model receipts through fenced outcome transactions. The pin also binds the immutable manifest hash, so a changed gate/report cannot reuse a cached worker or replace a saved cycle revision.

[Admission storage](../../../crates/argos-osint-core/src/storage/classifier_admission.rs) reads at most 512 thin headers per page, then reserves claims in transactions of at most 64 rows under the run/source/epoch fences. Reservation does not advance the durable page cursor. Outcome publication advances it for published work; pages with no claims use a guarded empty-page checkpoint. If a later reservation fails, the reserved prefix remains leased and recoverable after expiry under a new activation. Payload hydration and local inference remain bounded to 64 items. Foreground outcome batches publish in transactions of at most 16 items so their independent heartbeat can renew between commits. A fair gate shared by one admission owner gives heartbeat and foreground write transactions separate turns; database guards still fence other owners and processes. Each receipt/outcome/projection remains atomic; a later subgroup failure preserves earlier receipts and cursor progress for immutable replay. See the [admission contracts](../../../crates/argos-osint-core/tests/storage_classifier_admission.rs) for paging, reopen and ownership behavior.

## Verification entry points and evidence

The [implementation acceptance record](../../../.planning/2026-11-05-atlas-performance-completion/acceptance.md) distinguishes actual checks, immutable identities and pending gates.

Bundle commands are `node scripts/build_classifier_bundle.mjs --check` and `node scripts/test_classifier_bundle.mjs`; the parity tool can emit a reviewable golden oracle. Rust public contracts are in `tests/classifier_{contract,artifact,training_calibration,dataset,readiness}.rs` plus adversarial artifact/runtime targets. Follow the [contributor workflow](contributing.md) for authorized host-native embedded checks and required gate. Installed-build/no-Node, interruption/memory, atomic history, CLI/PTY and React browser evidence are separate acceptance checks.

## Historical admission and provider monitoring

Historical requests are idempotent by request ID, scope and exact model pin. The accepted cutoff and upper occurrence key freeze the retained cohort. Each activation processes at most 64 occurrences, capturing original metadata and provenance before scoring. Publication uses the same conditional outcome transaction as foreground admission, with actual maintenance task authority. Source conflicts retain immutable captures; repeated material changes block that occurrence's job until explicit resume. Successful classification is followed by authoritative metadata, Related, actor and lifecycle receipt audits. A job cannot finish while affected derivatives are pending. Related readers validate native pair fingerprints, separately from retained legacy display hashes, against current admission/source/metadata before pagination; see the [Related revision contract](../concepts/atlas-classification-and-related-articles.md#phase-6-and-cross-cycle-related). Rejection hides new admitted indexes while preserving historical evidence and original cycle statistics. A saved pin is checked without starting JavaScript for already-current or derivative-only work.

NewsData v1 and Currents v1 categories are captured from existing responses before country, time, duplicate and admission filters. Metadata observations are deduplicated independently of exact per-fetch/cycle origins. Capture uses 64-row pages and bounded JSON; credentials, full article bodies and unknown extra fields are excluded. Excluded category labels are filtered individually, not treated as article rejection. Unknown vocabularies abstain; AI tags never become category labels.

Terminal cycle publication and one calibration job/outbox keyed by cycle and observation watermark commit together. Pause does not enqueue. Empty cohorts complete as `no_new_data`; a missing saved model blocks the monitoring job. The separate scheduler pool processes at most 512 observations per activation and yields between 64-row pages when foreground work appears. Observation dispositions and counters checkpoint atomically; replay and restart do not count the same origin twice. Story families retain permanent partitions, indexed bounded candidate discovery, and conservative quarantine for ambiguous merges or exhausted retention capacity. The page checks retained-family capacity once under its shared writer guard and accounts for each new family in that transaction.

The provider mapping currently has **no independently reviewed precision certificate**. Its partial labels remain monitoring/review material; no provider-only candidate fitting, threshold adjustment or automatic promotion is authorized. The existing masked learner and promotion validator require independently reviewed complete calibration/development evidence, sufficient families, pair quality and latency/memory comparisons before they can authorize replacement. Synthetic installs retain their active model. Runtime observation storage remains separate from the seed CSV.
