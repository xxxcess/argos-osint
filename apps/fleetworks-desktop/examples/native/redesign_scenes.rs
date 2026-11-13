//! Current workstation-only production views with authoritative synthetic replies.
//! No scene opens a home, reads credentials, creates a folder or contacts a service.

use fleetworks_contracts::connect::RegistrationReceipt;
use fleetworks_contracts::context::RegisteredContext;
use fleetworks_contracts::cycle_history::CycleHistory;
use fleetworks_contracts::cycle_history::CycleSummary;
use fleetworks_contracts::operational_logs::LogPage;
use fleetworks_contracts::pairing::PairingChallenge;
use fleetworks_contracts::readiness::DatasetKind;
use fleetworks_contracts::readiness::DatasetRef;
use fleetworks_contracts::readiness::MaintenanceOperation;
use fleetworks_contracts::readiness::ReadinessAction;
use fleetworks_contracts::readiness::ReadinessActionId;
use fleetworks_contracts::readiness::ReadinessCheck;
use fleetworks_contracts::readiness::ReadinessCheckId;
use fleetworks_contracts::readiness::ReadinessState;
use fleetworks_contracts::readiness::WorkstationReadiness;
use fleetworks_desktop::app::Load;
use fleetworks_desktop::backend::ActivityJob;
use fleetworks_desktop::backend::JobDetail;
use fleetworks_desktop::logs::LogReply;
use fleetworks_desktop::logs::LogTarget;
use fleetworks_desktop::pairing::PairingFailure;
use fleetworks_desktop::pairing::PairingStage;
use fleetworks_desktop::workstation_dashboard::HostMetrics;
use fleetworks_desktop::Page;
use fleetworks_desktop::State;
use rune::TextEdit;
use serde_json::json;

const OBSERVED: &str = "2026-11-12T12:00:00Z";

fn base(state: &mut State) {
    super::connected(state);
    state.setup_gateway = false;
    state.connection.origin = TextEdit::new("http://127.0.0.1:8080");
    let receipt = RegistrationReceipt::new(
        "synthetic-deployment".into(),
        "synthetic-workstation".into(),
        "synthetic-registration".into(),
        None,
        1,
    );
    state.registration.context = Some(
        RegisteredContext::new(
            receipt.clone(),
            1,
            1,
            "synthetic-fresh-config-v1".into(),
            "synthetic-access-1".into(),
        )
        .expect("current workstation context without a folder or IP"),
    );
    state.registration.receipt = Load::Ready(receipt.clone());
    let checks = [
        ReadinessCheckId::Gateway,
        ReadinessCheckId::Datastore,
        ReadinessCheckId::Jobs,
        ReadinessCheckId::Brain,
        ReadinessCheckId::Control,
        ReadinessCheckId::Registration,
    ]
    .into_iter()
    .map(|id| {
        let mut check = ReadinessCheck::new(
            id,
            ReadinessState::Ready,
            "synthetic_ready".into(),
            "Synthetic service observation".into(),
            OBSERVED.into(),
        );
        check.fresh_for_ms = 30_000;
        check
    })
    .collect();
    let mut readiness =
        WorkstationReadiness::new("synthetic-deployment".into(), OBSERVED.into(), checks);
    readiness.registration = Some(receipt);
    state.readiness = Load::Ready(readiness);
    state.readiness_received_at = Some(0.0);
}

pub fn status(state: &mut State) {
    base(state);
    state.host_metrics = Load::Ready(HostMetrics {
        cpu_percent: Some(18.0),
        memory_percent: Some(42.0),
        disk_percent: Some(37.0),
        // Unsupported telemetry stays absent; this is an explicit synthetic API snapshot.
        network_kbps: None,
        platform: Some("Synthetic API host".into()),
        chipset: None,
    });
    state.page = Page::Workstation;
}

pub fn seed(state: &mut State) {
    base(state);
    super::profile_ready(state);
    let Load::Ready(readiness) = &mut state.readiness else {
        return;
    };
    for (id, action, operation, dataset) in [
        (
            ReadinessCheckId::ActorCatalog,
            ReadinessActionId::SeedActors,
            MaintenanceOperation::SeedActors,
            Some(
                DatasetRef::new(
                    "synthetic-actor-catalog".into(),
                    DatasetKind::ActorCatalog,
                    "a".repeat(64),
                    1024,
                )
                .expect("catalog reference"),
            ),
        ),
        (
            ReadinessCheckId::ActorSearch,
            ReadinessActionId::RebuildActorIndex,
            MaintenanceOperation::ReindexActors,
            None,
        ),
        (
            ReadinessCheckId::ClassifierTraining,
            ReadinessActionId::TrainClassifier,
            MaintenanceOperation::TrainClassifier,
            Some(
                DatasetRef::new(
                    "synthetic-natural-training".into(),
                    DatasetKind::ClassifierTraining,
                    "b".repeat(64),
                    2048,
                )
                .expect("training reference"),
            ),
        ),
    ] {
        let mut check = ReadinessCheck::new(
            id,
            ReadinessState::NotReady,
            "synthetic_configured".into(),
            "Configured input observed; operation has not been submitted.".into(),
            OBSERVED.into(),
        );
        check.fresh_for_ms = 30_000;
        check.actions.push(ReadinessAction::new(
            action,
            fleetworks_contracts::context::Permission::MaintainBackend,
            true,
        ));
        readiness.checks.push(check);
        readiness.configured_inputs.push(
            serde_json::from_value(
                json!({"operation":operation,"dataset":dataset,"configuration_revision":7}),
            )
            .expect("configured operation snapshot"),
        );
    }
    state.page = Page::Seed;
}

pub fn cycles(state: &mut State) {
    base(state);
    super::models_ready(state);
    let mut finished = CycleSummary::new(
        "synthetic-cycle-complete".into(),
        "synthetic-run-complete".into(),
        "completed".into(),
        OBSERVED.into(),
        Some("2026-11-12T12:03:00Z".into()),
    );
    finished.article_count = Some(128);
    let running = CycleSummary::new(
        "synthetic-cycle-running".into(),
        "synthetic-run-running".into(),
        "running".into(),
        "2026-11-12T12:10:00Z".into(),
        None,
    );
    state.cycles.history = Load::Ready(CycleHistory::new(vec![running, finished]));
    state.cycles.selected = Some("synthetic-cycle-complete".into());
    state.page = Page::Atlas;
}

pub fn brain(state: &mut State) {
    base(state);
    super::brain_dossier(state);
    state.page = Page::Brain;
}

pub fn tools(state: &mut State) {
    base(state);
    super::tools_ready(state);
    super::profile_ready(state);
    let Load::Ready(profile) = &mut state.backend.profile else {
        return;
    };
    profile.tool_credentials = serde_json::from_value(json!([
        {"provider":"firecrawl","primary_present":true,"fallback_present":false},
        {"provider":"newsapi","primary_present":false,"fallback_present":false}
    ]))
    .expect("synthetic credential-presence metadata only");
    state.page = Page::Tools;
}

pub fn models(state: &mut State) {
    base(state);
    super::profile_ready(state);
    super::models_ready(state);
    state.page = Page::Models;
}

pub fn jobs(state: &mut State) {
    base(state);
    let job: ActivityJob = serde_json::from_value(json!({
        "id":"synthetic-seed-job","app":"workstation","title":"Synthetic configured actor seed","operation":"actor_seed",
        "state":"running","phase":"seeding","progress_done":250,"progress_total":1000,"error_summary":"",
        "private_details_available":true,"activity_actions_available":false,"status_only":false
    })).expect("authoritative synthetic job snapshot");
    super::jobs_scenes::install(state, vec![job.clone()]);
    state.backend.selected_job = Some(job.id.clone());
    state.backend.detail = Load::Ready(JobDetail {
        job: job.clone(),
        stages: Vec::new(),
        private_details_available: true,
        activity_actions_available: false,
        maintenance_result: None,
    });
    state.page = Page::Jobs;
    state.sync_log_binding();
    state.scoped_logs.window_focused = true;
    state.scoped_logs.set_visible(true);
    state.scoped_logs.select_target(LogTarget::Job {
        id: job.id.clone(),
        descendants: false,
    });
    let ticket = state
        .scoped_logs
        .refresh()
        .expect("synthetic registered selected-job log query");
    let page: LogPage = serde_json::from_value(json!({
        "rows":[{"id":"synthetic-event-1","ts":OBSERVED,"severity":"info","app":"workstation","event_type":"progress","message":"Synthetic committed batch acknowledged","job_id":job.id,"task_id":"","attempt_id":"","run_id":""}],
        "counts":{"total":1,"info":1,"warn":0,"error":0,"debug":0},"apps":["workstation"],"apps_complete":true,
        "next_cursor":null,"follow_cursor":null,"available":true,"private_logs_only":true,"shared_private_logs_available":false,"reason_code":null,"narrowing_hint":null,"has_more":false
    })).expect("safe operational summary snapshot");
    state
        .scoped_logs
        .complete(&ticket, Ok(LogReply::Page(page)));
}

pub fn setup(state: &mut State) {
    state.setup_gateway = true;
    state.page = Page::Workstation;
    state.connection.origin = TextEdit::new("https://gateway.example.test");
    state.connection.status = fleetworks_desktop::connection::ConnectionStatus::Disconnected;
    state.registration.context = None;
    state.readiness = Load::Idle;
}

pub fn pairing_idle(state: &mut State) {
    setup(state);
    state.pairing.fixture_enabled = true;
    state.pairing.stage = PairingStage::Idle;
    state.fixture_effect = None;
}

pub fn pairing_waiting(state: &mut State) {
    pairing_idle(state);
    state.pairing.stage = PairingStage::Waiting(PairingChallenge {
        pairing_id: "synthetic-pairing-request-0001".into(),
        approval_code: "SYNTHETICAPPROVALCODE0001".into(),
        expires_at_ms: 1_900_000_000_000,
        poll_after_ms: 1500,
    });
}

pub fn pairing_keychain_denied(state: &mut State) {
    pairing_idle(state);
    state.pairing.stage = PairingStage::Failed(PairingFailure::KeychainDenied);
}

pub fn pairing_denied(state: &mut State) {
    pairing_idle(state);
    state.pairing.stage = PairingStage::Failed(PairingFailure::Denied);
}

pub fn pairing_expired(state: &mut State) {
    pairing_idle(state);
    state.pairing.stage = PairingStage::Failed(PairingFailure::Expired);
}

pub fn pairing_connected(state: &mut State) {
    base(state);
    state.setup_gateway = true;
    state.pairing.fixture_enabled = true;
    state.pairing.stage = PairingStage::Connected;
    state.fixture_effect = None;
}
