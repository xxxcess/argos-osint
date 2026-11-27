//! Local Embedded resource preferences, scoped to the companion's state directory.

use std::ffi::CString;
use std::fs::OpenOptions;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use fleetworks_contracts::embedded_resources::read_preferences;
use fleetworks_contracts::embedded_resources::EmbeddedResourcePreferences;
use fleetworks_contracts::embedded_resources::ResourcePreset;
use fleetworks_contracts::embedded_resources::FILE_NAME;
use fleetworks_workstation::native::NativeContainer;
use fleetworks_workstation::native::LOCAL_BACKEND_DIR;

use crate::app::Load;
use crate::app::State;
use crate::connection::GatewayEffects;
use crate::workstation_settings::SettingsSection;

#[derive(Clone, Copy)]
pub(crate) enum Resource {
    Cpu,
    Memory,
    Disk,
}

impl Resource {
    pub(crate) const ALL: [Self; 3] = [Self::Cpu, Self::Memory, Self::Disk];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Cpu => "CPU",
            Self::Memory => "Memory",
            Self::Disk => "Disk",
        }
    }

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Cpu => "settings-resource-cpu",
            Self::Memory => "settings-resource-memory",
            Self::Disk => "settings-resource-disk",
        }
    }

    fn preset(self, preferences: &mut EmbeddedResourcePreferences) -> &mut ResourcePreset {
        match self {
            Self::Cpu => &mut preferences.cpu,
            Self::Memory => &mut preferences.memory,
            Self::Disk => &mut preferences.disk,
        }
    }

    pub(crate) fn selected(self, mut preferences: EmbeddedResourcePreferences) -> ResourcePreset {
        *self.preset(&mut preferences)
    }
}

#[derive(Clone, PartialEq, Eq)]
enum Location {
    Explicit(PathBuf),
    Profile(String),
}

impl Location {
    fn resolve(
        &self,
        container: &NativeContainer,
        provision: bool,
    ) -> anyhow::Result<Option<PathBuf>> {
        let expected = match self {
            Self::Explicit(path) => path.clone(),
            Self::Profile(slug) => container.root().join(LOCAL_BACKEND_DIR).join(slug),
        };
        if !provision {
            match std::fs::symlink_metadata(&expected) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                result => {
                    result?;
                }
            }
        }
        let path = match self {
            Self::Explicit(path) => container.validate_local_backend_state_dir(path)?,
            Self::Profile(slug) => container.ensure_local_backend_state_dir(slug)?,
        };
        Ok(Some(crate::local_backend::validate_private_state_dir(
            &path,
        )?))
    }
}

#[derive(Clone, Copy)]
enum Operation {
    Read,
    Save(Resource, ResourcePreset),
}

pub(crate) struct ResourceSettings {
    pub(crate) preferences: Load<EmbeddedResourcePreferences>,
    pub(crate) saving: bool,
    pub(crate) save_failed: bool,
    location: Option<Location>,
    revision: u64,
}

impl Default for ResourceSettings {
    fn default() -> Self {
        Self {
            preferences: Load::Idle,
            saving: false,
            save_failed: false,
            location: None,
            revision: 0,
        }
    }
}

impl ResourceSettings {
    pub(crate) fn displayed(&self, fixture: bool) -> Option<EmbeddedResourcePreferences> {
        match self.preferences {
            Load::Ready(preferences) => Some(preferences),
            _ if fixture => Some(EmbeddedResourcePreferences::default()),
            _ => None,
        }
    }
}

impl State {
    fn selected_resource_location(&self) -> Option<Location> {
        self.is_local_backend().then(|| {
            self.local_backend
                .state_dir
                .clone()
                .map(Location::Explicit)
                .unwrap_or_else(|| Location::Profile(self.local_profile_slug()))
        })
    }

    pub(crate) fn poll_resource_settings(&mut self) -> bool {
        if self.fixture
            || !self.settings_open
            || self.settings_section != SettingsSection::Resources
        {
            return false;
        }
        let Some(location) = self.selected_resource_location() else {
            return false;
        };
        if self.resource_settings.location.as_ref() == Some(&location) {
            return false;
        }
        self.read_resource_settings();
        true
    }

    pub(crate) fn read_resource_settings(&mut self) {
        self.update_resource_settings(Operation::Read);
    }

    pub(crate) fn resource_save_pending(&self) -> bool {
        self.resource_settings.saving
            && self.resource_settings.location == self.selected_resource_location()
    }

    pub(crate) fn save_resource_setting(&mut self, resource: Resource, preset: ResourcePreset) {
        let settings = &self.resource_settings;
        if !self.is_local_backend()
            || settings.saving
            || settings
                .displayed(self.fixture)
                .is_none_or(|preferences| resource.selected(preferences) == preset)
        {
            return;
        }
        if self.fixture {
            let mut preferences = settings.displayed(true).expect("fixture resource defaults");
            *resource.preset(&mut preferences) = preset;
            self.resource_settings.preferences = Load::Ready(preferences);
            return;
        }
        if settings.location != self.selected_resource_location() {
            self.read_resource_settings();
            return;
        }
        self.update_resource_settings(Operation::Save(resource, preset));
    }

    fn update_resource_settings(&mut self, operation: Operation) {
        if self.fixture {
            return;
        }
        let Some(location) = self.selected_resource_location() else {
            return;
        };
        let settings = &mut self.resource_settings;
        settings.location = Some(location.clone());
        settings.revision = settings.revision.wrapping_add(1);
        settings.save_failed = false;
        settings.saving = matches!(operation, Operation::Save(..));
        if !settings.saving {
            settings.preferences = Load::Loading;
        }
        let revision = settings.revision;
        if self.services.as_ref().is_some_and(|services| {
            services.resource_preferences(
                self.workspaces.container.clone(),
                operation,
                location.clone(),
                revision,
            )
        }) {
            return;
        }
        self.complete_resource_settings(location, revision, Err(()));
    }

    fn complete_resource_settings(
        &mut self,
        location: Location,
        revision: u64,
        result: Result<EmbeddedResourcePreferences, ()>,
    ) {
        if self.resource_settings.revision != revision {
            return;
        }
        if self.selected_resource_location().as_ref() != Some(&location) {
            let settings = &mut self.resource_settings;
            settings.location = None;
            settings.preferences = Load::Idle;
            settings.saving = false;
            return;
        }
        let settings = &mut self.resource_settings;
        match result {
            Ok(preferences) => settings.preferences = Load::Ready(preferences),
            Err(()) if settings.saving => settings.save_failed = true,
            Err(()) => {
                settings.preferences =
                    Load::Failed("Resource preferences could not be loaded.".into())
            }
        }
        settings.saving = false;
    }
}

impl GatewayEffects {
    fn resource_preferences(
        &self,
        container: Option<Arc<NativeContainer>>,
        operation: Operation,
        location: Location,
        revision: u64,
    ) -> bool {
        let Some(ui) = self.ui.get().cloned() else {
            return false;
        };
        self.runtime.spawn(async move {
            let path = location.clone();
            let result = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
                let container = match container {
                    Some(container) => container,
                    None => Arc::new(NativeContainer::open_settings(&crate::platform::home()?)?),
                };
                let Some(root) =
                    path.resolve(&container, matches!(operation, Operation::Save(..)))?
                else {
                    return Ok(EmbeddedResourcePreferences::default());
                };
                let mut preferences = read_preferences(&root)?;
                if let Operation::Save(resource, preset) = operation {
                    *resource.preset(&mut preferences) = preset;
                    write_preferences(&root, preferences)?;
                }
                Ok(preferences)
            })
            .await
            .ok()
            .and_then(Result::ok)
            .ok_or(());
            ui.update(move |state, _| state.complete_resource_settings(location, revision, result));
        });
        true
    }
}

fn write_preferences(root: &Path, preferences: EmbeddedResourcePreferences) -> anyhow::Result<()> {
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(root)?;
    let metadata = directory.metadata()?;
    // SAFETY: geteuid takes no arguments and does not access caller memory.
    let owner = unsafe { libc::geteuid() };
    anyhow::ensure!(
        metadata.is_dir() && metadata.uid() == owner && metadata.mode() & 0o777 == 0o700,
        "Embedded state directory is not private"
    );
    let mut staged = tempfile::Builder::new()
        .prefix(".resource-preferences-")
        .tempfile_in(root)?;
    staged
        .as_file()
        .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    serde_json::to_writer(staged.as_file_mut(), &preferences)?;
    staged.as_file().sync_all()?;
    let source = staged
        .path()
        .file_name()
        .context("missing staged filename")?;
    let source = CString::new(source.as_bytes())?;
    let destination = CString::new(FILE_NAME)?;
    let fd = directory.as_raw_fd();
    // SAFETY: The directory remains open and both names are local NUL-terminated strings.
    let renamed = unsafe { libc::renameat(fd, source.as_ptr(), fd, destination.as_ptr()) };
    if renamed != 0 {
        return Err(io::Error::last_os_error()).context("install resource preferences");
    }
    directory.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_file_roundtrips_independent_presets_with_private_permissions() {
        let state = tempfile::tempdir().unwrap();
        std::fs::set_permissions(state.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let preferences = EmbeddedResourcePreferences {
            cpu: ResourcePreset::Sixty,
            memory: ResourcePreset::Eighty,
            ..EmbeddedResourcePreferences::default()
        };

        assert_eq!(
            read_preferences(state.path()).unwrap(),
            EmbeddedResourcePreferences::default()
        );
        write_preferences(state.path(), preferences).unwrap();

        assert_eq!(read_preferences(state.path()).unwrap(), preferences);
        let saved = std::fs::metadata(state.path().join(FILE_NAME)).unwrap();
        assert_eq!(saved.mode() & 0o777, 0o600);
    }
}
