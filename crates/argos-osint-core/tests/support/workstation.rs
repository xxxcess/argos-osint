//! Real workstation registration and leased Assistant turns for storage integration tests.

use argos_osint_core::storage::bootstrap;
use argos_osint_core::storage::connect_registry;
use argos_osint_core::storage::connect_registry::RegistryAuthority;
use argos_osint_core::storage::inbox_qa as qa;
use argos_osint_core::storage::job_dispatch;
use argos_osint_core::storage::repo::Repository;
use chrono::Utc;
use fleetworks_contracts::chat::ChatSourceKind;
use fleetworks_contracts::chat::ChatSourceReference;
use fleetworks_contracts::chat::QaTurnRequest;
use fleetworks_contracts::chat::QaTurnState;
use fleetworks_contracts::connect::RegistrationEnsureRequest;
use fleetworks_contracts::context::MutationContext;
use fleetworks_contracts::context::RegisteredContext;
use fleetworks_contracts::qa_worker::QaDialogueKind;
use fleetworks_contracts::qa_worker::QaPassage;
use fleetworks_contracts::qa_worker::QaWorkerDialogue;
use fleetworks_contracts::qa_worker::QaWorkerRequest;

pub(super) const QA_WORKER: &str = "qa-model-fixture";
pub(super) const QA_PLACEMENT: &str = "qa-placement-fixture";

pub(super) fn authority() -> RegistryAuthority {
    RegistryAuthority::new("scope-a", "fixture-native", "fixture-access")
        .expect("fixture authority")
}

pub(super) async fn register(repo: &Repository, node: &str, child: &str) -> RegisteredContext {
    bootstrap::ensure_network_identity(repo.db())
        .await
        .expect("explicit fixture bootstrap identity");
    connect_registry::initialize_control(repo.db())
        .await
        .expect("explicit fixture control");
    let ip = if node == "node-a" {
        "192.0.2.42"
    } else {
        "198.51.100.42"
    };
    let receipt = connect_registry::ensure(
        repo,
        &authority(),
        &RegistrationEnsureRequest::new(
            format!("ensure-{node}-{child}"),
            Some(ip.parse().expect("synthetic IP")),
            format!("installation-{node}"),
        ),
    )
    .await
    .expect("actual fixture registration")
    .receipt;
    connect_registry::registration_context(
        repo,
        &authority(),
        &receipt.registration_id,
        receipt.revision,
    )
    .await
    .expect("actual current registration context")
}

pub(super) async fn ready_qa_replica(repo: &Repository, generation: i64) {
    let db = repo.db();
    assert_eq!(
        qa::claim_replica(repo, QA_PLACEMENT, QA_WORKER, generation)
            .await
            .unwrap(),
        qa::ReplicaClaim::Warm,
    );
    assert!(qa::beat(repo, QA_PLACEMENT, QA_WORKER, generation, true)
        .await
        .unwrap());
    job_dispatch::enable_pool(db, qa::POOL).await.unwrap();
}

pub(super) async fn start_qa_fixture(
    repo: &Repository,
    actor: &qa::Actor,
    context: &RegisteredContext,
    enabled: qa::Setting,
    request_id: &str,
) -> (QaTurnRequest, Box<job_dispatch::DeliveryLease>) {
    ready_qa_replica(repo, enabled.generation).await;
    let request = QaTurnRequest::new(
        request_id.into(),
        MutationContext::new(context),
        "Which backend checks are ready?".into(),
        None,
    )
    .unwrap();
    let id = qa::accepted_turn_id(actor, &request, enabled.history_generation).unwrap();
    let at = Utc::now();
    let facts = [
        ("gateway", "Gateway", "Gateway is ready."),
        ("catalog", "Actor catalog", "Actor catalog is ready."),
    ];
    let mut worker_request = QaWorkerRequest::new(
        id.clone(),
        id,
        request.question.clone(),
        facts
            .iter()
            .map(|(id, _, text)| QaPassage::new((*id).into(), (*text).into()))
            .collect(),
    );
    worker_request.dialogue = Some(QaWorkerDialogue {
        kind: QaDialogueKind::Operational,
        history: Vec::new(),
    });
    let evidence = qa::FrozenEvidence {
        request: worker_request,
        sources: facts
            .iter()
            .map(|(id, label, _)| qa::Source {
                opaque_id: (*id).into(),
                reference: ChatSourceReference::new(ChatSourceKind::Readiness, (*id).into())
                    .unwrap(),
                label: (*label).into(),
                observed_at: at.to_rfc3339(),
                provenance: None,
                measurement: None,
                whole_passage: false,
            })
            .collect(),
        sampled_at_ms: at.timestamp_millis(),
    };
    let queued = qa::admit(
        repo,
        actor,
        &request,
        evidence,
        qa::AdmissionMetadata {
            subject: None,
            observation: None,
            related_activity: None,
            status_check_tally: None,
            used_previous_exchange: false,
            response_started_at_ms: None,
            conversation: Default::default(),
        },
        enabled.generation,
    )
    .await
    .unwrap();
    assert_eq!(queued.state, QaTurnState::Queued);
    (request, start_qa_delivery(repo).await)
}

pub(super) async fn start_qa_delivery(repo: &Repository) -> Box<job_dispatch::DeliveryLease> {
    let db = repo.db();
    let claim = job_dispatch::claim_ready(db, qa::POOL, "qa-fixture-relay", Utc::now())
        .await
        .unwrap()
        .expect("model delivery");
    let job_dispatch::DeliveryClaim::Claimed(lease) = job_dispatch::claim_delivery(
        db,
        &claim.envelope,
        QA_WORKER,
        30,
        Utc::now(),
        "qa-fixture-claim",
    )
    .await
    .unwrap() else {
        panic!("model delivery must be leased");
    };
    assert!(qa::start(repo, &lease).await.unwrap().is_some());
    lease
}
