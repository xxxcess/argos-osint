//! Purpose-validated bounded byte transfer to immutable backend-owned staging.
//!
//! Public inputs are a dataset kind/hash and raw bytes, never a server/Mac path.
//! A returned reference exists only after CSV validation and durable metadata.

use std::io::Read;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use argos_osint_core::storage::connect_registry::RegistryAuthority;
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::storage::setup_jobs;
use axum::body::Bytes;
use axum::extract::Query;
use axum::extract::Request;
use axum::extract::State;
use axum::http::header;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::Extension;
use axum::Json;
use fleetworks_contracts::context::CredentialPurpose;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::readiness::DatasetKind;
use fleetworks_contracts::readiness::DatasetRef;
use futures_util::Stream;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::json;
use sha2::Digest;
use sha2::Sha256;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;

use crate::ApiState;
use crate::AuthenticatedPrincipal;

/// A server-provisioned staging directory; never constructed from HTTP data.
#[derive(Clone)]
pub struct DatasetStore {
    root: Arc<PathBuf>,
    uploads: Arc<Semaphore>,
    #[cfg(unix)]
    identity: (u64, u64),
}

impl DatasetStore {
    /// Startup-only filesystem preparation. The parent must already be owned/provisioned.
    pub fn open(root: PathBuf) -> anyhow::Result<Self> {
        anyhow::ensure!(root.is_absolute(), "dataset staging root must be absolute");
        let parent = root
            .parent()
            .ok_or_else(|| anyhow::anyhow!("dataset staging parent required"))?;
        anyhow::ensure!(
            parent.canonicalize()? == parent,
            "dataset staging parent must be canonical"
        );
        if !root.exists() {
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(&root)?;
        }
        let metadata = std::fs::symlink_metadata(&root)?;
        anyhow::ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink() && root.canonicalize()? == root,
            "dataset staging root must be a real canonical directory"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            use std::os::unix::fs::PermissionsExt;
            anyhow::ensure!(
                metadata.permissions().mode() & 0o077 == 0,
                "dataset staging root must be private"
            );
            Ok(Self {
                root: Arc::new(root),
                uploads: Arc::new(Semaphore::new(2)),
                identity: (metadata.dev(), metadata.ino()),
            })
        }
        #[cfg(not(unix))]
        Ok(Self {
            root: Arc::new(root),
            uploads: Arc::new(Semaphore::new(2)),
        })
    }

    async fn verify_root(&self) -> anyhow::Result<()> {
        let metadata = tokio::fs::symlink_metadata(self.root.as_ref()).await?;
        anyhow::ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "dataset staging root changed"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            anyhow::ensure!(
                (metadata.dev(), metadata.ino()) == self.identity,
                "dataset staging root identity changed"
            );
        }
        Ok(())
    }

    fn path(&self, id: &str, suffix: &str) -> anyhow::Result<PathBuf> {
        anyhow::ensure!(
            (1..=128).contains(&id.len())
                && id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
            "invalid opaque dataset ID"
        );
        Ok(self.root.join(format!("{id}.{suffix}")))
    }

    /// Resolve pinned server bytes for a claimed job; no client-selected path.
    pub async fn path_for_job(
        &self,
        repo: &Repository,
        job_id: &str,
        expected: &DatasetRef,
    ) -> anyhow::Result<PathBuf> {
        setup_jobs::dataset_for_job(repo, job_id, expected).await?;
        self.verify_root().await?;
        let path = self.path(&expected.id, "csv")?;
        let metadata = tokio::fs::symlink_metadata(&path).await?;
        anyhow::ensure!(
            metadata.is_file()
                && !metadata.file_type().is_symlink()
                && metadata.len() == expected.size_bytes,
            "staged dataset identity unavailable"
        );
        Ok(path)
    }

    /// Explicit operator/configuration adapter for an allowlisted server dataset.
    /// This path is never an HTTP field; the public result remains an opaque ID.
    pub async fn stage_configured_file(
        &self,
        repo: &Repository,
        authority: &RegistryAuthority,
        path: PathBuf,
        kind: DatasetKind,
        sha256: String,
    ) -> anyhow::Result<DatasetRef> {
        anyhow::ensure!(path.is_absolute(), "configured dataset must be absolute");
        let metadata = tokio::fs::symlink_metadata(&path).await?;
        anyhow::ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "configured dataset must be a real file"
        );
        let file = tokio::fs::File::open(path).await?;
        let stream = futures_util::stream::unfold(file, |mut file| async move {
            let mut bytes = vec![0; 64 * 1024];
            match file.read(&mut bytes).await {
                Ok(0) => None,
                Ok(length) => {
                    bytes.truncate(length);
                    Some((Ok::<_, std::io::Error>(Bytes::from(bytes)), file))
                }
                Err(error) => Some((Err(error), file)),
            }
        });
        self.stage(repo, authority, kind, sha256, stream).await
    }

    /// Stream, hash, validate and register exact bytes; cancellation leaves an
    /// unregistered .part file which cannot be admitted and needs bounded expiry cleanup.
    pub async fn stage<S, E>(
        &self,
        repo: &Repository,
        authority: &RegistryAuthority,
        kind: DatasetKind,
        expected_sha256: String,
        stream: S,
    ) -> anyhow::Result<DatasetRef>
    where
        S: Stream<Item = Result<Bytes, E>> + Send,
        E: Send,
    {
        anyhow::ensure!(
            expected_sha256.len() == 64
                && expected_sha256.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "dataset SHA-256 required"
        );
        let _permit = self
            .uploads
            .clone()
            .try_acquire_owned()
            .map_err(|_| anyhow::anyhow!("dataset upload capacity busy"))?;
        self.verify_root().await?;
        let limit = setup_jobs::dataset_limit(kind);
        anyhow::ensure!(limit > 0, "unsupported dataset kind");
        let id = format!("dataset-{}", argos_osint_core::store::new_id());
        setup_jobs::begin_upload(repo, authority, &id, kind).await?;
        let part = self.path(&id, "part")?;
        let final_path = self.path(&id, "csv")?;
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            options.mode(0o600);
        }
        let mut file = options.open(&part).await?;
        let result = async {
            let mut sha = Sha256::new();
            let mut size = 0u64;
            futures_util::pin_mut!(stream);
            while let Some(chunk) = stream.next().await {
                let chunk =
                    chunk.map_err(|_| anyhow::anyhow!("dataset byte stream interrupted"))?;
                size = size
                    .checked_add(chunk.len() as u64)
                    .ok_or_else(|| anyhow::anyhow!("dataset size exceeded"))?;
                anyhow::ensure!(size <= limit, "dataset byte limit exceeded");
                sha.update(&chunk);
                file.write_all(&chunk).await?;
            }
            anyhow::ensure!(size > 0, "dataset bytes are empty");
            let actual = format!("{:x}", sha.finalize());
            anyhow::ensure!(
                actual == expected_sha256.to_ascii_lowercase(),
                "dataset SHA-256 mismatch"
            );
            file.flush().await?;
            file.sync_data().await?;
            let validation_path = part.clone();
            tokio::task::spawn_blocking(move || validate_csv(&validation_path, kind)).await??;
            self.verify_root().await?;
            // hard_link refuses an existing destination; an ordinary rename would overwrite it.
            tokio::fs::hard_link(&part, &final_path).await?;
            tokio::fs::remove_file(&part).await?;
            let reference = DatasetRef::new(id, kind, actual, size)?;
            setup_jobs::register_dataset(repo, authority, &reference, setup_jobs::DATASET_TTL)
                .await?;
            Ok(reference)
        }
        .await;
        if result.is_err() {
            // Only this attempt's .part file is removed. A possibly committed final
            // reference is preserved for reconciliation; no distributed-transaction claim.
            if let Err(error) = tokio::fs::remove_file(&part).await {
                if error.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(
                        code = "dataset_upload_cleanup_pending",
                        "Aborted upload cleanup requires expiry recovery"
                    );
                }
            }
        }
        result
    }

    /// Explicit worker/operator housekeeping, never called by a readiness GET.
    pub async fn expire_abandoned(&self, repo: &Repository) -> anyhow::Result<usize> {
        let expired = setup_jobs::claim_expired_uploads(repo).await?;
        let mut removed = 0;
        for item in expired {
            self.verify_root().await?;
            for suffix in ["part", "csv"] {
                let path = self.path(&item.dataset_id, suffix)?;
                match tokio::fs::symlink_metadata(&path).await {
                    Ok(metadata) => {
                        anyhow::ensure!(
                            metadata.is_file() && !metadata.file_type().is_symlink(),
                            "staging cleanup object changed"
                        );
                        tokio::fs::remove_file(path).await?;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
            setup_jobs::finish_upload_cleanup(repo, &item.dataset_id).await?;
            removed += 1;
        }
        Ok(removed)
    }
}

fn validate_csv(path: &Path, kind: DatasetKind) -> anyhow::Result<()> {
    match kind {
        DatasetKind::ActorCatalog => {
            let file = std::fs::File::open(path)?;
            argos_osint_core::actor_catalog::read(file.take(setup_jobs::ACTOR_DATASET_LIMIT + 1))?;
        }
        DatasetKind::ClassifierTraining => {
            let mut request = argos_osint_core::classifier::dataset::DatasetRequest::default();
            request.explicit_file = Some(path.to_owned());
            // Explicit selection is authoritative; no environment, legacy home or repo fallback.
            request.cwd = path
                .parent()
                .ok_or_else(|| anyhow::anyhow!("staging parent unavailable"))?
                .to_owned();
            request.home = request.cwd.clone();
            let resolution = argos_osint_core::classifier::dataset::resolve_dataset(&request)?;
            argos_osint_core::classifier::dataset::parse_csv(&resolution)?;
        }
        _ => anyhow::bail!("unsupported dataset kind"),
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UploadQuery {
    kind: DatasetKind,
    sha256: String,
}

pub(crate) async fn list(
    State(state): State<ApiState>,
    principal: Option<Extension<AuthenticatedPrincipal>>,
) -> Response {
    let Some(Extension(principal)) = principal else {
        return response(
            StatusCode::UNAUTHORIZED,
            json!({"error":"Purpose-scoped credential required"}),
        );
    };
    if !principal.permits(Permission::ReadBackend) {
        return response(
            StatusCode::FORBIDDEN,
            json!({"error":"Dataset metadata permission required"}),
        );
    }
    let authority = match RegistryAuthority::new(
        principal.scope_id(),
        principal.credential_id(),
        principal.access_epoch(),
    ) {
        Ok(value) => value,
        Err(_) => {
            return response(
                StatusCode::FORBIDDEN,
                json!({"error":"Dataset authority unavailable"}),
            )
        }
    };
    match setup_jobs::list_datasets(&state.repo, &authority).await {
        Ok((rows, truncated)) => response(
            StatusCode::OK,
            json!({"rows":rows,"truncated":truncated,"limits":{"actor_catalog":setup_jobs::ACTOR_DATASET_LIMIT,"classifier_training":setup_jobs::TRAINING_DATASET_LIMIT}}),
        ),
        Err(_) => response(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"error":"Dataset metadata unavailable"}),
        ),
    }
}

/// This route consumes raw bytes and enforces its own kind-specific bound.
/// Do not raise the ordinary small-JSON API body limit to accommodate it.
pub(crate) async fn upload(
    State(state): State<ApiState>,
    principal: Option<Extension<AuthenticatedPrincipal>>,
    store: Option<Extension<DatasetStore>>,
    Query(query): Query<UploadQuery>,
    request: Request,
) -> Response {
    let Some(Extension(principal)) = principal else {
        return response(
            StatusCode::UNAUTHORIZED,
            json!({"error":"Purpose-scoped credential required"}),
        );
    };
    if !principal.permits(Permission::MaintainBackend)
        || !matches!(
            principal.purpose(),
            CredentialPurpose::Native | CredentialPurpose::Operator
        )
    {
        return response(
            StatusCode::FORBIDDEN,
            json!({"error":"Backend maintenance permission required"}),
        );
    }
    let Some(Extension(store)) = store else {
        return response(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"error":"Dataset staging unavailable"}),
        );
    };
    if !matches!(
        request
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/csv" | "application/octet-stream")
    ) {
        return response(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            json!({"error":"Upload raw CSV bytes"}),
        );
    }
    let limit = setup_jobs::dataset_limit(query.kind);
    if request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|length| length > limit)
    {
        return response(
            StatusCode::PAYLOAD_TOO_LARGE,
            json!({"error":"Dataset byte limit exceeded"}),
        );
    }
    let authority = match RegistryAuthority::new(
        principal.scope_id(),
        principal.credential_id(),
        principal.access_epoch(),
    ) {
        Ok(value) => value,
        Err(_) => {
            return response(
                StatusCode::FORBIDDEN,
                json!({"error":"Dataset authority unavailable"}),
            )
        }
    };
    match store
        .stage(
            &state.repo,
            &authority,
            query.kind,
            query.sha256,
            request.into_body().into_data_stream(),
        )
        .await
    {
        Ok(reference) => response(
            StatusCode::CREATED,
            json!({"dataset":reference,"expires_in_seconds":setup_jobs::DATASET_TTL.as_secs()}),
        ),
        Err(_) => response(
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({"error":"Dataset upload, hash, schema or staging validation failed"}),
        ),
    }
}

fn response(status: StatusCode, value: serde_json::Value) -> Response {
    let mut response = (status, Json(value)).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store, private"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actor_and_training_schema_kinds_are_not_interchangeable() {
        let root = tempfile::tempdir().expect("temporary staging parent");
        let csv = root.path().join("actors.csv");
        std::fs::write(&csv,b"actor_id,actor_name,actor_type,aliases,name_normalized\nsynthetic-1,Harbor Union,organization,[],harbor union\n").expect("synthetic actor CSV");
        assert!(validate_csv(&csv, DatasetKind::ActorCatalog).is_ok());
        assert!(validate_csv(&csv, DatasetKind::ClassifierTraining).is_err());
        assert_eq!(
            setup_jobs::dataset_limit(DatasetKind::ClassifierTraining),
            128 * 1024 * 1024
        );
        assert_eq!(
            setup_jobs::dataset_limit(DatasetKind::ActorCatalog),
            256 * 1024 * 1024
        );
    }

    #[test]
    fn stage_paths_cannot_be_selected_by_opaque_id_traversal() {
        let root = tempfile::tempdir().expect("temporary staging parent");
        let parent = root
            .path()
            .canonicalize()
            .expect("canonical temporary parent");
        let store = DatasetStore::open(parent.join("uploads")).expect("owned staging root");
        for value in [
            "../secrets",
            "/Users/example/catalog.csv",
            "folder/file",
            "a:b",
            "",
        ] {
            assert!(store.path(value, "csv").is_err());
        }
    }
}
