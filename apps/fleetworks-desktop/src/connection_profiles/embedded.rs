//! The cold-launch catalog and folder picker for the Embedded companion.

use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use fleetworks_workstation::native::BackendKind;
use fleetworks_workstation::native::ConnectionProfilesSnapshot;
use fleetworks_workstation::native::NativeContainer;
use fleetworks_workstation::native::DEFAULT_LOCAL_ORIGIN;
use fleetworks_workstation::native::LOCAL_BACKEND_DIR;
use rune::FileDialogOptions;
use rune::TextEdit;
use rune::Tx;

use crate::app::Load;
use crate::app::State;
use crate::connection::ConnectionStatus;
use crate::connection::GatewayEffects;
use crate::connection::RequestTask;

const DEFAULT_FOLDER_NAME: &str = "Default folder";
const ADD_FOLDER_KEY: &str = "startup-add-folder";
const FOLDER_PATH_KEY: &str = "startup-folder-path/value/input";

pub(crate) struct EmbeddedTarget {
    /// The default folder has no saved profile ID.
    pub(crate) id: Option<String>,
    pub(crate) name: String,
    pub(crate) path: String,
}

#[derive(Default)]
pub(crate) struct EmbeddedFolderForm {
    pub(crate) open: bool,
    pub(crate) path: TextEdit,
    pub(crate) saving: bool,
    pub(crate) error: Option<String>,
    revision: u64,
    picker_revision: Option<u64>,
}

impl State {
    pub(crate) fn embedded_targets(&self) -> Vec<EmbeddedTarget> {
        let default = self.embedded_folder_path("embedded");
        let mut targets = vec![EmbeddedTarget {
            id: None,
            name: DEFAULT_FOLDER_NAME.into(),
            path: default.display().to_string(),
        }];
        if let Load::Ready(catalog) = &self.connection_profiles.catalog {
            for profile in &catalog.profiles {
                if profile.kind != BackendKind::Local {
                    continue;
                }
                let path = profile.local_state_dir.clone().unwrap_or_else(|| {
                    let slug = profile
                        .origin
                        .strip_prefix("local://")
                        .unwrap_or("embedded");
                    self.embedded_folder_path(slug)
                });
                if path != default {
                    targets.push(EmbeddedTarget {
                        id: Some(profile.id.clone()),
                        name: profile.name.clone(),
                        path: path.display().to_string(),
                    });
                }
            }
        }
        targets
    }

    fn embedded_folder_path(&self, slug: &str) -> PathBuf {
        let snapshot_root = match &self.workspaces.snapshot {
            Load::Ready(snapshot) => Some(snapshot.root.as_path()),
            _ => None,
        };
        self.workspaces
            .container
            .as_ref()
            .map(|container| container.root())
            .or(snapshot_root)
            .unwrap_or_else(|| Path::new("<workstation>"))
            .join(LOCAL_BACKEND_DIR)
            .join(slug)
    }

    pub(crate) fn reload_embedded_targets(&mut self) {
        if self.workspaces.container.is_some() {
            self.initialize_connection_profiles();
        } else {
            self.initialize_workspaces(None);
        }
    }

    pub(crate) fn open_embedded_folder(&mut self, tx: &mut Tx) {
        let folder = &mut self.connection_profiles.embedded_folder;
        if folder.open {
            return;
        }
        folder.open = true;
        tx.focus(FOLDER_PATH_KEY, tx.focus_visible());
    }

    pub(crate) fn close_embedded_folder(&mut self, tx: &mut Tx) {
        let folder = &mut self.connection_profiles.embedded_folder;
        if !folder.open || folder.saving {
            return;
        }
        *folder = EmbeddedFolderForm {
            revision: folder.revision.wrapping_add(1),
            ..EmbeddedFolderForm::default()
        };
        tx.focus(ADD_FOLDER_KEY, tx.focus_visible());
    }

    pub(crate) fn embedded_folder_path_changed(&mut self) {
        let folder = &mut self.connection_profiles.embedded_folder;
        folder.error = None;
        folder.revision = folder.revision.wrapping_add(1);
    }

    pub(crate) fn choose_embedded_folder(&mut self, tx: &mut Tx) {
        let folder = &mut self.connection_profiles.embedded_folder;
        if !folder.open || folder.saving || folder.picker_revision.is_some() {
            return;
        }
        let revision = folder.revision;
        folder.picker_revision = Some(revision);
        tx.open_folder(
            FileDialogOptions {
                title: "Choose an Embedded folder".into(),
                ..FileDialogOptions::default()
            },
            move |state: &mut State, result, tx| {
                let folder = &mut state.connection_profiles.embedded_folder;
                if folder.picker_revision != Some(revision) {
                    return;
                }
                folder.picker_revision = None;
                if !folder.open || folder.revision != revision || folder.saving {
                    return;
                }
                match result {
                    Ok(Some(path)) => match path.into_os_string().into_string() {
                        Ok(path) => {
                            folder.path.replace_text(&path);
                            state.embedded_folder_path_changed();
                        }
                        Err(_) => folder.error = Some("Choose a folder with a UTF-8 path.".into()),
                    },
                    Ok(None) => {}
                    Err(_) => {
                        folder.error = Some("Enter the folder path to add it manually.".into());
                    }
                }
                tx.focus(FOLDER_PATH_KEY, tx.focus_visible());
            },
        );
    }

    pub(crate) fn save_embedded_folder(&mut self, tx: &mut Tx) {
        let profiles = &mut self.connection_profiles;
        let folder = &mut profiles.embedded_folder;
        if !folder.open || profiles.busy {
            return;
        }
        let path = Path::new(&folder.path.text);
        if !path.is_absolute() || folder.path.text.len() > 4096 {
            folder.error = Some("Enter the full path to an Embedded folder.".into());
            tx.focus(FOLDER_PATH_KEY, tx.focus_visible());
            return;
        }
        let (Load::Ready(catalog), Some(services), Some(container)) = (
            &profiles.catalog,
            &self.services,
            &self.workspaces.container,
        ) else {
            folder.error = Some("Reload saved folders and try again.".into());
            return;
        };
        let revision = profiles.revision.wrapping_add(1);
        let Some(request) = services.save_embedded_folder(
            container.clone(),
            catalog.revision,
            path.to_path_buf(),
            revision,
        ) else {
            folder.error = Some("Saved folders are unavailable. Try again.".into());
            return;
        };
        folder.error = None;
        folder.saving = true;
        profiles.busy = true;
        profiles.revision = revision;
        profiles.request = Some(request);
    }

    fn complete_embedded_folder_save(
        &mut self,
        revision: u64,
        result: Result<ConnectionProfilesSnapshot, String>,
        tx: &mut Tx,
    ) {
        let profiles = &mut self.connection_profiles;
        if profiles.revision != revision || !profiles.embedded_folder.saving {
            return;
        }
        profiles.request = None;
        profiles.busy = false;
        profiles.embedded_folder.saving = false;
        match result {
            Ok(catalog) => {
                profiles.catalog = Load::Ready(catalog);
                self.close_embedded_folder(tx);
            }
            Err(error) => {
                profiles.embedded_folder.error = Some(error);
                tx.focus(FOLDER_PATH_KEY, tx.focus_visible());
                self.initialize_connection_profiles();
            }
        }
    }

    /// Select only the Local destination. The startup handler owns the explicit Start action.
    pub(crate) fn select_embedded_target(&mut self, id: Option<&str>) -> bool {
        if !self.startup.active()
            || self.setup_gateway
            || self.connection.status == ConnectionStatus::Connecting
            || self.connected()
            || self.local_backend.stopping
            || (!self.fixture && self.workspaces.container.is_none())
        {
            return false;
        }
        let profile = if let Some(id) = id {
            let Load::Ready(catalog) = &self.connection_profiles.catalog else {
                return false;
            };
            let Some(profile) = catalog
                .profiles
                .iter()
                .find(|profile| profile.kind == BackendKind::Local && profile.id == id)
            else {
                return false;
            };
            Some(profile.clone())
        } else {
            None
        };
        self.connection_profiles.initial_selection_applied = true;
        self.connection_profiles.operator_token_override = false;
        self.set_backend_kind(BackendKind::Local);
        if let Some(profile) = profile {
            self.install_profile(&profile);
        } else {
            self.connection_profiles.selected_id = None;
            self.connection_profiles.name = TextEdit::new(DEFAULT_FOLDER_NAME);
            self.connection_profiles.credential = None;
            self.connection_profiles.editing = false;
            self.connection.origin = TextEdit::new(DEFAULT_LOCAL_ORIGIN);
            self.connection.token_file = TextEdit::new("");
            self.local_backend.state_dir = None;
        }
        true
    }
}

impl GatewayEffects {
    fn save_embedded_folder(
        &self,
        container: Arc<NativeContainer>,
        expected: u64,
        path: PathBuf,
        revision: u64,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let task = self.runtime.spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                container.add_local_backend_folder(expected, path)
            })
            .await
            .map_err(|_| "Saved folder worker stopped".to_owned())
            .and_then(|result| result.map_err(|error| format!("{error:#}")));
            ui.update(move |state, tx| state.complete_embedded_folder_save(revision, result, tx));
        });
        Some(RequestTask(task.abort_handle()))
    }
}
