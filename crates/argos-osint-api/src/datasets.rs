//! Purpose-validated bounded byte transfer to immutable backend-owned staging.
//!
//! Public inputs are a dataset kind/hash and raw bytes, never a server/Mac path.
//! A returned reference exists only after CSV validation and durable metadata.

use std::io::Read;
use std::io::Seek;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use argos_osint_core::storage::connect_registry::RegistryAuthority;
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::storage::setup_jobs;
use argos_osint_core::storage::upload_ticket;
use argos_osint_core::storage::upload_ticket::UploadTicketBinding;
use axum::body::Bytes;
use axum::extract::Path as AxumPath;
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
use fleetworks_contracts::dataset_upload::DatasetUploadRequest;
use fleetworks_contracts::readiness::DatasetKind;
use fleetworks_contracts::readiness::DatasetRef;
use fleetworks_contracts::DatasetUpload;
use futures_util::Stream;
use futures_util::StreamExt;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;
use sha2::Digest;
use sha2::Sha256;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::sync::OwnedSemaphorePermit;
use tokio::sync::Semaphore;

use crate::request_context::ResolvedRequest;
use crate::ApiState;
use crate::AuthenticatedPrincipal;

const LOCAL_PART_ID_PREFIX: &str = "dataset-ipc-";

pub(crate) fn upload_binding(context: &ResolvedRequest) -> Option<UploadTicketBinding> {
    if !matches!(
        context.purpose(),
        CredentialPurpose::Native | CredentialPurpose::Operator
    ) || !context.permits(Permission::MaintainBackend)
    {
        return None;
    }
    UploadTicketBinding::new(
        context.scope_id(),
        context.credential_id(),
        context.access_epoch(),
        context.purpose(),
        context.registered().ok()?,
    )
    .ok()
}

#[derive(Default)]
pub(crate) struct StageExpectation<'a> {
    pub(crate) size: Option<u64>,
    pub(crate) context: Option<&'a ResolvedRequest>,
    pub(crate) permit: Option<OwnedSemaphorePermit>,
}

struct CancelValidation(Arc<AtomicBool>);
impl Drop for CancelValidation {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

/// A companion-owned partial file. Only a live upload owns its writable handle.
pub(crate) struct LocalDatasetPart {
    id: String,
    path: PathBuf,
    file: tokio::fs::File,
    preserve_link_identity: bool,
}

impl LocalDatasetPart {
    pub(crate) async fn write_chunk(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.file.write_all(bytes).await
    }
}

impl Drop for LocalDatasetPart {
    fn drop(&mut self) {
        // Unlink on cancellation or disconnect. The journal also covers an
        // interrupted process and any unlink that needs an expiry retry.
        if !self.preserve_link_identity {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// A server-provisioned staging directory; never constructed from HTTP data.
#[derive(Clone)]
pub struct DatasetStore {
    root: Arc<PathBuf>,
    pub(crate) uploads: Arc<Semaphore>,
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

    pub(crate) async fn verify_root(&self) -> anyhow::Result<()> {
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

    pub(crate) fn path(&self, id: &str, suffix: &str) -> anyhow::Result<PathBuf> {
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
        self.stage_checked(
            repo,
            authority,
            kind,
            expected_sha256,
            StageExpectation::default(),
            stream,
        )
        .await
        .map(|upload| upload.dataset)
    }

    /// Open the single companion-owned file that will be validated and published.
    pub(crate) async fn begin_local_part(
        &self,
        repo: &Repository,
        authority: &RegistryAuthority,
        kind: DatasetKind,
    ) -> anyhow::Result<LocalDatasetPart> {
        self.verify_root().await?;
        let id = format!(
            "{LOCAL_PART_ID_PREFIX}{}",
            argos_osint_core::store::new_id()
        );
        setup_jobs::begin_upload(repo, authority, &id, kind).await?;
        let path = self.path(&id, "part")?;
        self.verify_root().await?;
        // Opening and constructing the guard in the same blocking task covers
        // cancellation while the operating system is still creating the file.
        tokio::task::spawn_blocking(move || {
            let mut options = std::fs::OpenOptions::new();
            options.read(true).write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let file = tokio::fs::File::from_std(options.open(&path)?);
            Ok(LocalDatasetPart {
                id,
                path,
                file,
                preserve_link_identity: false,
            })
        })
        .await?
    }

    /// Verify the complete private file against the claimed ticket, then publish it.
    pub(crate) async fn finish_local_part(
        &self,
        repo: &Repository,
        context: &ResolvedRequest,
        mut part: LocalDatasetPart,
        expected: DatasetUploadRequest,
        permit: OwnedSemaphorePermit,
        cancelled: Arc<AtomicBool>,
    ) -> anyhow::Result<DatasetUpload> {
        let _cancel_validation = CancelValidation(Arc::clone(&cancelled));
        anyhow::ensure!(
            !cancelled.load(Ordering::Acquire),
            "dataset validation cancelled"
        );
        expected.validate()?;
        self.verify_root().await?;
        part.file.flush().await?;
        part.file.sync_data().await?;
        let reference = DatasetRef::new(
            part.id.clone(),
            expected.kind,
            expected.sha256,
            expected.size_bytes,
        )?;
        verify_staged_file(&part.path, &part.file, reference.size_bytes).await?;
        let file = part.file.try_clone().await?.into_std().await;
        let path = part.path.clone();
        let validation_reference = reference.clone();
        let permit = Arc::new(permit);
        let cpu_cancel = Arc::clone(&cancelled);
        let rows = spawn_validation(Arc::clone(&permit), move || {
            validate_local_part(file, &path, &validation_reference, &cpu_cancel)
        })
        .await??;
        context
            .revalidate(repo)
            .await
            .map_err(|_| anyhow::anyhow!("dataset upload authority changed"))?;
        self.verify_root().await?;
        verify_staged_file(&part.path, &part.file, reference.size_bytes).await?;
        let final_path = self.path(&reference.id, "csv")?;
        anyhow::ensure!(
            !cancelled.load(Ordering::Acquire),
            "dataset validation cancelled"
        );

        // Keep proof of ownership through registration. An interrupted link
        // or metadata commit leaves the journal enough information to recover.
        part.preserve_link_identity = true;
        if let Err(error) = tokio::fs::hard_link(&part.path, &final_path).await {
            part.preserve_link_identity = false;
            return Err(error.into());
        }
        verify_staged_file(&final_path, &part.file, reference.size_bytes).await?;
        if cancelled.load(Ordering::Acquire) {
            tokio::fs::remove_file(&final_path).await?;
            part.preserve_link_identity = false;
            anyhow::bail!("dataset validation cancelled");
        }
        let upload = register_publication(repo, context.authority(), reference, rows).await?;
        if tokio::fs::remove_file(&part.path).await.is_err() {
            tracing::warn!(
                code = "dataset_upload_cleanup_pending",
                "Validated upload retained its partial name for expiry recovery"
            );
        }
        Ok(upload)
    }

    pub(crate) async fn stage_checked<S, E>(
        &self,
        repo: &Repository,
        authority: &RegistryAuthority,
        kind: DatasetKind,
        expected_sha256: String,
        expectation: StageExpectation<'_>,
        stream: S,
    ) -> anyhow::Result<DatasetUpload>
    where
        S: Stream<Item = Result<Bytes, E>> + Send,
        E: Send,
    {
        let expected_size = expectation.size;
        let context = expectation.context;
        anyhow::ensure!(
            expected_sha256.len() == 64
                && expected_sha256.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "dataset SHA-256 required"
        );
        let permit = if let Some(permit) = expectation.permit {
            permit
        } else {
            self.uploads
                .clone()
                .try_acquire_owned()
                .map_err(|_| anyhow::anyhow!("dataset upload capacity busy"))?
        };
        // Both the stream and the CPU worker own this slot. Dropping an HTTP
        // future cannot release capacity while its blocking parser still runs.
        let permit = Arc::new(permit);
        let cancelled = Arc::new(AtomicBool::new(false));
        let _cancel_validation = CancelValidation(Arc::clone(&cancelled));
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
                anyhow::ensure!(
                    expected_size.is_none_or(|expected| size <= expected),
                    "dataset declared size exceeded"
                );
                sha.update(&chunk);
                file.write_all(&chunk).await?;
            }
            anyhow::ensure!(size > 0, "dataset bytes are empty");
            anyhow::ensure!(
                expected_size.is_none_or(|expected| size == expected),
                "dataset declared size mismatch"
            );
            let actual = format!("{:x}", sha.finalize());
            anyhow::ensure!(
                actual == expected_sha256.to_ascii_lowercase(),
                "dataset SHA-256 mismatch"
            );
            file.flush().await?;
            file.sync_data().await?;
            let validation_path = part.clone();
            let cpu_cancel = Arc::clone(&cancelled);
            let row_count = spawn_validation(Arc::clone(&permit), move || {
                validate_csv(&validation_path, kind, &cpu_cancel)
            })
            .await??;
            self.verify_root().await?;
            if let Some(context) = context {
                context
                    .revalidate(repo)
                    .await
                    .map_err(|_| anyhow::anyhow!("dataset upload authority changed"))?;
            }
            // hard_link refuses an existing destination.
            tokio::fs::hard_link(&part, &final_path).await?;
            tokio::fs::remove_file(&part).await?;
            let reference = DatasetRef::new(id, kind, actual, size)?;
            register_publication(repo, authority, reference, row_count).await
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
        // Ticket cleanup is metadata-only and bounded independently from the
        // existing partial-file/pinned-dataset expiry owner.
        upload_ticket::expire(repo).await?;
        let expired = setup_jobs::claim_expired_uploads(repo).await?;
        let mut removed = 0;
        for item in expired {
            self.verify_root().await?;
            let part = self.path(&item.dataset_id, "part")?;
            let final_path = self.path(&item.dataset_id, "csv")?;
            if item.dataset_id.starts_with(LOCAL_PART_ID_PREFIX) && !item.registered {
                // An unregistered final name can only belong to this local
                // upload if it still names the retained private inode.
                if same_staged_file(&part, &final_path).await? {
                    remove_staged_file(&final_path).await?;
                }
                remove_staged_file(&part).await?;
            } else {
                remove_staged_file(&part).await?;
                remove_staged_file(&final_path).await?;
            }
            setup_jobs::finish_upload_cleanup(repo, &item.dataset_id).await?;
            removed += 1;
        }
        Ok(removed)
    }
}

async fn register_publication(
    repo: &Repository,
    authority: &RegistryAuthority,
    reference: DatasetRef,
    row_count: u64,
) -> anyhow::Result<DatasetUpload> {
    setup_jobs::register_dataset(repo, authority, &reference, setup_jobs::DATASET_TTL).await?;
    let mut upload = DatasetUpload::new(reference, setup_jobs::DATASET_TTL.as_secs());
    upload.row_count = Some(row_count);
    Ok(upload)
}

async fn remove_staged_file(path: &Path) -> anyhow::Result<()> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => {
            anyhow::ensure!(metadata.is_file(), "staging cleanup object changed");
            tokio::fs::remove_file(path).await?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

async fn same_staged_file(first: &Path, second: &Path) -> anyhow::Result<bool> {
    async fn metadata(path: &Path) -> anyhow::Result<Option<std::fs::Metadata>> {
        match tokio::fs::symlink_metadata(path).await {
            Ok(metadata) => Ok(Some(metadata)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
    let Some(first) = metadata(first).await? else {
        return Ok(false);
    };
    let Some(second) = metadata(second).await? else {
        return Ok(false);
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(first.is_file()
            && second.is_file()
            && (first.dev(), first.ino()) == (second.dev(), second.ino()))
    }
    #[cfg(not(unix))]
    {
        let _ = (first, second);
        Ok(false)
    }
}

async fn verify_staged_file(path: &Path, file: &tokio::fs::File, size: u64) -> anyhow::Result<()> {
    let named = tokio::fs::symlink_metadata(path).await?;
    let opened = file.metadata().await?;
    anyhow::ensure!(
        named.is_file() && opened.is_file() && named.len() == size && opened.len() == size,
        "staged dataset file changed"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        anyhow::ensure!(
            (named.dev(), named.ino()) == (opened.dev(), opened.ino()),
            "staged dataset file identity changed"
        );
    }
    Ok(())
}

struct HashingReader<R> {
    file: R,
    digest: Sha256,
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self.file.read(buffer)?;
        self.digest.update(&buffer[..read]);
        Ok(read)
    }
}

fn validate_local_part(
    mut file: std::fs::File,
    path: &Path,
    expected: &DatasetRef,
    cancelled: &AtomicBool,
) -> anyhow::Result<u64> {
    file.rewind()?;
    fleetworks_contracts::dataset_upload::validate_csv_header(&mut file, expected.kind)?;
    file.rewind()?;
    let mut reader = HashingReader {
        file,
        digest: Sha256::new(),
    };
    let rows = if expected.kind == DatasetKind::ActorCatalog {
        // Hash the actual bytes as the bounded parser reads the open file.
        validate_actor_csv(&mut reader, cancelled)?
    } else {
        // Preserve the historical classifier parser's explicit-path interface.
        let mut buffer = vec![0; 64 * 1024];
        loop {
            anyhow::ensure!(
                !cancelled.load(Ordering::Acquire),
                "dataset validation cancelled"
            );
            if reader.read(&mut buffer)? == 0 {
                break;
            }
        }
        validate_csv(path, expected.kind, cancelled)?
    };
    anyhow::ensure!(
        format!("{:x}", reader.digest.finalize()) == expected.sha256,
        "dataset SHA-256 mismatch"
    );
    anyhow::ensure!(
        !cancelled.load(Ordering::Acquire),
        "dataset validation cancelled"
    );
    Ok(rows)
}

fn validate_actor_csv(reader: impl Read, cancelled: &AtomicBool) -> anyhow::Result<u64> {
    Ok(argos_osint_core::actor_catalog::scan_bounded(
        reader,
        &argos_osint_core::actor_catalog::ActorReadLimits::default(),
        cancelled,
        |_| Ok(()),
        |_| Ok(()),
    )?
    .rows as u64)
}

fn spawn_validation<T: Send + 'static>(
    work_slot: Arc<OwnedSemaphorePermit>,
    work: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> tokio::task::JoinHandle<anyhow::Result<T>> {
    tokio::task::spawn_blocking(move || {
        let _work_slot = work_slot;
        work()
    })
}

fn validate_csv(path: &Path, kind: DatasetKind, cancelled: &AtomicBool) -> anyhow::Result<u64> {
    anyhow::ensure!(
        !cancelled.load(Ordering::Acquire),
        "dataset validation cancelled"
    );
    let mut header = std::fs::File::open(path)?;
    fleetworks_contracts::dataset_upload::validate_csv_header(&mut header, kind)?;
    let rows = match kind {
        DatasetKind::ActorCatalog => validate_actor_csv(std::fs::File::open(path)?, cancelled)?,
        DatasetKind::ClassifierTraining => {
            let mut request = argos_osint_core::classifier::dataset::DatasetRequest::default();
            request.explicit_file = Some(path.to_owned());
            // Explicit selection is authoritative; no environment, legacy home or repo fallback.
            request.cwd = path
                .parent()
                .ok_or_else(|| anyhow::anyhow!("staging parent unavailable"))?
                .to_owned();
            request.executable = std::env::current_exe()?;
            request.home = request.cwd.clone();
            let resolution = argos_osint_core::classifier::dataset::resolve_dataset(&request)?;
            argos_osint_core::classifier::dataset::parse_csv(&resolution)?
                .summary
                .rows
        }
        _ => anyhow::bail!("unsupported dataset kind"),
    };
    anyhow::ensure!(
        !cancelled.load(Ordering::Acquire),
        "dataset validation cancelled"
    );
    Ok(rows)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UploadQuery {
    kind: DatasetKind,
    sha256: String,
}

pub(crate) async fn list_value(
    state: &ApiState,
    principal: &AuthenticatedPrincipal,
) -> Result<serde_json::Value, (StatusCode, &'static str)> {
    if !principal.permits(Permission::ReadBackend) {
        return Err((
            StatusCode::FORBIDDEN,
            "Dataset metadata permission required",
        ));
    }
    let authority = RegistryAuthority::new(
        principal.scope_id(),
        principal.credential_id(),
        principal.access_epoch(),
    )
    .map_err(|_| (StatusCode::FORBIDDEN, "Dataset authority unavailable"))?;
    match setup_jobs::list_datasets(&state.repo, &authority).await {
        Ok((rows, truncated)) => Ok(
            json!({"rows":rows,"truncated":truncated,"limits":{"actor_catalog":setup_jobs::ACTOR_DATASET_LIMIT,"classifier_training":setup_jobs::TRAINING_DATASET_LIMIT}}),
        ),
        Err(_) => Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Dataset metadata unavailable",
        )),
    }
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
    match list_value(&state, &principal).await {
        Ok(value) => response(StatusCode::OK, value),
        Err((status, message)) => response(status, json!({"error":message})),
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

pub(crate) async fn create_upload_ticket(
    state: &ApiState,
    context: &ResolvedRequest,
    store: Option<&DatasetStore>,
    input: &DatasetUploadRequest,
) -> Result<fleetworks_contracts::dataset_upload::DatasetUploadTicket, (StatusCode, &'static str)> {
    if input.validate().is_err() {
        return Err((StatusCode::BAD_REQUEST, "Invalid dataset upload metadata"));
    }
    let Some(binding) = upload_binding(context) else {
        return Err((
            StatusCode::FORBIDDEN,
            "Registered native maintenance authority required",
        ));
    };
    if context.revalidate(&state.repo).await.is_err() {
        return Err((
            StatusCode::UNAUTHORIZED,
            "Current upload authority unavailable",
        ));
    }
    let Some(store) = store else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Dataset staging unavailable",
        ));
    };
    if store.verify_root().await.is_err() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Dataset staging unavailable",
        ));
    }
    upload_ticket::issue(&state.repo, &binding, input)
        .await
        .map_err(|error| ticket_failure_status_message(error, StatusCode::SERVICE_UNAVAILABLE))
}

/// Issue an authenticated, short-lived destination for exact selected bytes.
pub(crate) async fn create_upload(
    State(state): State<ApiState>,
    Extension(context): Extension<ResolvedRequest>,
    store: Option<Extension<DatasetStore>>,
    Json(input): Json<DatasetUploadRequest>,
) -> Response {
    match create_upload_ticket(
        &state,
        &context,
        store.as_ref().map(|Extension(store)| store),
        &input,
    )
    .await
    {
        Ok(ticket) => response(StatusCode::CREATED, json!(ticket)),
        Err((status, message)) => response(status, json!({"error":message})),
    }
}

/// Claim a destination once, then hand the body stream to the existing bounded
/// upload worker. A failed/expired/used URL requires a newly issued destination;
/// completed DatasetRefs and accepted jobs are independent durable artifacts.
pub(crate) async fn put_upload(
    State(state): State<ApiState>,
    AxumPath(id): AxumPath<String>,
    Extension(context): Extension<ResolvedRequest>,
    store: Option<Extension<DatasetStore>>,
    request: Request,
) -> Response {
    let Some(binding) = upload_binding(&context) else {
        return response(
            StatusCode::FORBIDDEN,
            json!({"error":"Registered native maintenance authority required"}),
        );
    };
    if context.revalidate(&state.repo).await.is_err() {
        return response(
            StatusCode::UNAUTHORIZED,
            json!({"error":"Current upload authority unavailable"}),
        );
    }
    let Some(Extension(store)) = store else {
        return response(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"error":"Dataset staging unavailable"}),
        );
    };
    if request.uri().query().is_some()
        || request.uri().path()
            != format!(
                "{}/{}",
                fleetworks_contracts::dataset_upload::DATASET_UPLOADS_PATH,
                id
            )
        || id.len() != 64
        || !id.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return response(
            StatusCode::NOT_FOUND,
            json!({"error":"Upload URL unavailable; request a new destination"}),
        );
    }
    let permit = match store.uploads.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            return response(
                StatusCode::TOO_MANY_REQUESTS,
                json!({"error":"Upload capacity is busy; retry this destination later"}),
            )
        }
    };
    let lease = match upload_ticket::claim(&state.repo, &binding, &id).await {
        Ok(lease) => lease,
        Err(error) => return ticket_failure(error, StatusCode::GONE),
    };
    if !matches!(
        request
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("text/csv" | "application/octet-stream")
    ) {
        return response(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            json!({"error":"Upload raw CSV bytes to a new destination"}),
        );
    }
    if request
        .headers()
        .get(header::CONTENT_LENGTH)
        .is_some_and(|value| {
            value.to_str().ok().and_then(|v| v.parse::<u64>().ok()) != Some(lease.size_bytes)
        })
    {
        return response(
            StatusCode::BAD_REQUEST,
            json!({"error":"Declared upload size changed; request a new destination"}),
        );
    }
    let result = store
        .stage_checked(
            &state.repo,
            context.authority(),
            lease.kind,
            lease.sha256,
            StageExpectation {
                size: Some(lease.size_bytes),
                context: Some(&context),
                permit: Some(permit),
            },
            request.into_body().into_data_stream(),
        )
        .await;
    match result {
        Ok(upload) => {
            if context.revalidate(&state.repo).await.is_err() {
                return response(
                    StatusCode::UNAUTHORIZED,
                    json!({"error":"Upload authority changed; recover staged metadata after reconnecting"}),
                );
            }
            response(StatusCode::CREATED, upload)
        }
        Err(_) => response(
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({"error":"Upload validation failed or staging capacity is unavailable; request a new destination"}),
        ),
    }
}

pub(crate) fn ticket_failure_status_message(
    error: argos_osint_core::storage::tx::TxError,
    unavailable: StatusCode,
) -> (StatusCode, &'static str) {
    let status = match &error {
        argos_osint_core::storage::tx::TxError::Abort(code) if code == "upload_ticket:capacity" => {
            StatusCode::TOO_MANY_REQUESTS
        }
        argos_osint_core::storage::tx::TxError::Abort(code)
            if code == "upload_ticket:forbidden" =>
        {
            StatusCode::FORBIDDEN
        }
        argos_osint_core::storage::tx::TxError::Abort(code)
            if code == "upload_ticket:unavailable" =>
        {
            unavailable
        }
        argos_osint_core::storage::tx::TxError::Abort(code) if code == "upload_ticket:invalid" => {
            StatusCode::BAD_REQUEST
        }
        _ => StatusCode::SERVICE_UNAVAILABLE,
    };
    let message = match status {
        StatusCode::FORBIDDEN => "Upload URL belongs to another authenticated registration",
        StatusCode::TOO_MANY_REQUESTS => "Upload URL capacity is busy; retry later",
        StatusCode::GONE => "Upload URL expired or used; request a new destination",
        _ => "Upload destination authority is unavailable",
    };
    (status, message)
}

fn ticket_failure(
    error: argos_osint_core::storage::tx::TxError,
    unavailable: StatusCode,
) -> Response {
    let (status, message) = ticket_failure_status_message(error, unavailable);
    response(status, json!({"error":message}))
}

fn response<T: Serialize>(status: StatusCode, value: T) -> Response {
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
        assert_eq!(
            validate_csv(&csv, DatasetKind::ActorCatalog, &AtomicBool::new(false)).unwrap(),
            1
        );
        assert!(validate_csv(
            &csv,
            DatasetKind::ClassifierTraining,
            &AtomicBool::new(false)
        )
        .is_err());
        assert_eq!(
            setup_jobs::dataset_limit(DatasetKind::ClassifierTraining),
            128 * 1024 * 1024
        );
        assert_eq!(
            setup_jobs::dataset_limit(DatasetKind::ActorCatalog),
            256 * 1024 * 1024
        );

        let training = root.path().join("training.csv");
        let mut rows = String::from(
            "sample_id,title,description,domain_labels,primary_domain,secondary_domain,subcategories,source_kind,difficulty,dataset_version,annotation_notes,split,scenario_group,language,synthetic,labels_complete,expected_admission,rejection_reason\n",
        );
        for split in ["train", "calibration"] {
            for (group, (first, second)) in [
                ("governance", "security"),
                ("economic", "human"),
                ("environmental", "scientific"),
            ]
            .into_iter()
            .enumerate()
            {
                for repeat in 0..2 {
                    let id = format!("{split}-{group}-{repeat}");
                    rows.push_str(&format!(
                        "{id},Synthetic {first} {second} bulletin {repeat},Fictional {first} and {second} report,{first}|{second},{first},{second},,synthetic,cross_domain,synthetic-development-v1,,{split},{id},en,true,true,admit,\n"
                    ));
                }
            }
        }
        std::fs::write(&training, rows).expect("synthetic training CSV");
        assert_eq!(
            validate_csv(
                &training,
                DatasetKind::ClassifierTraining,
                &AtomicBool::new(false),
            )
            .unwrap(),
            12
        );
        assert!(validate_csv(
            &training,
            DatasetKind::ActorCatalog,
            &AtomicBool::new(false)
        )
        .is_err());
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

#[cfg(test)]
mod upload_tests;
