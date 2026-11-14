//! Genuine bounded Natural training in disposable homes; no installed models.

use std::fs;
use std::sync::Arc;

use crate::atlas_taxonomy::DomainId;
use crate::classifier::dataset;
use crate::classifier::dataset::DatasetRequest;
use crate::classifier::training;
use crate::classifier::WorkControl;

use super::*;

fn trained(root: &std::path::Path) -> ArtifactStore {
    let header="sample_id,title,description,domain_labels,primary_domain,secondary_domain,subcategories,source_kind,difficulty,dataset_version,annotation_notes,split,scenario_group,language,synthetic,labels_complete,expected_admission,rejection_reason\n";
    let mut csv = header.to_owned();
    for split in ["train", "calibration"] {
        for group in 0..3 {
            for repetition in 0..2 {
                let a = DomainId::ALL[group * 2].as_str();
                let b = DomainId::ALL[group * 2 + 1].as_str();
                let id = format!("{split}-{group}-{repetition}");
                csv.push_str(&format!("{id},Synthetic {a} {b} bulletin {repetition},Fictional {a} and {b} report,{a}|{b},{a},{b},,synthetic,cross_domain,synthetic-development-v1,,{split},{id},en,true,true,admit,\n"));
            }
        }
    }
    let file = root.join("training.csv");
    fs::write(&file, csv).expect("bounded genuine training input");
    let selection = DatasetRequest {
        explicit_file: Some(file),
        cwd: root.into(),
        home: root.into(),
        executable: root.join("fixture-program"),
        ..Default::default()
    };
    let parsed =
        dataset::parse_csv(&dataset::resolve_dataset(&selection).expect("selected synthetic CSV"))
            .expect("actual CSV validation");
    let store = ArtifactStore::new(root).expect("disposable artifact store");
    let lease = Arc::new(store.try_writer().expect("actual training writer"));
    let model = ModelId::parse("disposable-natural-candidate").expect("candidate ID");
    let control = WorkControl::new();
    let mut worker = crate::classifier::start_candidate_worker(&model, lease.clone(), &control)
        .expect("real Natural candidate worker");
    let request = TrainRequest {
        request_id: "setup-disposable-training".into(),
        model_id: model,
        provenance: Provenance::synthetic_baseline(
            &parsed.dataset_version,
            &parsed.corpus.dataset_sha256,
        ),
        synthetic_activation: SyntheticActivation::RetainPrior,
    };
    let candidate = training::train_candidate(
        worker.as_mut(),
        &request,
        &parsed.corpus,
        &control,
        &mut |_| {},
    )
    .expect("genuine candidate training");
    let manifest = training::candidate_manifest(&parsed.corpus, &request, &candidate)
        .expect("genuine immutable manifest");
    store
        .write_candidate(&lease, &manifest, &candidate.payload, &candidate.report)
        .expect("real candidate publication");
    drop(worker);
    drop(lease);
    assert!(store
        .read_active()
        .expect("no automatic activation")
        .is_none());
    store
}
fn request(preview: &CandidatePreview) -> ActivationRequest {
    let context=serde_json::from_value(serde_json::json!({"registration_id":"fixture-registration","expected_registration_revision":1,"expected_context_revision":1,"expected_configuration_revision":1})).expect("contract context fixture");
    ActivationRequest::new(
        "confirm-activation".into(),
        context,
        preview.candidate.clone(),
        preview.expected_prior.clone(),
        SyntheticBaselineConsent::ActivateSyntheticBaseline,
    )
    .expect("explicit confirmed activation")
}

#[test]
fn real_activation_requires_exact_hashes_prior_and_consent_and_replays_only_its_own_effect() {
    let _serial = crate::classifier::TEST_RUNTIME_LOCK.blocking_lock();
    let temporary = tempfile::tempdir().expect("temporary home");
    let store = trained(temporary.path());
    let candidate_preview = preview(
        &store,
        "setup-disposable-training",
        "disposable-natural-candidate",
    )
    .expect("actual immutable preview");
    assert!(candidate_preview.activation_supported && candidate_preview.synthetic_baseline);
    let request = request(&candidate_preview);
    let lease = store.try_writer().expect("exclusive activation writer");
    let cancel = AtomicBool::new(false);
    let mut denied = request.clone();
    denied.synthetic_baseline = SyntheticBaselineConsent::NotGranted;
    assert_eq!(
        execute(&store, &lease, "activation-no-consent", &denied, &cancel).err(),
        Some(ActivationFailure::ConsentRequired)
    );
    denied = request.clone();
    denied.candidate.model_sha256 = "a".repeat(64);
    assert_eq!(
        execute(&store, &lease, "activation-wrong-hash", &denied, &cancel).err(),
        Some(ActivationFailure::CandidateUnqualified)
    );
    denied = request.clone();
    denied.expected_prior = ActiveModelRevision::active("other-model".into(), 7, "b".repeat(64))
        .expect("explicit stale prior");
    assert_eq!(
        execute(&store, &lease, "activation-stale", &denied, &cancel).err(),
        Some(ActivationFailure::Stale)
    );
    assert!(store
        .read_active()
        .expect("retained no-active prior")
        .is_none());
    let result = execute(&store, &lease, "activation-owned", &request, &cancel)
        .expect("actual explicit activation");
    assert_eq!(result.state, ActivationState::Activated);
    assert_eq!(
        execute(&store, &lease, "activation-owned", &request, &cancel)
            .expect("same operation proof replay"),
        result
    );
    assert_eq!(
        store
            .read_active()
            .expect("active")
            .expect("saved model")
            .activation_generation,
        1
    );
    assert!(reconcile(&store, "another-activation", &request)
        .expect("other operation lookup")
        .is_none());
    let current = preview(
        &store,
        "setup-disposable-training",
        "disposable-natural-candidate",
    )
    .expect("post-effect preview");
    assert!(
        !current.activation_supported,
        "a previous activation uses explicit rollback rather than new candidate promotion"
    );
}

#[test]
fn started_without_owned_effect_stays_uncertain_even_when_another_operation_activated_exact_candidate(
) {
    let _serial = crate::classifier::TEST_RUNTIME_LOCK.blocking_lock();
    let temporary = tempfile::tempdir().expect("temporary home");
    let store = trained(temporary.path());
    let candidate_preview = preview(
        &store,
        "setup-disposable-training",
        "disposable-natural-candidate",
    )
    .expect("candidate");
    let request = request(&candidate_preview);
    let lease = store.try_writer().expect("writer");
    effect::begin(&store, "activation-uncertain", &request)
        .expect("durable intent before simulated crash");
    let mut other = request.clone();
    other.request_id = "another-confirmed-request".into();
    let actual = execute(
        &store,
        &lease,
        "activation-another-owner",
        &other,
        &AtomicBool::new(false),
    )
    .expect("another real explicit activation");
    assert_eq!(actual.state, ActivationState::Activated);
    let uncertain = execute(
        &store,
        &lease,
        "activation-uncertain",
        &request,
        &AtomicBool::new(false),
    )
    .expect("uncertain reconciliation");
    assert_eq!(uncertain.state, ActivationState::EffectUncertain);
    assert!(uncertain.activated_revision.is_none());
    assert!(reconcile(&store, "activation-uncertain", &request)
        .expect("no fabricated proof")
        .is_none());
    assert_eq!(
        store
            .read_active()
            .expect("observed current")
            .expect("other real effect")
            .activation_generation,
        1
    );
}
