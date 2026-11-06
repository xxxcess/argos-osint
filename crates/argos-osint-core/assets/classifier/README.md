# Pinned Natural Classifier Assets

Natural 8.2.6 (gitHead `e5bb63d6af741209e71691ae8702e91664d61c29`) and Apparatus 0.0.10 (gitHead `861cf4cbb0775e2fd12ffaf30d01b7d8f9497e12`), vendored into `upstream/` and bundled for QuickJS execution without Node.js dependencies.

| Local asset | Upstream source | Purpose |
| --- | --- | --- |
| `natural-classifier.js` | `natural@8.2.6` + `apparatus@0.0.10` closure | Committed IIFE QuickJS bundle defining `globalThis.argosNaturalBayes` |
| `source-manifest.json` | Pinned package registry info & file hashes | Build and adaptation provenance |
| `natural-bayes.patch` | Unified diff of adapted sources | Human-readable audit of all modifications |
| `natural-bayes.adapt.js` | Transform rules | Deterministic adaptations applied during build |
| `shims/` | Compatibility shims | Minimal shims for `events`, `util`, `fs`, `os`, `webworker-threads` |
| `upstream/` | Verified npm tarball files | Byte-exact upstream sources for auditing |

## Adaptations
- Removed Node.js `fs` module and filesystem persistence methods (`save`, `load`, `saveTo`, `loadFrom`, `removeDocument`).
- Replaced Node `events` with minimal bounded, count-only progress emitter shim.
- Replaced Node `util.inherits` with portable `Object.create` shim.
- Excluded parallel training module (`classifier_train_parallel.js`), `os`, and `webworker-threads`.
- Replaced object key iteration with monotonically append-only `featureOrder` array to ensure deterministic incremental training.
- Implemented log-space scoring (`logScore`) matching Apparatus `probabilityOfClass` without absent-token Bernoulli factor.
- Used `Object.setPrototypeOf` instead of deprecated `__proto__`.

The upstream MIT license is retained in `LICENSE`. Runtime checks bundle and bridge checksums before initialization. These assets are embedded only by the optional `classifier` feature.

## Phrase and vocabulary policy

The shared `classifier/features.js` policy `natural-en-phrases-v2` tokenizes title and description separately with pinned Natural RegexpTokenizer. It protects identifiers/acronyms, preserves UTF-8 byte spans and negation, and emits unambiguous binary unigram and adjacent-bigram IDs. Punctuation and field boundaries break phrases. Noise filtering follows phrase formation.

`classifier/vocabulary.js` prepares Train representatives, freezes document frequency/IDF and discriminative vocabulary selection, and reuses their prepared features during fitting. Natural TF-IDF is diagnostic/selection data; Bayes receives binary presence. Held-out inference cannot update these statistics. The bundle includes Natural NGrams, RegexpTokenizer and TF-IDF plus pinned underscore 1.13.8 and its license. TF-IDF filesystem methods are removed. Sparse scoring visits active indices in the same summation order as dense upstream scoring. Sparse binary fitting updates the same Apparatus counts and prior pseudo-examples without scanning a dense vector. Full independent feature-binding audits run at batch boundaries, serialization and restore, with constant-time shape checks between rows. Upstream parameter/count/score parity covers repeated sparse fitting rounds.

## Wire protocol and support

All five bridge methods accept and return JSON strings. Rust passes data through
function arguments, never interpolated JavaScript. Requests carry `version`,
`request_id` and `model_id`. Failures carry top-level `error` and `code`, plus diagnostic strings in `errors[]`.

- `trainBatch` preparation phases `vocabulary` and `freeze_vocabulary` carry bounded Train rows plus `feature_input_hash`, and return separately validated preparation/freeze telemetry. The fitting phase remains:
- `trainBatch`: `rows[].{row_id,title,description,membership}` where membership
  contains six Present/Absent/Unknown entries; Unknown contributes no example to
  that binary model. Response: `accepted`, `skipped`, `token_bytes`,
  `vocabulary_new`, `progress`, `round`, `vocabulary_sizes[]`, `fitted_memberships[]`, `feature_order_revisions[]`. Fitted counts describe actual completed examples, not queued documents.
- `finishTraining`: `max_documents_per_step` fits at most16 documents TOTAL across all six classifiers (allowed1–16). Response:
  `done`, `frozen_order_length` (sum of six vocabularies), `fit_step`, `round`, `fitted_memberships[6]`, `vocabulary_sizes[6]`, `feature_order_revisions[6]`, `errors[]`. Pending rounds exclude further collection, inference and serialization; completed finish is a no-op.
- `classifyBatch`: `items[].{id,title,description,language}`. Response:
  `items[].{id,results[]}`; each result has `domain`, `present_log_score`,
  `absent_log_score`, `raw_log_odds`, `known_feature_count`, `oov_count`, and bounded discriminative-support diagnostics. Explicit `explain` requests add strongest positive/negative contributions and original spans.
- `serializeModel`: response is directly the data-only model envelope with
  `feature_schema_version:"classifier-features-v3"` a frozen `feature_policy`, and `domains.<id>.{feature_order,feature_index,class_features,class_totals,total_examples,smoothing}`.
- `restoreModel`: request `artifact` contains that envelope (object or JSON
  string). Response `restored_model_id` identifies the restored model.

Apparatus `class_totals.present` and `.absent` include one prior pseudo-example
per class; manifest class counts record actual fitted rows and must equal the
respective totals minus one. Scoring retains upstream binary feature presence,
priors and smoothing. The present minus absent log score is a raw signal;
Rust calibrates each domain independently before selecting one primary and an optional secondary.

Feature order and independent `{token,index}` bindings are appended during real training and persisted per binary model. Missing bindings, mismatched order, missing order,
duplicates or indices outside that order fail with `feature_order_mismatch`.
Runtime checks bundle, adapter and bridge hashes against the embedded source
manifest before evaluation; artifacts must match the entire compiled identity.

`node scripts/build_classifier_bundle.mjs` regenerates the bundle, adaptation
patch and source manifest offline from vendored source. Run the same command
with `--check` to verify them. `node scripts/test_classifier_bundle.mjs` compares
against pinned upstream Natural; `--json` or `--emit-oracle` emits stable JSON
containing corpus, fixture scores, tolerance and measured deltas for review.
Standard Rust tests do not launch Node.

The adaptations remove filesystem and worker operations from Natural's base
classifier (`classifier.js`), preserve explicit feature order and bounded
fitting, and expose Apparatus Bayes scores in log space
(`apparatus/classifier/bayes_classifier.js`). The committed patch records exact
upstream locations. Licenses, upstream revisions and source hashes are retained
alongside the bundle in `source-manifest.json` and `upstream/`.

Mutation request IDs share FamilyId bounds: nonempty UTF-8, at most 256 bytes,
without control characters or NUL. Each worker retains at most 32,768 mutation
identities with 64MiB of exact serialized identity/digest/response bytes, without
eviction. It reserves identity bytes plus a 1024-byte closed telemetry reply
before mutation; malformed or oversized replies discard the candidate. Identical
replays return the exact cached response and consume no new entry. The worst
50,000-row all-six-label corpus with full 32-row envelopes needs 20,313 train/finish identities. Adaptive splitting by the actual escaped JSON envelope and per-round fitting can add calls; the general bound including vocabulary preparation/freezing from the 64MiB corpus limit and closed DTO frames remains below 26,000 identities. Host map and
string allocation overhead is additional to these serialized bytes and the
QuickJS heap/stack reservation; this is not a physical RSS cap.

A single owned startup restore accepts at most 64MiB before the worker opens its
ordinary 32-message/16MiB request queue. This distinct startup transfer still uses
the 128MiB QuickJS heap and 1MiB stack limits; file acceptance does not guarantee
that a model fits in that heap. A fresh engine may replace its unused runtime for
restore only after joining it. Hot restore is rejected and keeps the live model.
Completed training uses a terminal restore port: close/join the candidate worker,
restore/smoke in the released runtime slot, close that verifier, and retain the
writer lease. Success, failure or cancellation cannot resume candidate mutation.
Forwarding worker wrappers must delegate this port rather than rebuild a large
ordinary queue request. Existing two inference pins plus candidate/verifier use
at most three runtime reservations throughout the handoff.

Training packs at most32 supervised rows per call using the exact serialized
2MiB envelope size, including operation IDs and preprocessing. Escaped text is
never truncated or dropped; a row that would overflow an existing batch starts
the next batch. Oversized individual rows fail with their row identifier before
worker dispatch. Unsupervised rows update seen counts without empty wire batches.
The compiled algorithm fixes additive smoothing at1.0; persisted feature weights
must be that smoothing plus an integral binary occurrence count.

Explicit manual repair reads only strict bounded persisted identity and committed
history, independently of executable compatibility. The capability requires
synthetic opt-in, the exact writer acquisition, prior model/generation/immutable
checksum and a fully validated current candidate. Repair increments the existing
generation and preserves prior files; automatic promotion cannot use this path.
Missing active pointers with prior history and occupied next history generations
require explicit recovery. Activation records install without clobbering prior
records; mutable per-model history indices remain replaceable summaries.
