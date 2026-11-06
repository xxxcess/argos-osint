# Strategic domains taxonomy

[Concept route](index.md) · [Classifier implementation](../code/classifier.md) · [Wiki](../../index.md)

Atlas foreground admission uses this taxonomy through a pinned local classifier and current receipt projections. Historical reclassification and provider monitoring are separate durable jobs; Focus Brief and neighborhood summaries use captured evidence revisions on demand. Implementation contracts and verification results are separate; a trained synthetic model does not establish production quality.

## Official IDs and names

Taxonomy version: `strategic-domains-v1`.

| ID | Official name |
| --- | --- |
| `governance` | Governance & Political Stability |
| `security` | Security & Threat Environment |
| `economic` | Economic & Supply Chain Resilience |
| `human` | Human Security & Public Welfare |
| `environmental` | Environmental & Resource Security |
| `scientific` | Scientific & Technological Capability |

The feature-independent `atlas_taxonomy` module owns these exact IDs. Domains are attributes and filters, not six permanent Brain nodes. A domain is not a reliability grade, actor threat rating, event probability or proof of a relationship.

## Exact-two classifier policy

The engine records all six independent scores. If at least two domains pass their own calibrated threshold and informative-feature checks, it publishes exactly the top two, with stable domain-ID tie breaks. A strong first domain never manufactures a weak second domain. Synthetic activation does not bypass the gates.

| Input/result | Classifier outcome |
| --- | --- |
| Two qualifying domains | Admit that pair |
| Three or more qualifying domains | Admit exactly the top two |
| Fewer than two supported qualifying domains | Reject for insufficient domains |
| Blank, unsupported-language or wholly OOV input | Explicit corresponding rejection receipt |
| Missing/broken model, queue or storage fault | Operational failure, not an article label |
| Unknown provider membership | Omit that example from the affected binary model's fit |

Raw log odds are not probabilities and scores do not sum to one. Annotation primary/secondary order is editorial metadata, not live ranking authority. `unk` is neither a seventh domain nor a successful six-domain classifier result.

## Legacy history and reclassification

The prior `atlas-taxonomy-v2` has seven category names (geopolitical, economic, military, science, health, stability, technology), plus the rejection sentinel `unk`. Historical records remain readable. Only `economic` overlaps a new domain string; string membership alone therefore cannot establish the taxonomy of a retained row.

Current filters require explicit taxonomy, admission state and receipt authority before ranking or limits. The resumable maintenance job reclassifies retained original metadata without translating old names. It freezes its cohort and model pin, checkpoints bounded activations and completes affected derivative receipts. Rejection preserves already-published historical evidence and original cycle statistics. See [Atlas admission and retained history](atlas-classification-and-related-articles.md) and the [job contract](../code/classifier.md#historical-admission-and-provider-monitoring).

## Synthetic dataset boundary

The exact CSV header set is:

```text
sample_id,title,description,domain_labels,primary_domain,secondary_domain,subcategories,source_kind,difficulty,dataset_version,annotation_notes,split,scenario_group,language,synthetic,labels_complete,expected_admission,rejection_reason
```

Only title and description are model features. The delivered version contains 1,700 synthetic rows in 220 scenario families; variants of one family share one partition and complete membership. Train/calibration/test rows are 1,130/285/285; independent families are 142/39/39. Label annotations, IDs, fictional metadata, split and notes never become features or live Brain evidence.

Rows with no labels are explicit negatives for all six binary models; single-domain examples remain valid training material despite failing live exact-two admission. Partial provider labels use a separate masked typed pipeline. A CSV claiming `reviewed_real` is rejected; an unverified text field cannot confer independent review approval.

The seed supports development and explicit **Synthetic baseline** activation. Its correlated row count does not prove production accuracy, and it cannot authorize automatic promotion. See [dataset resolution and validation](../code/classifier.md#dataset-resolution-and-validation).
