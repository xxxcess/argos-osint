//! Operation-owned nonsecret effect proofs, separate from shared active pointers.

use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Read;
use std::io::Write;
use std::path::PathBuf;

use fleetworks_contracts::classifier_activation::ActivationRequest;
use fleetworks_contracts::classifier_activation::ActivationResult;
use fleetworks_contracts::classifier_activation::ActivationState;
use fleetworks_contracts::classifier_activation::ActiveModelRevision;
use serde::Deserialize;
use serde::Serialize;

use super::ActivationFailure;
use super::ArtifactStore;
use super::ModelId;
use crate::classifier::artifact;
use crate::classifier::artifact::ActivatedModelIndex;
use crate::classifier::artifact::ActivationRecord;

const LIMIT: u64 = 64 * 1024;
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Proof {
    version: u16,
    job_id: String,
    input_sha256: String,
    result: Option<ActivationResult>,
    activation_record_sha256: Option<String>,
}
fn checked_directory(store: &ArtifactStore) -> Result<bool, ActivationFailure> {
    let directory = store.root().join("activation-jobs");
    let metadata = match fs::symlink_metadata(&directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err(ActivationFailure::Unavailable),
    };
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || directory
            .canonicalize()
            .map_err(|_| ActivationFailure::Unavailable)?
            != directory
    {
        return Err(ActivationFailure::Unavailable);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let root = fs::metadata(store.root()).map_err(|_| ActivationFailure::Unavailable)?;
        if metadata.uid() != root.uid() || metadata.mode() & 0o077 != 0 {
            return Err(ActivationFailure::Unavailable);
        }
    }
    Ok(true)
}
fn path(store: &ArtifactStore, job: &str, complete: bool) -> Result<PathBuf, ActivationFailure> {
    if job.is_empty()
        || job.len() > 128
        || !job
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
    {
        return Err(ActivationFailure::Invalid);
    }
    Ok(store.root().join("activation-jobs").join(format!(
        "{job}.{}",
        if complete {
            "effect.json"
        } else {
            "started.json"
        }
    )))
}
fn read(path: &std::path::Path, limit: u64) -> Result<Option<Vec<u8>>, ActivationFailure> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(ActivationFailure::Unavailable),
    };
    let before = file
        .metadata()
        .map_err(|_| ActivationFailure::Unavailable)?;
    if !before.is_file() || before.len() > limit {
        return Err(ActivationFailure::Unavailable);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.nlink() != 1 {
            return Err(ActivationFailure::Unavailable);
        }
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ActivationFailure::Unavailable)?;
    if bytes.len() as u64 > limit {
        return Err(ActivationFailure::Unavailable);
    }
    Ok(Some(bytes))
}
fn write(path: &std::path::Path, proof: &Proof) -> Result<(), ActivationFailure> {
    let parent = path.parent().ok_or(ActivationFailure::Invalid)?;
    match fs::create_dir(parent) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(ActivationFailure::Unavailable),
    }
    let metadata = fs::symlink_metadata(parent).map_err(|_| ActivationFailure::Unavailable)?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || parent
            .canonicalize()
            .map_err(|_| ActivationFailure::Unavailable)?
            != parent
    {
        return Err(ActivationFailure::Unavailable);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .map_err(|_| ActivationFailure::Unavailable)?;
    }
    let bytes = serde_json::to_vec(proof).map_err(|_| ActivationFailure::Invalid)?;
    if bytes.len() as u64 > LIMIT {
        return Err(ActivationFailure::Invalid);
    }
    let mut staging =
        tempfile::NamedTempFile::new_in(parent).map_err(|_| ActivationFailure::Unavailable)?;
    staging
        .write_all(&bytes)
        .and_then(|_| staging.as_file().sync_all())
        .map_err(|_| ActivationFailure::Unavailable)?;
    staging
        .persist_noclobber(path)
        .map_err(|_| ActivationFailure::Unavailable)?;
    File::open(parent)
        .and_then(|file| file.sync_all())
        .map_err(|_| ActivationFailure::Unavailable)
}
pub(super) fn manifest_hash(
    store: &ArtifactStore,
    id: &ModelId,
) -> Result<String, ActivationFailure> {
    let bytes = read(
        &store.model_dir(id).join(artifact::MODEL_MANIFEST_FILE),
        artifact::MAX_MANIFEST_BYTES,
    )?
    .ok_or(ActivationFailure::Unavailable)?;
    Ok(artifact::sha256_hex(&bytes))
}

pub(super) fn previously_activated(
    store: &ArtifactStore,
    id: &ModelId,
) -> Result<bool, ActivationFailure> {
    let Some(bytes) = read(
        &store
            .root()
            .join("activated-models")
            .join(format!("{id}.json")),
        LIMIT,
    )?
    else {
        return Ok(false);
    };
    let index: ActivatedModelIndex =
        serde_json::from_slice(&bytes).map_err(|_| ActivationFailure::Unavailable)?;
    if index.model_id != *id || index.generation == 0 {
        return Err(ActivationFailure::Unavailable);
    }
    let record = read(
        &store
            .root()
            .join("activations")
            .join(format!("{}.json", index.generation)),
        LIMIT,
    )?
    .ok_or(ActivationFailure::Unavailable)?;
    if artifact::sha256_hex(&record) != index.activation_record_sha256 {
        return Err(ActivationFailure::Unavailable);
    }
    Ok(true)
}
fn checked(
    store: &ArtifactStore,
    job: &str,
    request: &ActivationRequest,
    complete: bool,
) -> Result<Option<Proof>, ActivationFailure> {
    if !checked_directory(store)? {
        return Ok(None);
    }
    let Some(bytes) = read(&path(store, job, complete)?, LIMIT)? else {
        return Ok(None);
    };
    let proof: Proof =
        serde_json::from_slice(&bytes).map_err(|_| ActivationFailure::Unavailable)?;
    if proof.version != 1
        || proof.job_id != job
        || proof.input_sha256 != super::input_hash(request)?
    {
        return Err(ActivationFailure::Unavailable);
    }
    Ok(Some(proof))
}
pub(super) fn started(
    store: &ArtifactStore,
    job: &str,
    request: &ActivationRequest,
) -> Result<bool, ActivationFailure> {
    Ok(checked(store, job, request, false)?.is_some())
}
pub(super) fn begin(
    store: &ArtifactStore,
    job: &str,
    request: &ActivationRequest,
) -> Result<(), ActivationFailure> {
    write(
        &path(store, job, false)?,
        &Proof {
            version: 1,
            job_id: job.into(),
            input_sha256: super::input_hash(request)?,
            result: None,
            activation_record_sha256: None,
        },
    )
}
pub(super) fn finish(
    store: &ArtifactStore,
    job: &str,
    request: &ActivationRequest,
    result: &ActivationResult,
) -> Result<(), ActivationFailure> {
    let record = match &result.activated_revision {
        Some(ActiveModelRevision::Active { generation, .. }) => Some(artifact::sha256_hex(
            &read(
                &store
                    .root()
                    .join("activations")
                    .join(format!("{generation}.json")),
                LIMIT,
            )?
            .ok_or(ActivationFailure::Unavailable)?,
        )),
        None => None,
        _ => return Err(ActivationFailure::Invalid),
    };
    write(
        &path(store, job, true)?,
        &Proof {
            version: 1,
            job_id: job.into(),
            input_sha256: super::input_hash(request)?,
            result: Some(result.clone()),
            activation_record_sha256: record,
        },
    )
}
pub(super) fn read_result(
    store: &ArtifactStore,
    job: &str,
    request: &ActivationRequest,
) -> Result<Option<ActivationResult>, ActivationFailure> {
    let Some(proof) = checked(store, job, request, true)? else {
        return Ok(None);
    };
    if checked(store, job, request, false)?.is_none() {
        return Err(ActivationFailure::Unavailable);
    }
    let result = proof.result.ok_or(ActivationFailure::Unavailable)?;
    if result.job_id != job || result.candidate != request.candidate {
        return Err(ActivationFailure::Unavailable);
    }
    match (&result.state, &result.activated_revision) {
        (ActivationState::Rejected, None) => Ok(Some(result)),
        (
            ActivationState::Activated,
            Some(ActiveModelRevision::Active {
                model_id,
                generation,
                manifest_sha256,
            }),
        ) => {
            let bytes = read(
                &store
                    .root()
                    .join("activations")
                    .join(format!("{generation}.json")),
                LIMIT,
            )?
            .ok_or(ActivationFailure::Unavailable)?;
            let record: ActivationRecord =
                serde_json::from_slice(&bytes).map_err(|_| ActivationFailure::Unavailable)?;
            let expected_generation = match &request.expected_prior {
                ActiveModelRevision::NeverActivated => 1,
                ActiveModelRevision::Active { generation, .. } => generation
                    .checked_add(1)
                    .ok_or(ActivationFailure::Invalid)?,
                _ => return Err(ActivationFailure::Invalid),
            };
            if proof.activation_record_sha256.as_deref()
                != Some(artifact::sha256_hex(&bytes).as_str())
                || *generation != expected_generation
                || record.generation != *generation
                || record.model_id.as_str() != model_id
                || record.model_sha256 != request.candidate.model_sha256
                || record.immutable_manifest_sha256 != *manifest_sha256
                || *manifest_sha256 != request.candidate.manifest_sha256
                || !record.committed
            {
                return Err(ActivationFailure::Unavailable);
            }
            Ok(Some(result))
        }
        _ => Err(ActivationFailure::Unavailable),
    }
}
