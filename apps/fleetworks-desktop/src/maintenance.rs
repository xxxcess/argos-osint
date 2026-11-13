//! Explicit dataset transfer and confirmed backend maintenance admission.

use std::path::PathBuf;

use fleetworks_contracts::context::MutationContext;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::context::RegisteredContext;
use fleetworks_contracts::context::ResourceScope;
use fleetworks_contracts::readiness::ConfiguredMaintenanceInput;
use fleetworks_contracts::readiness::DatasetKind;
use fleetworks_contracts::readiness::DatasetRef;
use fleetworks_contracts::readiness::MaintenanceOperation;
use fleetworks_contracts::readiness::MaintenanceReceipt;
use fleetworks_contracts::readiness::MaintenanceRequest;
use fleetworks_contracts::readiness::ReadinessActionId;
use fleetworks_contracts::readiness::ACTOR_SEED_PATH;
use fleetworks_contracts::readiness::CLASSIFIER_TRAIN_PATH;
use fleetworks_contracts::search::ACTOR_REINDEX_PATH;
use fleetworks_contracts::Client;
use fleetworks_contracts::DatasetUpload;
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
    pub(crate) last_request: Option<MaintenanceRequest>,
    pub(crate) upload_revision: u64,
    pub(crate) datasets_revision: u64,
    pub(crate) operation_revision: u64,
    pub(crate) upload_request: Option<RequestTask>,
    pub(crate) datasets_request: Option<RequestTask>,
    pub(crate) operation_request: Option<RequestTask>,
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
            last_request: None,
            upload_revision: 0,
            datasets_revision: 0,
            operation_revision: 0,
            upload_request: None,
            datasets_request: None,
            operation_request: None,
        }
    }
}

impl Maintenance {
    pub(crate) fn invalidate_origin(&mut self) {
        self.confirmation = None;
        self.last_request = None;
        self.operation_request = None;
        self.operation_revision = self.operation_revision.wrapping_add(1);
        self.receipt = Load::Idle;
    }
    pub(crate) fn cancel_requests(&mut self) {
        self.upload_request = None;
        self.datasets_request = None;
        self.operation_request = None;
    }
}

impl State {
    pub fn choose_dataset(&mut self, kind: DatasetKind, tx: &mut Tx) {
        if !self.permission(Permission::MaintainBackend) {
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
                    Ok(Some(path)) => state.select_local_dataset(path, kind),
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
        self.maintenance.confirmation = None;
        self.maintenance.message = Some(
            "Review the selected CSV purpose and size limit, then upload its bytes to the backend"
                .into(),
        );
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
        self.maintenance.upload = Load::Loading;
        self.maintenance.upload_revision = self.maintenance.upload_revision.wrapping_add(1);
        self.maintenance.upload_request = services.upload_dataset(
            client.clone(),
            selection,
            self.connection.epoch,
            self.maintenance.upload_revision,
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
        self.maintenance.datasets = Load::Loading;
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
                let Some(input) = self.configured_maintenance_input(operation) else {
                    return Some(
                        "This operation has no unambiguous server-configured input".into(),
                    );
                };
                if Some(input.configuration_revision) != self.configuration_revision() {
                    return Some("Configured operation inputs changed; refresh backend checks and configuration".into());
                }
                if let Some(kind) = dataset_kind(operation) {
                    if input
                        .dataset
                        .as_ref()
                        .is_none_or(|dataset| dataset.kind != kind || dataset.validate().is_err())
                    {
                        return Some("A validated server-configured dataset is required".into());
                    }
                } else if input.dataset.is_some() {
                    return Some("The configured operation input is inconsistent".into());
                }
                if matches!(self.maintenance.receipt, Load::Loading) {
                    return Some("A maintenance request is awaiting its acceptance receipt".into());
                }
                return None;
            }
        }
        Some("This gateway has not advertised a durable consumer for this operation".into())
    }

    fn configured_maintenance_input(
        &self,
        operation: MaintenanceOperation,
    ) -> Option<&ConfiguredMaintenanceInput> {
        let Load::Ready(readiness) = &self.readiness else {
            return None;
        };
        let mut matches = readiness
            .configured_inputs
            .iter()
            .filter(|input| input.operation == operation);
        let input = matches.next()?;
        matches.next().is_none().then_some(input)
    }

    pub fn preview_maintenance(&mut self, operation: MaintenanceOperation) {
        if let Some(reason) = self.maintenance_disabled_reason(operation) {
            self.maintenance.message = Some(reason);
            return;
        }
        let (Some(context), Some(input)) = (
            &self.registration.context,
            self.configured_maintenance_input(operation),
        ) else {
            return;
        };
        self.maintenance.confirmation = Some(MaintenanceConfirmation {
            operation,
            context: context.clone(),
            origin_revision: self.origin_revision,
            configuration_revision: input.configuration_revision,
            dataset: input.dataset.clone(),
        });
        self.reveal_control("maintenance-confirmation");
    }

    pub fn confirm_maintenance(&mut self) {
        let Some(confirmation) = self.maintenance.confirmation.take() else {
            return;
        };
        if confirmation.origin_revision != self.origin_revision
            || self.registration.context.as_ref() != Some(&confirmation.context)
            || self.configuration_revision() != Some(confirmation.configuration_revision)
            || self
                .configured_maintenance_input(confirmation.operation)
                .is_none_or(|input| {
                    input.configuration_revision != confirmation.configuration_revision
                        || input.dataset != confirmation.dataset
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
        self.record(Intent::Maintenance {
            operation: confirmation.operation,
        });
        if self.fixture {
            return;
        }
        let (Some(services), Some(client)) = (&self.services, &self.client) else {
            return;
        };
        let client = match client.with_registered_context(&confirmation.context) {
            Ok(client) => client,
            Err(error) => {
                self.maintenance.message = Some(error.to_string());
                return;
            }
        };
        self.maintenance.receipt = Load::Loading;
        self.maintenance.operation_revision = self.maintenance.operation_revision.wrapping_add(1);
        self.maintenance.operation_request = services.maintenance(
            client,
            confirmation,
            self.maintenance.last_request.clone(),
            self.connection.epoch,
            self.maintenance.operation_revision,
        );
    }
}

impl GatewayEffects {
    fn upload_dataset(
        &self,
        client: Client,
        selection: LocalDataset,
        epoch: u64,
        revision: u64,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let task = self.runtime.spawn(async move {
            let result = upload(&client, selection).await;
            ui.update(move |state, _| {
                if state.connection.epoch != epoch || state.maintenance.upload_revision != revision
                {
                    return;
                }
                state.maintenance.upload_request = None;
                state.maintenance.upload = match result {
                    Ok(upload) => {
                        state.maintenance.selected_dataset = Some(upload.dataset.clone());
                        state.maintenance.confirmation = None;
                        state.maintenance.last_request = None;
                        Load::Ready(upload)
                    }
                    Err(error) => Load::Failed(error),
                };
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
                state.maintenance.datasets = match result {
                    Ok(list)
                        if list.rows.len() <= 100
                            && list.rows.iter().all(|dataset| dataset.validate().is_ok()) =>
                    {
                        Load::Ready(list)
                    }
                    Ok(_) => Load::Failed("Gateway returned an invalid dataset inventory".into()),
                    Err(error) => Load::Failed(error.to_string()),
                };
            });
        });
        Some(RequestTask(task.abort_handle()))
    }

    fn maintenance(
        &self,
        client: Client,
        confirmation: MaintenanceConfirmation,
        previous: Option<MaintenanceRequest>,
        epoch: u64,
        revision: u64,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let task = self.runtime.spawn(async move {
            let origin_revision = confirmation.origin_revision;
            let prepared = tokio::task::spawn_blocking({
                let confirmation = confirmation.clone();
                move || {
                    let context = MutationContext::new(&confirmation.context)
                        .with_configuration_revision(confirmation.configuration_revision);
                    if let Some(previous) = previous {
                        if previous.context == context
                            && previous.dataset == confirmation.dataset
                            && previous.validate(confirmation.operation).is_ok()
                        {
                            return Ok(previous);
                        }
                    }
                    let id = platform::request_id().map_err(|error| error.to_string())?;
                    match confirmation.operation {
                        MaintenanceOperation::SeedActors => MaintenanceRequest::for_seed(
                            id,
                            context,
                            confirmation
                                .dataset
                                .ok_or_else(|| "Actor catalog dataset required".to_owned())?,
                        )
                        .map_err(|error| error.to_string()),
                        MaintenanceOperation::TrainClassifier => MaintenanceRequest::for_training(
                            id,
                            context,
                            confirmation
                                .dataset
                                .ok_or_else(|| "Training dataset required".to_owned())?,
                        )
                        .map_err(|error| error.to_string()),
                        MaintenanceOperation::ReindexActors => {
                            MaintenanceRequest::for_reindex(id, context)
                                .map_err(|error| error.to_string())
                        }
                        _ => Err("Unsupported maintenance operation".into()),
                    }
                }
            })
            .await
            .map_err(|_| "Maintenance request worker stopped".to_owned())
            .and_then(|result| result);
            let saved_request = prepared.as_ref().ok().cloned();
            let result = async {
                let request = prepared?;
                let route = match confirmation.operation {
                    MaintenanceOperation::SeedActors => ACTOR_SEED_PATH,
                    MaintenanceOperation::TrainClassifier => CLASSIFIER_TRAIN_PATH,
                    MaintenanceOperation::ReindexActors => ACTOR_REINDEX_PATH,
                    _ => return Err("Unsupported maintenance operation".to_owned()),
                };
                let receipt: MaintenanceReceipt = client
                    .post(route, &request)
                    .await
                    .map_err(|error| error.to_string())?;
                if receipt.job_id.is_empty()
                    || receipt.operation != confirmation.operation
                    || receipt.resource_scope != ResourceScope::Backend
                    || receipt.dataset != request.dataset
                    || receipt.origin.request_id != request.request_id
                    || receipt.origin.registration_id != confirmation.context.registration_id
                    || receipt.origin.workspace_id.is_some()
                    || receipt.origin.workstation_id != confirmation.context.workstation_id
                    || receipt.origin.deployment_id != confirmation.context.deployment_id
                    || receipt.origin.configuration_revision != confirmation.configuration_revision
                {
                    return Err(
                        "Gateway returned an acceptance receipt for a different request".into(),
                    );
                }
                Ok(receipt)
            }
            .await;
            ui.update(move |state, _| {
                if state.connection.epoch != epoch
                    || state.origin_revision != origin_revision
                    || state.maintenance.operation_revision != revision
                {
                    return;
                }
                state.maintenance.operation_request = None;
                state.maintenance.last_request = saved_request;
                state.maintenance.receipt = match result {
                    Ok(receipt) => Load::Ready(receipt),
                    Err(error) => Load::Failed(error),
                };
            });
        });
        Some(RequestTask(task.abort_handle()))
    }
}

async fn upload(client: &Client, selection: LocalDataset) -> Result<DatasetUpload, String> {
    let kind = selection.kind;
    let (file, sha256, size) = tokio::task::spawn_blocking(move || open_and_hash(selection))
        .await
        .map_err(|_| "Dataset verification worker stopped".to_owned())??;
    client
        .upload_dataset(kind, &sha256, tokio::fs::File::from_std(file), size)
        .await
        .map_err(|error| error.to_string())
}

fn open_and_hash(selection: LocalDataset) -> Result<(std::fs::File, String, u64), String> {
    use std::io::Read;
    use std::io::Seek;
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::fs::OpenOptionsExt;
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
    let mut sha = Sha256::new();
    let mut count = 0_u64;
    let mut bytes = vec![0; 64 * 1024];
    loop {
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
    if count != opened.len() {
        return Err("Selected CSV size changed while hashing".into());
    }
    let sha256 = format!("{:x}", sha.finalize());
    file.rewind()
        .map_err(|_| "Cannot rewind the selected CSV".to_owned())?;
    Ok((file, sha256, count))
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
