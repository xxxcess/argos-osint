//! Synthetic boundary observations through the unchanged production Workstation
//! page. No scene submits work, reads the user's home or performs inference.

use fleetworks_contracts::context::Permission;
use fleetworks_contracts::readiness::ReadinessAction;
use fleetworks_contracts::readiness::ReadinessActionId;
use fleetworks_contracts::readiness::ReadinessCheck;
use fleetworks_contracts::readiness::ReadinessCheckId;
use fleetworks_contracts::readiness::ReadinessState;
use fleetworks_contracts::readiness::WorkstationReadiness;
use fleetworks_desktop::app::Load;
use fleetworks_desktop::connection::ConnectionStatus;
use fleetworks_desktop::Page;
use fleetworks_desktop::State;

const OBSERVED: &str = "2026-01-01T00:00:00Z";

fn check(
    id: ReadinessCheckId,
    state: ReadinessState,
    reason: &str,
    explanation: &str,
) -> ReadinessCheck {
    let mut check = ReadinessCheck::new(
        id,
        state,
        reason.into(),
        explanation.into(),
        OBSERVED.into(),
    );
    check.fresh_for_ms = 30_000;
    check
}

pub fn observed(state: &mut State) {
    super::registered(state);
    let mut seed = check(
        ReadinessCheckId::ActorCatalog,
        ReadinessState::NotReady,
        "not_seeded",
        "The actor catalog has not been seeded.",
    );
    let mut seed_action = ReadinessAction::new(
        ReadinessActionId::SeedActors,
        Permission::MaintainBackend,
        false,
    );
    seed_action.disabled_reason = Some("Select a validated actor dataset before seeding".into());
    seed.actions.push(seed_action);
    let checks = vec![
        check(
            ReadinessCheckId::Gateway,
            ReadinessState::Ready,
            "authenticated",
            "The synthetic gateway authenticated this connection.",
        ),
        check(
            ReadinessCheckId::Jobs,
            ReadinessState::Ready,
            "jobs_observed",
            "The backend job dispatcher was observed.",
        ),
        seed,
        check(
            ReadinessCheckId::ActorSearch,
            ReadinessState::NotReady,
            "not_built",
            "Actor search has no serving generation yet.",
        ),
        check(
            ReadinessCheckId::ClassifierInference,
            ReadinessState::Unknown,
            "inference_not_observed",
            "No real inference receipt has been observed. Model assets or training alone do not prove inference readiness.",
        ),
        check(
            ReadinessCheckId::ClassifierTraining,
            ReadinessState::NotApplicable,
            "optional_natural_training",
            "Natural classifier training is optional and has not been requested.",
        ),
    ];
    let mut readiness =
        WorkstationReadiness::new("synthetic-deployment".into(), OBSERVED.into(), checks);
    if let Load::Ready(receipt) = &state.registration.receipt {
        readiness.registration = Some(receipt.clone());
    }
    state.readiness = Load::Ready(readiness);
    state.readiness_received_at = Some(0.0);
    state.page = Page::Workstation;
}

pub fn connecting(state: &mut State) {
    state.page = Page::Workstation;
    state.connection.status = ConnectionStatus::Connecting;
    state.readiness = Load::Idle;
}
pub fn unsupported(state: &mut State) {
    super::connected(state);
    let mut capabilities = state
        .connection
        .capabilities
        .take()
        .expect("synthetic capabilities");
    capabilities.api_version = 99;
    let error = capabilities
        .require_native_contract()
        .expect_err("unsupported real contract validation");
    state.connection.status = ConnectionStatus::Failed(error.to_string());
    state.readiness = Load::Idle;
    state.page = Page::Workstation;
}
pub fn stale(state: &mut State) {
    observed(state);
    if let Load::Ready(readiness) = &mut state.readiness {
        for check in &mut readiness.checks {
            check.fresh_for_ms = 0;
        }
    }
}
pub fn checks_error(state: &mut State) {
    observed(state);
    state.readiness = Load::Failed("Synthetic gateway could not return current checks. Retry when the connection recovers; no maintenance was started.".into());
}
pub fn permission_denied(state: &mut State) {
    observed(state);
    state
        .connection
        .capabilities
        .as_mut()
        .expect("synthetic permissions")
        .permissions
        .retain(|permission| *permission != Permission::MaintainBackend);
    if let Load::Ready(readiness) = &mut state.readiness {
        for check in &mut readiness.checks {
            for action in &mut check.actions {
                action.allowed = false;
                action.disabled_reason = Some("Backend maintenance permission required".into());
            }
        }
    }
}
pub fn partial_seed(state: &mut State) {
    observed(state);
    if let Load::Ready(readiness) = &mut state.readiness {
        for check in &mut readiness.checks {
            match check.id {
                ReadinessCheckId::ActorCatalog => {
                    *check = self::check(
                        ReadinessCheckId::ActorCatalog,
                        ReadinessState::NotReady,
                        "seed_incomplete",
                        "A seed stopped before completion. Previously committed rows remain available; no completed seed result is implied.",
                    );
                    check.job_ids = vec!["synthetic-partial-seed-job".into()];
                }
                ReadinessCheckId::ActorSearch => {
                    *check = self::check(
                        ReadinessCheckId::ActorSearch,
                        ReadinessState::Running,
                        "partial_indexing",
                        "Committed actors are still indexing. Serving visibility has not caught up to the committed seed range.",
                    )
                }
                _ => {}
            }
        }
    }
}
pub fn partial_seed_progress(state: &mut State) {
    super::jobs_ready(state);
    if let Load::Ready(detail) = &mut state.backend.detail {
        detail.job.title = "Synthetic interrupted actor seed".into();
        detail.job.operation = "actor_seed".into();
        detail.job.state = "failed".into();
        detail.job.phase = "seed_interrupted".into();
        detail.job.progress_done = Some(650);
        detail.job.progress_total = Some(1000);
        detail.job.error_summary = "Synthetic seed stopped after the last committed batch. Previously committed actors remain; the full dataset is not seeded.".into();
        detail.maintenance_result = None;
        if let Load::Ready(jobs) = &mut state.backend.jobs {
            jobs.rows = vec![detail.job.clone()];
        }
    }
}

pub fn training(state: &mut State) {
    observed(state);
    if let Load::Ready(readiness) = &mut state.readiness {
        for check in &mut readiness.checks {
            if check.id == ReadinessCheckId::ClassifierTraining {
                *check = self::check(
                    ReadinessCheckId::ClassifierTraining,
                    ReadinessState::Running,
                    "training_running",
                    "A Natural candidate is training on the backend. The prior active model remains unchanged; activation is a separate action.",
                );
                check.job_ids = vec!["synthetic-training-job".into()];
            }
        }
    }
}
pub fn inference_unverified(state: &mut State) {
    observed(state);
}
pub fn inference_error(state: &mut State) {
    observed(state);
    if let Load::Ready(readiness) = &mut state.readiness {
        for check in &mut readiness.checks {
            if check.id == ReadinessCheckId::ClassifierInference {
                *check = self::check(
                    ReadinessCheckId::ClassifierInference,
                    ReadinessState::Unavailable,
                    "inference_probe_failed",
                    "The backend reported a failed real-inference check. Existing training metadata does not make inference ready.",
                );
            }
        }
    }
}
