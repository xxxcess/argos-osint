# Fleetworks classifier v2 distinct-scenario dataset

`argos_atlas_strategic_domains_v2.csv` is the labeled synthetic dataset for the optional Natural classifier. Select it through native Workstation readiness and explicitly confirm durable candidate-only Natural training; activation is a separate consented operation. The original CSV remains a frozen v1 baseline. The dataset version is `strategic-domains-v2-distinct-1`. [Training setup](../deploy/README.md#optional-maintenance-and-inference)

## What changed

There are **1,020 separately authored synthetic article scenarios**, each with its own title, description and story family. There are no location substitutions, prefix variations or multiple editorial versions of one scenario. The earlier 1,700-row file had only 220 families. This file deliberately prioritizes scenario diversity over row count.

| Group | Train | Calibration | Development | Test | Total |
|---|---:|---:|---:|---:|---:|
| Single-domain | 300 | 180 | 120 | 120 | 720 |
| Two-domain | 90 | 30 | 30 | 30 | 180 |
| Out-of-scope/hard negative | 50 | 20 | 15 | 15 | 100 |
| Three-domain challenge | 0 | 0 | 0 | 20 | 20 |
| Total | 440 | 230 | 165 | 185 | 1,020 |

Each domain has **120 focused articles**, 60 two-domain memberships and 10 three-domain memberships: **190 positive scenarios per domain**. Positive split counts are 80 training, 40 calibration, 30 development and 40 test per domain. All 15 pairs have 12 stories; every possible three-domain combination has one test story. The 20 three-domain stories are a held-out challenge, not a basis for tuning a new feature after test inspection.

Forty positive calibration families make the existing 20-accepted-family requirement possible; they do not guarantee that a model will accept enough at the desired precision. The same applies to the 30 positive development families. Never duplicate rows or relax evaluation solely to pass a gate.

## Provenance and annotation limits

Every row is fictional synthetic text authored for this deliverable. No source publisher, real event, date, independent human review or empirical independence is claimed. The family IDs denote distinct authored scenarios, not verified real-world event identities. Common event mechanisms and topical vocabulary inevitably recur. Single-domain judgments intentionally cover explicitly reported content rather than all imaginable downstream implications; category boundaries still need review against real articles.

The corpus covers oversight/diplomacy/elections; cyber, crime and armed threats; logistics/finance/production; care/displacement/basic services; water/pollution/hazards/ecosystems; and research/technical capability. It includes denials, unconfirmed attribution, replication failures, fictional/metaphorical headlines and source limitations. Its style, class balance and English-only coverage are designed and do not represent live provider distributions. No geopolitical country names were used as class shortcuts. No classifier was trained, no production quality was established, and more authored rows are not a substitute for real reviewed data.

## Important compatibility change

Current nightly's CSV parser rejects Development rows and more than two complete labels. It also expects single-domain rows to be rejected. **Implement the new spec before importing this file.** Do not delete Development rows, move them into training or truncate three-domain labels to satisfy the old parser.

This dataset adopts the recommended policy: **one supported primary domain, optional second, maximum two published tags**. Single-domain rows therefore say `admit`. Negative rows say `reject`. Full ground truth may contain three or more domains even though at most two publish. If retaining exact-two behavior for a baseline experiment, derive its expected admission in the evaluator and identify the policy explicitly; do not overwrite the CSV's annotations.

## CSV schema

The UTF-8 CSV retains the original 18 field names. Quoted commas and apostrophes are ordinary text. A pipe separates memberships inside a cell.

| Field | Interpretation |
|---|---|
| sample_id | Stable unique row ID, e.g. `D2-0001` |
| title, description | The only article content supplied as classifier input |
| domain_labels | Complete set of supported domains; empty means out of scope |
| primary_domain, secondary_domain | Editorial ordering suggestions; for ≥3 labels these are only two members of the full set |
| subcategories | Empty, reserved for optional context |
| source_kind | Always `synthetic` |
| difficulty | `single_domain_boundary`, `cross_domain`, `multi_domain`, `out_of_scope`, `hard_negative` |
| dataset_version | Always `strategic-domains-v2-distinct-1` |
| annotation_notes | Scope/rationale; never a feature |
| split | `train`, `calibration`, `development`, `test` |
| scenario_group | Stable story ID derived from title/description; one row per family |
| language | `en` |
| synthetic, labels_complete | Boolean text `true` |
| expected_admission | `admit` for supported nonempty membership; otherwise `reject` |
| rejection_reason | Empty for admitted rows; `out_of_scope` for negatives |

Stable domains:

- `governance`: Politics & Governance.
- `security`: Security & Conflict.
- `economic`: Economy & Supply Chains.
- `human`: Health & Public Welfare.
- `environmental`: Environment & Resources.
- `scientific`: Science & Technology.

Membership is separate from truth or stance: a report denying an armed strike remains Security, but the classifier does not establish that the strike occurred. Fictional game/film metaphors are negatives because they report no actual strategic development. For test triads, any accepted subset of the ground-truth set is label-valid; assess omissions and primary ranking separately.

## Use and evaluation

Train six binary membership models on Train only: positive for included labels, negative for every other domain in this fully annotated seed. Single-domain examples are essential. Fit vocabulary, IDF, phrase selection and discriminative statistics only on Train. Fit score transforms/thresholds on Calibration. Use Development for model/policy selection and promotion assessment with correct synthetic provenance. Open Test only after freezing the selected pipeline; never claim repeatedly used test scores are untouched.

Keep source_kind, split, group, notes, labels, expected admission and row IDs out of model inputs. Never use provider absence as negative evidence when importing external observations. Approved weak labels remain partial; reviewed-real evaluation needs actual review references and complete labels.

To establish real-world quality, review a representative sample of admitted and rejected provider articles, preserving uncertainty and sampling bias. Group syndicated stories and later revisions by event before splitting. Include time-separated and publisher-diverse holdouts. The current corpus is not a production approval certificate.

Validation completed: unique IDs/titles/descriptions/families; no family partition leakage; nonempty metadata; six valid labels; complete pair/triad coverage; CSV roundtrip; split/count consistency. A content-token Jaccard screen found no pair at or above 0.60 (highest 0.3333). This lexical test cannot prove absence of semantic similarity. The validation JSON records exact counts and SHA-256 `625f10150ba413f2fcae49ef425f9829004daaf9e69e49eecf66f7598de80d93`.
