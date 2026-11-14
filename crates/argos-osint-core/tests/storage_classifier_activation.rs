//! Real durable training result, scoped preview and explicit Natural promotion.
#![cfg(all(feature = "surreal", feature = "classifier"))]

mod support;
#[path = "fixtures/classifier/activation_training.rs"]
mod training;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use argos_osint_core::storage::bootstrap;
use argos_osint_core::storage::classifier_activation;
use argos_osint_core::storage::classifier_activation::ActivationWork;
use argos_osint_core::storage::connect_registry;
use argos_osint_core::storage::connect_registry::RegistryAuthority;
use argos_osint_core::storage::job_context::InteractiveAdmission;
use argos_osint_core::storage::job_dispatch;
use argos_osint_core::storage::job_dispatch::DeliveryClaim;
use argos_osint_core::storage::job_dispatch::DeliveryLease;
use argos_osint_core::storage::job_dispatch::DeliveryOutcome;
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::storage::setup_jobs;
use argos_osint_core::storage::tx;
use chrono::Utc;
use fleetworks_contracts::classifier_activation::ActivationRequest;
use fleetworks_contracts::classifier_activation::ActivationState;
use fleetworks_contracts::classifier_activation::SyntheticBaselineConsent;
use fleetworks_contracts::connect::RegistrationEnsureRequest;
use fleetworks_contracts::context::JobOrigin;
use fleetworks_contracts::context::MutationContext;
use fleetworks_contracts::context::OriginSource;
use fleetworks_contracts::context::RegisteredContext;
use fleetworks_contracts::readiness::MaintenanceOperation;
use fleetworks_contracts::readiness::MaintenanceRequest;
use serde_json::json;
use support::Server;

fn authority() -> RegistryAuthority {
    RegistryAuthority::new("activation-scope", "fixture-native", "activation-access")
        .expect("explicit fixture authority")
}
async fn context(
    repo: &Repository,
    installation: &str,
    ip: &str,
    request: &str,
) -> RegisteredContext {
    let registration = connect_registry::ensure(
        repo,
        &authority(),
        &RegistrationEnsureRequest::new(
            format!("ensure-{installation}-{request}"),
            Some(ip.parse().expect("synthetic IP")),
            format!("fixture-installation-{installation}"),
        ),
    )
    .await
    .expect("real registration")
    .receipt;
    connect_registry::registration_context(
        repo,
        &authority(),
        &registration.registration_id,
        registration.revision,
    )
    .await
    .expect("actual current context")
}
fn admission(context: &RegisteredContext, request: &str) -> InteractiveAdmission {
    InteractiveAdmission::native(
        authority(),
        context.clone(),
        JobOrigin::new(
            context,
            OriginSource::Native,
            request.into(),
            Utc::now().to_rfc3339(),
            1,
            Vec::new(),
        ),
    )
    .expect("server-captured admission fixture")
}
async fn claim(repo: &Repository, pool: &str) -> Box<DeliveryLease> {
    let relay = job_dispatch::claim_ready(repo.db(), pool, "fixture-relay", Utc::now())
        .await
        .expect("actual outbox claim")
        .expect("durable outbox delivery");
    job_dispatch::mark_published(repo.db(), &relay, Utc::now())
        .await
        .expect("actual publish receipt");
    match job_dispatch::claim_delivery(
        repo.db(),
        &relay.envelope,
        "fixture-worker",
        300,
        Utc::now(),
        &argos_osint_core::store::new_id(),
    )
    .await
    .expect("real worker lease")
    {
        DeliveryClaim::Claimed(lease) => lease,
        _ => panic!("expected actual ready delivery"),
    }
}

#[tokio::test]
async fn actual_training_candidate_requires_scoped_private_source_and_explicit_guarded_activation()
{
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("fixture", "explicit_natural_activation").await;
    bootstrap::ensure_network_identity(repo.db())
        .await
        .expect("actual deployment identity");
    connect_registry::initialize_control(repo.db())
        .await
        .expect("actual control bootstrap");
    repo.db()
        .query(classifier_activation::SCHEMA)
        .await
        .expect("activation schema")
        .check()
        .expect("schema statements");
    let creator = context(&repo, "creator", "192.0.2.42", "initial").await;
    let other_installation = context(&repo, "independent", "192.0.2.42", "initial").await;
    let other_node = context(&repo, "foreign", "198.51.100.42", "initial").await;
    assert_ne!(creator.workstation_id, other_installation.workstation_id);
    assert_ne!(creator.registration_id, other_installation.registration_id);
    assert_eq!(creator.workstation_ip, other_installation.workstation_ip);
    let temporary = tempfile::tempdir().expect("disposable training home");
    let (parsed, dataset) = training::prepare(temporary.path());
    setup_jobs::register_dataset(&repo, &authority(), &dataset, setup_jobs::DATASET_TTL)
        .await
        .expect("server-validated exact bytes");
    job_dispatch::ensure_queue_transport(repo.db(), setup_jobs::TRAIN_POOL)
        .await
        .expect("actual training queue transport");
    job_dispatch::enable_pool(repo.db(), setup_jobs::TRAIN_POOL)
        .await
        .expect("training consumer");
    setup_jobs::enable_operation(repo.db(), MaintenanceOperation::TrainClassifier, true)
        .await
        .expect("explicit training operation");
    let request = MaintenanceRequest::for_training(
        "train-once".into(),
        MutationContext::new(&creator).with_configuration_revision(1),
        dataset.clone(),
    )
    .expect("no activation training request");
    let accepted = tx::run_tx(repo.db(), tx::RetryPolicy::default(), |transaction| {
        let creator = &creator;
        let request = &request;
        async move {
            let capture = admission(creator, "train-once");
            setup_jobs::admit_in(
                &transaction,
                capture.authority(),
                creator,
                capture.origin(),
                MaintenanceOperation::TrainClassifier,
                request,
            )
            .await
        }
    })
    .await
    .expect("atomic attributed training admission")
    .value;
    let training_lease = claim(&repo, setup_jobs::TRAIN_POOL).await;
    let home = temporary.path().to_path_buf();
    let source_job = accepted.job_id.clone();
    let preview = tokio::task::spawn_blocking(move || training::train(&home, &parsed, &source_job))
        .await
        .expect("bounded genuine training task");
    setup_jobs::record_result(&repo,&training_lease.task,&json!({"operation":"classifier_train","engine":"natural","dataset_sha256":dataset.sha256,"candidate_model_id":preview.candidate.model_id,"activation_requested":false})).await.expect("real live-fenced training result");
    job_dispatch::finish_delivery(
        repo.db(),
        &training_lease,
        DeliveryOutcome::Complete {
            result_ref: "fixture-trained-candidate",
        },
        Utc::now(),
    )
    .await
    .expect("actual training completion");
    assert!(classifier_activation::training_source(
        &repo,
        "activation-scope",
        &other_installation,
        &accepted.job_id
    )
    .await
    .is_err());
    assert!(classifier_activation::training_source(
        &repo,
        "activation-scope",
        &other_node,
        &accepted.job_id
    )
    .await
    .is_err());
    let source = classifier_activation::training_source(
        &repo,
        "activation-scope",
        &creator,
        &accepted.job_id,
    )
    .await
    .expect("actual selected private training source");
    assert_eq!(source.model_id, preview.candidate.model_id);
    let mut foreign = creator.clone();
    foreign.deployment_id = "different-deployment".into();
    assert!(classifier_activation::training_source(
        &repo,
        "activation-scope",
        &foreign,
        &accepted.job_id
    )
    .await
    .is_err());
    job_dispatch::ensure_queue_transport(repo.db(), classifier_activation::POOL)
        .await
        .expect("activation transport");
    classifier_activation::configure_consumer(&repo, true)
        .await
        .expect("actual activation consumer");
    let request = ActivationRequest::new(
        "activate-once".into(),
        MutationContext::new(&creator).with_configuration_revision(1),
        preview.candidate.clone(),
        preview.expected_prior.clone(),
        SyntheticBaselineConsent::ActivateSyntheticBaseline,
    )
    .expect("separate explicit confirmation");
    let capture = admission(&creator, "activate-once");
    let mut no_consent = request.clone();
    no_consent.synthetic_baseline = SyntheticBaselineConsent::NotGranted;
    assert!(classifier_activation::admit(&repo, &capture, &no_consent)
        .await
        .is_err());
    let receipt = classifier_activation::admit(&repo, &capture, &request)
        .await
        .expect("durable explicit activation accepted");
    let repeated = classifier_activation::admit(&repo, &capture, &request)
        .await
        .expect("exact accepted replay");
    assert_eq!(repeated.job_id, receipt.job_id);
    assert_eq!(repeated.origin, receipt.origin);
    assert!(repeated.reused);
    let mut conflicting = request.clone();
    conflicting.request_id = "competing-activation".into();
    assert!(
        classifier_activation::admit(
            &repo,
            &admission(&creator, "competing-activation"),
            &conflicting
        )
        .await
        .is_err(),
        "same Natural resource excludes competing operations"
    );
    let activation_lease = claim(&repo, classifier_activation::POOL).await;
    let result = classifier_activation::execute_claimed(
        &repo,
        temporary.path().into(),
        &activation_lease.task,
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .expect("actual guarded artifact activation");
    let ActivationWork::Complete { result_ref } = result else {
        panic!("genuine candidate activation must complete");
    };
    job_dispatch::finish_delivery(
        repo.db(),
        &activation_lease,
        DeliveryOutcome::Complete {
            result_ref: &result_ref,
        },
        Utc::now(),
    )
    .await
    .expect("actual activation terminal commit");
    let saved = classifier_activation::result_for_job(&repo, &receipt.job_id)
        .await
        .expect("owned result")
        .expect("saved result");
    assert_eq!(saved.state, ActivationState::Activated);
    assert!(saved.activated_revision.is_some());
    assert_eq!(
        classifier_activation::admit(&repo, &capture, &request)
            .await
            .expect("post-effect receipt replay")
            .job_id,
        receipt.job_id
    );
    // Controlled durable-state fault after a genuine admitted/claimed effect:
    // generic job/task are terminal, but unresolved operation state still pins
    // exactly its associated workstation until a proven terminal state returns.
    let relay = RegistryAuthority::new("activation-scope", "fixture-relay", "activation-access")
        .expect("separate synthetic relay authority");
    let session = connect_registry::create_session(&repo, &relay, &creator.connect_id)
        .await
        .expect("actual session before retirement");
    let pending = connect_registry::begin_retirement(
        &repo,
        &authority(),
        &creator.registration_id,
        creator.registration_revision,
    )
    .await
    .expect("retirement fence after real activation");
    assert!(
        connect_registry::registration_context(
            &repo,
            &authority(),
            &creator.registration_id,
            creator.registration_revision,
        )
        .await
        .is_err(),
        "retirement fences current registration authority"
    );
    assert!(
        connect_registry::validate_session(&repo, &relay, session.token.expose_secret(), None)
            .await
            .is_err(),
        "retirement revokes existing browser authority"
    );
    let mut after_retirement = request.clone();
    after_retirement.request_id = "after-retirement".into();
    assert!(
        classifier_activation::admit(
            &repo,
            &admission(&creator, "after-retirement"),
            &after_retirement,
        )
        .await
        .is_err(),
        "retirement fences new activation admission"
    );
    for state in ["running", "effect_uncertain"] {
        repo.db()
            .query("UPDATE $record SET state=$state RETURN NONE")
            .bind((
                "record",
                surrealdb::types::RecordId::new(
                    "classifier_activation_operation",
                    receipt.job_id.clone(),
                ),
            ))
            .bind(("state", state))
            .await
            .expect("controlled unfinished activation state")
            .check()
            .expect("state statement");
        assert!(
            matches!(
                connect_registry::finish_retirement(
                    &repo,
                    &authority(),
                    &creator.registration_id,
                    pending.revision
                )
                .await
                .expect("auxiliary effect drain check"),
                connect_registry::RetirementResult::Pending(_)
            ),
            "terminal generic job/task cannot prove activation effect drained"
        );
    }
    let unrelated = connect_registry::begin_retirement(
        &repo,
        &authority(),
        &other_installation.registration_id,
        other_installation.registration_revision,
    )
    .await
    .expect("unrelated installation retirement");
    assert!(
        matches!(
            connect_registry::finish_retirement(
                &repo,
                &authority(),
                &other_installation.registration_id,
                unrelated.revision
            )
            .await
            .expect("unrelated installation drain"),
            connect_registry::RetirementResult::Complete(_)
        ),
        "another workstation's activation cannot pin this unrelated installation"
    );
    repo.db()
        .query("UPDATE $record SET state='activated' RETURN NONE")
        .bind((
            "record",
            surrealdb::types::RecordId::new(
                "classifier_activation_operation",
                receipt.job_id.clone(),
            ),
        ))
        .await
        .expect("restore genuinely completed operation state")
        .check()
        .expect("restored state statement");
    let completed = connect_registry::finish_retirement(
        &repo,
        &authority(),
        &creator.registration_id,
        pending.revision,
    )
    .await
    .expect("settled owned activation drain");
    assert!(matches!(
        &completed,
        connect_registry::RetirementResult::Complete(_)
    ));
    let replay = connect_registry::begin_retirement(
        &repo,
        &authority(),
        &creator.registration_id,
        creator.registration_revision,
    )
    .await
    .expect("idempotent terminal retirement fence");
    assert_eq!(
        connect_registry::finish_retirement(
            &repo,
            &authority(),
            &creator.registration_id,
            replay.revision,
        )
        .await
        .expect("idempotent terminal retirement receipt"),
        completed
    );
}
