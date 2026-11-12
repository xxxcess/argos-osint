//! Explicit current-format profile byte transfer; paths stay on this Mac.

use std::path::PathBuf;

use fleetworks_contracts::context::Permission;
use fleetworks_contracts::Client;
use fleetworks_contracts::ClientError;
use rune::FileDialogOptions;
use rune::Tx;

use crate::app::Intent;
use crate::app::Load;
use crate::app::State;
use crate::backend::ReadRequest;
use crate::connection::GatewayEffects;
use crate::connection::RequestTask;

const LIMIT: u64 = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransferAction {
    Export,
    Import,
}

/// This may contain credentials. Do not derive Debug or put it in intents,
/// traces, view trees, synthetic snapshots or user-visible error strings.
pub enum TransferConfirmation {
    Export {
        expected_revision: u64,
        origin_revision: u64,
    },
    Import {
        expected_revision: u64,
        origin_revision: u64,
        file_name: String,
        bytes: Vec<u8>,
    },
}

impl TransferConfirmation {
    pub fn action(&self) -> TransferAction {
        match self {
            Self::Export { .. } => TransferAction::Export,
            Self::Import { .. } => TransferAction::Import,
        }
    }
    pub fn expected_revision(&self) -> u64 {
        match self {
            Self::Export {
                expected_revision, ..
            }
            | Self::Import {
                expected_revision, ..
            } => *expected_revision,
        }
    }
    fn origin_revision(&self) -> u64 {
        match self {
            Self::Export {
                origin_revision, ..
            }
            | Self::Import {
                origin_revision, ..
            } => *origin_revision,
        }
    }
    pub fn summary(&self) -> String {
        match self { Self::Export { .. } => "Export this backend's current profile, including credentials, to a new private file on this Mac.".into(),
            Self::Import { file_name, .. } => format!("Import current-format profile {file_name}. This explicitly replaces the backend profile and credentials after full server validation.") }
    }
}

#[derive(Default)]
pub struct Transfer {
    pub confirmation: Option<TransferConfirmation>,
    pub result: Load<String>,
    pub(crate) request: Option<RequestTask>,
    pub(crate) revision: u64,
}

impl State {
    fn can_transfer(&self) -> bool {
        self.permission(Permission::ManageConfiguration)
            && matches!(&self.backend.profile, Load::Ready(profile) if profile.configuration_format == 1 && profile.portable_schema_version == 4)
            && !matches!(self.transfer.result, Load::Loading)
    }

    pub fn preview_profile_export(&mut self) {
        if !self.can_transfer() {
            return;
        }
        let Some(expected_revision) = self.configuration_revision() else {
            return;
        };
        self.transfer.confirmation = Some(TransferConfirmation::Export {
            expected_revision,
            origin_revision: self.origin_revision,
        });
        self.reveal_control("profile-transfer-confirmation");
    }

    pub fn choose_profile_import(&mut self, tx: &mut Tx) {
        if !self.can_transfer() {
            return;
        }
        let epoch = self.connection.epoch;
        tx.open_file(
            FileDialogOptions {
                title: "Choose a current-format profile export".into(),
                ..Default::default()
            }
            .filter("JSON", &["json"]),
            move |state: &mut State, result, _| {
                if state.connection.epoch != epoch {
                    return;
                }
                match result {
                    Ok(Some(path)) => state.prepare_profile_import(path),
                    Ok(None) => {}
                    Err(_) => {
                        state.transfer.result =
                            Load::Failed("Profile chooser could not open a file".into())
                    }
                }
            },
        );
    }

    pub fn prepare_profile_import(&mut self, path: PathBuf) {
        if !self.can_transfer() {
            return;
        }
        let Some(expected_revision) = self.configuration_revision() else {
            return;
        };
        self.record(Intent::PrepareProfileImport);
        if self.fixture {
            return;
        }
        let Some(services) = &self.services else {
            return;
        };
        self.transfer.result = Load::Loading;
        self.transfer.revision = self.transfer.revision.wrapping_add(1);
        self.transfer.request = services.prepare_profile_import(
            path,
            expected_revision,
            self.connection.epoch,
            self.origin_revision,
            self.transfer.revision,
        );
    }

    pub fn confirm_profile_transfer(&mut self, tx: &mut Tx) {
        let Some(confirmation) = self.transfer.confirmation.take() else {
            return;
        };
        if !self.can_transfer()
            || confirmation.origin_revision() != self.origin_revision
            || self.configuration_revision() != Some(confirmation.expected_revision())
        {
            self.transfer.result = Load::Failed(
                "Context or configuration changed; refresh and review profile transfer again"
                    .into(),
            );
            return;
        }
        self.record(Intent::ProfileTransfer {
            action: confirmation.action(),
        });
        if self.fixture {
            return;
        }
        match confirmation {
            TransferConfirmation::Export {
                expected_revision,
                origin_revision,
            } => {
                let epoch = self.connection.epoch;
                tx.save_file(
                    FileDialogOptions {
                        title: "Export profile with credentials to a new private file".into(),
                        file_name: Some("fleetworks-profile.json".into()),
                        ..Default::default()
                    }
                    .filter("JSON", &["json"]),
                    move |state: &mut State, result, _| {
                        if state.connection.epoch != epoch
                            || state.origin_revision != origin_revision
                            || state.configuration_revision() != Some(expected_revision)
                        {
                            return;
                        }
                        match result {
                            Ok(Some(path)) => state.export_profile(path, expected_revision),
                            Ok(None) => {}
                            Err(_) => {
                                state.transfer.result =
                                    Load::Failed("Profile export destination chooser failed".into())
                            }
                        }
                    },
                );
            }
            TransferConfirmation::Import {
                expected_revision,
                bytes,
                ..
            } => {
                let (Some(services), Some(client)) = (&self.services, &self.client) else {
                    return;
                };
                self.transfer.result = Load::Loading;
                self.transfer.revision = self.transfer.revision.wrapping_add(1);
                self.transfer.request = services.import_profile(
                    client.clone(),
                    expected_revision,
                    bytes,
                    self.connection.epoch,
                    self.transfer.revision,
                );
            }
        }
    }

    fn export_profile(&mut self, path: PathBuf, expected_revision: u64) {
        if !self.permission(Permission::ManageConfiguration) {
            return;
        }
        let (Some(services), Some(client)) = (&self.services, &self.client) else {
            return;
        };
        self.transfer.result = Load::Loading;
        self.transfer.revision = self.transfer.revision.wrapping_add(1);
        self.transfer.request = services.export_profile(
            client.clone(),
            path,
            expected_revision,
            self.connection.epoch,
            self.transfer.revision,
        );
    }
}

impl GatewayEffects {
    fn prepare_profile_import(
        &self,
        path: PathBuf,
        expected_revision: u64,
        epoch: u64,
        origin_revision: u64,
        revision: u64,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let task = self.runtime.spawn(async move {
            let result = tokio::task::spawn_blocking(move || read_import(path)).await
                .map_err(|_| "Profile import reader stopped".to_owned()).and_then(|result| result);
            ui.update(move |state, tx| {
                if state.connection.epoch != epoch || state.transfer.revision != revision { return; }
                state.transfer.request = None;
                if state.origin_revision != origin_revision || state.configuration_revision() != Some(expected_revision) {
                    state.transfer.result = Load::Failed("Context changed while reading the profile; choose it again after reviewing the backend revision".into()); return;
                }
                match result {
                    Ok((file_name, bytes)) => { state.transfer.result = Load::Idle;
                        state.transfer.confirmation = Some(TransferConfirmation::Import { expected_revision, origin_revision, file_name, bytes });
                        tx.scroll_into_view("profile-transfer-confirmation"); }
                    Err(error) => state.transfer.result = Load::Failed(error),
                }
            });
        });
        Some(RequestTask(task.abort_handle()))
    }

    fn export_profile(
        &self,
        client: Client,
        path: PathBuf,
        expected_revision: u64,
        epoch: u64,
        revision: u64,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let task = self.runtime.spawn(async move {
            let result: Result<String, String> = async {
                let bytes = client
                    .export_configuration(expected_revision)
                    .await
                    .map_err(|error| error.to_string())?;
                tokio::task::spawn_blocking(move || write_private_export(path, bytes))
                    .await
                    .map_err(|_| "Profile export writer stopped".to_owned())??;
                Ok("Created a new private profile export (mode 0600)".to_owned())
            }
            .await;
            ui.update(move |state, _| {
                if state.connection.epoch != epoch || state.transfer.revision != revision {
                    return;
                }
                state.transfer.request = None;
                state.transfer.result = match result {
                    Ok(message) => Load::Ready(message),
                    Err(error) => Load::Failed(error),
                };
            });
        });
        Some(RequestTask(task.abort_handle()))
    }

    fn import_profile(
        &self,
        client: Client,
        expected_revision: u64,
        bytes: Vec<u8>,
        epoch: u64,
        revision: u64,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let task = self.runtime.spawn(async move {
            let result = client.import_configuration(expected_revision, bytes).await;
            ui.update(move |state, _| {
                if state.connection.epoch != epoch || state.transfer.revision != revision {
                    return;
                }
                state.transfer.request = None;
                state.transfer.result = match result {
                    Ok(receipt)
                        if receipt.saved && receipt.configuration_revision > expected_revision =>
                    {
                        state.configuration.confirmation = None;
                        state.maintenance.confirmation = None;
                        state.submissions.invalidate_confirmations();
                        state.classifier_activation.invalidate_confirmation();
                        Load::Ready(format!(
                            "Current-format profile imported at backend revision {}",
                            receipt.configuration_revision
                        ))
                    }
                    Ok(_) => {
                        Load::Failed("Gateway returned an invalid profile import receipt".into())
                    }
                    Err(ClientError::Status(409)) => Load::Failed(
                        "Backend revision changed; refresh and review import again".into(),
                    ),
                    Err(error) => Load::Failed(error.to_string()),
                };
                state.backend_read(ReadRequest::Profile);
                state.backend_read(ReadRequest::Models);
            });
        });
        Some(RequestTask(task.abort_handle()))
    }
}

fn read_import(path: PathBuf) -> Result<(String, Vec<u8>), String> {
    use std::io::Read;
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::fs::OpenOptionsExt;
    let metadata = std::fs::symlink_metadata(&path)
        .map_err(|_| "Cannot inspect selected profile".to_owned())?;
    if !path.is_absolute()
        || !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() == 0
        || metadata.len() > LIMIT
    {
        return Err("Select a non-empty regular current-format profile within 2 MiB; symbolic links are rejected".into());
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
        .map_err(|_| "Cannot open selected profile".to_owned())?;
    let opened = file
        .metadata()
        .map_err(|_| "Cannot inspect opened profile".to_owned())?;
    if metadata.dev() != opened.dev()
        || metadata.ino() != opened.ino()
        || metadata.len() != opened.len()
    {
        return Err("Profile identity changed before reading".into());
    }
    let mut bytes = Vec::new();
    file.take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read selected profile".to_owned())?;
    if bytes.len() as u64 > LIMIT || bytes.len() as u64 != opened.len() {
        return Err("Profile size changed while reading".into());
    }
    let document: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| "Selected profile is not valid JSON".to_owned())?;
    if document
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        != Some(4)
    {
        return Err("Only explicit current-format schema 4 profile exports can be imported".into());
    }
    let file_name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    Ok((file_name, bytes))
}

fn write_private_export(path: PathBuf, bytes: Vec<u8>) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if !path.is_absolute() || bytes.is_empty() || bytes.len() as u64 > LIMIT {
        return Err("Export destination or byte bound is invalid".into());
    }
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(path)
        .map_err(|_| "Choose a new export destination; existing files and symbolic links are never overwritten".to_owned())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| {
            "Private export write was incomplete; review its destination before retrying".to_owned()
        })
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn private_export_never_overwrites_existing_or_linked_data() {
        let temporary = tempfile::tempdir().expect("owned synthetic transfer directory");
        let path = temporary.path().join("profile.json");
        super::write_private_export(path.clone(), br#"{"schema_version":4}"#.to_vec())
            .expect("new private export");
        assert_eq!(
            std::fs::metadata(&path)
                .expect("created file")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let original = std::fs::read(&path).expect("original export");
        assert!(super::write_private_export(path.clone(), b"replacement".to_vec()).is_err());
        assert_eq!(std::fs::read(&path).expect("preserved original"), original);
        let alias = temporary.path().join("alias.json");
        std::os::unix::fs::symlink(&path, &alias).expect("synthetic alias");
        assert!(super::write_private_export(alias, b"replacement".to_vec()).is_err());
        assert_eq!(
            std::fs::read(path).expect("preserved linked target"),
            original
        );
    }

    #[test]
    fn import_preflight_requires_current_regular_bounded_bytes() {
        let temporary = tempfile::tempdir().expect("owned synthetic transfer directory");
        let path = temporary.path().join("profile.json");
        std::fs::write(&path, br#"{"schema_version":3}"#).expect("synthetic legacy bytes");
        assert!(super::read_import(path.clone()).is_err());
        std::fs::write(&path, br#"{"schema_version":4}"#).expect("synthetic current envelope");
        assert!(
            super::read_import(path.clone()).is_ok(),
            "backend still validates the full current document at import"
        );
        let alias = temporary.path().join("alias.json");
        std::os::unix::fs::symlink(&path, &alias).expect("synthetic alias");
        assert!(super::read_import(alias).is_err());
        std::fs::write(&path, vec![b' '; super::LIMIT as usize + 1])
            .expect("synthetic oversize bytes");
        assert!(super::read_import(path).is_err());
    }
}
