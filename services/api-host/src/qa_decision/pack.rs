//! A release record is compiled into the companion only after native qualification.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::path::PathBuf;

use anyhow::ensure;
use anyhow::Result;
use fleetworks_contracts::chat::QaDecisionAvailabilityReason as Reason;
use fleetworks_contracts::chat::QaDecisionPreference as Preference;
use serde::Deserialize;
use sha2::Digest;
use sha2::Sha256;
use tempfile::TempDir;

const PINS: &str = include_str!("../../../../.github/fixtures/fleetworks-decision-pins.json");
const QUALIFIED: &[Qualification] = &[];
const WORKER: &str = "llama-server";

pub(super) struct Qualification {
    preference: Preference,
    target: &'static str,
    pub(super) alias: &'static str,
    runtime_revision: &'static str,
    model_revision: &'static str,
    runtime_files: &'static [RuntimeFile],
    pub(super) resident_bytes: u64,
    pub(super) threshold: f64,
    pub(super) margin: f64,
}

struct RuntimeFile {
    name: &'static str,
    sha256: &'static str,
    bytes: u64,
}

pub(super) struct VerifiedPack {
    pub(super) worker: PathBuf,
    pub(super) model: PathBuf,
    pub(super) socket_dir: TempDir,
}

#[derive(Deserialize)]
struct Pins {
    runtime: Runtime,
    models: BTreeMap<String, Model>,
}

#[derive(Deserialize)]
struct Runtime {
    revision: String,
}

#[derive(Deserialize)]
struct Model {
    revision: String,
    bytes: u64,
    sha256: String,
}

pub(super) fn qualified(preference: Preference) -> Option<&'static Qualification> {
    let alias = match preference {
        Preference::Julia1Q8 => "julia-1",
        Preference::VllmD3EdgeQ8 => "vllm-d3-edge",
        _ => return None,
    };
    let target = current_target()?;
    QUALIFIED.iter().find(|record| {
        record.preference == preference && record.target == target && record.alias == alias
    })
}

fn current_target() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        _ => None,
    }
}

fn pinned_model(record: &Qualification) -> Result<Model> {
    let mut pins: Pins = serde_json::from_str(PINS)?;
    let model = pins
        .models
        .remove(record.alias)
        .ok_or_else(|| anyhow::anyhow!("unknown decision model"))?;
    ensure!(
        record.runtime_revision == pins.runtime.revision && record.model_revision == model.revision,
        "decision qualification does not match the exact runtime and checkpoint pins"
    );
    ensure!(
        record.threshold > 0.0
            && record.threshold <= 1.0
            && record.margin > 0.0
            && record.margin <= 1.0
            && record.resident_bytes >= model.bytes,
        "invalid decision qualification budget"
    );
    Ok(model)
}

pub(super) fn verify(root: &Path, record: &Qualification) -> Result<VerifiedPack, Reason> {
    let model = pinned_model(record).map_err(|_| Reason::AwaitingQualification)?;
    let distribution = root.join("qa-decision");
    let platform = distribution.join(record.target);
    let bundle = platform.join(record.alias);
    let runtime = bundle.join("runtime");
    for path in [root, &distribution, &platform, &bundle, &runtime] {
        match private_directory(path) {
            Ok(true) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(Reason::PackMissing)
            }
            _ => return Err(Reason::WorkerStartFailed),
        }
    }
    let model_file = bundle.join("model.gguf");
    let worker = runtime.join(WORKER);
    verify_file(&model_file, model.bytes, &model.sha256).map_err(|_| Reason::WorkerStartFailed)?;
    let entries = fs::read_dir(&runtime).map_err(|_| Reason::WorkerStartFailed)?;
    let mut seen = 0;
    for entry in entries {
        let entry = entry.map_err(|_| Reason::WorkerStartFailed)?;
        let name = entry.file_name();
        let pinned = record
            .runtime_files
            .iter()
            .find(|pinned| name == pinned.name)
            .ok_or(Reason::WorkerStartFailed)?;
        verify_file(&entry.path(), pinned.bytes, pinned.sha256)
            .map_err(|_| Reason::WorkerStartFailed)?;
        seen += 1;
    }
    if seen != record.runtime_files.len()
        || !record.runtime_files.iter().any(|file| file.name == WORKER)
        || fs::metadata(&worker)
            .map_err(|_| Reason::WorkerStartFailed)?
            .mode()
            & 0o100
            == 0
    {
        return Err(Reason::WorkerStartFailed);
    }
    let socket_dir = tempfile::Builder::new()
        .prefix("fw-decision-")
        .tempdir_in("/tmp")
        .map_err(|_| Reason::WorkerStartFailed)?;
    Ok(VerifiedPack {
        worker,
        model: model_file,
        socket_dir,
    })
}

fn private_directory(path: &Path) -> io::Result<bool> {
    let metadata = fs::symlink_metadata(path)?;
    Ok(metadata.is_dir() && metadata.uid() == uid() && metadata.mode() & 0o7777 == 0o700)
}

fn uid() -> u32 {
    // SAFETY: geteuid has no arguments or memory effects.
    unsafe { libc::geteuid() }
}

fn verify_file(path: &Path, expected_bytes: u64, expected_sha: &str) -> Result<()> {
    use std::io::Read as _;

    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.uid() == uid()
            && metadata.mode() & 0o022 == 0
            && metadata.len() == expected_bytes,
        "invalid decision pack entry"
    );
    let mut digest = Sha256::new();
    let bytes = io::copy(
        &mut (&mut file).take(expected_bytes.saturating_add(1)),
        &mut digest,
    )?;
    ensure!(
        bytes == expected_bytes && format!("{:x}", digest.finalize()) == expected_sha,
        "decision pack entry differs from its verified pin"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qualification_matches_both_revisions() {
        let mut fixture = Qualification {
            preference: Preference::Julia1Q8,
            target: "x86_64-unknown-linux-gnu",
            alias: "julia-1",
            runtime_revision: "d141011c25781519123be51019441cd710260cb2",
            model_revision: "1e6b48a6983d7467f8eedc2cb0d21267b5fc052d",
            runtime_files: &[],
            resident_bytes: 400_000_000,
            threshold: 0.8,
            margin: 0.1,
        };
        assert!(pinned_model(&fixture).is_ok());
        fixture.model_revision = "unapproved";
        assert!(pinned_model(&fixture).is_err());
        assert!(qualified(Preference::Julia1Q8).is_none());
        assert!(qualified(Preference::VllmD3EdgeQ8).is_none());
    }

    #[test]
    fn file_verification_rejects_extra_bytes_and_links() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("original");
        let linked = dir.path().join("linked");
        fs::write(&original, b"verified").unwrap();
        std::os::unix::fs::symlink(&original, &linked).unwrap();
        let sha = format!("{:x}", Sha256::digest(b"verified"));
        assert!(verify_file(&original, 8, &sha).is_ok());
        assert!(verify_file(&original, 7, &sha).is_err());
        assert!(verify_file(&linked, 8, &sha).is_err());
    }
}
