//! Disposable synthetic fixtures only; production support remains disabled.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;

use tokio::sync::RwLock;

use super::*;

struct Observer {
    current: RwLock<RuntimeObservation>,
}
impl PolicyObserver for Observer {
    fn observe<'a>(&'a self, _: &'a ResetAuthority) -> ObservationFuture<'a> {
        Box::pin(async move { Ok(self.current.read().await.clone()) })
    }
    fn clear_owned_caches<'a>(
        &'a self,
        _: &'a ResetAuthority,
        configuration_epoch: &'a str,
    ) -> ClearCachesFuture<'a> {
        Box::pin(async move {
            for owner in &mut self.current.write().await.owners {
                owner.configuration_epoch = configuration_epoch.into();
                owner.cache_epoch = configuration_epoch.into();
            }
            Ok(())
        })
    }
}

fn observation(deployment_id: &str) -> RuntimeObservation {
    RuntimeObservation {
        scope_id: "fixture-scope".into(),
        deployment_id: deployment_id.into(),
        independent_operator_id: "fixture-operator".into(),
        independent_operator_fingerprint: CredentialFingerprint([99; 32]),
        users: vec![
            LoadedUserCredential {
                scope_id: "fixture-scope".into(),
                purpose: CredentialPurpose::Native,
                access_epoch: "old-access".into(),
                fingerprint: CredentialFingerprint([1; 32]),
            },
            LoadedUserCredential {
                scope_id: "fixture-scope".into(),
                purpose: CredentialPurpose::WebRelay,
                access_epoch: "old-access".into(),
                fingerprint: CredentialFingerprint([2; 32]),
            },
        ],
        owners: vec![OwnerObservation {
            scope_id: "fixture-scope".into(),
            deployment_id: deployment_id.into(),
            service: "fixture-tools".into(),
            instance_id: "fixture-process-1".into(),
            access_epoch: "old-access".into(),
            configuration_epoch: "old-configuration".into(),
            cache_epoch: "old-configuration".into(),
            reader_contract: OWNER_CONTRACT,
        }],
    }
}
fn write(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).expect("synthetic file write");
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).expect("private synthetic file");
}
fn files(root: &Path) -> BackendFiles {
    fs::set_permissions(root, fs::Permissions::from_mode(0o700)).expect("private root");
    BackendFiles::new(
        root.canonicalize().expect("synthetic canonical root"),
        fs::metadata(root).expect("root metadata").uid(),
    )
    .expect("owned fixture root")
}
fn coordinator(files: BackendFiles, observer: Arc<Observer>) -> Coordinator {
    Coordinator::new(files, observer, BTreeSet::from(["fixture-tools".into()]))
        .expect("fixed provisioned owner set")
}
fn authority() -> ResetAuthority {
    ResetAuthority::new("fixture-scope", "fixture-operator").expect("fixture operator")
}

#[test]
fn unsupported_build_and_wrong_owner_observations_fail_before_any_file_write() {
    let temporary = tempfile::tempdir().expect("temporary owner");
    let path = temporary.path().join("config.toml");
    write(&path, b"synthetic-old-user-key");
    let observer = Arc::new(Observer {
        current: RwLock::new(observation("fixture-deployment")),
    });
    let coordinator = coordinator(files(temporary.path()), observer);
    assert_eq!(coordinator.gate(), Err(ResetError::UnsupportedBuild));
    assert_eq!(
        fs::read(&path).expect("retained bytes"),
        b"synthetic-old-user-key"
    );
    assert!(!temporary.path().join(".argos-config.guard").exists());
    let mut observed = observation("fixture-deployment");
    observed.users[0].scope_id = "other-scope".into();
    assert_eq!(
        coordinator.check_observation(&authority(), "fixture-deployment", &observed),
        Err(ResetError::Forbidden)
    );
    observed = observation("fixture-deployment");
    observed.owners[0].deployment_id = "other-deployment".into();
    assert_eq!(
        coordinator.check_observation(&authority(), "fixture-deployment", &observed),
        Err(ResetError::UnsupportedBuild)
    );
}

#[test]
fn frozen_files_resume_only_empty_prefixes_and_preserve_other_data_and_new_keys() {
    let temporary = tempfile::tempdir().expect("temporary owner");
    let files = files(temporary.path());
    write(&temporary.path().join("config.toml"), b"synthetic-old-key");
    write(
        &temporary.path().join("auth.json"),
        b"synthetic-old-account",
    );
    write(
        &temporary.path().join("config.toml.4.bak"),
        b"synthetic-backup-key",
    );
    write(
        &temporary.path().join("domain.db"),
        b"protected-domain-bytes",
    );
    let lock = files.lock().expect("configuration lock");
    let inventory = files.inventory(&lock).expect("frozen exact inventory");
    // Simulate a crash during the first checked inode write. The lock protects
    // this deliberately synthetic mutation; no real user path is involved.
    write(
        &temporary.path().join("config.toml"),
        b"configuration_format = 1\n",
    );
    files
        .clear(&lock, &inventory, true)
        .expect("finite interrupted recovery");
    assert!(fs::read(temporary.path().join("config.toml.4.bak"))
        .expect("emptied exact backup")
        .is_empty());
    assert_eq!(
        fs::read(temporary.path().join("domain.db")).expect("protected domain"),
        b"protected-domain-bytes"
    );
    let snapshot = crate::current_configuration::read_locked(lock.root(), &lock)
        .expect("current-format empty configuration");
    assert!(snapshot.auth.accounts.is_empty());
    for role in crate::provider::CANONICAL_ROLE_KEYS {
        let assignment = snapshot
            .settings
            .defaults
            .role(role)
            .expect("explicit role");
        assert!(assignment.provider.is_empty());
        assert!(assignment.model.is_empty());
        assert!(assignment.account.is_empty());
        assert!(assignment.fallbacks.is_empty());
    }
    write(
        &temporary.path().join("auth.json"),
        b"newly-entered-synthetic-key",
    );
    assert_eq!(
        files.clear(&lock, &inventory, true),
        Err(ResetError::UnsafeFiles)
    );
    assert_eq!(
        fs::read(temporary.path().join("auth.json")).expect("new key retained"),
        b"newly-entered-synthetic-key"
    );
}

#[test]
fn checked_files_reject_substitution_symlink_hardlink_and_unreviewed_staging() {
    use std::os::unix::fs::symlink;
    let temporary = tempfile::tempdir().expect("temporary owner");
    let files = files(temporary.path());
    let config = temporary.path().join("config.toml");
    write(&config, b"old-synthetic-key");
    let lock = files.lock().expect("configuration lock");
    let inventory = files.inventory(&lock).expect("frozen inventory");
    fs::rename(&config, temporary.path().join("protected-original"))
        .expect("synthetic substitution");
    write(&config, b"old-synthetic-key");
    assert_eq!(
        files.clear(&lock, &inventory, true),
        Err(ResetError::UnsafeFiles)
    );
    fs::remove_file(&config).expect("remove synthetic substitute");
    symlink(temporary.path().join("protected-original"), &config).expect("synthetic symlink");
    assert_eq!(files.inventory(&lock).err(), Some(ResetError::UnsafeFiles));
    fs::remove_file(&config).expect("remove synthetic symlink");
    fs::hard_link(temporary.path().join("protected-original"), &config)
        .expect("synthetic hardlink");
    assert_eq!(files.inventory(&lock).err(), Some(ResetError::UnsafeFiles));
    fs::remove_file(&config).expect("remove synthetic hardlink");
    write(&config, b"old-synthetic-key");
    write(
        &temporary.path().join(".argos-tmp-unreviewed"),
        b"synthetic-staging",
    );
    assert_eq!(
        files.inventory(&lock).err(),
        Some(ResetError::UnreviewedSources)
    );
    assert_eq!(
        fs::read(temporary.path().join("protected-original")).expect("retained original"),
        b"old-synthetic-key"
    );
}

#[cfg(feature = "embedded")]
async fn assert_new_work_fenced(repo: &Repository, phase: &str) {
    use super::super::jobs::NewJob;
    for (kind, parent) in [("root", None), ("child", Some("fixture-accepted"))] {
        let id = format!("late-{kind}-{phase}");
        assert!(
            jobs::create_job(
                repo.db(),
                &NewJob {
                    id: &id,
                    kind: "task",
                    operation: "fixture_late_reset",
                    parent,
                    created_at: chrono::Utc::now(),
                },
                &[]
            )
            .await
            .is_err(),
            "{phase} cannot admit a late {kind}"
        );
    }
    let mut rows = repo
        .db()
        .query("SELECT VALUE record::id(id) FROM job WHERE operation='fixture_late_reset' LIMIT 1")
        .await
        .expect("bounded late work read")
        .check()
        .expect("late work statement");
    assert!(
        rows.take::<Vec<String>>(0)
            .expect("late work identities")
            .is_empty(),
        "denied root and causal creation both roll back"
    );
}

#[cfg(feature = "embedded")]
#[tokio::test]
async fn actual_fence_drain_epoch_once_reprovision_and_completed_replay_preserve_new_keys() {
    use super::super::bootstrap;
    use super::super::jobs::NewJob;
    use super::super::jobs::NewTask;
    use super::super::open::Target;
    use chrono::Utc;

    let temporary = tempfile::tempdir().expect("temporary reset fixture");
    let config_root = temporary.path().join("configuration-owner");
    fs::create_dir(&config_root).expect("temporary owner directory");
    let backend_files = files(&config_root);
    write(&config_root.join("config.toml"), b"old-synthetic-key");
    write(&config_root.join("auth.json"), b"old-synthetic-account");
    let repo = Repository::connect(
        &Target::Embedded {
            path: temporary.path().join("store"),
        },
        "fixture",
        "reset",
    )
    .await
    .expect("disposable isolated datastore");
    repo.db()
        .query(SCHEMA)
        .await
        .expect("reset schema transport")
        .check()
        .expect("reset schema statements");
    bootstrap::ensure_network_identity(repo.db())
        .await
        .expect("actual identity bootstrap");
    connect_registry::initialize_control(repo.db())
        .await
        .expect("actual control bootstrap");
    let deployment = run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
        connect_registry::deployment_state_in(&tx).await
    })
    .await
    .expect("actual deployment state")
    .value;
    let mut observed = observation(&deployment.deployment_id);
    observed.owners[0].configuration_epoch = deployment.configuration_epoch.clone();
    observed.owners[0].cache_epoch = deployment.configuration_epoch.clone();
    let observer = Arc::new(Observer {
        current: RwLock::new(observed),
    });
    let mut coordinator = coordinator(backend_files, observer.clone());
    let request = ResetRequest::new(
        "fixture-reset".into(),
        "a".repeat(64),
        deployment.deployment_id.clone(),
        deployment.configuration_epoch.clone(),
        "old-access".into(),
    );
    assert_eq!(
        coordinator
            .prepare(&repo, &authority(), &request)
            .await
            .err(),
        Some(ResetError::UnsupportedBuild)
    );
    assert!(!config_root.join(".argos-config.guard").exists());
    // Only this private inline fixture can construct the sealed support marker.
    coordinator.support = Some(CurrentOnlyBuild);
    let registry =
        connect_registry::RegistryAuthority::new("fixture-scope", "fixture-native", "old-access")
            .expect("actual fixture native authority");
    let registration = connect_registry::ensure(
        &repo,
        &registry,
        &fleetworks_contracts::connect::RegistrationEnsureRequest::new(
            "fixture-register".into(),
            "192.0.2.42".parse().expect("fixture IP"),
            fleetworks_contracts::connect::WorkspaceName::new("research").expect("fixture child"),
            "fixture-reset-installation".into(),
            "fixture-reset-workspace".into(),
        ),
    )
    .await
    .expect("actual prior registration")
    .receipt;
    let relay =
        connect_registry::RegistryAuthority::new("fixture-scope", "fixture-relay", "old-access")
            .expect("actual fixture relay authority");
    let old_session = connect_registry::create_session(&repo, &relay, &registration.connect_id)
        .await
        .expect("actual prior browser session");
    let now = Utc::now();
    jobs::create_job(
        repo.db(),
        &NewJob {
            id: "fixture-accepted",
            kind: "task",
            operation: "manual_tool",
            parent: None,
            created_at: now,
        },
        &[NewTask {
            id: "fixture-task",
            job: "fixture-accepted",
            pool: "manual_tools",
            operation: "manual_tool",
            priority: 100,
            deps: Vec::new(),
            input_hash: "synthetic",
            created_at: now,
        }],
    )
    .await
    .expect("real accepted job and task");
    repo.db().query("UPDATE job SET state='completed'; UPDATE task SET state='running'; CREATE reset_protected:domain SET value='retained';")
        .await.expect("synthetic terminal job/live task").check().expect("fixture statements");
    let prepared = coordinator
        .prepare(&repo, &authority(), &request)
        .await
        .expect("actual durable fence");
    assert_eq!(prepared.state, "draining");
    assert!(
        !prepared.work_drained,
        "terminal jobs with live tasks are not drained"
    );
    assert!(
        prepared.admission_fenced && prepared.first_use_blocked && prepared.user_access_revoked
    );
    assert_new_work_fenced(&repo, "prepared").await;
    assert!(connect_registry::validate_session(
        &repo,
        &relay,
        old_session.token.expose_secret(),
        Some(old_session.context.context_revision)
    )
    .await
    .is_err());
    let mut revoked = repo
        .db()
        .query("SELECT VALUE revoked FROM connect_session")
        .await
        .expect("actual session revocation read")
        .check()
        .expect("session revocation statement");
    assert_eq!(
        revoked.take::<Vec<bool>>(0).expect("revoked session"),
        vec![true]
    );
    assert_eq!(
        fs::read(config_root.join("config.toml")).expect("no premature wipe"),
        b"old-synthetic-key"
    );
    let repeated = coordinator
        .prepare(&repo, &authority(), &request)
        .await
        .expect("same reset replay");
    assert_eq!(repeated.configuration_epoch, prepared.configuration_epoch);
    assert_eq!(repeated.access_epoch, prepared.access_epoch);
    let old_fingerprint = CredentialFingerprint([1; 32]);
    assert_eq!(
        authorize_user_access(&repo, "fixture-scope", "old-access", &old_fingerprint).await,
        Err(ResetError::Forbidden)
    );
    repo.db()
        .query("UPDATE task SET state='completed' RETURN NONE")
        .await
        .expect("synthetic worker completion")
        .check()
        .expect("completion statement");
    // A controlled activation-state fault must keep the global reset fenced
    // even after the generic job/task reached their terminal states.
    repo.db().query("CREATE classifier_activation_operation:controlled_activation SET scope_id='fixture-scope',request_json='{}',request_hash='fixture-controlled-state',receipt_json='{}',state='running',result_json='',updated_at=time::now() RETURN NONE")
        .await.expect("controlled unresolved activation").check().expect("activation state statement");
    for state in ["running", "effect_uncertain"] {
        repo.db().query("UPDATE classifier_activation_operation:controlled_activation SET state=$state RETURN NONE").bind(("state",state))
            .await.expect("controlled activation fault").check().expect("fault statement");
        let unresolved = coordinator
            .prepare(&repo, &authority(), &request)
            .await
            .expect("actual auxiliary drain observation");
        assert_eq!(unresolved.state, "draining");
        assert!(!unresolved.work_drained);
    }
    repo.db().query("UPDATE classifier_activation_operation:controlled_activation SET state='rejected' RETURN NONE")
        .await.expect("end controlled unfinished state").check().expect("terminal state statement");
    let ready = coordinator
        .prepare(&repo, &authority(), &request)
        .await
        .expect("actual drained proof");
    assert_eq!(ready.state, "prepared");
    let input = CompletionRequest {
        version: 1,
        reset_id: request.reset_id.clone(),
        plan_sha256: request.plan_sha256.clone(),
        deployment_id: request.deployment_id.clone(),
        expected_configuration_epoch: request.expected_configuration_epoch.clone(),
        expected_access_epoch: request.expected_access_epoch.clone(),
        local_receipts: vec![LocalReceipt {
            target_name: "fixture-local".into(),
            inventory_sha256: request.plan_sha256.clone(),
            state: "files_reset".into(),
        }],
    };
    let cleared = coordinator
        .complete(&repo, &authority(), &input)
        .await
        .expect("actual backend payload/cache completion");
    assert_eq!(cleared.state, "backend_complete");
    assert!(cleared.admission_fenced && cleared.backend_configuration_cleared);
    assert_new_work_fenced(&repo, "backend-complete").await;
    assert_eq!(
        coordinator
            .finalize(&repo, &authority(), &input)
            .await
            .err(),
        Some(ResetError::ReprovisioningRequired)
    );
    {
        let mut policy = observer.current.write().await;
        for user in &mut policy.users {
            user.access_epoch = cleared.access_epoch.clone();
        }
        for owner in &mut policy.owners {
            owner.access_epoch = cleared.access_epoch.clone();
        }
    }
    assert_eq!(
        coordinator
            .finalize(&repo, &authority(), &input)
            .await
            .err(),
        Some(ResetError::ReprovisioningRequired),
        "relabeling old token epochs is insufficient"
    );
    {
        let mut policy = observer.current.write().await;
        for (index, user) in policy.users.iter_mut().enumerate() {
            user.fingerprint = CredentialFingerprint([10 + index as u8; 32]);
        }
    }
    let finished = coordinator
        .finalize(&repo, &authority(), &input)
        .await
        .expect("actual new policy and owner observations");
    assert_eq!(finished.state, "complete");
    assert!(!finished.admission_fenced && !finished.first_use_blocked);
    assert_eq!(
        authorize_user_access(
            &repo,
            "fixture-scope",
            &finished.access_epoch,
            &old_fingerprint
        )
        .await,
        Err(ResetError::Forbidden)
    );
    authorize_user_access(
        &repo,
        "fixture-scope",
        &finished.access_epoch,
        &CredentialFingerprint([10; 32]),
    )
    .await
    .expect("fresh current user credential");
    assert_eq!(
        authorize_independent_access(&repo, &old_fingerprint).await,
        Err(ResetError::Forbidden),
        "a revoked user bearer cannot regain authority as a relabeled Operator"
    );
    authorize_independent_access(&repo, &CredentialFingerprint([99; 32]))
        .await
        .expect("original independent recovery credential remains usable");
    jobs::create_job(
        repo.db(),
        &NewJob {
            id: "new-epoch-work",
            kind: "task",
            operation: "manual_tool",
            parent: None,
            created_at: Utc::now(),
        },
        &[],
    )
    .await
    .expect("new work after first-use reopens");
    write(
        &config_root.join("auth.json"),
        b"newly-entered-synthetic-key",
    );
    let repeated_complete = coordinator
        .complete(&repo, &authority(), &input)
        .await
        .expect("completed reset never rewrites");
    assert!(
        repeated_complete.work_drained,
        "durable old-epoch drain survives unrelated new accepted work"
    );
    coordinator
        .finalize(&repo, &authority(), &input)
        .await
        .expect("lost final response reconciliation");
    coordinator
        .prepare(&repo, &authority(), &request)
        .await
        .expect("completed prepare replay");
    assert_eq!(
        fs::read(config_root.join("auth.json")).expect("new key retained"),
        b"newly-entered-synthetic-key"
    );
    let mut wrong = request.clone();
    wrong.plan_sha256 = "b".repeat(64);
    assert_eq!(
        coordinator.prepare(&repo, &authority(), &wrong).await.err(),
        Some(ResetError::Conflict)
    );
    let mut retained = repo
        .db()
        .query("SELECT VALUE value FROM reset_protected:domain; SELECT VALUE <string>id FROM job;")
        .await
        .expect("protected data read")
        .check()
        .expect("read statements");
    assert_eq!(
        retained.take::<Vec<String>>(0).expect("retained domain"),
        vec!["retained"]
    );
    assert_eq!(
        retained
            .take::<Vec<String>>(1)
            .expect("retained jobs")
            .len(),
        2
    );
}
