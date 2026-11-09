# Argos strategic domains — synthetic seed dataset

Place `argos_atlas_strategic_domains.csv` in `<repo>/datasets/`. Version: `strategic-domains-v1-seed-1`. Use with revision 6 of the Atlas cycle performance implementation spec. Do not mix with the old seven-topic dataset. Every statement is fictional synthetic training material, not sourced reporting; never ingest this CSV into live Atlas/Brain evidence.

## Composition

| Group | Train | Calibration | Test | Total |
|---|---:|---:|---:|---:|
| Single-domain boundaries | 840 | 180 | 180 | 1,200 |
| Two-domain overlaps | 150 | 75 | 75 | 300 |
| Out-of-scope/hard negatives | 140 | 30 | 30 | 200 |
| Total | 1,130 | 285 | 285 | 1,700 |

Each domain has 200 focused rows plus 100 overlap memberships, including 190 positive training rows. Each of 15 unordered pairs has 20 rows. There are 220 independent scenario *families*, not 1,700 independent stories: 20 focused families/domain with 10 geography/editorial variants each; four families/pair with five variants each; 40 negative families with five variants each. Split all variants together. Fictional places are balanced across classes. No real publisher, author, source URL or human review is asserted.

## Label interpretation

| ID | Scope |
|---|---|
| governance | Politics & Governance: institutions, elections, authority, diplomacy |
| security | Security & Conflict: armed threats, crime, cyber incidents, protective actions |
| economic | Economy & Supply Chains: finance, trade, production, logistics |
| human | Health & Public Welfare: health, displacement, food access, essential services |
| environmental | Environment & Resources: hazards, pollution, water, ecosystems |
| scientific | Science & Technology: research, technical performance, infrastructure capability |

Use explicit article evidence; broad conceivable consequences do not justify another label. For example, an administrative election procedure is governance-only; threats against election offices support governance + security. Laboratory battery performance is scientific-only; input shortages blocking pilot production also support economic. Negation/denial preserves the topic but does not establish the disputed event as true.

Domain annotations are original synthetic author judgments, not independent human-verified gold. Closely related concepts (e.g. health research, environmental measurement) merit review. Do not use absent commentary about a domain as a universal proof of irrelevance outside this fully annotated seed corpus.

## CSV schema

UTF-8, RFC-style quoted CSV; pipe separates labels inside `domain_labels`.

| Field | Type and example |
|---|---|
| sample_id | Unique string, `SID2-1201` |
| title / description | Required synthetic article text; sole inference features |
| domain_labels | Complete unordered membership set, `governance|security`; empty means no relevant domain |
| primary_domain / secondary_domain | Ordered editorial annotations; blank when absent. Rank is a development annotation, not independent truth |
| subcategories | Optional string; empty in this release |
| source_kind | `synthetic` for every delivered row |
| difficulty | `single_domain_boundary`, `cross_domain`, `out_of_scope`, `hard_negative` |
| dataset_version | `strategic-domains-v1-seed-1` |
| annotation_notes | Rationale/admission explanation; never a model feature |
| split | `train`, `calibration`, `test` |
| scenario_group | Stable family ID, e.g. `governance-security-00` |
| language | `en` |
| synthetic / labels_complete | Boolean text `true` |
| expected_admission | `admit` for exactly two annotations, otherwise `reject` |
| rejection_reason | Empty, `insufficient_domains`, or `out_of_scope` |

## Training and evaluation

Fit six independent binary membership models using only train rows. Treat the complete annotated set as positives and its complement as negatives. Empty-label rows are negatives for all domains, not an `unk` model class. Single-domain rows teach topic membership and refusal to invent a second label. They must not be discarded during training. Score all six models; gate independently and publish only the top two when both qualify. More than two domain scores may be high, but admission stores two tags.

Use calibration only for threshold/hyperparameter selection and test only for final evaluation. Variant-level scores overstate independence: report family-weighted metrics and family-level confidence intervals; weight families equally or select a reproducible representative per family when fitting. Keep the same policy for model comparisons. Do not learn from ID, group, notes, split, labels or expected-admission fields. Primary/secondary ordering should not be used to enforce a fixed domain priority; calibrate membership and evaluate unordered selected-pair correctness separately from optional editorial ranking.

The held-out pairs include only one family per pair per partition. This seed is useful for smoke tests, bootstrapping and pipeline development, not a production-quality accuracy claim. It has an artificial class/admission balance and correlated lexical patterns. Collect independent, fully reviewed real articles, including denials, genuine single-domain news, ≥3-domain overlaps, sparse descriptions, regional styles and hard negatives. Add real-data splits by event/time/publisher before automatic model promotion. No model was trained or benchmarked in preparing these deliverables.

Provider data remains a separate corpus with raw NewsData/Currents labels, provenance, masked unknown memberships and a mapping version. Never label an excluded provider category automatically as out-of-scope. Never let the active model's own predictions become ground truth. Never lower the second-domain gate solely on provider agreement.

Validation: 1,700 unique IDs/titles, nonempty metadata, valid labels, roundtrip CSV equality, all 15 pairs present and no scenario-family partition leakage. SHA-256 and exact counts are in `argos_atlas_strategic_domains.validation.json`.
