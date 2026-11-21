//! Installed CSV resources and their checked-in content identities.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::Path;
use std::path::PathBuf;
use std::sync::OnceLock;

use fleetworks_contracts::readiness::DatasetKind;
use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct DatasetPin {
    pub(crate) filename: String,
    pub(crate) size_bytes: u64,
    pub(crate) sha256: String,
    pub(crate) download_url: String,
}

#[derive(Deserialize)]
struct Pins {
    schema: u8,
    datasets: BTreeMap<DatasetKind, DatasetPin>,
}

pub(crate) fn pin(kind: DatasetKind) -> Option<&'static DatasetPin> {
    static PINS: OnceLock<Pins> = OnceLock::new();
    let pins = PINS.get_or_init(|| {
        serde_json::from_str(include_str!("../../../datasets/native-defaults.json"))
            .expect("checked-in native dataset pins")
    });
    (pins.schema == 1).then_some(())?;
    pins.datasets.get(&kind)
}

pub(crate) fn installed_path(kind: DatasetKind) -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let bin = executable.parent()?;
    let package = bin.parent()?;
    let path = match std::env::consts::OS {
        "macos"
            if bin.file_name() == Some(OsStr::new("MacOS"))
                && package.file_name() == Some(OsStr::new("Contents")) =>
        {
            package.join("Resources/datasets")
        }
        "linux" if bin.file_name() == Some(OsStr::new("bin")) => {
            package.join("share/fleetworks/resources/datasets")
        }
        _ => return None,
    };
    let filename = &pin(kind)?.filename;
    if Path::new(filename).file_name() != Some(OsStr::new(filename)) {
        return None;
    }
    let file = path.join(filename);
    file.symlink_metadata().ok()?.is_file().then_some(file)
}
