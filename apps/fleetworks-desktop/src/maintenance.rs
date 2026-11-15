//! Explicit dataset transfer and confirmed backend maintenance admission.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::OnceLock;

use fleetworks_contracts::context::JobRequestAssociation;
use fleetworks_contracts::context::MutationContext;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::context::RegisteredContext;
use fleetworks_contracts::context::ResourceScope;
use fleetworks_contracts::context::WorkstationRequestDetailQuery;
use fleetworks_contracts::readiness::DatasetKind;
use fleetworks_contracts::readiness::DatasetRef;
use fleetworks_contracts::readiness::MaintenanceOperation;
use fleetworks_contracts::readiness::MaintenanceReceipt;
use fleetworks_contracts::readiness::MaintenanceRequest;
use fleetworks_contracts::readiness::ReadinessActionId;
use fleetworks_contracts::readiness::ReadinessCheckId;
use fleetworks_contracts::readiness::ReadinessState;
use fleetworks_contracts::readiness::ACTOR_REINDEX_OPERATION;
use fleetworks_contracts::readiness::ACTOR_SEED_PATH;
use fleetworks_contracts::readiness::CLASSIFIER_TRAIN_PATH;
use fleetworks_contracts::search::ACTOR_REINDEX_PATH;
use fleetworks_contracts::Client;
use fleetworks_contracts::ClientError;
use fleetworks_contracts::DatasetUpload;
use fleetworks_workstation::native::AcceptedActorReindex;
use fleetworks_workstation::native::NativeContainer;
use fleetworks_workstation::native::PendingActorReindex;
use rune::FileDialogOptions;
use rune::Tx;
use serde::Deserialize;
use sha2::Digest;
use sha2::Sha256;

use crate::app::Intent;
use crate::app::Load;
use crate::app::State;
use crate::connection::GatewayEffects;
use crate::connection::RequestTask;
use crate::platform;

#[derive(Clone, Debug)]
pub struct LocalDataset {
    pub path: PathBuf,
    pub kind: DatasetKind,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DatasetList {
    pub rows: Vec<DatasetRef>,
    pub truncated: bool,
}

#[derive(Clone, Debug)]
pub struct MaintenanceConfirmation {
    pub operation: MaintenanceOperation,
    pub context: RegisteredContext,
    pub origin_revision: u64,
    pub configuration_revision: u64,
    pub dataset: Option<DatasetRef>,
}

pub struct Maintenance {
    pub local_dataset: Option<LocalDataset>,
    pub datasets: Load<DatasetList>,
    pub upload: Load<DatasetUpload>,
    pub selected_dataset: Option<DatasetRef>,
    pub confirmation: Option<MaintenanceConfirmation>,
    pub receipt: Load<MaintenanceReceipt>,
    pub message: Option<String>,
    pub(crate) pending_preview: Option<MaintenanceOperation>,
    pub(crate) seed_accepted_at: Option<String>,
    pub(crate) last_request: Option<MaintenanceRequest>,
    pub(crate) pending_reindex: Option<PendingActorReindex>,
    pub(crate) reindex_association: Option<JobRequestAssociation>,
    pub(crate) reindex_details_open: bool,
    pub(crate) upload_revision: u64,
    pub(crate) datasets_revision: u64,
    pub(crate) operation_revision: u64,
    pub(crate) upload_request: Option<RequestTask>,
    pub(crate) datasets_request: Option<RequestTask>,
    pub(crate) operation_request: Option<RequestTask>,
    pub(crate) reindex_journal_request: Option<RequestTask>,
    pub(crate) reindex_reconcile_request: Option<RequestTask>,
}

impl Default for Maintenance {
    fn default() -> Self {
        Self {
            local_dataset: None,
            datasets: Load::Idle,
            upload: Load::Idle,
            selected_dataset: None,
            confirmation: None,
            receipt: Load::Idle,
            message: None,
            pending_preview: None,
            seed_accepted_at: None,
            last_request: None,
            pending_reindex: None,
            reindex_association: None,
            reindex_details_open: false,
            upload_revision: 0,
            datasets_revision: 0,
            operation_revision: 0,
            upload_request: None,
            datasets_request: None,
            operation_request: None,
            reindex_journal_request: None,
            reindex_reconcile_request: None,
        }
    }
}

impl Maintenance {
    pub(crate) fn busy(&self) -> bool {
        self.upload_request.is_some()
            || self.datasets_request.is_some()
            || self.operation_request.is_some()
            || self.reindex_reconcile_request.is_some()
    }
    pub(crate) fn invalidate_origin(&mut self) {
        self.pending_preview = None;
        self.seed_accepted_at = None;
        self.confirmation = None;
        self.last_request = None;
        self.pending_reindex = None;
        self.reindex_association = None;
        self.reindex_details_open = false;
        self.operation_request = None;
        self.reindex_journal_request = None;
        self.reindex_reconcile_request = None;
        self.operation_revision = self.operation_revision.wrapping_add(1);
        self.receipt = Load::Idle;
    }
    pub(crate) fn cancel_requests(&mut self) {
        self.upload_request = None;
        self.datasets_request = None;
        self.operation_request = None;
        self.reindex_journal_request = None;
        self.reindex_reconcile_request = None;
    }
}

impl State {
    pub fn choose_dataset(&mut self, kind: DatasetKind, tx: &mut Tx) {
        let operation = match kind {
            DatasetKind::ActorCatalog => MaintenanceOperation::SeedActors,
            DatasetKind::ClassifierTraining => MaintenanceOperation::TrainClassifier,
            _ => return,
        };
        if let Some(reason) = self.maintenance_disabled_reason(operation) {
            self.maintenance.message = Some(reason);
            return;
        }
        self.record(Intent::ChooseDataset { kind });
        if self.fixture {
            return;
        }
        let epoch = self.connection.epoch;
        tx.open_file(
            FileDialogOptions {
                title: format!("Select {} CSV", dataset_kind_label(kind)),
                ..Default::default()
            }
            .filter("CSV", &["csv"]),
            move |state: &mut State, result, _| {
                if state.connection.epoch != epoch {
                    return;
                }
                match result {
                    Ok(Some(path)) => {
                        state.select_local_dataset(path, kind);
                        state.upload_selected_dataset();
                    }
                    Ok(None) => {}
                    Err(_) => {
                        state.maintenance.message =
                            Some("The dataset chooser could not open a file".into())
                    }
                }
            },
        );
    }

    pub fn select_local_dataset(&mut self, path: PathBuf, kind: DatasetKind) {
        self.maintenance.local_dataset = Some(LocalDataset { path, kind });
        self.maintenance.upload = Load::Idle;
        self.maintenance.selected_dataset = None;
        self.maintenance.pending_preview = None;
        self.maintenance.confirmation = None;
        self.maintenance.message = None;
    }

    pub fn upload_selected_dataset(&mut self) {
        if !self.permission(Permission::MaintainBackend)
            || matches!(self.maintenance.upload, Load::Loading)
        {
            return;
        }
        let Some(selection) = self.maintenance.local_dataset.clone() else {
            return;
        };
        self.record(Intent::UploadDataset {
            kind: selection.kind,
        });
        if self.fixture {
            return;
        }
        let (Some(services), Some(client)) = (&self.services, &self.client) else {
            return;
        };
        let Some(context) = &self.registration.context else {
            return;
        };
        let client = match client.with_registered_context(context) {
            Ok(client) => client,
            Err(error) => {
                self.maintenance.message = Some(error.to_string());
                return;
            }
        };
        self.maintenance.upload = Load::Loading;
        self.maintenance.upload_revision = self.maintenance.upload_revision.wrapping_add(1);
        self.maintenance.upload_request = services.upload_dataset(
            client,
            selection,
            self.connection.epoch,
            self.maintenance.upload_revision,
            self.origin_revision,
        );
    }

    pub fn refresh_datasets(&mut self) {
        if !self.permission(Permission::ReadBackend) {
            return;
        }
        self.record(Intent::Datasets);
        if self.fixture {
            return;
        }
        let (Some(services), Some(client)) = (&self.services, &self.client) else {
            return;
        };
        if !matches!(self.maintenance.datasets, Load::Ready(_)) {
            self.maintenance.datasets = Load::Loading;
        }
        self.maintenance.datasets_revision = self.maintenance.datasets_revision.wrapping_add(1);
        self.maintenance.datasets_request = services.datasets(
            client.clone(),
            self.connection.epoch,
            self.maintenance.datasets_revision,
        );
    }

    pub fn select_backend_dataset(&mut self, id: &str) {
        let Load::Ready(list) = &self.maintenance.datasets else {
            return;
        };
        let Some(dataset) = list.rows.iter().find(|dataset| dataset.id == id).cloned() else {
            return;
        };
        if dataset.validate().is_err() {
            return;
        }
        self.maintenance.selected_dataset = Some(dataset);
        self.maintenance.confirmation = None;
        self.maintenance.last_request = None;
    }

    pub fn maintenance_disabled_reason(&self, operation: MaintenanceOperation) -> Option<String> {
        if operation == MaintenanceOperation::TrainClassifier
            && self.is_local_backend()
            && self
                .connection
                .capabilities
                .as_ref()
                .is_some_and(|capabilities| {
                    !crate::local_backend::assess_local_capabilities(capabilities)
                        .classifier_available
                })
        {
            return Some(
                "Local companion backend was built without the optional classifier capability."
                    .into(),
            );
        }
        if !self.permission(Permission::MaintainBackend) {
            return Some("Backend maintenance permission required".into());
        }
        let Some(context) = &self.registration.context else {
            return Some("Register this workstation first".into());
        };
        let Load::Ready(readiness) = &self.readiness else {
            return Some("Refresh backend checks to observe operation availability".into());
        };
        if readiness.registration.as_ref().is_none_or(|receipt| {
            receipt.registration_id != context.registration_id
                || receipt.revision != context.registration_revision
        }) {
            return Some("Refresh checks for the current registered context".into());
        }
        let Some(action_id) = action_id(operation) else {
            return Some("This maintenance operation is unsupported".into());
        };
        for check in &readiness.checks {
            if let Some(action) = check.actions.iter().find(|action| action.id == action_id) {
                let fresh = self.readiness_check_fresh(check);
                if !fresh {
                    return Some("The operation observation is stale; refresh checks".into());
                }
                if !action.allowed {
                    return Some(
                        action
                            .disabled_reason
                            .clone()
                            .unwrap_or_else(|| "Backend has not enabled this operation".into()),
                    );
                }
                if self.configuration_revision().is_none() {
                    return Some(
                        "Refresh Profile or Models to confirm the backend configuration revision"
                            .into(),
                    );
                }
                if operation == MaintenanceOperation::ReindexActors {
                    if self.maintenance.pending_reindex.is_some() {
                        return Some(
                            "An unresolved actor index request exists; check request status or clear stale attempt"
                                .into(),
                        );
                    }
                    let catalog_ready = readiness.checks.iter().any(|check| {
                        check.id == ReadinessCheckId::ActorCatalog
                            && check.state == ReadinessState::Ready
                            && self.readiness_check_fresh(check)
                            && self
                                .maintenance
                                .seed_accepted_at
                                .as_ref()
                                .is_none_or(|accepted| {
                                    chrono::DateTime::parse_from_rfc3339(&check.observed_at)
                                        .ok()
                                        .zip(chrono::DateTime::parse_from_rfc3339(accepted).ok())
                                        .is_some_and(|(observed, accepted)| observed > accepted)
                                })
                    });
                    if !catalog_ready {
                        return Some("Complete actor seeding with actors in the database before building the index".into());
                    }
                }
                if matches!(self.maintenance.upload, Load::Loading) {
                    return Some("Validating and uploading the selected CSV…".into());
                }
                if self.maintenance.operation_request.is_some()
                    || matches!(self.maintenance.receipt, Load::Loading)
                {
                    return Some("A maintenance request is awaiting its acceptance receipt".into());
                }
                return None;
            }
        }
        Some("This gateway has not advertised a durable consumer for this operation".into())
    }

    pub(crate) fn finish_dataset_preview(&mut self) {
        if let Some(operation) = self.maintenance.pending_preview {
            if self.maintenance_disabled_reason(operation).is_none() {
                self.preview_maintenance(operation);
                self.maintenance.pending_preview = None;
            }
        }
    }

    pub(crate) fn complete_dataset_upload(
        &mut self,
        epoch: u64,
        revision: u64,
        origin_revision: u64,
        kind: DatasetKind,
        result: Result<DatasetUpload, String>,
    ) {
        if self.connection.epoch != epoch
            || self.origin_revision != origin_revision
            || self.maintenance.upload_revision != revision
        {
            return;
        }
        self.maintenance.upload_request = None;
        match result {
            Ok(upload) if upload.dataset.kind == kind && upload.dataset.validate().is_ok() => {
                self.maintenance.selected_dataset = Some(upload.dataset.clone());
                self.maintenance.upload = Load::Ready(upload);
                self.maintenance.last_request = None;
                self.maintenance.pending_preview = Some(match kind {
                    DatasetKind::ActorCatalog => MaintenanceOperation::SeedActors,
                    DatasetKind::ClassifierTraining => MaintenanceOperation::TrainClassifier,
                    _ => return,
                });
                self.refresh_readiness();
                self.finish_dataset_preview();
            }
            Ok(_) => {
                self.maintenance.upload =
                    Load::Failed("Gateway returned a dataset for a different purpose".into())
            }
            Err(error) => self.maintenance.upload = Load::Failed(error),
        }
    }

    pub fn preview_maintenance(&mut self, operation: MaintenanceOperation) {
        if let Some(reason) = self.maintenance_disabled_reason(operation) {
            self.maintenance.message = Some(reason);
            return;
        }
        let (Some(context), Some(configuration_revision)) =
            (&self.registration.context, self.configuration_revision())
        else {
            return;
        };
        let dataset = match dataset_kind(operation) {
            Some(kind) => {
                let Some(dataset) = self
                    .maintenance
                    .selected_dataset
                    .as_ref()
                    .filter(|dataset| dataset.kind == kind && dataset.validate().is_ok())
                else {
                    self.maintenance.message =
                        Some("Select and upload a valid CSV for this operation first".into());
                    return;
                };
                Some(dataset.clone())
            }
            None => None,
        };
        self.maintenance.message = None;
        self.maintenance.confirmation = Some(MaintenanceConfirmation {
            operation,
            context: context.clone(),
            origin_revision: self.origin_revision,
            configuration_revision,
            dataset,
        });
        if self.page == crate::app::Page::Workstation
            && operation == MaintenanceOperation::ReindexActors
        {
            self.confirm_maintenance_dialog = true;
        }
        self.reveal_control("maintenance-confirmation");
    }

    pub fn cancel_maintenance_confirmation(&mut self) {
        self.maintenance.confirmation = None;
        self.confirm_maintenance_dialog = false;
    }

    pub fn confirm_maintenance(&mut self) {
        if self.maintenance.operation_request.is_some()
            || matches!(self.maintenance.receipt, Load::Loading)
        {
            return;
        }
        let Some(confirmation) = self.maintenance.confirmation.take() else {
            return;
        };
        self.confirm_maintenance_dialog = false;
        if confirmation.origin_revision != self.origin_revision
            || self.registration.context.as_ref() != Some(&confirmation.context)
            || self.configuration_revision() != Some(confirmation.configuration_revision)
            || confirmation.dataset.as_ref().map(|dataset| dataset.kind)
                != dataset_kind(confirmation.operation)
            || confirmation.dataset.as_ref().is_some_and(|dataset| {
                self.maintenance.selected_dataset.as_ref() != Some(dataset)
                    || Some(dataset.kind) != dataset_kind(confirmation.operation)
                    || dataset.validate().is_err()
            })
        {
            self.maintenance.message =
                Some("Origin or backend configuration changed; review the operation again".into());
            return;
        }
        if let Some(reason) = self.maintenance_disabled_reason(confirmation.operation) {
            self.maintenance.message = Some(reason);
            return;
        }
        self.submit_maintenance(confirmation, None);
    }

    fn submit_maintenance(
        &mut self,
        confirmation: MaintenanceConfirmation,
        replay: Option<MaintenanceRequest>,
    ) {
        self.record(Intent::Maintenance {
            operation: confirmation.operation,
        });
        if self.fixture {
            return;
        }
        let (Some(services), Some(client)) = (&self.services, &self.client) else {
            return;
        };
        if confirmation.operation == MaintenanceOperation::ReindexActors
            && self.workspaces.container.is_none()
        {
            self.maintenance.message = Some(
                "Protected workstation storage is unavailable; cannot persist actor index attempt"
                    .into(),
            );
            return;
        }
        let client = match client.with_registered_context(&confirmation.context) {
            Ok(client) => client,
            Err(error) => {
                self.maintenance.message = Some(error.to_string());
                return;
            }
        };
        self.maintenance.message = None;
        self.maintenance.reindex_association = None;
        self.maintenance.receipt = Load::Loading;
        self.maintenance.operation_revision = self.maintenance.operation_revision.wrapping_add(1);
        self.maintenance.operation_request = services.maintenance(
            client,
            self.workspaces.container.clone(),
            self.connection.origin.text.clone(),
            confirmation,
            self.maintenance.last_request.clone(),
            replay,
            self.connection.epoch,
            self.maintenance.operation_revision,
        );
    }

    pub(crate) fn restore_actor_reindex(&mut self) {
        if self.fixture
            || self.maintenance.pending_reindex.is_some()
            || self.maintenance.reindex_association.is_some()
            || self.maintenance.reindex_journal_request.is_some()
        {
            return;
        }
        let (Some(services), Some(container), Some(_context)) = (
            &self.services,
            self.workspaces.container.clone(),
            self.registration.context.clone(),
        ) else {
            return;
        };
        let Some(ui) = services.ui.get().cloned() else {
            return;
        };
        let origin = self.connection.origin.text.clone();
        let epoch = self.connection.epoch;
        let origin_revision = self.origin_revision;
        let revision = self.maintenance.operation_revision;
        let task = services.runtime.spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                let pending = container.pending_actor_reindex(&origin)?;
                let accepted = container.accepted_actor_reindex(&origin)?;
                Ok::<_, anyhow::Error>((pending, accepted))
            })
            .await;
            ui.update(move |state, _| {
                if state.connection.epoch != epoch
                    || state.origin_revision != origin_revision
                    || state.maintenance.operation_revision != revision
                {
                    return;
                }
                state.maintenance.reindex_journal_request = None;
                match result {
                    Ok(Ok((pending, accepted))) => {
                        state.apply_restored_actor_reindex_journal(pending, accepted);
                    }
                    _ => {
                        state.maintenance.message = Some(
                            "Could not read protected actor index journal; resolve storage before building the index"
                                .into(),
                        );
                    }
                }
            });
        });
        self.maintenance.reindex_journal_request = Some(RequestTask(task.abort_handle()));
    }

    pub(crate) fn apply_restored_actor_reindex_journal(
        &mut self,
        pending: Option<PendingActorReindex>,
        accepted: Option<AcceptedActorReindex>,
    ) {
        if let Some(pending) = pending {
            self.recover_pending_actor_reindex(pending);
        } else if let Some(accepted) = accepted {
            self.recover_accepted_actor_reindex(accepted);
        }
    }

    pub(crate) fn recover_accepted_actor_reindex(&mut self, accepted: AcceptedActorReindex) {
        if self.registration.context.as_ref() != Some(&accepted.context) {
            return;
        }
        self.maintenance.pending_reindex = None;
        self.maintenance.reindex_association = Some(accepted.association.clone());
        self.maintenance.message = Some(format!(
            "Recovered accepted job {} for actor index request {}",
            accepted.association.job_id, accepted.request.request_id
        ));
        self.refresh_readiness();
    }

    pub(crate) fn recover_pending_actor_reindex(&mut self, pending: PendingActorReindex) {
        if let Some(job_id) = pending.accepted_job_id.clone() {
            let recovered_origin = fleetworks_contracts::context::JobOrigin::new(
                &pending.context,
                fleetworks_contracts::context::OriginSource::Native,
                pending.request.request_id.clone(),
                String::new(),
                pending
                    .request
                    .context
                    .expected_configuration_revision
                    .unwrap_or_default(),
                Vec::new(),
            );
            let origin = pending.origin;
            let context = pending.context;
            let request = pending.request;
            let association =
                JobRequestAssociation::new(job_id.clone(), recovered_origin, String::new());
            self.maintenance.pending_reindex = None;
            self.maintenance.reindex_association = Some(association.clone());
            self.maintenance.message = Some(format!(
                "Recovered accepted job {job_id} for actor index request {}",
                request.request_id
            ));
            self.refresh_readiness();
            if !self.fixture {
                if let (Some(services), Some(container)) =
                    (&self.services, self.workspaces.container.clone())
                {
                    services.runtime.spawn(async move {
                        let _ = tokio::task::spawn_blocking(move || {
                            container.record_accepted_actor_reindex_association(
                                &origin,
                                &context,
                                &request,
                                &association,
                            )
                        })
                        .await;
                    });
                }
            }
            return;
        }
        let context_matches = self.registration.context.as_ref() == Some(&pending.context);
        self.maintenance.pending_reindex = Some(pending);
        if context_matches {
            self.reconcile_actor_reindex();
        }
    }

    pub(crate) fn pending_actor_reindex_matches_current(&self) -> bool {
        let Some(pending) = &self.maintenance.pending_reindex else {
            return false;
        };
        let Ok(current_origin) =
            fleetworks_workstation::native::canonical_target_origin(&self.connection.origin.text)
        else {
            return false;
        };
        pending.origin == current_origin
            && self.registration.context.as_ref() == Some(&pending.context)
            && self.configuration_revision()
                == pending.request.context.expected_configuration_revision
    }

    pub fn reconcile_actor_reindex(&mut self) {
        if self.fixture || self.maintenance.reindex_reconcile_request.is_some() {
            return;
        }
        let Some(pending) = self.maintenance.pending_reindex.clone() else {
            return;
        };
        let (Some(services), Some(client), Some(context)) = (
            &self.services,
            &self.client,
            self.registration.context.clone(),
        ) else {
            return;
        };
        let Ok(client) = client.with_registered_context(&context) else {
            return;
        };
        let Some(ui) = services.ui.get().cloned() else {
            return;
        };
        let query = WorkstationRequestDetailQuery::new(
            pending.request.request_id.clone(),
            ACTOR_REINDEX_OPERATION.to_owned(),
            Some(pending.context.registration_id.clone()),
        );
        if query.validate().is_err() {
            self.maintenance.message = Some(
                "Persisted actor index request ID is malformed; clear the stale attempt".into(),
            );
            return;
        }
        let container = self.workspaces.container.clone();
        let origin = self.connection.origin.text.clone();
        let epoch = self.connection.epoch;
        let origin_revision = self.origin_revision;
        let revision = self.maintenance.operation_revision;
        let task = services.runtime.spawn(async move {
            let outcome = client.request_detail(&query).await;
            let cleared = if let Ok(association) = &outcome {
                if valid_reindex_association(&pending, association) {
                    if let Some(container) = container {
                        let origin = origin.clone();
                        let context = pending.context.clone();
                        let request = pending.request.clone();
                        let association = association.clone();
                        tokio::task::spawn_blocking(move || {
                            container.record_accepted_actor_reindex_association(
                                &origin,
                                &context,
                                &request,
                                &association,
                            )
                        })
                        .await
                        .ok()
                        .and_then(|result| result.ok())
                        .is_some()
                    } else {
                        true
                    }
                } else {
                    false
                }
            } else {
                false
            };
            ui.update(move |state, _| {
                if state.connection.epoch != epoch
                    || state.origin_revision != origin_revision
                    || state.maintenance.operation_revision != revision
                {
                    return;
                }
                state.maintenance.reindex_reconcile_request = None;
                match outcome {
                    Ok(association) if cleared => {
                        state.maintenance.pending_reindex = None;
                        state.maintenance.reindex_association = Some(association.clone());
                        state.maintenance.message = Some(format!(
                            "Recovered accepted job {} for actor index request {}",
                            association.job_id, association.origin.request_id
                        ));
                        state.refresh_readiness();
                    }
                    Ok(_) => {
                        state.maintenance.message = Some(
                            "Gateway returned a mismatched request-detail record".into(),
                        );
                    }
                    Err(ClientError::Status(404)) => {
                        state.maintenance.message = Some(
                            "Previous actor index request is not recorded on this gateway. Retry the exact request if current, or clear this stale attempt."
                                .into(),
                        );
                    }
                    Err(error) => {
                        state.maintenance.message = Some(format!(
                            "Could not confirm previous actor index request ({error}). Retry checking or clear the stale attempt."
                        ));
                    }
                }
            });
        });
        self.maintenance.reindex_reconcile_request = Some(RequestTask(task.abort_handle()));
    }

    pub fn retry_pending_actor_reindex(&mut self) {
        if self.maintenance.busy() || matches!(self.maintenance.receipt, Load::Loading) {
            return;
        }
        let Some(pending) = self.maintenance.pending_reindex.clone() else {
            return;
        };
        if !self.pending_actor_reindex_matches_current() {
            self.maintenance.message = Some(
                "Unresolved attempt came from an earlier registration or configuration; check request status or clear the stale attempt"
                    .into(),
            );
            return;
        }
        let Some(config_rev) = pending.request.context.expected_configuration_revision else {
            return;
        };
        let confirmation = MaintenanceConfirmation {
            operation: MaintenanceOperation::ReindexActors,
            context: pending.context.clone(),
            origin_revision: self.origin_revision,
            configuration_revision: config_rev,
            dataset: None,
        };
        self.submit_maintenance(confirmation, Some(pending.request));
    }

    pub fn clear_pending_actor_reindex(&mut self) {
        let Some(pending) = self.maintenance.pending_reindex.take() else {
            return;
        };
        self.maintenance.reindex_association = None;
        self.maintenance.message = None;
        if let Load::Failed(_) = self.maintenance.receipt {
            self.maintenance.receipt = Load::Idle;
        }
        if self.fixture {
            return;
        }
        if let (Some(services), Some(container)) =
            (&self.services, self.workspaces.container.clone())
        {
            let origin = pending.origin;
            let request_id = pending.request.request_id;
            services.runtime.spawn(async move {
                let _ = tokio::task::spawn_blocking(move || {
                    container.clear_stale_actor_reindex(&origin, &request_id)
                })
                .await;
            });
        }
    }
}

impl GatewayEffects {
    fn upload_dataset(
        &self,
        client: Client,
        selection: LocalDataset,
        epoch: u64,
        revision: u64,
        origin_revision: u64,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let task = self.runtime.spawn(async move {
            let kind = selection.kind;
            let result = upload(&client, selection).await;
            ui.update(move |state, _| {
                state.complete_dataset_upload(epoch, revision, origin_revision, kind, result);
            });
        });
        Some(RequestTask(task.abort_handle()))
    }

    fn datasets(&self, client: Client, epoch: u64, revision: u64) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let task = self.runtime.spawn(async move {
            let result: Result<DatasetList, _> = client.get("/api/v1/workstation/datasets").await;
            ui.update(move |state, _| {
                if state.connection.epoch != epoch
                    || state.maintenance.datasets_revision != revision
                {
                    return;
                }
                state.maintenance.datasets_request = None;
                match result {
                    Ok(list)
                        if list.rows.len() <= 100
                            && list.rows.iter().all(|dataset| dataset.validate().is_ok()) =>
                    {
                        state.maintenance.datasets = Load::Ready(list);
                    }
                    Ok(_) => {
                        if !matches!(state.maintenance.datasets, Load::Ready(_)) {
                            state.maintenance.datasets = Load::Failed(
                                "Gateway returned an invalid dataset inventory".into(),
                            );
                        } else {
                            state.maintenance.message =
                                Some("Gateway returned an invalid dataset inventory".into());
                        }
                    }
                    Err(error) => {
                        if !matches!(state.maintenance.datasets, Load::Ready(_)) {
                            state.maintenance.datasets = Load::Failed(error.to_string());
                        } else {
                            state.maintenance.message = Some(error.to_string());
                        }
                    }
                };
            });
        });
        Some(RequestTask(task.abort_handle()))
    }

    #[allow(clippy::too_many_arguments)]
    fn maintenance(
        &self,
        client: Client,
        container: Option<Arc<NativeContainer>>,
        origin: String,
        confirmation: MaintenanceConfirmation,
        previous: Option<MaintenanceRequest>,
        replay: Option<MaintenanceRequest>,
        epoch: u64,
        revision: u64,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let task = self.runtime.spawn(async move {
            let origin_revision = confirmation.origin_revision;
            let operation = confirmation.operation;
            let origin_for_ui = origin.clone();
            let result = admit_maintenance_request(
                &client,
                container.as_ref(),
                &origin,
                &confirmation,
                previous,
                replay,
            )
            .await;
            ui.update(move |state, _| {
                if state.connection.epoch != epoch
                    || state.origin_revision != origin_revision
                    || state.maintenance.operation_revision != revision
                {
                    return;
                }
                state.maintenance.operation_request = None;
                match result {
                    Ok(admitted) => {
                        state.apply_maintenance_outcome(operation, admitted, &origin_for_ui)
                    }
                    Err(error) => {
                        state.maintenance.receipt = Load::Failed(error);
                    }
                }
            });
        });
        Some(RequestTask(task.abort_handle()))
    }
}

impl State {
    pub(crate) fn apply_maintenance_outcome(
        &mut self,
        operation: MaintenanceOperation,
        admitted: AdmittedMaintenance,
        origin: &str,
    ) {
        self.maintenance.last_request = admitted.saved_request.clone();
        if operation == MaintenanceOperation::ReindexActors {
            self.maintenance.pending_reindex = admitted.pending_entry;
        }
        match admitted.outcome {
            MaintenanceOutcome::Receipt(receipt) => {
                if receipt.operation == MaintenanceOperation::SeedActors {
                    self.maintenance.seed_accepted_at = Some(receipt.origin.submitted_at.clone());
                }
                let persisted_association =
                    if receipt.operation == MaintenanceOperation::ReindexActors {
                        self.maintenance.pending_reindex = None;
                        let association = JobRequestAssociation::new(
                            receipt.job_id.clone(),
                            receipt.origin.clone(),
                            String::new(),
                        );
                        self.maintenance.reindex_association = Some(association.clone());
                        Some(association)
                    } else {
                        None
                    };
                self.maintenance.receipt = Load::Ready(receipt.clone());
                self.refresh_readiness();
                // Promote the pending guard into the bounded accepted-job record only after UI state holds the receipt.
                if let (
                    Some(association),
                    false,
                    Some(services),
                    Some(container),
                    Some(context),
                    Some(request),
                ) = (
                    persisted_association,
                    self.fixture,
                    &self.services,
                    self.workspaces.container.clone(),
                    admitted.saved_context,
                    admitted.saved_request,
                ) {
                    let origin = origin.to_owned();
                    services.runtime.spawn(async move {
                        let _ = tokio::task::spawn_blocking(move || {
                            container.record_accepted_actor_reindex_association(
                                &origin,
                                &context,
                                &request,
                                &association,
                            )
                        })
                        .await;
                    });
                }
            }
            MaintenanceOutcome::Reconciled(association) => {
                self.maintenance.pending_reindex = None;
                self.maintenance.reindex_association = Some(association.clone());
                self.maintenance.message = Some(format!(
                    "Recovered accepted job {} for actor index request {}",
                    association.job_id, association.origin.request_id
                ));
                self.maintenance.receipt = Load::Idle;
                self.refresh_readiness();
            }
            MaintenanceOutcome::Failed(error) => {
                self.maintenance.receipt = Load::Failed(error);
            }
        }
    }
}

pub(crate) struct AdmittedMaintenance {
    saved_request: Option<MaintenanceRequest>,
    saved_context: Option<RegisteredContext>,
    pending_entry: Option<PendingActorReindex>,
    outcome: MaintenanceOutcome,
}

async fn admit_maintenance_request(
    client: &Client,
    container: Option<&Arc<NativeContainer>>,
    origin: &str,
    confirmation: &MaintenanceConfirmation,
    previous: Option<MaintenanceRequest>,
    replay: Option<MaintenanceRequest>,
) -> Result<AdmittedMaintenance, String> {
    let prepared = tokio::task::spawn_blocking({
        let confirmation = confirmation.clone();
        let container = container.cloned();
        let origin = origin.to_owned();
        move || {
            prepare_maintenance_attempt(
                &confirmation,
                container.as_deref(),
                &origin,
                previous,
                replay,
            )
        }
    })
    .await
    .map_err(|_| "Maintenance request worker stopped".to_owned())
    .and_then(|result| result);
    let (saved_request, mut pending_entry, request, exact_retry) = match prepared {
        Ok(prepared) => (
            Some(prepared.request.clone()),
            prepared.pending,
            prepared.request,
            prepared.exact_retry,
        ),
        Err(error) => return Err(error),
    };
    let route = match confirmation.operation {
        MaintenanceOperation::SeedActors => ACTOR_SEED_PATH,
        MaintenanceOperation::TrainClassifier => CLASSIFIER_TRAIN_PATH,
        MaintenanceOperation::ReindexActors => ACTOR_REINDEX_PATH,
        _ => return Err("Unsupported maintenance operation".into()),
    };
    let outcome = match client.post::<MaintenanceReceipt>(route, &request).await {
        Ok(receipt) if valid_receipt(&receipt, confirmation, &request) => {
            if confirmation.operation == MaintenanceOperation::ReindexActors {
                if let Some(container) = container.cloned() {
                    let origin_for_accept = origin.to_owned();
                    let req_for_accept = request.clone();
                    let job_id = receipt.job_id.clone();
                    if let Ok(Ok(updated)) = tokio::task::spawn_blocking(move || {
                        container.record_accepted_actor_reindex(
                            &origin_for_accept,
                            &req_for_accept,
                            &job_id,
                        )
                    })
                    .await
                    {
                        pending_entry = Some(updated);
                    } else if let Some(entry) = pending_entry.as_mut() {
                        entry.accepted_job_id = Some(receipt.job_id.clone());
                    }
                } else if let Some(entry) = pending_entry.as_mut() {
                    entry.accepted_job_id = Some(receipt.job_id.clone());
                }
            }
            MaintenanceOutcome::Receipt(receipt)
        }
        Ok(_) => MaintenanceOutcome::Failed(
            "Gateway returned an acceptance receipt for a different request".into(),
        ),
        Err(ClientError::Status(status))
            if is_definitive_client_rejection(status)
                && !(confirmation.operation == MaintenanceOperation::ReindexActors
                    && exact_retry
                    && matches!(status, 401 | 403)) =>
        {
            if confirmation.operation == MaintenanceOperation::ReindexActors {
                if let Some(container) = container.cloned() {
                    let origin_for_clear = origin.to_owned();
                    let req_for_clear = request.clone();
                    let _ = tokio::task::spawn_blocking(move || {
                        container.complete_actor_reindex(&origin_for_clear, &req_for_clear)
                    })
                    .await;
                }
                pending_entry = None;
            }
            MaintenanceOutcome::Failed(ClientError::Status(status).to_string())
        }
        Err(error) if confirmation.operation == MaintenanceOperation::ReindexActors => {
            reconcile_uncertain_reindex(client, container, origin, &mut pending_entry, error).await
        }
        Err(error) => MaintenanceOutcome::Failed(error.to_string()),
    };
    Ok(AdmittedMaintenance {
        saved_request,
        saved_context: Some(confirmation.context.clone()),
        pending_entry,
        outcome,
    })
}

enum MaintenanceOutcome {
    Receipt(MaintenanceReceipt),
    Reconciled(JobRequestAssociation),
    Failed(String),
}

struct PreparedMaintenance {
    request: MaintenanceRequest,
    pending: Option<PendingActorReindex>,
    exact_retry: bool,
}

#[cfg(test)]
fn prepare_maintenance_request(
    confirmation: &MaintenanceConfirmation,
    container: Option<&NativeContainer>,
    origin: &str,
    previous: Option<MaintenanceRequest>,
    replay: Option<MaintenanceRequest>,
) -> Result<(MaintenanceRequest, Option<PendingActorReindex>), String> {
    prepare_maintenance_attempt(confirmation, container, origin, previous, replay)
        .map(|prepared| (prepared.request, prepared.pending))
}

fn prepare_maintenance_attempt(
    confirmation: &MaintenanceConfirmation,
    container: Option<&NativeContainer>,
    origin: &str,
    previous: Option<MaintenanceRequest>,
    replay: Option<MaintenanceRequest>,
) -> Result<PreparedMaintenance, String> {
    let context = MutationContext::new(&confirmation.context)
        .with_configuration_revision(confirmation.configuration_revision);
    if confirmation.operation == MaintenanceOperation::ReindexActors {
        let container = container.ok_or_else(|| {
            "Protected workstation storage is unavailable; cannot persist actor index attempt"
                .to_owned()
        })?;
        if let Some(pending) = container
            .pending_actor_reindex(origin)
            .map_err(|error| error.to_string())?
        {
            if let Some(job_id) = &pending.accepted_job_id {
                return Err(format!(
                    "Actor index request {} was already accepted as job {job_id}; recover or clear it before starting another build",
                    pending.request.request_id
                ));
            }
            if let Some(replay_request) = replay {
                if pending.request == replay_request && pending.context == confirmation.context {
                    return Ok(PreparedMaintenance {
                        request: pending.request.clone(),
                        pending: Some(pending),
                        exact_retry: true,
                    });
                }
            }
            if pending.request.context == context
                && pending
                    .request
                    .validate(MaintenanceOperation::ReindexActors)
                    .is_ok()
            {
                return Ok(PreparedMaintenance {
                    request: pending.request.clone(),
                    pending: Some(pending),
                    exact_retry: true,
                });
            }
            return Err(
                "Unresolved actor index attempt exists for an earlier registration or configuration; check or clear it first"
                    .to_owned(),
            );
        }
        let exact_retry = replay.is_some();
        let request = match replay {
            Some(request) if request.context == context => {
                request
                    .validate(MaintenanceOperation::ReindexActors)
                    .map_err(|error| error.to_string())?;
                request
            }
            Some(_) => {
                return Err(
                    "Replay request does not match current workstation registration or configuration"
                        .to_owned(),
                );
            }
            None => {
                let id = platform::request_id().map_err(|error| error.to_string())?;
                MaintenanceRequest::for_reindex(id, context).map_err(|error| error.to_string())?
            }
        };
        let entry = container
            .begin_actor_reindex(origin, &confirmation.context, &request)
            .map_err(|error| error.to_string())?;
        return Ok(PreparedMaintenance {
            request,
            pending: Some(entry),
            exact_retry,
        });
    }
    if let Some(previous) = previous {
        if previous.context == context
            && previous.dataset == confirmation.dataset
            && previous.validate(confirmation.operation).is_ok()
        {
            return Ok(PreparedMaintenance {
                request: previous,
                pending: None,
                exact_retry: replay.is_some(),
            });
        }
    }
    let id = platform::request_id().map_err(|error| error.to_string())?;
    let request = match confirmation.operation {
        MaintenanceOperation::SeedActors => MaintenanceRequest::for_seed(
            id,
            context,
            confirmation
                .dataset
                .clone()
                .ok_or_else(|| "Actor catalog dataset required".to_owned())?,
        )
        .map_err(|error| error.to_string())?,
        MaintenanceOperation::TrainClassifier => MaintenanceRequest::for_training(
            id,
            context,
            confirmation
                .dataset
                .clone()
                .ok_or_else(|| "Training dataset required".to_owned())?,
        )
        .map_err(|error| error.to_string())?,
        _ => return Err("Unsupported maintenance operation".into()),
    };
    Ok(PreparedMaintenance {
        request,
        pending: None,
        exact_retry: replay.is_some(),
    })
}

fn valid_receipt(
    receipt: &MaintenanceReceipt,
    confirmation: &MaintenanceConfirmation,
    request: &MaintenanceRequest,
) -> bool {
    !receipt.job_id.is_empty()
        && receipt.operation == confirmation.operation
        && receipt.resource_scope == ResourceScope::Backend
        && receipt.dataset == request.dataset
        && receipt.origin.request_id == request.request_id
        && receipt.origin.registration_id == confirmation.context.registration_id
        && receipt.origin.workspace_id.is_none()
        && receipt.origin.workstation_id == confirmation.context.workstation_id
        && receipt.origin.deployment_id == confirmation.context.deployment_id
        && receipt.origin.configuration_revision == confirmation.configuration_revision
}

fn valid_reindex_association(
    pending: &PendingActorReindex,
    association: &JobRequestAssociation,
) -> bool {
    !association.job_id.is_empty()
        && association.origin.request_id == pending.request.request_id
        && association.origin.deployment_id == pending.context.deployment_id
        && association.origin.workstation_id == pending.context.workstation_id
        && association.origin.workspace_id.is_none()
}

fn is_definitive_client_rejection(status: u16) -> bool {
    (400..500).contains(&status) && !matches!(status, 408 | 409 | 429)
}

async fn reconcile_uncertain_reindex(
    client: &Client,
    container: Option<&Arc<NativeContainer>>,
    origin: &str,
    pending_entry: &mut Option<PendingActorReindex>,
    post_error: ClientError,
) -> MaintenanceOutcome {
    let Some(pending) = pending_entry.clone() else {
        return MaintenanceOutcome::Failed(post_error.to_string());
    };
    let query = WorkstationRequestDetailQuery::new(
        pending.request.request_id.clone(),
        ACTOR_REINDEX_OPERATION.to_owned(),
        Some(pending.context.registration_id.clone()),
    );
    if query.validate().is_ok() {
        if let Ok(association) = client.request_detail(&query).await {
            if valid_reindex_association(&pending, &association) {
                if let Some(container) = container.cloned() {
                    let origin = origin.to_owned();
                    let context = pending.context.clone();
                    let request = pending.request.clone();
                    let association_for_journal = association.clone();
                    let persisted = tokio::task::spawn_blocking(move || {
                        container.record_accepted_actor_reindex_association(
                            &origin,
                            &context,
                            &request,
                            &association_for_journal,
                        )
                    })
                    .await
                    .ok()
                    .and_then(|result| result.ok())
                    .is_some();
                    if persisted {
                        *pending_entry = None;
                    }
                } else {
                    *pending_entry = None;
                }
                return MaintenanceOutcome::Reconciled(association);
            }
        }
    }
    MaintenanceOutcome::Failed(format!(
        "Actor index delivery outcome uncertain ({post_error}). Use Check request or retry exact request."
    ))
}

async fn upload(client: &Client, selection: LocalDataset) -> Result<DatasetUpload, String> {
    let kind = selection.kind;
    let cancel = Arc::new(AtomicBool::new(false));
    let _cancel = CancelPreparation(Arc::clone(&cancel));
    let permit = preparation_slots()
        .acquire_owned()
        .await
        .map_err(|_| "CSV preparation is unavailable".to_owned())?;
    let (file, sha256, size) = tokio::task::spawn_blocking(move || {
        // The blocking reader retains capacity even if its awaiting UI task is dropped.
        let _permit = permit;
        open_and_hash_cancellable(selection, &cancel)
    })
    .await
    .map_err(|_| "Dataset verification worker stopped".to_owned())??;
    client
        .upload_dataset(kind, &sha256, tokio::fs::File::from_std(file), size)
        .await
        .map_err(|error| error.to_string())
}

fn preparation_slots() -> Arc<tokio::sync::Semaphore> {
    static SLOTS: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    Arc::clone(SLOTS.get_or_init(|| Arc::new(tokio::sync::Semaphore::new(1))))
}

struct CancelPreparation(Arc<AtomicBool>);

impl Drop for CancelPreparation {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

#[cfg(test)]
fn open_and_hash(selection: LocalDataset) -> Result<(std::fs::File, String, u64), String> {
    open_and_hash_cancellable(selection, &AtomicBool::new(false))
}

fn open_and_hash_cancellable(
    selection: LocalDataset,
    cancel: &AtomicBool,
) -> Result<(std::fs::File, String, u64), String> {
    use std::io::Seek;
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::fs::OpenOptionsExt;
    check_preparation_cancelled(cancel)?;
    let metadata = std::fs::symlink_metadata(&selection.path)
        .map_err(|_| "Cannot inspect the selected CSV".to_owned())?;
    let limit = dataset_limit(selection.kind);
    if !selection.path.is_absolute()
        || !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() == 0
        || metadata.len() > limit
    {
        return Err("Select a non-empty regular CSV within its purpose's byte limit; symbolic links are rejected".into());
    }
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&selection.path)
        .map_err(|_| "Cannot open the selected CSV".to_owned())?;
    let opened = file
        .metadata()
        .map_err(|_| "Cannot inspect the opened CSV".to_owned())?;
    if metadata.dev() != opened.dev()
        || metadata.ino() != opened.ino()
        || metadata.len() != opened.len()
    {
        return Err("Selected CSV identity changed before it was opened".into());
    }
    fleetworks_contracts::dataset_upload::validate_csv_header(&mut file, selection.kind)
        .map_err(|error| error.to_string())?;
    check_preparation_cancelled(cancel)?;
    file.rewind()
        .map_err(|_| "Cannot rewind the validated CSV".to_owned())?;
    let (sha256, count) = hash_selected_bytes(&mut file, limit, opened.len(), cancel)?;
    file.rewind()
        .map_err(|_| "Cannot rewind the selected CSV".to_owned())?;
    Ok((file, sha256, count))
}

fn check_preparation_cancelled(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        return Err("CSV preparation was cancelled".into());
    }
    Ok(())
}

fn hash_selected_bytes(
    file: &mut impl std::io::Read,
    limit: u64,
    expected_size: u64,
    cancel: &AtomicBool,
) -> Result<(String, u64), String> {
    let mut sha = Sha256::new();
    let mut count = 0_u64;
    let mut bytes = vec![0; 64 * 1024];
    loop {
        check_preparation_cancelled(cancel)?;
        let read = file
            .read(&mut bytes)
            .map_err(|_| "Cannot read the selected CSV".to_owned())?;
        if read == 0 {
            break;
        }
        count += read as u64;
        if count > limit {
            return Err("Selected CSV grew beyond its byte limit".into());
        }
        sha.update(&bytes[..read]);
    }
    if count != expected_size {
        return Err("Selected CSV size changed while hashing".into());
    }
    let sha256 = format!("{:x}", sha.finalize());
    Ok((sha256, count))
}

pub fn dataset_kind_label(kind: DatasetKind) -> &'static str {
    match kind {
        DatasetKind::ActorCatalog => "actor catalog",
        DatasetKind::ClassifierTraining => "Natural training corpus",
        _ => "unsupported dataset",
    }
}
pub fn dataset_limit(kind: DatasetKind) -> u64 {
    match kind {
        DatasetKind::ActorCatalog => 256 * 1024 * 1024,
        DatasetKind::ClassifierTraining => 128 * 1024 * 1024,
        _ => 0,
    }
}
fn dataset_kind(operation: MaintenanceOperation) -> Option<DatasetKind> {
    match operation {
        MaintenanceOperation::SeedActors => Some(DatasetKind::ActorCatalog),
        MaintenanceOperation::TrainClassifier => Some(DatasetKind::ClassifierTraining),
        _ => None,
    }
}
fn action_id(operation: MaintenanceOperation) -> Option<ReadinessActionId> {
    match operation {
        MaintenanceOperation::SeedActors => Some(ReadinessActionId::SeedActors),
        MaintenanceOperation::TrainClassifier => Some(ReadinessActionId::TrainClassifier),
        MaintenanceOperation::ReindexActors => Some(ReadinessActionId::RebuildActorIndex),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
