//! Explicit offline ablations use one corpus and never score Test text.
#![cfg(feature = "classifier")]
use argos_osint_core::classifier::calibration::{self, CalibrationPolicy, GateSet, ScoredFamily};
use argos_osint_core::classifier::training::{FamilyId, LabelMembership, Split};
use argos_osint_core::classifier::{artifact, ClassifyInput, ModelId, Provenance};
use serde_json::{json, Value};

fn selected_summary(rows: &[ScoredFamily], gates: &GateSet, legacy: bool) -> Value {
    let (mut admitted, mut invalid, mut false_oos, mut retained_single, mut ties) = (0, 0, 0, 0, 0);
    let (mut selected_memberships, mut correct_memberships, mut truth_memberships) = (0, 0, 0);
    for row in rows {
        let truth: Vec<_> = row
            .membership
            .iter()
            .enumerate()
            .filter(|(_, membership)| **membership == LabelMembership::Present)
            .map(|(index, _)| index)
            .collect();
        truth_memberships += truth.len();
        let selected: Vec<usize> = if legacy {
            // Frozen v1 policy: rank all scores lexically on ties, then require
            // both top slots to pass. The same frozen gates isolate policy effect.
            let mut ranked: Vec<_> = gates
                .domains
                .iter()
                .enumerate()
                .map(|(index, gate)| {
                    (
                        index,
                        calibration::score(row.raw[index].raw_log_odds, gate).unwrap(),
                    )
                })
                .collect();
            ranked.sort_by(|a, b| {
                b.1.total_cmp(&a.1).then_with(|| {
                    gates.domains[a.0]
                        .domain
                        .as_str()
                        .cmp(gates.domains[b.0].domain.as_str())
                })
            });
            if ranked[..2].iter().all(|(index, relevance)| {
                *relevance >= gates.domains[*index].threshold
                    && row.raw[*index].known_feature_count
                        >= gates.domains[*index].min_informative_features
            }) {
                ranked[..2].iter().map(|(index, _)| *index).collect()
            } else {
                Vec::new()
            }
        } else {
            let receipt = calibration::apply_gates(
                &ClassifyInput::new(
                    &row.representative_id,
                    "Offline cohort",
                    "Frozen scores",
                    Some("en".into()),
                ),
                &row.raw,
                &ModelId::parse("ablation").unwrap(),
                gates,
            )
            .unwrap();
            ties += usize::from(receipt.primary_rank_ambiguous);
            receipt
                .selected_domains
                .iter()
                .map(|domain| {
                    gates
                        .domains
                        .iter()
                        .position(|gate| &gate.domain == domain)
                        .unwrap()
                })
                .collect()
        };
        if !selected.is_empty() {
            admitted += 1;
            false_oos += usize::from(truth.is_empty());
            invalid += usize::from(selected.iter().any(|index| !truth.contains(index)));
            retained_single += usize::from(truth.len() == 1 && selected == truth);
        }
        selected_memberships += selected.len();
        correct_memberships += selected
            .iter()
            .filter(|index| truth.contains(index))
            .count();
    }
    json!({"families":rows.len(),"admitted":admitted,"coverage":admitted as f64/rows.len() as f64,"invalid_selected_sets":invalid,"false_oos_admissions":false_oos,"single_domain_retained":retained_single,"primary_rank_ambiguous":ties,"selected_memberships":selected_memberships,"correct_selected_memberships":correct_memberships,"truth_memberships":truth_memberships})
}

#[test]
#[ignore = "explicit score extraction and independent Development ablation; Test excluded"]
fn frozen_models_and_policies_share_the_identical_development_cohort() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let directory = root.join(".agent-scratch/classifier-quality-v2/ablations");
    let manifest: Value = serde_json::from_slice(
        &std::fs::read(directory.join("manifest.json"))
            .expect("run scripts/classifier_quality_ablation.mjs first"),
    )
    .unwrap();
    let dataset_hash = artifact::sha256_hex(
        &std::fs::read(root.join("datasets/argos_atlas_strategic_domains_v2.csv")).unwrap(),
    );
    assert_eq!(manifest["dataset_sha256"], dataset_hash);
    assert_eq!(manifest["test_opened"], false);
    let mut reports = Vec::new();
    let mut common_ids = None;
    for variant in manifest["variants"].as_array().unwrap() {
        let bytes = std::fs::read(directory.join(variant["file"].as_str().unwrap())).unwrap();
        assert_eq!(artifact::sha256_hex(&bytes), variant["sha256"]);
        let scored: Value = serde_json::from_slice(&bytes).unwrap();
        let rows: Vec<ScoredFamily> = scored["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| ScoredFamily {
                family_id: FamilyId::parse(row["family_id"].as_str().unwrap()).unwrap(),
                representative_id: row["representative_id"].as_str().unwrap().into(),
                split: serde_json::from_value(row["split"].clone()).unwrap(),
                provenance: Provenance::synthetic_baseline(
                    "strategic-domains-v2-distinct",
                    &dataset_hash,
                ),
                membership: serde_json::from_value(row["membership"].clone()).unwrap(),
                raw: serde_json::from_value(row["raw"].clone()).unwrap(),
            })
            .collect();
        assert_eq!(rows.len(), 395);
        let ids: Vec<_> = rows
            .iter()
            .map(|row| (row.family_id.clone(), row.split))
            .collect();
        if let Some(common) = &common_ids {
            assert_eq!(common, &ids)
        } else {
            common_ids = Some(ids)
        }
        let calibration: Vec<_> = rows
            .iter()
            .filter(|row| row.split == Split::Calibration)
            .cloned()
            .collect();
        let development: Vec<_> = rows
            .iter()
            .filter(|row| row.split == Split::Development)
            .cloned()
            .collect();
        assert_eq!(calibration.len(), 230);
        assert_eq!(development.len(), 165);
        let isotonic = calibration::fit(&calibration, &CalibrationPolicy).unwrap();
        let sigmoid = calibration::fit_sigmoid(&calibration);
        reports.push(json!({"variant":variant,"isotonic":{"development_brier":calibration::brier_score(&development,&isotonic.gates).unwrap(),"development":calibration::evaluate(&development,&isotonic.gates).unwrap(),"legacy_exact_two_policy":selected_summary(&development,&isotonic.gates,true),"primary_optional_secondary_policy":selected_summary(&development,&isotonic.gates,false)},"sigmoid":match sigmoid {Ok(fit)=>json!({"development_brier":calibration::brier_score(&development,&fit.gates).unwrap(),"development":calibration::evaluate(&development,&fit.gates).unwrap()}),Err(error)=>json!({"failure":error.to_string()})}}));
    }
    let report = json!({"schema_version":"classifier-ablation-evaluation-v2","dataset_sha256":dataset_hash,"test_opened":false,"selection_note":"Diagnostic Development comparisons; no variant is promoted or selected using Test","variants":reports});
    std::fs::write(
        directory.join("evaluation.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    println!("ARGOS_ABLATION_RESULT {}", report);
}
