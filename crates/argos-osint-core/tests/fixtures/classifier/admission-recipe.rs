//! Offline fixtures for legacy Natural training and live zero-shot admission.
#[path = "support.rs"]
mod recipes;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use argos_osint_core::atlas_taxonomy::DomainId;
use argos_osint_core::classifier::artifact;
use argos_osint_core::classifier::calibration::{
    self, ActivationAuthorizationDecision, CalibrationPolicy, ScoredFamily,
};
use argos_osint_core::classifier::readiness::ClassifierHome;
use argos_osint_core::classifier::training::{
    FamilyId, LabelMembership, Split, SyntheticActivation,
};
use argos_osint_core::classifier::{Provenance, RawDomainScore};

pub struct Installed {
    pub directory: tempfile::TempDir,
    pub home: Arc<ClassifierHome>,
}
pub async fn serial() -> tokio::sync::MutexGuard<'static, ()> {
    static WORKERS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    WORKERS.lock().await
}
/// Uses the same real learner and gates with caller-owned fictional vocabulary.
pub fn install_with_titles(titles: [&str; 3]) -> Installed {
    install_with_disabled_domains(titles, &[])
}

/// Storage boundary fixture with only the primary score over the live gate.
#[allow(dead_code)] // Shared fixture module; used only by the single-domain contract target.
pub fn install_primary_only() -> Installed {
    install_with_disabled_domains(
        [
            "Ballot patrol common",
            "Freight clinic common",
            "River sensor common",
        ],
        &[1, 3, 5],
    )
}

fn install_with_disabled_domains(titles: [&str; 3], disabled: &[usize]) -> Installed {
    let mut fixture = recipes::Fixture::new("admission-fixture");
    let pairs = [[0, 1], [2, 3], [4, 5]];
    let membership = |pair: usize| -> [LabelMembership; 6] {
        core::array::from_fn(|domain| {
            if pairs[pair].contains(&domain) {
                LabelMembership::Present
            } else {
                LabelMembership::Absent
            }
        })
    };
    let rows: Vec<_> = titles
        .iter()
        .enumerate()
        .map(|(index, title)| {
            serde_json::json!({
                "row_id": format!("fit-{index}"),
                "title": title,
                "description": "Fictional independent training example",
                "membership": membership(index),
            })
        })
        .collect();
    fixture.fit(serde_json::json!(rows));
    let provenance = Provenance::synthetic_baseline("fixture-v1", "0".repeat(64));
    let scored: Vec<_> = titles
        .iter()
        .enumerate()
        .map(|(index, title)| {
            let raw: [RawDomainScore; 6] = serde_json::from_value(fixture.classify(title)).unwrap();
            ScoredFamily {
                family_id: FamilyId::parse(&format!("calibration-{index}")).unwrap(),
                representative_id: format!("heldout-{index}"),
                split: Split::Calibration,
                provenance: provenance.clone(),
                membership: membership(index),
                raw,
            }
        })
        .collect();
    let mut fit = calibration::fit(&scored, &CalibrationPolicy).unwrap();
    for index in disabled {
        fit.gates.domains[*index].threshold = calibration::DISABLED_THRESHOLD;
    }
    fit.metrics = calibration::evaluate(&scored, &fit.gates).unwrap();
    let mut candidate = recipes::Candidate::from_fixture(fixture);
    candidate.report.gate_set = fit.gates;
    candidate.report.evaluation = fit.metrics.clone();
    candidate.report.domain_decisions = fit.metrics.domains;
    let counts: Vec<_> = DomainId::ALL
        .iter()
        .enumerate()
        .map(|(index, domain)| {
            serde_json::json!({
                "domain": domain,
                "accepted_families": candidate.report.evaluation.domains[index].accepted_families,
            })
        })
        .collect();
    candidate.report.family_counts = serde_json::from_value(serde_json::json!(counts)).unwrap();
    let thresholds: Vec<_> = candidate
        .report
        .gate_set
        .domains
        .iter()
        .map(|gate| serde_json::json!({"domain": gate.domain, "threshold": gate.threshold}))
        .collect();
    candidate.manifest.thresholds = serde_json::from_value(serde_json::json!(thresholds)).unwrap();
    candidate.manifest.family_counts = candidate.report.family_counts.to_vec();
    candidate.manifest.metrics = serde_json::to_value(&candidate.report).unwrap();
    let report_bytes = format!(
        "{}\n",
        serde_json::to_string_pretty(&candidate.report).unwrap()
    );
    candidate.manifest.report_sha256 = artifact::sha256_hex(report_bytes.as_bytes());
    let validated = candidate.write();
    let capability = match calibration::authorize_activation(
        &candidate.fixture.lease,
        &validated,
        &candidate.report,
        &candidate.request(SyntheticActivation::ExplicitOptIn),
        None,
        None,
    )
    .unwrap()
    {
        ActivationAuthorizationDecision::Authorized(capability) => capability,
        other => panic!("explicit synthetic fixture activation denied: {other:?}"),
    };
    candidate
        .store
        .activate(
            &candidate.fixture.model,
            &capability,
            &candidate.fixture.lease,
        )
        .unwrap();
    drop(candidate.fixture.worker);
    drop(candidate.fixture.lease);
    let directory = candidate.fixture.home;
    install_zero_shot(directory.path(), titles, disabled);
    let home = Arc::new(ClassifierHome::new(directory.path().to_path_buf()));
    Installed { directory, home }
}

/// Model files for the offline stdio test worker, using the production home layout.
fn install_zero_shot(home: &Path, titles: [&str; 3], disabled: &[usize]) {
    let classifier = home.join("classifier");
    let model_dir = classifier.join("fixture-bert");
    std::fs::create_dir_all(&model_dir).unwrap();
    let worker_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/fixtures/atlas_bert")
        .canonicalize()
        .unwrap();
    let routes: BTreeMap<_, _> = titles
        .iter()
        .enumerate()
        .map(|(index, title)| {
            let mut scores = [0.1; 6];
            for (domain, score) in [(index * 2, 0.99), (index * 2 + 1, 0.94)] {
                if !disabled.contains(&domain) {
                    scores[domain] = score;
                }
            }
            (title.to_ascii_lowercase(), scores)
        })
        .collect();
    std::fs::write(
        model_dir.join("config.json"),
        serde_json::to_vec(&serde_json::json!({"routes": routes})).unwrap(),
    )
    .unwrap();
    for name in ["vocab.txt", "rust_model.ot"] {
        std::fs::write(model_dir.join(name), b"offline worker protocol fixture").unwrap();
    }
    std::fs::write(
        classifier.join("zero-shot.json"),
        serde_json::to_vec(&serde_json::json!({
            "worker_path": worker_path,
            "model_dir": model_dir,
        }))
        .unwrap(),
    )
    .unwrap();
}
