//! Embedded single-process local backend served strictly over stdio IPC frames.
//!
//! Binds zero network listeners, directs all logging to `stderr`, validates
//! private state-directory ownership (`0o700`) and single-writer locking,
//! bootstraps embedded SurrealDB, Jobs SQLite/RocksDB, and Tantivy search
//! in-process, and cleanly relinquishes active worker leases on shutdown.

use std::future::Future;
use std::path::PathBuf;

use anyhow::Result;

/// Entry point for `--role local-ipc`. Runs entirely over stdin/stdout IPC frames.
pub(crate) async fn run(
    state_dir: PathBuf,
    nlp_cmd: Option<PathBuf>,
    classifier_cmd: Option<PathBuf>,
    signal: impl Future<Output = ()>,
) -> Result<()> {
    #[cfg(not(feature = "embedded"))]
    {
        let _ = (state_dir, nlp_cmd, classifier_cmd, signal);
        anyhow::bail!("--role local-ipc requires the `embedded` feature");
    }

    #[cfg(feature = "embedded")]
    {
        embedded::run_embedded(state_dir, nlp_cmd, classifier_cmd, signal).await
    }
}

#[cfg(feature = "embedded")]
mod embedded {
    use std::fs;
    use std::fs::File;
    use std::fs::OpenOptions;
    use std::future::Future;
    use std::io::Read;
    use std::io::Write;
    use std::path::Path;
    use std::path::PathBuf;
    use std::process::Stdio;
    use std::sync::Arc;

    #[cfg(unix)]
    use std::os::unix::fs::DirBuilderExt;
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use anyhow::anyhow;
    use anyhow::ensure;
    use anyhow::Context;
    use anyhow::Result;
    use argos_osint_api::actor_search::ActorSearchBackend;
    use argos_osint_api::classifier_activation::ActivationArtifacts;
    use argos_osint_api::datasets::DatasetStore;
    use argos_osint_api::local_dispatch::LocalCapabilityConfig;
    use argos_osint_api::local_dispatch::LocalGateway;
    use argos_osint_api::workstation::ReadinessOwners;
    use argos_osint_api::ApiState;
    use argos_osint_api::AuthenticatedPrincipal;
    use argos_osint_api::PrivateServices;
    use argos_osint_api::SearchClient;
    use argos_osint_core::storage::bootstrap;
    use argos_osint_core::storage::connect_registry;
    use argos_osint_core::storage::jobs_client;
    use argos_osint_core::storage::open::Target;
    use argos_osint_core::storage::repo::Repository;
    use argos_osint_core::storage::server;
    use fleetworks_contracts::actor_index::ActorIndexLimits;
    use fleetworks_contracts::context::Permission;
    use fleetworks_contracts::ipc::read_frame;
    use fleetworks_contracts::ipc::write_frame;
    use fleetworks_contracts::ipc::IpcEnvelope;
    use fleetworks_contracts::ipc::IpcEvent;
    use fleetworks_contracts::ipc::IpcHello;
    use fleetworks_contracts::ipc::IpcHelloAck;
    use fleetworks_contracts::ipc::IpcOperation;
    use fleetworks_contracts::ipc::IpcOutcome;
    use fleetworks_contracts::ipc::IPC_MIN_PROTOCOL_VERSION;
    use fleetworks_contracts::ipc::IPC_PROTOCOL_VERSION;
    use fleetworks_contracts::ipc::MAX_IPC_INFLIGHT;
    use fleetworks_contracts::ipc::MAX_IPC_QUEUE_DEPTH;
    use fleetworks_jobs::queue::Queue;
    use fleetworks_jobs::Store;
    use fleetworks_search_index::config::LocalEngineConfig;
    use fleetworks_search_index::engine::Engine;
    use fleetworks_search_index::service::EngineSlot;
    use fleetworks_search_index::service::SearchService;
    use tokio::io::AsyncRead;
    use tokio::io::AsyncWrite;
    use tokio::io::AsyncWriteExt;
    use tokio::process::Child;
    use tokio::process::Command;
    use tokio::sync::mpsc;
    use tokio::sync::oneshot;
    use tokio::sync::Semaphore;
    use tokio::task::JoinSet;

    use crate::actor_index_worker::ActorIndexWorker;
    use crate::actor_index_worker::LocalActorSearchBackend;
    use crate::native_worker;
    use crate::native_worker::LocalWorkerConfig;
    use crate::queue_client::Client;

    const LOCK_FILENAME: &str = ".local-backend.lock";
    const SOURCE_EPOCH_FILENAME: &str = "source-epoch";
    const NATIVE_QUEUE_NAME: &str = "fleetworks.native";
    const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

    /// Advisory exclusive lock held for the lifetime of the local IPC backend.
    struct StateDirLock {
        _file: File,
    }

    /// Supervised optional child helpers (`--nlp-cmd`, `--classifier-cmd`).
    struct HelperChildren {
        nlp: Option<Child>,
        classifier: Option<Child>,
    }

    impl HelperChildren {
        async fn shutdown(mut self) {
            for child in [&mut self.nlp, &mut self.classifier].into_iter().flatten() {
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
        }
    }

    /// Subdirectory layout under `--state-dir`.
    struct StateLayout {
        root: PathBuf,
        surreal: PathBuf,
        jobs: PathBuf,
        search: PathBuf,
        staged_datasets: PathBuf,
    }

    pub(super) async fn run_embedded(
        state_dir: PathBuf,
        nlp_cmd: Option<PathBuf>,
        classifier_cmd: Option<PathBuf>,
        signal: impl Future<Output = ()>,
    ) -> Result<()> {
        let context = LocalBackendContext::open(state_dir, nlp_cmd, classifier_cmd, true).await?;
        let stdin = tokio::io::stdin();
        let stdout = tokio::io::stdout();
        context.serve(stdin, stdout, signal).await
    }

    /// Fully initialized in-process backend ready to serve one IPC stream pair.
    struct LocalBackendContext {
        gateway: Arc<LocalGateway>,
        principal: AuthenticatedPrincipal,
        repo: Repository,
        state_root: PathBuf,
        worker_config: Option<LocalWorkerConfig>,
        helpers: Option<HelperChildren>,
        _jobs_store: Store,
        _search_service: Arc<SearchService>,
        _engine_slot: Arc<EngineSlot>,
        _lock: StateDirLock,
    }

    impl LocalBackendContext {
        async fn open(
            state_dir: PathBuf,
            nlp_cmd: Option<PathBuf>,
            classifier_cmd: Option<PathBuf>,
            install_process_globals: bool,
        ) -> Result<Self> {
            let (layout, lock, validated_nlp, validated_classifier) =
                tokio::task::spawn_blocking(move || {
                    let (layout, lock) = prepare_state_layout(&state_dir)?;
                    let validated_nlp = nlp_cmd
                        .as_deref()
                        .map(validate_helper_command)
                        .transpose()?;
                    let validated_classifier = classifier_cmd
                        .as_deref()
                        .map(validate_helper_command)
                        .transpose()?;
                    Ok::<_, anyhow::Error>((layout, lock, validated_nlp, validated_classifier))
                })
                .await??;

            let helpers = spawn_helpers(validated_nlp.as_deref(), validated_classifier.as_deref())?;

            let repo = Repository::connect(
                &Target::Embedded {
                    path: layout.surreal.clone(),
                },
                server::NS,
                server::DB,
            )
            .await?;
            bootstrap::ensure_network_identity(repo.db()).await?;
            connect_registry::initialize_control(repo.db()).await?;
            let deployment = connect_registry::deployment_state(&repo).await?;

            let jobs_store = Store::open(layout.jobs.join("jobs.db")).await?;
            let jobs_queue = Arc::new(Queue::open(layout.jobs.join("queue")).await?);
            if install_process_globals {
                jobs_client::install_local(jobs_store.clone())?;
                jobs_client::install_queue((*jobs_queue).clone())?;
            }

            let source_epoch = tokio::task::spawn_blocking({
                let search_dir = layout.search.clone();
                move || load_or_create_source_epoch(&search_dir)
            })
            .await??;

            let limits = ActorIndexLimits::default();
            let engine_config = LocalEngineConfig::for_identity(
                layout.search.clone(),
                deployment.deployment_id.clone(),
                source_epoch,
                limits,
            )?;
            let engine = tokio::task::spawn_blocking({
                let config = engine_config.engine.clone();
                move || Engine::open(config)
            })
            .await??;
            let engine_slot = Arc::new(EngineSlot::new(Some(Arc::new(engine))));
            let search_service = Arc::new(SearchService::new(
                engine_config,
                Arc::clone(&engine_slot),
                "actor_index_unavailable",
            )?);

            let dataset_store = tokio::task::spawn_blocking({
                let staged = layout.staged_datasets.clone();
                move || DatasetStore::open(staged)
            })
            .await??;
            let activation = tokio::task::spawn_blocking({
                let root = layout.root.clone();
                move || ActivationArtifacts::new(root)
            })
            .await??;

            let readiness = ReadinessOwners::new(deployment.deployment_id.clone(), Vec::new())?;
            let search_backend: Arc<dyn ActorSearchBackend> =
                Arc::new(LocalActorSearchBackend(Arc::clone(&search_service)));
            let services = PrivateServices::default()
                .with_search(SearchClient::from_local(search_backend))
                .with_workstation(readiness, dataset_store.clone())
                .with_activation_artifacts(activation);

            let state = ApiState::new(
                repo.clone(),
                format!("local-ipc-{}", argos_osint_core::store::new_id()),
                0,
                Arc::new(Vec::new()),
            )
            .with_workspace_root(layout.root.clone());

            let capability_config = LocalCapabilityConfig {
                nlp_enabled: helpers.nlp.is_some(),
                classifier_enabled: helpers.classifier.is_some(),
                search_enabled: true,
                jobs_enabled: true,
            };
            let gateway = Arc::new(LocalGateway::new(state, services, capability_config)?);

            let principal = AuthenticatedPrincipal::for_local_ipc(
                "local-ipc-workstation",
                deployment.deployment_id,
                format!("epoch-{}", deployment.configuration_epoch),
                native_ipc_permissions(),
            )?;

            let index_worker = ActorIndexWorker::local(
                Arc::clone(&search_service),
                format!("actor-index-{}", argos_osint_core::store::new_id()),
            )?;
            let worker_config = LocalWorkerConfig {
                broker: Client::local(jobs_queue, NATIVE_QUEUE_NAME.to_owned()),
                store: dataset_store,
                index: Some(index_worker),
                relinquish_on_shutdown: true,
            };

            tracing::info!(
                role = "local-ipc",
                nlp = helpers.nlp.is_some(),
                classifier = helpers.classifier.is_some(),
                search = true,
                jobs = true,
                "Fleetworks startup summary: embedded local-ipc backend ready"
            );

            Ok(Self {
                gateway,
                principal,
                repo,
                state_root: layout.root,
                worker_config: Some(worker_config),
                helpers: Some(helpers),
                _jobs_store: jobs_store,
                _search_service: search_service,
                _engine_slot: engine_slot,
                _lock: lock,
            })
        }

        async fn serve<R, W>(
            mut self,
            reader: R,
            writer: W,
            signal: impl Future<Output = ()>,
        ) -> Result<()>
        where
            R: AsyncRead + Unpin + Send + 'static,
            W: AsyncWrite + Unpin + Send + 'static,
        {
            let worker_config = self
                .worker_config
                .take()
                .ok_or_else(|| anyhow!("local worker configuration already consumed"))?;
            let result = serve_streams(
                reader,
                writer,
                Arc::clone(&self.gateway),
                self.principal.clone(),
                self.repo.clone(),
                self.state_root.clone(),
                worker_config,
                signal,
            )
            .await;
            if let Some(helpers) = self.helpers.take() {
                helpers.shutdown().await;
            }
            result
        }
    }

    fn native_ipc_permissions() -> Vec<Permission> {
        vec![
            Permission::ReadBackend,
            Permission::RegisterWorkstation,
            Permission::RebindWorkstation,
            Permission::ManageConfiguration,
            Permission::MaintainBackend,
            Permission::SubmitInteractive,
            Permission::ReadWorkstationActivity,
            Permission::RetryJob,
            Permission::CancelJob,
        ]
    }

    fn ensure_private_dir(path: &Path) -> Result<()> {
        let created_fresh = match fs::symlink_metadata(path) {
            Ok(meta) => {
                ensure!(
                    !meta.file_type().is_symlink() && meta.is_dir(),
                    "directory must not be a symlink and must be a real directory"
                );
                false
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let mut builder = fs::DirBuilder::new();
                builder.recursive(true);
                #[cfg(unix)]
                builder.mode(0o700);
                builder.create(path)?;
                #[cfg(unix)]
                fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
                true
            }
            Err(err) => return Err(err.into()),
        };

        let meta = fs::symlink_metadata(path)?;
        ensure!(
            !meta.file_type().is_symlink() && meta.is_dir(),
            "directory identity must be a real directory"
        );
        #[cfg(unix)]
        {
            // SAFETY: `geteuid` takes no pointers and always succeeds.
            let current_uid = unsafe { libc::geteuid() };
            ensure!(
                meta.uid() == current_uid,
                "directory must be owned by the current user"
            );
            if created_fresh {
                fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
            }
            let refreshed = fs::symlink_metadata(path)?;
            ensure!(
                refreshed.permissions().mode() & 0o077 == 0,
                "directory permissions must not permit group or world access"
            );
        }
        let _ = created_fresh;
        Ok(())
    }

    fn prepare_state_layout(raw_state_dir: &Path) -> Result<(StateLayout, StateDirLock)> {
        ensure!(
            !raw_state_dir.as_os_str().is_empty(),
            "state-dir is required"
        );
        ensure_private_dir(raw_state_dir).context("prepare state-dir")?;
        let root = fs::canonicalize(raw_state_dir).context("canonicalize state-dir")?;
        ensure_private_dir(&root).context("verify canonical state-dir")?;

        let lock = acquire_state_lock(&root.join(LOCK_FILENAME))?;

        let surreal = root.join("surreal");
        let jobs = root.join("jobs");
        let search = root.join("search");
        let staged_datasets = root.join("staged-datasets");
        for subdir in [&surreal, &jobs, &search, &staged_datasets] {
            ensure_private_dir(subdir)?;
        }

        Ok((
            StateLayout {
                root,
                surreal,
                jobs,
                search,
                staged_datasets,
            },
            lock,
        ))
    }

    fn acquire_state_lock(lock_path: &Path) -> Result<StateDirLock> {
        if let Ok(meta) = fs::symlink_metadata(lock_path) {
            ensure!(
                !meta.file_type().is_symlink() && meta.is_file(),
                "state lock path must be a regular file"
            );
            #[cfg(unix)]
            {
                // SAFETY: `geteuid` takes no pointers and always succeeds.
                let uid = unsafe { libc::geteuid() };
                ensure!(
                    meta.uid() == uid,
                    "state lock must be owned by current user"
                );
            }
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(lock_path).context("open local backend lock")?;
        match file.try_lock() {
            Ok(()) => {}
            Err(fs::TryLockError::WouldBlock) => {
                anyhow::bail!("local backend state directory is already locked by another process");
            }
            Err(fs::TryLockError::Error(error)) => {
                return Err(error).context("acquire local backend lock");
            }
        }
        file.set_len(0)?;
        writeln!(file, "{}", std::process::id())?;
        file.sync_all()?;
        Ok(StateDirLock { _file: file })
    }

    fn load_or_create_source_epoch(search_dir: &Path) -> Result<String> {
        let epoch_path = search_dir.join(SOURCE_EPOCH_FILENAME);
        match fs::symlink_metadata(&epoch_path) {
            Ok(meta) => {
                ensure!(
                    !meta.file_type().is_symlink() && meta.is_file(),
                    "search source-epoch marker must be a regular file"
                );
                let mut raw = String::new();
                File::open(&epoch_path)?
                    .take(257)
                    .read_to_string(&mut raw)?;
                let trimmed = raw.trim().to_owned();
                ensure!(
                    (1..=128).contains(&trimmed.len())
                        && trimmed.bytes().all(|byte| byte.is_ascii_alphanumeric()
                            || matches!(byte, b'-' | b'_' | b'.')),
                    "corrupt search source-epoch marker"
                );
                return Ok(trimmed);
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }

        let epoch = format!("local-{}", argos_osint_core::store::new_id());
        let temp_path = search_dir.join(format!("{SOURCE_EPOCH_FILENAME}.tmp"));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&temp_path)?;
        file.write_all(epoch.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp_path, &epoch_path)?;
        Ok(epoch)
    }

    /// Validate an optional external helper command binary before spawning it.
    pub(crate) fn validate_helper_command(path: &Path) -> Result<PathBuf> {
        ensure!(path.is_absolute(), "helper command path must be absolute");
        let meta = fs::symlink_metadata(path).context("helper command binary must exist")?;
        ensure!(
            !meta.file_type().is_symlink() && meta.is_file(),
            "helper command must be a regular non-symlink file"
        );
        let canonical = fs::canonicalize(path).context("canonicalize helper command")?;
        ensure!(
            canonical == path,
            "helper command path must be canonical without symlink components"
        );
        let parent = canonical
            .parent()
            .ok_or_else(|| anyhow!("helper command requires a parent directory"))?;
        let parent_meta = fs::symlink_metadata(parent)?;
        ensure!(
            !parent_meta.file_type().is_symlink() && parent_meta.is_dir(),
            "helper command parent directory must not be a symlink"
        );
        #[cfg(unix)]
        {
            // SAFETY: `geteuid` takes no pointers and always succeeds.
            let current_uid = unsafe { libc::geteuid() };
            let file_mode = meta.permissions().mode();
            let parent_mode = parent_meta.permissions().mode();
            ensure!(
                file_mode & 0o002 == 0,
                "helper command binary must not be world-writable"
            );
            ensure!(
                parent_mode & 0o002 == 0,
                "helper command directory must not be world-writable"
            );
            ensure!(
                meta.uid() == current_uid || meta.uid() == 0,
                "helper command must be owned by the current user or root"
            );
            ensure!(
                file_mode & 0o111 != 0,
                "helper command binary must be executable"
            );
        }
        Ok(canonical)
    }

    fn spawn_helper_child(path: &Path) -> Result<Child> {
        let mut command = Command::new(path);
        command.env_clear();
        for var in ["PATH", "HOME", "TMPDIR", "LANG", "RUST_LOG"] {
            if let Some(value) = std::env::var_os(var) {
                command.env(var, value);
            }
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        let mut child = command.spawn().context("spawn supervised helper command")?;
        if let Some(status) = child.try_wait()? {
            anyhow::bail!("supervised helper command exited immediately with status {status}");
        }
        Ok(child)
    }

    fn spawn_helpers(
        nlp_path: Option<&Path>,
        classifier_path: Option<&Path>,
    ) -> Result<HelperChildren> {
        let nlp = nlp_path.map(spawn_helper_child).transpose()?;
        let classifier = classifier_path.map(spawn_helper_child).transpose()?;
        Ok(HelperChildren { nlp, classifier })
    }

    fn hello_compatible(hello: &IpcHello) -> bool {
        hello.min_protocol_version <= hello.protocol_version
            && hello.protocol_version >= IPC_MIN_PROTOCOL_VERSION
            && hello.min_protocol_version <= IPC_PROTOCOL_VERSION
    }

    fn request_affects_readiness(op: &IpcOperation) -> bool {
        matches!(
            op,
            IpcOperation::RegistrationEnsure(_)
                | IpcOperation::RegistrationRebind(_)
                | IpcOperation::RegistrationRetire(_)
                | IpcOperation::SeedActors(_)
                | IpcOperation::TrainClassifier(_)
                | IpcOperation::ReindexActors(_)
                | IpcOperation::ClassifierActivate(_)
        )
    }

    /// Serve local IPC frames over an `AsyncRead + AsyncWrite` pair until EOF,
    /// explicit `Shutdown` frame, or external `signal`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn serve_streams<R, W>(
        mut reader: R,
        mut writer: W,
        gateway: Arc<LocalGateway>,
        principal: AuthenticatedPrincipal,
        worker_repo: Repository,
        worker_workspace: PathBuf,
        worker_config: LocalWorkerConfig,
        signal: impl Future<Output = ()>,
    ) -> Result<()>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let handshake = tokio::time::timeout(HANDSHAKE_TIMEOUT, read_frame(&mut reader))
            .await
            .context("local IPC handshake timed out")??;
        let Some(first_frame) = handshake else {
            return Ok(());
        };
        let IpcEnvelope::Hello(hello) = first_frame else {
            anyhow::bail!("expected Hello frame before IPC requests");
        };
        if !hello_compatible(&hello) {
            let ack = IpcEnvelope::HelloAck(IpcHelloAck::incompatible(
                IPC_PROTOCOL_VERSION,
                Some(format!(
                    "unsupported IPC protocol range {}..={}",
                    hello.min_protocol_version, hello.protocol_version
                )),
            ));
            write_frame(&mut writer, &ack).await?;
            return Ok(());
        }

        let caps = gateway
            .capabilities(principal.permissions().collect())
            .await
            .map_err(|(status, message)| {
                anyhow!("local capabilities error ({status}): {message}")
            })?;
        let ready = gateway.ready().await;
        let ack = IpcEnvelope::HelloAck(IpcHelloAck::compatible(Some(caps), Some(ready)));
        write_frame(&mut writer, &ack).await?;

        let (worker_stop_tx, worker_stop_rx) = oneshot::channel::<()>();
        let mut worker_task = tokio::spawn(native_worker::run_with_config(
            worker_repo,
            worker_workspace,
            worker_config,
            async move {
                let _ = worker_stop_rx.await;
            },
        ));

        let (outbound_tx, mut outbound_rx) = mpsc::channel::<IpcEnvelope>(MAX_IPC_QUEUE_DEPTH);
        let writer_task = tokio::spawn(async move {
            while let Some(frame) = outbound_rx.recv().await {
                if write_frame(&mut writer, &frame).await.is_err() {
                    break;
                }
            }
            let _ = writer.flush().await;
        });

        let (inbound_tx, mut inbound_rx) = mpsc::channel(MAX_IPC_QUEUE_DEPTH);
        let reader_task = tokio::spawn(async move {
            loop {
                let frame = read_frame(&mut reader).await;
                let terminal = matches!(frame, Ok(None) | Err(_));
                if inbound_tx.send(frame).await.is_err() || terminal {
                    break;
                }
            }
        });

        let inflight = Arc::new(Semaphore::new(MAX_IPC_INFLIGHT));
        let mut request_tasks: JoinSet<()> = JoinSet::new();
        let mut worker_result: Option<Result<usize>> = None;
        let mut shutdown_requested = false;
        let mut stream_broken = false;

        tokio::pin!(signal);
        loop {
            tokio::select! {
                biased;
                _ = &mut signal => {
                    break;
                }
                ended = &mut worker_task => {
                    worker_result = Some(ended.map_err(anyhow::Error::from).and_then(|res| res));
                    tracing::warn!(role = "local-ipc", "Maintenance worker exited during IPC session");
                    break;
                }
                Some(_) = request_tasks.join_next(), if !request_tasks.is_empty() => {}
                inbound = inbound_rx.recv() => {
                    let Some(frame) = inbound else {
                        stream_broken = true;
                        break;
                    };
                    match frame {
                        Ok(Some(IpcEnvelope::Request { request_id, registration, op })) => {
                            match Arc::clone(&inflight).try_acquire_owned() {
                                Ok(permit) => {
                                    let gateway = Arc::clone(&gateway);
                                    let principal = principal.clone();
                                    let tx = outbound_tx.clone();
                                    let readiness_event = request_affects_readiness(&op);
                                    request_tasks.spawn(async move {
                                        let _permit = permit;
                                        let outcome = gateway
                                            .dispatch(&principal, registration.as_ref(), op)
                                            .await;
                                        let ok = matches!(outcome, IpcOutcome::Ok(_));
                                        let _ = tx
                                            .send(IpcEnvelope::Response {
                                                request_id,
                                                outcome,
                                            })
                                            .await;
                                        if ok && readiness_event {
                                            let _ = tx.try_send(IpcEnvelope::Event(
                                                IpcEvent::WorkstationReadinessChanged,
                                            ));
                                        }
                                    });
                                }
                                Err(_) => {
                                    let _ = outbound_tx
                                        .try_send(IpcEnvelope::Response {
                                            request_id,
                                            outcome: IpcOutcome::Err {
                                                status: 503,
                                                code: "ipc_inflight_full".into(),
                                                message: "local IPC in-flight request limit reached".into(),
                                                retry_after_seconds: Some(1),
                                            },
                                        });
                                }
                            }
                        }
                        Ok(Some(IpcEnvelope::Shutdown { .. })) => {
                            shutdown_requested = true;
                            break;
                        }
                        Ok(Some(_)) => {
                            tracing::warn!(role = "local-ipc", "Ignoring unexpected client IPC frame");
                        }
                        Ok(None) => {
                            stream_broken = true;
                            break;
                        }
                        Err(error) => {
                            tracing::warn!(role = "local-ipc", %error, "Local IPC reader ended");
                            stream_broken = true;
                            break;
                        }
                    }
                }
            }
        }

        drop(inbound_rx);
        reader_task.abort();
        let _ = reader_task.await;

        let _ = worker_stop_tx.send(());
        let relinquished_leases = match worker_result {
            Some(res) => res.unwrap_or(0),
            None => match worker_task.await {
                Ok(Ok(count)) => count,
                Ok(Err(error)) => {
                    tracing::warn!(role = "local-ipc", %error, "Local worker shutdown reported error");
                    0
                }
                Err(error) => {
                    tracing::warn!(role = "local-ipc", %error, "Local worker join failed");
                    0
                }
            },
        };

        while request_tasks.join_next().await.is_some() {}

        if !stream_broken {
            let _ = outbound_tx.try_send(IpcEnvelope::Event(IpcEvent::ShutdownNotice {
                checkpointed: true,
                relinquished_leases,
            }));
            if shutdown_requested {
                let _ = outbound_tx
                    .send(IpcEnvelope::ShutdownAck {
                        checkpointed: true,
                        relinquished_leases,
                    })
                    .await;
            }
        }

        drop(outbound_tx);
        let _ = writer_task.await;
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use std::time::Duration;

        use fleetworks_contracts::connect::RegistrationEnsureRequest;
        use fleetworks_contracts::connect::RegistrationReceipt;
        use fleetworks_contracts::context::MutationContext;
        use fleetworks_contracts::context::RegisteredContext;
        use fleetworks_contracts::dataset_upload::DatasetUploadRequest;
        use fleetworks_contracts::dataset_upload::DatasetUploadTicket;
        use fleetworks_contracts::dataset_upload::DATASET_UPLOADS_PATH;
        use fleetworks_contracts::ipc::LocalIpcClient;
        use fleetworks_contracts::ipc::ShutdownAck;
        use fleetworks_contracts::readiness::DatasetKind;
        use fleetworks_contracts::readiness::MaintenanceReceipt;
        use fleetworks_contracts::readiness::MaintenanceRequest;
        use fleetworks_contracts::readiness::WorkstationReadiness;
        use fleetworks_contracts::search::ActorSearchQuery;
        use fleetworks_contracts::search::ActorSearchResponse;
        use fleetworks_contracts::search::IndexState;
        use fleetworks_contracts::DatasetUpload;
        use fleetworks_contracts::Jobs;
        use sha2::Digest;

        use super::*;

        fn test_base64_encode(input: &[u8]) -> String {
            const ALPHABET: &[u8; 64] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            if input.is_empty() {
                return String::new();
            }
            let mut out = Vec::with_capacity(input.len().div_ceil(3) * 4);
            for chunk in input.chunks(3) {
                let b0 = chunk[0];
                let b1 = if chunk.len() > 1 { chunk[1] } else { 0 };
                let b2 = if chunk.len() > 2 { chunk[2] } else { 0 };
                let n = ((b0 as u32) << 16) | ((b1 as u32) << 8) | (b2 as u32);
                out.push(ALPHABET[((n >> 18) & 0x3f) as usize]);
                out.push(ALPHABET[((n >> 12) & 0x3f) as usize]);
                out.push(if chunk.len() > 1 {
                    ALPHABET[((n >> 6) & 0x3f) as usize]
                } else {
                    b'='
                });
                out.push(if chunk.len() > 2 {
                    ALPHABET[(n & 0x3f) as usize]
                } else {
                    b'='
                });
            }
            String::from_utf8(out).expect("base64 ASCII")
        }

        #[cfg(unix)]
        #[test]
        fn state_dir_validation_rejects_broad_permissions_and_concurrent_lock() {
            let dir = tempfile::tempdir().expect("tempdir");
            let base = dir.path().canonicalize().expect("canonical");

            let too_open = base.join("open-state");
            fs::create_dir(&too_open).expect("create open dir");
            fs::set_permissions(&too_open, fs::Permissions::from_mode(0o755)).expect("chmod 755");
            assert!(prepare_state_layout(&too_open).is_err());

            let valid = base.join("private-state");
            let (_layout, _lock) =
                prepare_state_layout(&valid).expect("private state lock acquired");
            assert_eq!(
                fs::symlink_metadata(&valid)
                    .expect("meta")
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            // Second concurrent lock on the exact same state directory must fail closed.
            assert!(prepare_state_layout(&valid).is_err());
        }

        #[test]
        fn helper_command_validation_rejects_relative_and_missing_paths() {
            assert!(validate_helper_command(Path::new("relative/helper")).is_err());
            let dir = tempfile::tempdir().expect("tempdir");
            let base = dir.path().canonicalize().expect("canonical");
            assert!(validate_helper_command(&base.join("missing-helper")).is_err());

            #[cfg(unix)]
            {
                let file = base.join("helper");
                fs::write(&file, b"#!/bin/sh\nexit 0\n").expect("write helper");
                fs::set_permissions(&file, fs::Permissions::from_mode(0o700)).expect("chmod 700");
                assert_eq!(validate_helper_command(&file).expect("valid helper"), file);

                let symlink = base.join("helper-link");
                std::os::unix::fs::symlink(&file, &symlink).expect("symlink");
                assert!(validate_helper_command(&symlink).is_err());

                fs::set_permissions(&file, fs::Permissions::from_mode(0o777)).expect("chmod 777");
                assert!(validate_helper_command(&file).is_err());
            }
        }

        #[cfg(feature = "embedded")]
        #[tokio::test]
        async fn hello_handshake_advertises_local_capabilities_and_rejects_incompatible_versions() {
            let dir = tempfile::tempdir().expect("tempdir");
            let state_dir = dir.path().canonicalize().expect("canonical").join("state");

            // Incompatible protocol range receives HelloAck { compatible: false }.
            let context1 = LocalBackendContext::open(state_dir.clone(), None, None, false)
                .await
                .expect("open context");
            let (server_io, client_io) = tokio::io::duplex(64 * 1024);
            let (server_read, server_write) = tokio::io::split(server_io);
            let (mut client_read, mut client_write) = tokio::io::split(client_io);
            let (_sig_tx, sig_rx) = oneshot::channel::<()>();
            let server_task = tokio::spawn(context1.serve(server_read, server_write, async move {
                let _ = sig_rx.await;
            }));
            let bad_hello: IpcHello = serde_json::from_value(serde_json::json!({
                "protocol_version": 99,
                "min_protocol_version": 99,
                "client_name": "incompatible-test",
                "client_version": "99.0.0",
            }))
            .expect("deserialize incompatible hello");
            write_frame(&mut client_write, &IpcEnvelope::Hello(bad_hello))
                .await
                .expect("write incompatible hello");
            match read_frame(&mut client_read).await.expect("read HelloAck") {
                Some(IpcEnvelope::HelloAck(ack)) => {
                    assert!(!ack.compatible);
                    assert_eq!(ack.protocol_version, IPC_PROTOCOL_VERSION);
                    assert!(ack.reason.is_some());
                }
                other => panic!("expected incompatible HelloAck, got {other:?}"),
            }
            server_task.await.expect("join").expect("clean reject");

            // Compatible handshake advertises accurate local capabilities without faking NLP/Classifier.
            let context2 = LocalBackendContext::open(state_dir, None, None, false)
                .await
                .expect("reopen context");
            let (server_io, client_io) = tokio::io::duplex(64 * 1024);
            let (server_read, server_write) = tokio::io::split(server_io);
            let (client_read, client_write) = tokio::io::split(client_io);
            let (_sig2_tx, sig2_rx) = oneshot::channel::<()>();
            let server_task2 =
                tokio::spawn(context2.serve(server_read, server_write, async move {
                    let _ = sig2_rx.await;
                }));

            let client = LocalIpcClient::connect_streams(
                client_read,
                client_write,
                "local-ipc-test",
                "1.0.0",
            )
            .await
            .expect("handshake ok");
            let caps = client.capabilities().expect("capabilities present");
            let ready = client.ready_snapshot().expect("ready present");
            caps.require_native_contract()
                .expect("HelloAck capabilities satisfy native contract");
            assert!(caps
                .deployment_id
                .as_deref()
                .is_some_and(|id| !id.is_empty()));
            assert!(caps
                .configuration_epoch
                .as_deref()
                .is_some_and(|epoch| !epoch.is_empty()));
            assert!(caps.auth);
            assert!(ready.ready);
            assert!(ready.storage);
            let ack: ShutdownAck = client
                .shutdown("handshake-done")
                .await
                .expect("shutdown ack");
            assert!(ack.checkpointed);
            server_task2.await.expect("join 2").expect("serve 2 ok");
        }

        #[cfg(feature = "embedded")]
        #[tokio::test]
        async fn serve_streams_preserves_slow_chunked_frame_across_select_completions() {
            use tokio::io::AsyncWriteExt;

            let dir = tempfile::tempdir().expect("tempdir");
            let state_dir = dir.path().canonicalize().expect("canonical").join("state");
            let context = LocalBackendContext::open(state_dir, None, None, false)
                .await
                .expect("open context");
            let (server_io, client_io) = tokio::io::duplex(64 * 1024);
            let (server_read, server_write) = tokio::io::split(server_io);
            let (mut client_read, mut client_write) = tokio::io::split(client_io);
            let (_sig_tx, sig_rx) = oneshot::channel::<()>();
            let server_task = tokio::spawn(context.serve(server_read, server_write, async move {
                let _ = sig_rx.await;
            }));

            write_frame(
                &mut client_write,
                &IpcEnvelope::Hello(fleetworks_contracts::ipc::IpcHello::new(
                    "slow-chunk-test".to_owned(),
                    "1.0.0".to_owned(),
                )),
            )
            .await
            .expect("write Hello");
            match read_frame(&mut client_read).await.expect("read HelloAck") {
                Some(IpcEnvelope::HelloAck(ack)) => {
                    assert!(ack.compatible);
                    ack.capabilities
                        .expect("HelloAck capabilities")
                        .require_native_contract()
                        .expect("native contract valid");
                }
                other => panic!("expected HelloAck, got {other:?}"),
            }

            // Encode two requests; write Request 1 completely, then write only a split prefix
            // of Request 2 while Request 1 finishes and `request_tasks.join_next()` wakes `select!`.
            let req1 = IpcEnvelope::Request {
                request_id: 1,
                registration: None,
                op: IpcOperation::Ready,
            };
            let req2 = IpcEnvelope::Request {
                request_id: 2,
                registration: None,
                op: IpcOperation::Capabilities,
            };
            let req2_body = serde_json::to_vec(&req2).expect("serialize req2");
            let mut req2_wire = Vec::with_capacity(4 + req2_body.len());
            req2_wire.extend_from_slice(&(req2_body.len() as u32).to_be_bytes());
            req2_wire.extend_from_slice(&req2_body);

            write_frame(&mut client_write, &req1)
                .await
                .expect("write req1");

            // Send Request 2 slowly: 2 bytes of its 4-byte length prefix BEFORE reading req1's response.
            let (head, rest) = req2_wire.split_at(2);
            client_write
                .write_all(head)
                .await
                .expect("write partial length prefix");
            client_write.flush().await.expect("flush partial prefix");

            // Wait until Request 1 completes and its Response frame arrives (confirming
            // `request_tasks.join_next()` has woken and iterated `tokio::select!`).
            match read_frame(&mut client_read)
                .await
                .expect("read req1 response")
            {
                Some(IpcEnvelope::Response {
                    request_id: 1,
                    outcome: IpcOutcome::Ok(_),
                }) => {}
                other => panic!("expected req1 Ok response, got {other:?}"),
            }
            tokio::time::sleep(Duration::from_millis(25)).await;

            // Now trickle remaining bytes in two chunks across a short pause.
            let mid = rest.len() / 2;
            client_write
                .write_all(&rest[..mid])
                .await
                .expect("write req2 chunk 1");
            client_write.flush().await.expect("flush chunk 1");
            tokio::time::sleep(Duration::from_millis(10)).await;
            client_write
                .write_all(&rest[mid..])
                .await
                .expect("write req2 chunk 2");
            client_write.flush().await.expect("flush chunk 2");

            match read_frame(&mut client_read)
                .await
                .expect("read req2 response")
            {
                Some(IpcEnvelope::Response {
                    request_id: 2,
                    outcome: IpcOutcome::Ok(_),
                }) => {}
                other => panic!("expected req2 Ok response, got {other:?}"),
            }

            write_frame(
                &mut client_write,
                &IpcEnvelope::Shutdown {
                    reason: "slow_chunk_done".into(),
                },
            )
            .await
            .expect("write Shutdown");
            server_task.await.expect("join server").expect("server ok");
        }

        #[cfg(feature = "embedded")]
        #[tokio::test]
        async fn ipc_lifecycle_registration_seed_and_restart_resume_preserves_job_without_duplicates(
        ) {
            let _lifecycle_guard = crate::tests::EMBEDDED_LIFECYCLE_TEST_LOCK.lock().await;
            let dir = tempfile::tempdir().expect("tempdir");
            let state_dir = dir.path().canonicalize().expect("canonical").join("state");

            // Session 1: register workstation, inspect readiness, upload CSV, trigger SeedActors, then shut down.
            let context1 = LocalBackendContext::open(state_dir.clone(), None, None, false)
                .await
                .expect("open session 1");
            let (server_io, client_io) = tokio::io::duplex(256 * 1024);
            let (server_read, server_write) = tokio::io::split(server_io);
            let (client_read, client_write) = tokio::io::split(client_io);
            let (_sig1_tx, sig1_rx) = oneshot::channel::<()>();
            let server1 = tokio::spawn(context1.serve(server_read, server_write, async move {
                let _ = sig1_rx.await;
            }));

            let client1 = LocalIpcClient::connect_streams(
                client_read,
                client_write,
                "local-ipc-lifecycle",
                "1.0.0",
            )
            .await
            .expect("session 1 handshake");

            let receipt: RegistrationReceipt = client1
                .request_json(
                    None,
                    IpcOperation::RegistrationEnsure(RegistrationEnsureRequest::new(
                        "reg-local-1".into(),
                        None,
                        "install-local-0001".into(),
                    )),
                )
                .await
                .expect("registration ensure");
            assert!(receipt.is_consistent());

            let context_op = IpcOperation::from_post(
                "/api/v1/connect/registrations/context",
                &serde_json::json!({
                    "registration_id": receipt.registration_id,
                    "expected_revision": receipt.revision,
                }),
            )
            .expect("build context op");
            #[derive(serde::Deserialize)]
            struct ContextResponse {
                context: RegisteredContext,
            }
            let context_resp: ContextResponse = client1
                .request_json(None, context_op)
                .await
                .expect("registration context");
            let registered = context_resp.context;

            let readiness: WorkstationReadiness = client1
                .request_json(Some(registered.clone()), IpcOperation::WorkstationReadiness)
                .await
                .expect("workstation readiness");
            assert_eq!(readiness.deployment_id, registered.deployment_id);

            let csv = b"actor_id,actor_name,actor_type,aliases,name_normalized\nactor-beacon-1,Harbor Beacon Collective,organization,[],harbor beacon collective\n";
            let sha256 = format!("{:x}", sha2::Sha256::digest(csv));
            let upload_ticket: DatasetUploadTicket = client1
                .request_json(
                    Some(registered.clone()),
                    IpcOperation::DatasetUploadStart(
                        DatasetUploadRequest::new(
                            DatasetKind::ActorCatalog,
                            sha256,
                            csv.len() as u64,
                        )
                        .expect("upload req"),
                    ),
                )
                .await
                .expect("start upload");
            let upload_id = upload_ticket
                .upload_path
                .strip_prefix(DATASET_UPLOADS_PATH)
                .and_then(|rest| rest.strip_prefix('/'))
                .expect("upload id")
                .to_owned();

            let _chunk_empty = client1
                .request_payload(
                    Some(registered.clone()),
                    IpcOperation::DatasetUploadChunk {
                        upload_id: upload_id.clone(),
                        offset: 0,
                        data_b64: test_base64_encode(csv),
                    },
                )
                .await
                .expect("upload chunk");
            let uploaded: DatasetUpload = client1
                .request_json(
                    Some(registered.clone()),
                    IpcOperation::DatasetUploadFinish { upload_id },
                )
                .await
                .expect("finish upload");

            let seed_receipt: MaintenanceReceipt = client1
                .request_json(
                    Some(registered.clone()),
                    IpcOperation::SeedActors(
                        MaintenanceRequest::for_seed(
                            "seed-ipc-1".into(),
                            MutationContext::new(&registered).with_configuration_revision(0),
                            uploaded.dataset,
                        )
                        .expect("seed req"),
                    ),
                )
                .await
                .expect("seed actors submitted");
            let job_id = seed_receipt.job_id.clone();

            let ack = client1.shutdown("restart").await.expect("shutdown 1");
            assert!(ack.checkpointed);
            server1
                .await
                .expect("join server 1")
                .expect("server 1 clean");

            // Session 2: restart against the exact same --state-dir, verify the persisted seed job
            // finishes without duplicate jobs, then complete ReindexActors and verify actor search.
            let context2 = LocalBackendContext::open(state_dir, None, None, false)
                .await
                .expect("open session 2");
            let (server2_io, client2_io) = tokio::io::duplex(256 * 1024);
            let (server2_read, server2_write) = tokio::io::split(server2_io);
            let (client2_read, client2_write) = tokio::io::split(client2_io);
            let (_sig2_tx, sig2_rx) = oneshot::channel::<()>();
            let server2 = tokio::spawn(context2.serve(server2_read, server2_write, async move {
                let _ = sig2_rx.await;
            }));

            let client2 = LocalIpcClient::connect_streams(
                client2_read,
                client2_write,
                "local-ipc-lifecycle-2",
                "1.0.0",
            )
            .await
            .expect("session 2 handshake");

            let seed_deadline = tokio::time::Instant::now() + Duration::from_secs(20);
            loop {
                let jobs: Jobs = client2
                    .request_json(
                        Some(registered.clone()),
                        IpcOperation::JobsList(Default::default()),
                    )
                    .await
                    .expect("jobs list");
                let seed_count = jobs.rows.iter().filter(|row| row.id == job_id).count();
                assert_eq!(seed_count, 1, "seed job must exist without duplicates");
                if jobs
                    .rows
                    .iter()
                    .find(|row| row.id == job_id)
                    .is_some_and(|row| row.state == "completed")
                {
                    break;
                }
                assert!(
                    tokio::time::Instant::now() < seed_deadline,
                    "timed out waiting for resumed seed job {job_id} completion; rows={:?}",
                    jobs.rows
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }

            let reindex_receipt: MaintenanceReceipt = client2
                .request_json(
                    Some(registered.clone()),
                    IpcOperation::ReindexActors(
                        MaintenanceRequest::for_reindex(
                            "reindex-ipc-1".into(),
                            MutationContext::new(&registered).with_configuration_revision(0),
                        )
                        .expect("reindex req"),
                    ),
                )
                .await
                .expect("reindex actors submitted after seed");

            let search_deadline = tokio::time::Instant::now() + Duration::from_secs(20);
            loop {
                let jobs: Jobs = client2
                    .request_json(
                        Some(registered.clone()),
                        IpcOperation::JobsList(Default::default()),
                    )
                    .await
                    .expect("jobs list");
                assert_eq!(
                    jobs.rows
                        .iter()
                        .filter(|row| row.id == reindex_receipt.job_id)
                        .count(),
                    1,
                    "reindex job must exist without duplicates"
                );
                let reindex_done = jobs
                    .rows
                    .iter()
                    .find(|row| row.id == reindex_receipt.job_id)
                    .is_some_and(|row| row.state == "completed");
                if reindex_done {
                    let search: ActorSearchResponse = client2
                        .request_json(
                            Some(registered.clone()),
                            IpcOperation::ActorsSearch(
                                ActorSearchQuery::new("Harbor Beacon".into(), 10)
                                    .expect("valid search query"),
                            ),
                        )
                        .await
                        .expect("actor search");
                    if search.index.state == IndexState::Ready
                        && search.rows.iter().any(|hit| {
                            !hit.actor_id.is_empty() && hit.label == "Harbor Beacon Collective"
                        })
                    {
                        break;
                    }
                }
                assert!(
                    tokio::time::Instant::now() < search_deadline,
                    "timed out waiting for reindex completion and search visibility"
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }

            let ack2 = client2.shutdown("finished").await.expect("shutdown 2");
            assert!(ack2.checkpointed);
            server2
                .await
                .expect("join server 2")
                .expect("server 2 clean");
        }
    }
}
