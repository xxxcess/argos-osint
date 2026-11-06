# Classifier contract fixtures

` six-model.json ` is a reviewable input recipe, not a hand-authored model.
The test adapter trains actual Natural Bayes in QuickJS, serializes its real
payload, and creates isolated temporary artifacts. Actual positive/negative
support is `domains.<id>.class_totals.<label> - 1`; upstream adds one smoothing
pseudo-example per label. Feature indices follow the persisted per-model
`feature_order` array. Raw emitted artifacts do not retain training example IDs;
contradictory family labels are checked at the normalized corpus boundary.

`bayes-parity.json` is generated offline by pinned Natural 8.2.6 / apparatus
0.0.10 using upstream `probabilityOfClass`, followed by `Math.log`. It records
exact tokens, feature order, binary query vectors and six independent scores,
with a fixed absolute tolerance of 1e-9. Source manifest and asset hashes bind
it to the compiled implementation. It does not copy the adapted scoring math.

To review an intentional update, first review the source/recipe diff, then run:

```sh
node crates/argos-osint-core/tests/fixtures/classifier/generate-parity.mjs --write
node crates/argos-osint-core/tests/fixtures/classifier/generate-parity.mjs --check
node scripts/build_classifier_bundle.mjs --check
node scripts/test_classifier_bundle.mjs
```

Ordinary Rust tests never invoke Node, package installation, providers, or the
customer CSV. `corrupt-models.json` contains independent mutation recipes applied
to real emitted payloads; tests repair payload hashes so structural failures
cannot be hidden by a checksum failure. Runtime tests serialize worker creation
within each binary and keep homes alive until worker teardown. Feature-off tests
exercise the callable unavailable facade and ID validation explicitly.

These toy fixtures do not establish production precision or performance.
Receipt tests cover occurrence and input identity; durable run/revision/lease
commits remain Atlas integration contracts. Inline engine queue/resource and
artifact bounded-reader/fault-injection proofs are separate from these public
integration tests and must be reported individually.
