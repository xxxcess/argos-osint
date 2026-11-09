# Strategic domains taxonomy

[Concept route](index.md) · [Classifier implementation](../code/classifier.md) · [Wiki](../../index.md)

Atlas foreground admission uses this taxonomy through a pinned local Rust-BERT worker and current receipt projections. Historical reclassification and provider monitoring are separate durable jobs; Focus Brief and neighborhood summaries use captured evidence revisions on demand. Implementation contracts and verification results are separate; a trained synthetic model does not establish production quality.

## Canonical IDs and display names

Taxonomy version: `strategic-domains-v1`.

| ID | Display name |
| --- | --- |
| `governance` | Politics & Governance |
| `security` | Security & Conflict |
| `economic` | Economy & Supply Chains |
| `human` | Health & Public Welfare |
| `environmental` | Environment & Resources |
| `scientific` | Science & Technology |

The feature-independent `atlas_taxonomy` module owns the IDs, display names, and domain definitions. Use the display names in article labels and category controls. Compact terminal tabs use Politics, Security, Economy, Health, Environment, and Science. Store and query by ID, and use the full definitions to guide classification and evidence selection. Domains are article attributes and filters. A domain is not a reliability grade, actor threat rating, event probability or proof of a relationship.

## Primary and optional secondary policy

The live engine records all six independent entailment scores. Each domain must clear the configured threshold before Atlas selects a primary and an optional second eligible domain. The legacy Natural engine additionally requires discriminative support. Equal scores are recorded as ambiguous rather than evidence of primary dominance. Exactly two tied eligible domains preserve both; competing second-slot ties retain an unambiguous primary only, and an ambiguous primary among more than two candidates abstains. A strong first domain never manufactures a weak second domain. Synthetic activation does not bypass the gates.

| Input/result | Classifier outcome |
| --- | --- |
| Two qualifying domains | Admit that pair |
| Three or more qualifying domains | Select at most two; record ties and abstain when primary ranking is ambiguous |
| One supported qualifying domain | Admit that primary domain |
| No supported qualifying domain | Reject for insufficient domains |
| Blank, unsupported-language or wholly OOV input (Natural only for OOV) | Explicit corresponding rejection receipt |
| Missing/broken model, queue or storage fault | Operational failure, not an article label |
| Unknown provider membership | Keep the membership unknown in monitoring and any legacy binary fit |

Independent entailment scores need not sum to one. Natural model raw log odds are separate from calibrated scores. Annotation primary/secondary order is editorial metadata, not live ranking authority. `unk` is neither a seventh domain nor a successful six-domain classifier result.

## Legacy history and reclassification

The prior `atlas-taxonomy-v2` has seven category names (geopolitical, economic, military, science, health, stability, technology), plus the rejection sentinel `unk`. Historical records remain readable. Only `economic` overlaps a new domain string; string membership alone therefore cannot establish the taxonomy of a retained row.

Current filters require explicit taxonomy, admission state and receipt authority before ranking or limits. The resumable maintenance job reclassifies retained original metadata without translating old names. It freezes its cohort and model pin, checkpoints bounded activations and completes affected derivative receipts. Rejection preserves already-published historical evidence and original cycle statistics. See [Atlas admission and retained history](atlas-classification-and-related-articles.md) and the [job contract](../code/classifier.md#historical-admission-and-provider-monitoring).

## Synthetic dataset boundary

The exact CSV header set is:

```text
sample_id,title,description,domain_labels,primary_domain,secondary_domain,subcategories,source_kind,difficulty,dataset_version,annotation_notes,split,scenario_group,language,synthetic,labels_complete,expected_admission,rejection_reason
```

Only title and description are model features. The frozen v1 version contains 1,700 synthetic rows in 220 scenario families; variants of one family share one partition and complete membership. Train/calibration/test rows are 1,130/285/285; independent families are 142/39/39. Label annotations, IDs, fictional metadata, split and notes never become features or live Brain evidence.

Rows with no labels are explicit negatives for all six binary models; single-domain examples can be admitted under the current policy; the frozen v1 annotation contract retains its original exact-two expectations. Partial provider labels use a separate masked typed pipeline. A CSV claiming `reviewed_real` is rejected; an unverified text field cannot confer independent review approval.

The seed supports development and explicit **Synthetic baseline** activation. Its correlated row count does not prove production accuracy, and it cannot authorize automatic promotion. See [dataset resolution and validation](../code/classifier.md#legacy-dataset-resolution-and-validation).

## Versioned v2 dataset and policy

The default `argos_atlas_strategic_domains_v2.csv` contains 1,020 distinct scenarios, with Train/Calibration/Development/Test counts 440/230/165/185. Complete memberships allow 0–6 domains. Editorial annotations never replace the complete truth set. The frozen original CSV keeps its v1 exact-two interpretation when explicitly selected. Dataset versions are allowlisted; unknown versions fail.

The taxonomy IDs remain `strategic-domains-v1`; admission is versioned separately as `primary-optional-secondary-v2`, receipts as `classifier-receipt-v2`, and domain context as `domain-context-v2`. Legacy exact-two receipts are explicitly decoded as legacy and cannot authorize current publication. Synthetic baseline provenance remains visible regardless of synthetic evaluation scores.
