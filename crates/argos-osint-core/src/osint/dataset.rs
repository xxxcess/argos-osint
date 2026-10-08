//! Shared snapshot storage and versioning for external datasets (WhatsMyName, DorkSearch).
//!
//! Stores immutable content-addressed versions under `$ARGOS_HOME/datasets/<name>/versions/<sha256>/`
//! and atomically updates `$ARGOS_HOME/datasets/<name>/active.json`.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::paths;

pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Active manifest describing the currently activated dataset snapshot.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DatasetManifest {
    pub dataset: String,
    pub active_version: String,
    pub retrieved_at: String,
    pub last_checked: String,
    pub source_url: String,
    pub license_status: String,
    pub total_count: usize,
    pub supported_count: usize,
    pub skipped_count: usize,
    pub schema_version: u32,
}

/// Status summary suitable for CLI and UI reporting.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DatasetStatus {
    pub dataset: String,
    pub is_available: bool,
    pub active_version: Option<String>,
    pub retrieved_at: Option<String>,
    pub last_checked: Option<String>,
    pub source_url: Option<String>,
    pub license_status: Option<String>,
    pub total_count: usize,
    pub supported_count: usize,
    pub skipped_count: usize,
}

static ACTIVE_REFRESHES: LazyLock<Arc<Mutex<HashSet<String>>>> =
    LazyLock::new(|| Arc::new(Mutex::new(HashSet::new())));

/// Acquire a non-blocking refresh lease for `dataset_name`.
/// Returns `None` if a refresh is already in progress.
pub fn acquire_refresh_lease(dataset_name: &str) -> Option<RefreshLease> {
    let mut active = ACTIVE_REFRESHES.lock().unwrap();
    if active.insert(dataset_name.to_string()) {
        Some(RefreshLease {
            dataset: dataset_name.to_string(),
        })
    } else {
        None
    }
}

pub struct RefreshLease {
    dataset: String,
}

impl Drop for RefreshLease {
    fn drop(&mut self) {
        if let Ok(mut active) = ACTIVE_REFRESHES.lock() {
            active.remove(&self.dataset);
        }
    }
}

/// Directory for a dataset: `$ARGOS_HOME/datasets/<name>/`.
pub fn dataset_root(name: &str) -> PathBuf {
    paths::dataset_dir(name)
}

/// Path to active.json manifest: `$ARGOS_HOME/datasets/<name>/active.json`.
pub fn active_manifest_path(name: &str) -> PathBuf {
    dataset_root(name).join("active.json")
}

/// Directory for immutable versions: `$ARGOS_HOME/datasets/<name>/versions/`.
pub fn versions_dir(name: &str) -> PathBuf {
    dataset_root(name).join("versions")
}

/// Version directory for a specific SHA256 digest:
/// `$ARGOS_HOME/datasets/<name>/versions/<sha256>/`.
pub fn version_dir(name: &str, sha256_hex: &str) -> PathBuf {
    versions_dir(name).join(sha256_hex)
}

/// Load the active manifest if it exists.
pub fn load_active_manifest(name: &str) -> Option<DatasetManifest> {
    let path = active_manifest_path(name);
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Query dataset status.
pub fn get_status(name: &str) -> DatasetStatus {
    match load_active_manifest(name) {
        Some(manifest) => DatasetStatus {
            dataset: manifest.dataset,
            is_available: true,
            active_version: Some(manifest.active_version),
            retrieved_at: Some(manifest.retrieved_at),
            last_checked: Some(manifest.last_checked),
            source_url: Some(manifest.source_url),
            license_status: Some(manifest.license_status),
            total_count: manifest.total_count,
            supported_count: manifest.supported_count,
            skipped_count: manifest.skipped_count,
        },
        None => DatasetStatus {
            dataset: name.to_string(),
            is_available: false,
            active_version: None,
            retrieved_at: None,
            last_checked: None,
            source_url: None,
            license_status: None,
            total_count: 0,
            supported_count: 0,
            skipped_count: 0,
        },
    }
}

/// Read active dataset content bytes and its manifest.
pub fn read_active_data(
    name: &str,
    data_filename: &str,
) -> Result<Option<(DatasetManifest, Vec<u8>)>> {
    let manifest = match load_active_manifest(name) {
        Some(m) => m,
        None => return Ok(None),
    };
    let file_path = version_dir(name, &manifest.active_version).join(data_filename);
    if !file_path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(&file_path)
        .with_context(|| format!("failed to read active dataset file {}", file_path.display()))?;
    Ok(Some((manifest, bytes)))
}

/// Store an immutable snapshot version and atomically activate it.
pub fn store_and_activate(
    name: &str,
    data_filename: &str,
    data_bytes: &[u8],
    mut manifest: DatasetManifest,
) -> Result<DatasetManifest> {
    // 1. Compute SHA256 of data bytes
    let sha256_hex = to_hex(&Sha256::digest(data_bytes));
    manifest.active_version = sha256_hex.clone();

    let root = dataset_root(name);
    fs::create_dir_all(&root)
        .with_context(|| format!("failed to create dataset root {}", root.display()))?;

    let vdir = version_dir(name, &sha256_hex);
    fs::create_dir_all(&vdir)
        .with_context(|| format!("failed to create version dir {}", vdir.display()))?;

    // 2. Write data file and manifest into version directory
    let vdata_path = vdir.join(data_filename);
    let vmanifest_path = vdir.join("manifest.json");

    fs::write(&vdata_path, data_bytes)
        .with_context(|| format!("failed to write {}", vdata_path.display()))?;

    let manifest_bytes =
        serde_json::to_vec_pretty(&manifest).context("failed to serialize manifest")?;
    fs::write(&vmanifest_path, &manifest_bytes)
        .with_context(|| format!("failed to write {}", vmanifest_path.display()))?;

    // 3. Atomically activate manifest by writing to a temp file in root and renaming
    let temp_manifest_path = root.join(format!(
        ".active.tmp.{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    ));
    fs::write(&temp_manifest_path, &manifest_bytes)
        .with_context(|| format!("failed to write {}", temp_manifest_path.display()))?;

    let target_manifest_path = active_manifest_path(name);
    fs::rename(&temp_manifest_path, &target_manifest_path).with_context(|| {
        format!(
            "failed to atomically replace {}",
            target_manifest_path.display()
        )
    })?;

    Ok(manifest)
}

/// Update only `last_checked` timestamp of the active manifest when an upstream check reveals no change.
pub fn update_last_checked(name: &str) -> Result<()> {
    let mut manifest = match load_active_manifest(name) {
        Some(m) => m,
        None => return Ok(()),
    };
    manifest.last_checked = Utc::now().to_rfc3339();
    let root = dataset_root(name);
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    let temp_manifest_path = root.join(format!(
        ".active.tmp.{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    ));
    fs::write(&temp_manifest_path, &manifest_bytes)?;
    fs::rename(&temp_manifest_path, active_manifest_path(name))?;
    Ok(())
}

/// Import a dataset from a local file path.
pub fn import_from_file(
    name: &str,
    data_filename: &str,
    file_path: &Path,
    source_url: &str,
    license_status: &str,
    total_count: usize,
    supported_count: usize,
    skipped_count: usize,
    schema_version: u32,
) -> Result<DatasetManifest> {
    let bytes = fs::read(file_path)
        .with_context(|| format!("failed to read import file {}", file_path.display()))?;
    let now = Utc::now().to_rfc3339();
    let manifest = DatasetManifest {
        dataset: name.to_string(),
        active_version: String::new(),
        retrieved_at: now.clone(),
        last_checked: now,
        source_url: source_url.to_string(),
        license_status: license_status.to_string(),
        total_count,
        supported_count,
        skipped_count,
        schema_version,
    };
    store_and_activate(name, data_filename, &bytes, manifest)
}
