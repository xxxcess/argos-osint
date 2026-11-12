# Classifier runtime

[Code route](index.md) · [Taxonomy](../concepts/strategic-domains-taxonomy.md) · [Wiki](../../index.md)

@tag:classifier

New Atlas cycles use a private Linux Rust-BERT zero-shot worker. The worker evaluates each article against the six canonical domain definitions and returns independent relevance scores. Atlas admits up to two qualifying domains. Historical admission and provider monitoring have separate durable jobs. The retained Natural engine supports optional explicit training and exact-pin replay of Natural runs. Native macOS pages send gateway requests; the desktop owns neither classifier nor NLP engines.

## Ownership and public entry points

The feature-independent [taxonomy](../../../crates/argos-osint-core/src/atlas_taxonomy.rs) owns the six stable IDs, display names and domain definitions. The standalone Rust-BERT executable keeps LibTorch separate from the Fleetworks service host build. The optional `classifier` Cargo feature adds QuickJS for Natural training and replay independently of `nlp`. The Compose service host enables `server,nlp,classifier`.

| Source | Responsibility |
| --- | --- |
| [tools/atlas-bert](../../../tools/atlas-bert) | Linux CPU worker for zero-shot inference with deployment-owned DistilBERT MNLI assets |
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
| [data_paths.rs](../../../crates/argos-osint-core/src/data_paths.rs) | Saved dataset/artifact locations for backend owners and retained operator clients |
| [dataset.rs](../../../crates/argos-osint-core/src/classifier/dataset.rs) | Read-only runtime resolution and bounded synthetic CSV validation |
| [readiness.rs](../../../crates/argos-osint-core/src/classifier/readiness.rs) | Shared home probe/cache, manual jobs, pin/evaluate/rollback and cycle-preflight permit |

The neutral [runtime budget](../../../crates/argos-osint-core/src/js_runtime_budget.rs) enforces classifier reservations without importing NLP policy. No JavaScript values cross worker threads. Runtime errors, queue contention, cancellation and file failures remain operational faults; they never become an article `unk` label or a fabricated successful rejection.

## Fleetworks classifier service

The [Atlas Compose overlay](../../../deploy/atlas-dispatch.md) runs an authenticated private classifier role. The service keeps the pinned Rust-BERT worker warm and checks exact inference receipts. Supply the Linux worker and checkpoint files under the workspace using paths visible inside the container. The service can report transport readiness before model assets are available. Atlas reports a pin failure on the durable cycle job.

The retained operator command `venture train-legacy` snapshots the selected dataset for one-shot Natural training. Its explicit `--activate` option selects that Natural artifact under the existing compatibility contract. The native maintenance flow below separates those operations. Fresh Atlas cycles use Rust-BERT.

## Native maintenance and optional Natural activation

Workstation offers training only when the gateway reports an actual permitted durable consumer. The user selects a configured dataset or uploads bounded CSV bytes; the server verifies their hash and purpose and returns an opaque dataset reference. The [maintenance contract](../../../crates/fleetworks-contracts/src/readiness.rs) binds that reference, registered origin, request ID and confirmed configuration revision. A local Mac path grants no server filesystem access. The [gateway admission](../../../crates/argos-osint-api/src/maintenance.rs) returns a durable Job receipt; acceptance and completion remain separate. Status and authorized results come from registered Jobs.

This operation trains only optional Natural. The [Linux setup executor](../../../services/api-host/src/setup_jobs.rs) explicitly retains the prior active model and records the candidate/dataset identity with `activation_requested=false`. Successful training neither activates its candidate nor retrains Rust-BERT. To activate, the native user separately inspects the authorized training job's exact candidate hashes and expected prior active revision, then confirms the [activation request](../../../crates/fleetworks-contracts/src/classifier_activation.rs). Synthetic baselines need their own explicit consent. The [activation gateway](../../../crates/argos-osint-api/src/classifier_activation.rs) enforces current Native maintenance/activity permissions, registration and private job eligibility; its backend worker owns artifact validation, writer fences and effect reconciliation. A lost or uncertain effect remains unresolved until operation-owned readback proves the outcome. Fresh Atlas collection continues to pin Rust-BERT independently of the Natural active pointer.

The new [Linux verification/bundle helper](../../../tools/atlas-bert/scripts/verify_linux.py) and [classifier CI lane](ci.md#atlas-bertyml--classifier-worker-checks) configure native AMD64 and ARM64 build/link, real pinned-checkpoint inference and extracted-bundle checks inside Debian bookworm. Both-architecture Linux inference execution remains **pending** cutover evidence. Native fixture/unit results and installed Mac GUI evidence are separate; this maintenance path claims no completed installed-app journey or model-performance result.

## Linux deployment and retained standalone worker setup

Prepare the CPU worker and pinned English NLI checkpoint for the container's Linux architecture. The Linux helper above packages the worker, matching native libraries and verified assets with ABI/hash checks; the external operator supplies container-visible configuration. The worker runs offline once its assets are installed. The following standalone build instructions retain the earlier operator/direct-worker compatibility path, including its Intel macOS example; they are not setup steps for the Rune desktop and do not establish Linux deployment or installed-app acceptance.

1. Set up CPU LibTorch 2.1.0 and Python 3.11. The pinned PyTorch wheel supplies both the worker's LibTorch and the one-time model converter. On Intel macOS, install the Xcode Command Line Tools (`xcode-select --install`) and use an x86_64 Python and Rust toolchain. PyTorch 2.1.0 publishes an Intel macOS wheel. [PyTorch release files](https://pypi.org/project/torch/2.1.0/#files) · [tch 0.14.0 setup](https://docs.rs/crate/tch/0.14.0/source/README.md)

   On macOS, run the following in the shell you will use to build and run the native worker.

   ```sh
   python3.11 -m venv /absolute/path/atlas-bert-venv
   . /absolute/path/atlas-bert-venv/bin/activate
   python -m pip install 'numpy==1.26.4' 'torch==2.1.0'
   export LIBTORCH_USE_PYTORCH=1
   atlas_torch_lib=$(python -c 'import pathlib, torch; print(pathlib.Path(torch.__file__).parent / "lib")')
   export DYLD_LIBRARY_PATH="$atlas_torch_lib${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}"
   ```

   On Intel macOS, `rustc -vV` should report `host: x86_64-apple-darwin`. On Linux use the [PyTorch CPU wheel index](https://download.pytorch.org/whl/cpu) to install `torch==2.1.0` and set `LD_LIBRARY_PATH` in place of `DYLD_LIBRARY_PATH`. For a standalone CPU LibTorch 2.1.0 distribution, set `LIBTORCH` to its absolute root, omit `LIBTORCH_USE_PYTORCH`, and add its `lib` directory to the platform loader path. Keep the same loader environment when running the native worker.

2. From the repository root, prepare the [pinned Typeform DistilBERT MNLI checkpoint](https://huggingface.co/typeform/distilbert-base-uncased-mnli/tree/cfa538a0fddbbd978fefe8966c1aeff7ad409c90). Choose a new model directory.

   ```sh
   atlas_model_dir=/absolute/path/atlas-distilbert-mnli
   bash tools/atlas-bert/scripts/prepare-model.sh "$atlas_model_dir"
   ```

   The script verifies the source files and converts the weights with the Rust-BERT 0.22.0 converter and the worker's Cargo lockfile. Keep the resulting `config.json`, `vocab.txt`, and `rust_model.ot` together in the model directory.

3. From the repository root, build the separate worker.

   ```sh
   cargo build --release --locked --manifest-path tools/atlas-bert/Cargo.toml \
       --target-dir "$PWD/target/atlas-bert"
   ```

4. For Compose, stage the Linux executable, required LibTorch libraries and model under the Fleetworks workspace. Set the container-visible paths in workspace `classifier/zero-shot.json` as shown in the [Atlas overlay guide](../../../deploy/atlas-dispatch.md). The optional threshold defaults to `0.8` and accepts values in `[0.5, 1)`. Both slots use the same threshold.

5. Smoke-test the native worker directly before preparing the packaged assets.

   ```sh
   printf '%s\n' '{"texts":["A ministry signed a diplomatic agreement."]}' |
     /absolute/path/to/argos-atlas-bert serve /absolute/path/atlas-distilbert-mnli
   ```

   Start Venture with `up --atlas`, submit a cycle through the Atlas client and inspect the returned job through Jobs. The classifier service can report transport readiness before its model assets are available. Atlas verifies the pin before a new collection and records configuration or inference errors on the job.

A run pins the executable, model bytes and admission threshold. Resume of acquisition or admission requires those exact assets. Proven derivative-only work can resume after the classifier is no longer loaded.

## Legacy Natural model, bundle and receipt contract

Six independent Natural binary Bayes models classify `present` versus `absent`, one model per domain. The JavaScript bridge tokenizes title/description once and shares that token array across models. Features use binary unigram/bigram presence; TF-IDF and document frequency are frozen Train-only selection/diagnostic statistics. `Unknown` membership omits a provider example from that binary model; missing provider labels do not establish negatives. Vocabulary fitting uses training representatives only. The shared versioned feature extractor preserves original UTF-8 spans, protects identifiers/acronyms, and forms adjacent bigrams before filtering grammatical noise. Field and punctuation boundaries break phrases. Sparse inference reads active feature indices rather than allocating six dense vocabulary vectors. Discriminative support counts nonoverlapping positive-evidence spans; vocabulary matches and class priors alone are insufficient support.

Each model persists its own append-only feature order and independent token-to-index bindings. Restore never reconstructs it from object-key enumeration; numeric-looking tokens must not reorder earlier feature indices. Log-space present/absent scores produce per-domain raw log odds. These are relevance signals, not probabilities, and are calibrated independently rather than normalized across domains.

The generated [bundle](../../../crates/argos-osint-core/assets/classifier/natural-classifier.js) exposes `globalThis.argosNaturalBayes`; the fixed [bridge](../../../crates/argos-osint-core/src/classifier/bridge.js) exposes `globalThis.argosClassifier`. All five operations (`trainBatch`, `finishTraining`, `classifyBatch`, `serializeModel`, `restoreModel`) exchange data-only JSON strings. Model artifacts cannot supply code, filesystem bindings or runtime Node imports.

Pinned upstream is Natural 8.2.6 and apparatus 0.0.10. Vendored files, licenses, adaptation patch, source hashes and emitted protocol details live in the [asset README](../../../crates/argos-osint-core/assets/classifier/README.md) and [source manifest](../../../crates/argos-osint-core/assets/classifier/source-manifest.json). Adaptations remove Node persistence/events/parallel helpers and the unrelated top-level apparatus closure, preserve token semantics and stop words, maintain explicit feature order, and expose stable log-space scores. Build and parity tooling use Node; the shipped engine does not.

Runtime limits begin at 128 MiB heap and 1 MiB stack per worker, with a 512 MiB process reservation ceiling. Inference batches have at most 64 items. Candidate fitting uses a separate cancellable worker and one OS writer per home. The ordinary bridge queue is bounded to 16 MiB. Immutable restore has a separate owned startup lane for payloads up to the 64 MiB artifact limit; completed candidates close their fitting worker before terminal restore, reusing its runtime slot. Replay responses are bounded separately from QuickJS heap/stack reservations. These controls do not assert a 512 MiB process RSS limit. Queue count/bytes, queue timeout, bridge-step deadline and overall candidate deadline are separate bounds; serialization/restore have a separate ten-second execution budget while inference and training batches retain their two-second step budget; the default overall training deadline is 30 minutes, not a renewed per-batch allowance. Limits are implementation controls, not measured throughput claims.

A receipt retains six scores/raw diagnostics and publishes one or two independently qualifying domains. Informative-feature and domain thresholds still apply to a manually activated seed. Eligible domains are ranked after gating. Exactly two tied eligible domains preserve both with ambiguous-primary telemetry. An unambiguous primary survives competing second-slot ties; a tied primary among more than two candidates abstains. Blank, unsupported-language, OOV and insufficient-domain inputs have explicit rejection reasons; model failures propagate as errors. Receipt schema, taxonomy, preprocessing, model and admission-policy identities are validated when persisted results are consumed.

## Legacy Natural storage and activation

Files live in the configured `[data_paths].classifier_dir` directory, defaulting to `<ARGOS_HOME>/classifier/`, independently of SurrealDB:

- `models/<model_id>/manifest.json`, `model.json`, `metrics.json`: immutable candidate inputs, learned parameters and actual evaluation report.
- `active.json`: atomic active pointer with immutable manifest/payload/report hashes and checked activation generation.
- `writer.lock`: stable OS lock shared by manual and background fitting; never unlink it to clear contention.
- `activations/<generation>.json` and `activated-models/<model_id>.json`: bounded committed history proving named rollback eligibility.
- `manual-jobs/<job_id>/`: immutable request, bounded progress/result and cancellation records. Per-job ownership is checked alongside the shared writer; a PID or timestamp alone is not authority.

Validation binds all compiled identities, the six domains and binary label support, finite parameters, report/payload consistency and actual immutable bytes. Activation and rollback revalidate and restore before replacing the pointer. A candidate alone is not an activation capability. Named rollback is limited to models proven previously activated; an arbitrary synthetic candidate cannot bypass consent through rollback. A failure after pointer replacement is reported as uncertain and re-read under ownership rather than claiming the previous model certainly survived.

Synthetic installs retain candidates by default. Explicit synthetic activation has visible **Synthetic baseline** provenance and does not grant production approval. Automatic promotion needs independent reviewed-real calibration/development families, sufficient accepted family counts, complete pair evidence, nonregression and measured runtime budgets. Seed row counts or provider/model agreement cannot satisfy those requirements.

## Legacy dataset resolution and validation

Resolution order is explicit `--file` → captured `ARGOS_CLASSIFIER_CSV` → saved `[data_paths].article_csv` → runtime repository `dataset/` → repository `datasets/` → `<ARGOS_HOME>/dataset/`, using the exact filename `argos_atlas_strategic_domains_v2.csv`. Explicit/env/saved selections are authoritative, including an explicitly empty invalid value. Only an absent automatic candidate permits fallback; a malformed selected CSV never silently selects a different corpus.

The resolver recognizes a workspace through bounded Cargo markers while walking captured cwd ancestors, then executable-parent ancestors. Installed builds have no embedded developer-path fallback and do not need `.git`. Resolution probes regular-file metadata and readability without parsing CSV contents. Parsing checks the same bounded open stream, exact-byte hash and before/after file identity; file changes abort the operation.

The parser accepts RFC quoted commas/quotes/newlines, LF/CRLF and one leading UTF-8 BOM. The header is the exact unique 18-field set in the [dataset contract](../concepts/strategic-domains-taxonomy.md), in any column order. Limits include 128 MiB input, 128 KiB raw records, 16 KiB metadata fields, 32 KiB title/description per row, 64 MiB total feature text and 50,000 rows. No truncation, lossy decoding or label/notes-as-feature behavior is permitted.

The default v2 release has 1,020 distinct synthetic scenarios and four disjoint partitions: Train 440, Calibration 230, Development 165, Test 185. Complete labels contain zero to six domains; editorial primary/secondary annotations are subsets. Unknown dataset versions fail explicitly. The original unversioned filename remains the frozen v1 fixture, selectable explicitly; it retains exact-two validation. Every CLI CSV row is complete synthetic English annotation. The frozen v1 seed version (`strategic-domains-v1-seed-1`) additionally requires 1,700 rows, 220 families, 1,130/285/285 split rows, 142/39/39 split families, focused/pair/negative family cardinalities and the prescribed domain/pair counts. The explicitly supported development versions (`synthetic-development-v1` and `synthetic-development-v2`) retain strict schema, uniqueness and family-leakage checks but do not claim delivered-release validation. Reviewed-real evidence uses the typed reviewed pipeline; a CSV string cannot assert review approval. Canonical hashes bind all four splits plus permanent family assignments. Calibration fits isotonic and regularized monotone sigmoid alternatives, including discriminative-support gates; Development compares calibration error and admission quality. Reports distinguish Calibration fit diagnostics from independent Development evaluation. A failed sigmoid fit retains isotonic with an explicit reason. Test is read only by explicit final evaluation.

The frozen v1 seed's 55 positive calibration rows per domain represent eight independent positive families, not 55 independent stories. This does not meet the 20-accepted-family production gate.

## Workspace training

This retained operator/CLI path runs the optional Natural trainer against a selected deployment-workspace dataset. Native Workstation uses the separate registered maintenance and candidate-activation flow above.

```sh
venture --workspace ./review-workspace train-legacy --file datasets/argos_atlas_strategic_domains_v2.csv
```

Venture snapshots the selected bytes for the training worker. Use `--activate` for explicit activation. [Training and dataset setup](../../../deploy/README.md#train-the-optional-natural-classifier)

Manual training persists a bounded request before returning acceptance. Replaying the same ID/body returns the original operation, even if the CSV later changes or disappears; changed consent/file identity conflicts. Candidate-only completion is successful and distinct from activation, including when an existing pointer is invalid or incompatible. Explicit synthetic repair requires a strictly verified prior activation identity and preserves its history/generation; unreadable or malformed lineage is never treated as an absent prior. A durable terminal result precedes final owner/writer teardown. Readiness reports retryable `writer_busy` and disables new training until the existing writer is free; status observes that transition without creating a lock. Starting the next operation reaps its exact terminal monitor through a bounded retained join. Cancellation and shutdown retain owned worker handles, release only owned leases and preserve the prior model before commit. Evaluation pins once, scores deterministic Test-family representatives with frozen gates and does not fit, calibrate or activate.

The `preflight_cycle` API supplies a private permit retaining a real model pin. Fresh acquisition pins Rust-BERT. Resumed acquisition or admission requires the exact saved pin. Atlas consumes the permit before run mutation, inventory or acquisition. The run-scoped adapter batches inference and persists occurrence/source/model receipts through fenced outcome transactions.

[Admission storage](../../../crates/argos-osint-core/src/storage/classifier_admission.rs) reads at most 512 thin headers per page, then reserves claims in transactions of at most 64 rows under the run/source/epoch fences. Reservation does not advance the durable page cursor. Outcome publication advances it for published work; pages with no claims use a guarded empty-page checkpoint. If a later reservation fails, the reserved prefix remains leased and recoverable after expiry under a new activation. Payload hydration and local inference remain bounded to 64 items. Foreground outcome batches publish in transactions of at most 16 items so their independent heartbeat can renew between commits. A fair gate shared by one admission owner gives heartbeat and foreground write transactions separate turns; database guards still fence other owners and processes. Each receipt/outcome/projection remains atomic; a later subgroup failure preserves earlier receipts and cursor progress for immutable replay. See the [admission contracts](../../../crates/argos-osint-core/tests/storage_classifier_admission.rs) for paging, reopen and ownership behavior.

## Verification entry points and evidence

The [classifier quality v2 acceptance record](../../../.planning/2026-11-06-classifier-quality-v2/acceptance.md) distinguishes actual checks, immutable identities, quality limitations and pending gates. The [earlier admission acceptance record](../../../.planning/2026-11-05-atlas-performance-completion/acceptance.md) retains the prior implementation evidence.

Bundle commands are `node scripts/build_classifier_bundle.mjs --check` and `node scripts/test_classifier_bundle.mjs`; the parity tool can emit a reviewable golden oracle. Rust public contracts are in `tests/classifier_{contract,artifact,training_calibration,dataset,readiness}.rs` plus adversarial artifact/runtime targets. Follow the [contributor workflow](contributing.md) for authorized host-native embedded checks and required gate. Installed-build/no-Node, interruption/memory, atomic history, CLI/PTY and React browser evidence are separate acceptance checks.

## Historical admission and provider monitoring

Historical requests are idempotent by request ID, scope and exact model pin. The accepted cutoff and upper occurrence key freeze the retained cohort. Each activation processes at most 64 occurrences, capturing original metadata and provenance before scoring. Publication uses the same conditional outcome transaction as foreground admission, with actual maintenance task authority. Source conflicts retain immutable captures; repeated material changes block that occurrence's job until explicit resume. Successful classification is followed by authoritative metadata, Related, actor and lifecycle receipt audits. A job cannot finish while affected derivatives are pending. Related readers validate native pair fingerprints, separately from retained legacy display hashes, against current admission/source/metadata before pagination; see the [Related revision contract](../concepts/atlas-classification-and-related-articles.md#phase-6-and-cross-cycle-related). Rejection hides new admitted indexes while preserving historical evidence and original cycle statistics. A saved pin is checked without starting JavaScript for already-current or derivative-only work.

NewsData v1 and Currents v1 categories are captured from existing responses before country, time, duplicate and admission filters. Metadata observations are deduplicated independently of exact per-fetch/cycle origins. Capture uses 64-row pages and bounded JSON; credentials, full article bodies and unknown extra fields are excluded. Excluded category labels are filtered individually, not treated as article rejection. Unknown vocabularies abstain; AI tags never become category labels.

Terminal cycle publication and one calibration job/outbox keyed by cycle and observation watermark commit together. Pause does not enqueue. Empty cohorts complete as `no_new_data`. A zero-shot cohort records provider monitoring and completes as `zero_shot_provider_monitoring_only` without queuing Natural training or promotion. A retained Natural cohort follows its legacy monitoring and training contract. The separate scheduler pool processes at most 512 observations per activation and yields between 64-row pages when foreground work appears. Observation dispositions and counters checkpoint atomically; replay and restart do not count the same origin twice. Story families retain permanent partitions, indexed bounded candidate discovery, and conservative quarantine for ambiguous merges or exhausted retention capacity. The page checks retained-family capacity once under its shared writer guard and accounts for each new family in that transaction.

The provider mapping currently has **no independently reviewed precision certificate**. Its partial labels remain monitoring/review material; no provider-only candidate fitting, threshold adjustment or automatic promotion is authorized. The existing masked learner and promotion validator require independently reviewed complete calibration/development evidence, sufficient families, pair quality and latency/memory comparisons before they can authorize replacement. Synthetic installs retain their active model. Runtime observation storage remains separate from the seed CSV.

## Independent derivative scheduling

Admission and the durable per-occurrence derivative job commit atomically. Collection completes after admission/handoff; Metadata, Related, Actors and Reconcile execute under the job’s own task lease. Collection pause/completion supplies no derivative authority and cannot cancel a separate job. Use `classifier derivative-job --job-id … --action status|pause|resume|cancel` for explicit controls. The scheduler resumes durable work on service startup; a short-lived CLI leaves pending work for the next startup.

One worker per home uses the existing serialized publisher. An activation currently dispatches one occurrence and at most one continuation, below the sixteen-unit ceiling, with a two-second dispatch budget checked before claim/dispatch. In-flight transactions may overrun; queue wait, publisher execution and traversal timings remain separate. Foreground requests receive priority with a fair derivative turn. Required source/body/catalog fences and atomic actor output, receipt, outbox and reconciliation handoff remain intact.

Committed publication also owns any remaining lifecycle cleanup. If the occurrence is restaged or rejected, a selectively queued cleanup job or a subsequent live derivative task resumes the saved post-publication frontier under its own lease. Cleanup preserves the original mutation and affected identities, verifies the still-current publication batch, and revalidates current source/support guards. New actor publication waits for earlier cleanup; stale receipts cannot authorize native output. Fresh rejection without an owed frontier queues no derivative work.

Run snapshots expose separate current-receipt actor and reconciliation readiness counters. Their denominator is admitted eligible occurrences, not all fetched articles. Collection completion, actor readiness, reconciliation readiness and index-outbox delivery are distinct facts. Native actor index receipts bind the publication generation as well as the current input revision. Topology, actor discovery and provenance readers exclude stale native mentions and relationships while deferred cleanup is pending. Curated, pinned and validation-bearing evidence retains its own authority. Topology discovers both current graph projections and retained legacy endpoint indexes. Background scheduling is not evidence of faster total convergence; fixed-cohort timings are still required.

## Review tooling

Bounded exports contain captured observations, source hashes, permanent family/split ownership and explicit sampling bias; ground truth starts unreviewed. Imports require actual reviewer references, complete zero-to-six memberships or abstention, and matching source/revision/partition. Atomic replay is idempotent; conflicts roll back the exchange. Mapping approval derives precision counts from complete reviewed Development families. Missing provider categories remain Unknown. Approval alone neither activates a model nor proves production quality.

## Legacy Natural learning inputs

After terminal-cycle monitoring, an approved mapping and at least 100 newly eligible families can queue a separate `classifier_learn` job. Its bounded immutable capture contains seed and actual reviewed rows plus approved partial provider rows. Source provenance remains distinct; provider examples never become reviewed ground truth, and permanent holdouts cannot enter weak training. The captured fit excludes Test text. Identical input hashes create one root job. The capture binds a compiled pipeline hash (bundle/source manifest and learning, calibration, report and admission policy versions). Captures from an incompatible pipeline are retained with an explicit incompatibility result instead of replaying under changed code.

Fitting uses the shared writer lease and manifest builder, with an independent task lease and asynchronous heartbeats. Foreground work cooperatively cancels fitting and schedules a checkpoint retry; the owned fit handle joins before the writer is released. Successful candidate creation marks its eligible families consumed for the trigger. The worker requires complete reviewed Calibration and independent Development evidence before comparative promotion. It measures one fixed reviewed cohort against the pinned baseline, including warm single-request latency and foreground requests overlapping actual training. Candidate and baseline use the same inputs; insufficient overlap or unavailable measurements retain the candidate. Fixed-code native heap/stack checks use the same configured ceilings as the worker. Review import and approval share the artifact writer through COMMIT, preventing review revisions from changing between capture validation and atomic activation. Quality and resource failures retain the prior model; an uncertain filesystem activation is resolved by durable readback and reported honestly.

Review exports include a bounded diagnostics sidecar replayed with the exact saved run pin, never the currently active replacement model. It records scores, ranking reasons and immutable hashes while leaving reviewer labels blank. Natural replays can also record lexical support, OOV counts and bounded feature spans. Test partitions are excluded from diagnostic replay. Duplicate-review leads use exact URL/content identity first, then at most a small indexed candidate set reranked with pinned Natural Dice/Jaro-Winkler. Cross-partition candidates are flagged for review; lexical similarity cannot merge families or copy labels. Routine logs omit article prose.

Phrase eligibility uses the original normalized grammatical-noise flag, before stemming: “are the” cannot become support through `ar`. Hard line/paragraph boundaries break phrases as punctuation does; unigrams retain their original field spans. Generation-zero lifecycle frontiers query the six current-priority prefixes only, because the protocol creates nonnegative generations. Positive generations retain all twelve current/older probes for lazy reset; source/support/lease fences and actual cleanup traversal are unchanged.

Actor discovery excludes missing/superseded publication stamps before the indexed retrieval cap. Full source/body/catalog validation still follows that bounded read. Its continuation tracks the scanned window even when validation excludes every returned mention, so stale native output cannot falsely exhaust discovery or hide later current occurrences.

Actor dossiers recognize a validated `mentioned_in` article association in both its native-mention and materialized Link projection. Topology merges those receipts into one association; generation filtering and curated ownership checks apply before dossier classification.

A new body acquisition without its current validated cleanup makes the whole occurrence's native actor generation stale, including its metadata spans. Those native associations are hidden until source validation and current native refresh restore them. Independently curated and validation-bearing evidence retains its own authority.
