//! Named local gateway profiles and a separate non-promoting Test operation.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use fleetworks_workstation::native::ConnectionProfilesSnapshot;
use fleetworks_workstation::native::CredentialReference;
use fleetworks_workstation::native::GatewayProfile;
use fleetworks_workstation::native::GatewayProfileInput;
use fleetworks_workstation::native::NativeContainer;
use rune::FileDialogOptions;
use rune::TextEdit;
use rune::Tx;

use crate::app::Intent;
use crate::app::Load;
use crate::app::State;
use crate::connection::ConnectionCredential;
use crate::connection::ConnectionTestFailure;
use crate::connection::ConnectionTestSummary;
use crate::connection::GatewayEffects;
use crate::connection::RequestTask;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProfileAction {
    Refresh,
    New,
    Edit,
    Select(String),
    Save,
    Test,
    Connect,
}

#[derive(Default)]
pub struct ConnectionProfiles {
    pub catalog: Load<ConnectionProfilesSnapshot>,
    pub name: TextEdit,
    pub selected_id: Option<String>,
    pub credential: Option<CredentialReference>,
    pub editing: bool,
    pub busy: bool,
    pub test: Load<ConnectionTestSummary>,
    pub test_failure: Option<ConnectionTestFailure>,
    pub confirmation: Option<ProfileAction>,
    pub message: Option<String>,
    pub(crate) launch_override: bool,
    pub(crate) operator_token_override: bool,
    pub(crate) initial_selection_applied: bool,
    pub(crate) draft_revision: u64,
    revision: u64,
    test_revision: u64,
    request: Option<RequestTask>,
    test_request: Option<RequestTask>,
    credential_request: Option<RequestTask>,
    pending_install: bool,
}

impl ConnectionProfiles {
    pub(crate) fn initial() -> Self {
        Self {
            editing: true,
            ..Self::default()
        }
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn test_revision(&self) -> u64 {
        self.test_revision
    }
    pub(crate) fn cancel_tests(&mut self) {
        self.test_request = None;
        self.credential_request = None;
        self.test_revision = self.test_revision.wrapping_add(1);
        self.test = Load::Idle;
        self.test_failure = None;
    }
}

impl State {
    /// Restore local names/editor selection only, never a network connection.
    pub(crate) fn initialize_connection_profiles(&mut self) {
        if self.fixture || self.connection_profiles.busy {
            return;
        }
        let (Some(services), Some(container)) = (&self.services, &self.workspaces.container) else {
            return;
        };
        self.connection_profiles.busy = true;
        self.connection_profiles.revision = self.connection_profiles.revision.wrapping_add(1);
        self.connection_profiles.request =
            services.profile_catalog(container.clone(), None, self.connection_profiles.revision);
    }

    fn has_backend_edits(&self) -> bool {
        self.configuration.draft_revision != self.configuration.saved_draft_revision
            || self.configuration.confirmation.is_some()
            || self.transfer.confirmation.is_some()
            || self.maintenance.confirmation.is_some()
            || self.submissions.atlas.confirmation.is_some()
            || self.submissions.tools.confirmation.is_some()
            || self.submissions.atlas.delivery_uncertain
            || self.submissions.tools.delivery_uncertain
            || self.classifier_activation.uncertain
            || self.classifier_activation.confirmation.is_some()
            || self.backend.tool_inputs.text != "{}"
    }

    pub fn profile_action(&mut self, action: ProfileAction) {
        if self.connection_profiles.busy {
            return;
        }
        if matches!(
            action,
            ProfileAction::New
                | ProfileAction::Edit
                | ProfileAction::Select(_)
                | ProfileAction::Connect
        ) && self.has_backend_edits()
        {
            self.connection_profiles.confirmation = Some(action);
            self.reveal_control("connection-profile-switch-confirmation");
            return;
        }
        self.apply_profile_action(action);
    }
    pub fn confirm_profile_switch(&mut self) {
        let Some(action) = self.connection_profiles.confirmation.take() else {
            return;
        };
        self.apply_profile_action(action);
    }
    fn apply_profile_action(&mut self, action: ProfileAction) {
        match action {
            ProfileAction::New => {
                self.disconnect();
                self.connection_profiles.selected_id = None;
                self.connection_profiles.name = TextEdit::new("");
                self.connection.origin = TextEdit::new(fleetworks_contracts::DEFAULT_API_ORIGIN);
                self.connection.token_file = TextEdit::new("");
                self.connection_profiles.credential = None;
                self.connection_profiles.editing = true;
                self.profile_editor_changed(false);
            }
            ProfileAction::Edit => {
                self.disconnect();
                self.connection_profiles.editing = true;
                self.profile_editor_changed(false);
            }
            ProfileAction::Select(id) => {
                let Load::Ready(catalog) = &self.connection_profiles.catalog else {
                    return;
                };
                let Some(profile) = catalog
                    .profiles
                    .iter()
                    .find(|profile| profile.id == id)
                    .cloned()
                else {
                    return;
                };
                let expected = catalog.revision;
                self.disconnect();
                self.install_profile(&profile);
                self.persist_profile_selection(id, expected);
            }
            ProfileAction::Refresh => self.refresh_profiles(),
            ProfileAction::Save => self.save_profile(),
            ProfileAction::Test => self.test_profile(),
            ProfileAction::Connect => self.connect(),
        }
    }

    fn install_profile(&mut self, profile: &GatewayProfile) {
        self.connection_profiles.selected_id = Some(profile.id.clone());
        self.connection_profiles.name = TextEdit::new(&profile.name);
        self.connection.origin = TextEdit::new(&profile.origin);
        self.connection.token_file = TextEdit::new(profile.credential.path().to_string_lossy());
        self.connection_profiles.credential = Some(profile.credential.clone());
        self.connection_profiles.editing = false;
        self.connection_profiles.draft_revision =
            self.connection_profiles.draft_revision.wrapping_add(1);
    }

    /// Editing the destination cannot carry an old origin's credential binding.
    pub fn profile_editor_changed(&mut self, origin_changed: bool) {
        if origin_changed {
            self.disconnect();
        }
        self.connection_profiles.cancel_tests();
        self.connection_profiles.confirmation = None;
        self.connection_profiles.draft_revision =
            self.connection_profiles.draft_revision.wrapping_add(1);
        if origin_changed {
            self.connection_profiles.operator_token_override = false;
            self.connection.token_file = TextEdit::new("");
            self.connection_profiles.credential = None;
        }
        self.connection_profiles.message = None;
        self.connection_profiles.initial_selection_applied = true;
    }
    pub fn profile_credential_edited(&mut self) {
        self.disconnect();
        self.connection_profiles.credential = None;
        self.profile_editor_changed(false);
    }
    pub fn choose_profile_credential(&mut self, tx: &mut Tx) {
        if !self.connection_profiles.editing || self.connection_profiles.busy || self.connected() {
            return;
        }
        if self.fixture {
            self.record(Intent::ChooseConnectionCredential);
            return;
        }
        let epoch = self.connection.epoch;
        tx.open_file(
            FileDialogOptions {
                title: "Select a protected gateway token file".into(),
                ..FileDialogOptions::default()
            },
            move |state: &mut State, result, _| {
                if state.connection.epoch != epoch {
                    return;
                }
                match result {
                    Ok(Some(path)) => state.select_profile_credential(path),
                    Ok(None) => {}
                    Err(_) => {
                        state.connection_profiles.message =
                            Some("Credential file chooser failed".into())
                    }
                }
            },
        );
    }
    pub fn select_profile_credential(&mut self, path: PathBuf) {
        self.connection.token_file = TextEdit::new(path.to_string_lossy());
        self.profile_credential_edited();
        self.record(Intent::ConnectionCredentialSelected);
        if self.fixture {
            return;
        }
        let Some(services) = &self.services else {
            return;
        };
        self.connection_profiles.credential_request = services.capture_profile_credential(
            path,
            self.connection.origin.text.clone(),
            self.connection.epoch,
            self.connection_profiles.draft_revision,
        );
    }

    pub(crate) fn editor_credential(&self) -> ConnectionCredential {
        if self.connection_profiles.operator_token_override {
            return ConnectionCredential::Explicit(PathBuf::from(&self.connection.token_file.text));
        }
        match &self.workspaces.container {
            Some(container) if !self.fixture => {
                ConnectionCredential::Provisioned(container.clone())
            }
            _ if self.fixture => match &self.connection_profiles.credential {
                Some(reference) => ConnectionCredential::Reference(reference.clone()),
                None => ConnectionCredential::None,
            },
            _ => ConnectionCredential::Unavailable,
        }
    }

    fn test_profile(&mut self) {
        self.connection_profiles.cancel_tests();
        self.connection_profiles.test = Load::Loading;
        let revision = self.connection_profiles.test_revision;
        self.record(Intent::ConnectionProfile {
            action: ProfileAction::Test,
        });
        if self.fixture {
            return;
        }
        if let Some(services) = &self.services {
            self.connection_profiles.test_request = services.test_profile(
                self.connection.origin.text.clone(),
                self.editor_credential(),
                self.connection.epoch,
                self.connection_profiles.draft_revision,
                revision,
            );
        }
    }
    pub fn complete_profile_test(
        &mut self,
        revision: u64,
        result: Result<ConnectionTestSummary, ConnectionTestFailure>,
    ) {
        if self.connection_profiles.test_revision != revision {
            return;
        }
        self.connection_profiles.test_request = None;
        match result {
            Ok(summary) => {
                self.connection_profiles.credential = summary.credential.clone();
                self.connection_profiles.test_failure = None;
                self.connection_profiles.test = Load::Ready(summary);
            }
            Err(error) => {
                self.connection_profiles.test = Load::Failed(error.message.clone());
                self.connection_profiles.test_failure = Some(error);
            }
        }
    }

    fn refresh_profiles(&mut self) {
        self.record(Intent::ConnectionProfile {
            action: ProfileAction::Refresh,
        });
        if self.fixture {
            return;
        }
        self.initialize_connection_profiles();
    }
    fn persist_profile_selection(&mut self, id: String, expected: u64) {
        self.connection_profiles.busy = true;
        self.connection_profiles.pending_install = true;
        self.connection_profiles.revision = self.connection_profiles.revision.wrapping_add(1);
        self.record(Intent::ConnectionProfile {
            action: ProfileAction::Select(id.clone()),
        });
        if self.fixture {
            return;
        }
        let (Some(services), Some(container)) = (&self.services, &self.workspaces.container) else {
            return;
        };
        self.connection_profiles.request = services.profile_catalog(
            container.clone(),
            Some((expected, id)),
            self.connection_profiles.revision,
        );
    }
    fn save_profile(&mut self) {
        let Load::Ready(catalog) = &self.connection_profiles.catalog else {
            self.connection_profiles.message =
                Some("Open or refresh the protected saved-connection catalog first".into());
            return;
        };
        if !self.connection_profiles.editing {
            return;
        }
        let expected = catalog.revision;
        let id = self.connection_profiles.selected_id.clone();
        self.connection_profiles.busy = true;
        self.connection_profiles.pending_install = true;
        self.connection_profiles.revision = self.connection_profiles.revision.wrapping_add(1);
        self.record(Intent::ConnectionProfile {
            action: ProfileAction::Save,
        });
        if self.fixture {
            return;
        }
        let (Some(services), Some(container)) = (&self.services, &self.workspaces.container) else {
            return;
        };
        self.connection_profiles.request = services.save_profile(
            container.clone(),
            expected,
            id,
            self.connection_profiles.name.text.clone(),
            self.connection.origin.text.clone(),
            self.editor_credential(),
            self.connection_profiles.revision,
        );
    }

    pub fn complete_profile_catalog(
        &mut self,
        revision: u64,
        result: Result<ConnectionProfilesSnapshot, String>,
    ) {
        if self.connection_profiles.revision != revision {
            return;
        }
        self.connection_profiles.request = None;
        self.connection_profiles.busy = false;
        match result {
            Ok(catalog) => {
                let source_changed = !self.connection_profiles.editing
                    && self
                        .connection_profiles
                        .selected_id
                        .as_ref()
                        .is_some_and(|id| {
                            catalog
                                .profiles
                                .iter()
                                .find(|profile| &profile.id == id)
                                .is_none_or(|profile| {
                                    profile.origin != self.connection.origin.text
                                        || self.connection_profiles.credential.as_ref()
                                            != Some(&profile.credential)
                                })
                        });
                if source_changed {
                    self.disconnect();
                    self.connection_profiles.selected_id = None;
                    self.connection_profiles.credential = None;
                    self.connection.token_file = TextEdit::new("");
                    self.connection_profiles.editing = true;
                }
                let selected = catalog
                    .selected_profile_id
                    .as_ref()
                    .and_then(|id| catalog.profiles.iter().find(|profile| &profile.id == id))
                    .cloned();
                let apply = self.connection_profiles.pending_install
                    || !self.connection_profiles.initial_selection_applied
                        && !self.connection_profiles.launch_override;
                self.connection_profiles.pending_install = false;
                self.connection_profiles.initial_selection_applied = true;
                self.connection_profiles.catalog = Load::Ready(catalog);
                if apply {
                    if let Some(profile) = selected {
                        self.install_profile(&profile);
                    }
                }
                self.connection_profiles.message = Some(if source_changed {
                    "The selected saved connection changed or was cleared. The previous connection was disconnected; select a current record or credential reference.".into()
                } else {
                    "Saved connections loaded. Connecting remains an explicit action.".into()
                });
            }
            Err(error) => {
                if !self.connection_profiles.editing
                    && self.connection_profiles.selected_id.is_some()
                {
                    self.disconnect();
                    self.connection_profiles.selected_id = None;
                    self.connection_profiles.credential = None;
                    self.connection.token_file = TextEdit::new("");
                    self.connection_profiles.editing = true;
                }
                self.connection_profiles.pending_install = false;
                self.connection_profiles.catalog = Load::Failed(error.clone());
                self.connection_profiles.message = Some(error);
            }
        }
    }
}

impl GatewayEffects {
    fn profile_catalog(
        &self,
        container: Arc<NativeContainer>,
        selection: Option<(u64, String)>,
        revision: u64,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let task = self.runtime.spawn(async move {
            let result = tokio::task::spawn_blocking(move || match selection {
                Some((expected, id)) => container.select_connection_profile(expected, &id),
                None => container.connection_profiles(),
            })
            .await
            .map_err(|_| "Saved connection worker stopped".to_owned())
            .and_then(|result| result.map_err(|error| error.to_string()));
            ui.update(move |state, _| state.complete_profile_catalog(revision, result));
        });
        Some(RequestTask(task.abort_handle()))
    }
    #[allow(clippy::too_many_arguments)]
    fn save_profile(
        &self,
        container: Arc<NativeContainer>,
        expected: u64,
        id: Option<String>,
        name: String,
        origin: String,
        credential: ConnectionCredential,
        revision: u64,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let task = self.runtime.spawn(async move {
            let result =
                tokio::task::spawn_blocking(move || -> Result<ConnectionProfilesSnapshot> {
                    let credential = credential.reference(&origin)?.ok_or_else(|| {
                        anyhow::anyhow!("Select a protected credential file before saving")
                    })?;
                    let input = GatewayProfileInput::new(name, origin, credential)?;
                    container.save_connection_profile(expected, id.as_deref(), input)
                })
                .await
                .map_err(|_| "Saved connection worker stopped".to_owned())
                .and_then(|result| result.map_err(|error| error.to_string()));
            ui.update(move |state, _| state.complete_profile_catalog(revision, result));
        });
        Some(RequestTask(task.abort_handle()))
    }
    fn capture_profile_credential(
        &self,
        path: PathBuf,
        origin: String,
        epoch: u64,
        draft: u64,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let task = self.runtime.spawn(async move {
            let result =
                tokio::task::spawn_blocking(move || CredentialReference::capture(&path, &origin))
                    .await
                    .map_err(|_| "Credential reference worker stopped".to_owned())
                    .and_then(|result| {
                        result.map_err(|_| {
                    "Select an owned private regular token file, with no symbolic or hard links"
                        .to_owned()
                })
                    });
            ui.update(move |state, _| {
                if state.connection.epoch != epoch
                    || state.connection_profiles.draft_revision != draft
                {
                    return;
                }
                state.connection_profiles.credential_request = None;
                match result {
                    Ok(reference) => state.connection_profiles.credential = Some(reference),
                    Err(error) => state.connection_profiles.message = Some(error),
                }
            });
        });
        Some(RequestTask(task.abort_handle()))
    }
}

#[cfg(test)]
mod tests;
