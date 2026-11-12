//! Actual scoped action transactions and retirement fences; no operator bypass.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::storage::connect_registry;
use argos_osint_core::storage::connect_registry::RegistryAuthority;
use argos_osint_core::storage::connect_registry::RegistryError;
use argos_osint_core::storage::connect_registry::RetirementResult;
use argos_osint_core::storage::job_actions;
use argos_osint_core::storage::job_actions::Action;
use argos_osint_core::storage::job_context::InteractiveAdmission;
use argos_osint_core::storage::job_dispatch;
use argos_osint_core::storage::jobs;
use argos_osint_core::storage::jobs::NewJob;
use argos_osint_core::storage::jobs::NewTask;
use argos_osint_core::storage::repo::Repository;
use chrono::Utc;
use fleetworks_contracts::connect::RegistrationEnsureRequest;
use fleetworks_contracts::connect::RegistrationReceipt;
use fleetworks_contracts::connect::WorkspaceName;
use fleetworks_contracts::context::JobOrigin;
use fleetworks_contracts::context::OriginSource;
use fleetworks_contracts::context::RegisteredContext;
use support::Server;

fn authority(credential: &str) -> RegistryAuthority {
    RegistryAuthority::new("fixture-scope", credential, "fixture-access")
        .expect("fixture authority")
}

async fn initialize(repo: &Repository) {
    repo.db()
        .query(
            "UPSERT storage_meta:identity SET instance_id='fixture-actions-deployment' RETURN NONE",
        )
        .await
        .expect("fixture deployment")
        .check()
        .expect("deployment statement");
    connect_registry::initialize_control(repo.db())
        .await
        .expect("explicit fixture control");
    job_dispatch::enable_pool(repo.db(), job_dispatch::ATLAS_POOL)
        .await
        .expect("explicit Atlas fixture consumer");
    repo.db()
        .query(job_actions::SCHEMA)
        .await
        .expect("action audit schema")
        .check()
        .expect("action schema statements");
}

async fn register(repo: &Repository, node: &str, name: &str, marker: &str) -> RegistrationReceipt {
    let ip = if node == "node-a" {
        "192.0.2.42"
    } else {
        "198.51.100.42"
    };
    connect_registry::ensure(
        repo,
        &authority("native-fixture"),
        &RegistrationEnsureRequest::new(
            format!("ensure-{node}-{marker}"),
            ip.parse().expect("synthetic node IP"),
            WorkspaceName::new(name).expect("fixture workspace"),
            format!("fixture-installation-{node}"),
            format!("fixture-workspace-{marker}"),
        ),
    )
    .await
    .expect("fixture registration")
    .receipt
}

async fn context(repo: &Repository, receipt: &RegistrationReceipt) -> RegisteredContext {
    connect_registry::registration_context(
        repo,
        &authority("native-fixture"),
        &receipt.registration_id,
        receipt.revision,
    )
    .await
    .expect("actual current registration context")
}

fn native(context: &RegisteredContext, request: &str) -> InteractiveAdmission {
    let origin = JobOrigin::new(
        context,
        OriginSource::Native,
        request.into(),
        Utc::now().to_rfc3339(),
        1,
        Vec::new(),
    );
    InteractiveAdmission::native(authority("native-fixture"), context.clone(), origin)
        .expect("trusted native adapter")
}

async fn enqueue(repo: &Repository, admission: &InteractiveAdmission, id: &str) {
    let task_id = format!("{id}:execute");
    job_dispatch::submit_interactive(
        repo.db(),
        &NewJob {
            id,
            kind: "task",
            operation: job_dispatch::ATLAS_POOL,
            parent: None,
            created_at: Utc::now(),
        },
        &NewTask {
            id: &task_id,
            job: id,
            pool: job_dispatch::ATLAS_POOL,
            operation: job_dispatch::ATLAS_POOL,
            priority: 100,
            deps: Vec::new(),
            input_hash: "fixture-input",
            created_at: Utc::now(),
        },
        admission,
    )
    .await
    .expect("actual attributed job/task/outbox admission");
}

async fn state(repo: &Repository, table: &str, id: &str) -> Option<String> {
    let mut response = repo
        .db()
        .query("RETURN type::record($table,$id).state;")
        .bind(("table", table.to_owned()))
        .bind(("id", id.to_owned()))
        .await
        .expect("fixture state read")
        .check()
        .expect("state statement");
    response.take(0).expect("bounded state")
}

#[tokio::test]
async fn queued_cancel_is_atomic_idempotent_and_shared_or_attempted_work_is_not_controllable() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "scoped_cancel_actions").await;
    initialize(&repo).await;
    let receipt = register(&repo, "node-a", "research", "research").await;
    let creator = context(&repo, &receipt).await;
    let other_receipt = register(&repo, "node-b", "analysis", "analysis").await;
    let other = context(&repo, &other_receipt).await;
    enqueue(&repo, &native(&creator, "submit-cancel"), "queued-owned").await;
    let action = native(&creator, "cancel-once");
    let availability =
        job_actions::availability(&repo, "fixture-scope", &creator, &["queued-owned".into()])
            .await
            .expect("batch availability");
    assert!(availability[0].can_cancel && !availability[0].can_retry);
    let before: Vec<String> = repo
        .db()
        .query("SELECT VALUE origin_json FROM job_origin WHERE job_id='queued-owned'")
        .await
        .expect("origin audit")
        .check()
        .expect("origin statement")
        .take(0)
        .expect("creator origin");
    let first = job_actions::perform(&repo, &action, Action::Cancel, "queued-owned")
        .await
        .expect("guarded cancellation");
    assert_eq!(first.state, "cancelled");
    assert_eq!(
        state(&repo, "job", "queued-owned").await.as_deref(),
        Some("cancelled")
    );
    assert_eq!(
        state(&repo, "task", "queued-owned:execute")
            .await
            .as_deref(),
        Some("cancelled")
    );
    assert_eq!(
        job_actions::perform(&repo, &action, Action::Cancel, "queued-owned")
            .await
            .expect("uncertain-response replay"),
        first
    );
    let mut response=repo.db().query("SELECT VALUE request_id FROM job_action WHERE job_id='queued-owned' LIMIT 2; SELECT VALUE origin_json FROM job_origin WHERE job_id='queued-owned'; SELECT VALUE [scope_id,deployment_id,registration_id,workstation_id,workspace_id,origin_json] FROM job_request WHERE job_id='queued-owned' LIMIT 2;")
        .await.expect("separate action audit").check().expect("audit statements");
    assert_eq!(
        response.take::<Vec<String>>(0).expect("one action receipt"),
        vec!["cancel-once"]
    );
    assert_eq!(
        response.take::<Vec<String>>(1).expect("unchanged creator"),
        before
    );
    assert_eq!(
        response
            .take::<Vec<Vec<String>>>(2)
            .expect("exact original request membership"),
        vec![vec![
            "fixture-scope".into(),
            creator.deployment_id.clone(),
            creator.registration_id.clone(),
            creator.workstation_id.clone(),
            creator.workspace_id.clone(),
            before[0].clone(),
        ]],
        "the separate action receipt cannot rewrite or add requester membership"
    );
    enqueue(&repo, &native(&creator, "submit-second"), "second-owned").await;
    assert!(
        job_actions::perform(&repo, &action, Action::Cancel, "second-owned")
            .await
            .is_err(),
        "one request ID cannot control a second job"
    );
    assert!(job_actions::perform(
        &repo,
        &native(&other, "guessed-cancel"),
        Action::Cancel,
        "second-owned"
    )
    .await
    .is_err());
    enqueue(
        &repo,
        &native(&other, "genuine-shared-request"),
        "second-owned",
    )
    .await;
    for actor in [&creator, &other] {
        assert!(
            job_actions::perform(
                &repo,
                &native(actor, "shared-cancel"),
                Action::Cancel,
                "second-owned"
            )
            .await
            .is_err(),
            "genuine shared membership is not cancel authority"
        );
    }
    assert_eq!(
        state(&repo, "job", "second-owned").await.as_deref(),
        Some("queued")
    );
    enqueue(
        &repo,
        &native(&creator, "submit-attempted"),
        "recovered-attempt",
    )
    .await;
    repo.db()
        .query("UPDATE type::record('task','recovered-attempt:execute') SET attempts=1 RETURN NONE")
        .await
        .expect("uncertain prior attempt fixture")
        .check()
        .expect("attempt statement");
    assert!(
        job_actions::perform(
            &repo,
            &native(&creator, "cancel-attempted"),
            Action::Cancel,
            "recovered-attempt"
        )
        .await
        .is_err(),
        "recovered queued state cannot prove a provider never started"
    );
}

#[tokio::test]
async fn atlas_retry_retains_creator_and_browser_revocation_is_checked_at_actual_commit() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "scoped_retry_actions").await;
    initialize(&repo).await;
    let receipt = register(&repo, "node-a", "research", "research").await;
    let creator = context(&repo, &receipt).await;
    enqueue(&repo, &native(&creator, "submit-retry"), "retry-owned").await;
    repo.db().query("UPDATE type::record('job','retry-owned') SET state='failed',finished_at=time::now() RETURN NONE; UPDATE type::record('task','retry-owned:execute') SET state='failed',attempts=1,finished_at=time::now() RETURN NONE;")
        .await.expect("finite failed worker outcome fixture").check().expect("failed outcome statements");
    let action = native(&creator, "retry-once");
    let before: Vec<String> = repo
        .db()
        .query("SELECT VALUE origin_json FROM job_origin WHERE job_id='retry-owned'")
        .await
        .expect("saved creator")
        .check()
        .expect("creator statement")
        .take(0)
        .expect("creator JSON");
    let retried = job_actions::perform(&repo, &action, Action::Retry, "retry-owned")
        .await
        .expect("actual guarded Atlas retry");
    assert_eq!(retried.state, "queued");
    assert_eq!(
        job_actions::perform(&repo, &action, Action::Retry, "retry-owned")
            .await
            .expect("idempotent retry receipt"),
        retried
    );
    let after: Vec<String> = repo
        .db()
        .query("SELECT VALUE origin_json FROM job_origin WHERE job_id='retry-owned'")
        .await
        .expect("creator after retry")
        .check()
        .expect("creator statement")
        .take(0)
        .expect("creator JSON");
    assert_eq!(before, after);
    assert_eq!(
        state(&repo, "task", "retry-owned:execute").await.as_deref(),
        Some("queued")
    );
    enqueue(&repo, &native(&creator, "submit-web-cancel"), "web-owned").await;
    let relay = authority("web-fixture");
    let session = connect_registry::create_session(&repo, &relay, &receipt.connect_id)
        .await
        .expect("actual browser session");
    let origin = JobOrigin::new(
        &session.context,
        OriginSource::IntelWeb,
        "browser-action".into(),
        Utc::now().to_rfc3339(),
        1,
        Vec::new(),
    );
    let browser = InteractiveAdmission::web(
        relay.clone(),
        session.context.clone(),
        origin,
        session.token.expose_secret().into(),
        session.context.context_revision,
    )
    .expect("protected browser proof");
    connect_registry::revoke_session(&repo, &relay, session.token.expose_secret())
        .await
        .expect("logout after preflight");
    assert!(
        job_actions::perform(&repo, &browser, Action::Cancel, "web-owned")
            .await
            .is_err(),
        "preflight cannot survive logout before the action transaction"
    );
    assert_eq!(
        state(&repo, "job", "web-owned").await.as_deref(),
        Some("queued")
    );
    let mut result = repo
        .db()
        .query("SELECT VALUE count() FROM job_action WHERE job_id='web-owned' GROUP ALL;")
        .await
        .expect("denied audit read")
        .check()
        .expect("audit statement");
    assert_eq!(
        result
            .take::<Vec<i64>>(0)
            .expect("denied action count")
            .first()
            .copied()
            .unwrap_or(0),
        0
    );
}

#[tokio::test]
async fn final_retirement_waits_for_tasks_and_causal_children_then_releases_only_name_claims() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "complete_workspace_retirement").await;
    initialize(&repo).await;
    let default = register(&repo, "node-a", "default", "default").await;
    assert!(matches!(
        connect_registry::begin_retirement(
            &repo,
            &authority("native-fixture"),
            &default.registration_id,
            default.revision
        )
        .await,
        Err(RegistryError::ProtectedDefault)
    ));
    let receipt = register(&repo, "node-a", "research", "old-research").await;
    let creator = context(&repo, &receipt).await;
    enqueue(
        &repo,
        &native(&creator, "submit-before-retire"),
        "retiring-root",
    )
    .await;
    let relay = authority("web-fixture");
    let session = connect_registry::create_session(&repo, &relay, &receipt.connect_id)
        .await
        .expect("session before retirement");
    let pending = connect_registry::begin_retirement(
        &repo,
        &authority("native-fixture"),
        &receipt.registration_id,
        receipt.revision,
    )
    .await
    .expect("admission retirement fence");
    assert!(connect_registry::validate_session(
        &repo,
        &relay,
        session.token.expose_secret(),
        Some(session.context.context_revision)
    )
    .await
    .is_err());
    assert!(matches!(
        connect_registry::finish_retirement(
            &repo,
            &authority("native-fixture"),
            &receipt.registration_id,
            pending.revision
        )
        .await
        .expect("active job drain check"),
        RetirementResult::Pending(_)
    ));
    repo.db().query("UPDATE type::record('job','retiring-root') SET state='completed' RETURN NONE; UPDATE type::record('task','retiring-root:execute') SET state='running' RETURN NONE;")
        .await.expect("terminal job/live task fixture").check().expect("state statements");
    assert!(matches!(
        connect_registry::finish_retirement(
            &repo,
            &authority("native-fixture"),
            &receipt.registration_id,
            pending.revision
        )
        .await
        .expect("active task drain check"),
        RetirementResult::Pending(_)
    ));
    jobs::create_job(
        repo.db(),
        &NewJob {
            id: "accepted-causal-child",
            kind: "task",
            operation: "fixture_child",
            parent: Some("retiring-root"),
            created_at: Utc::now(),
        },
        &[],
    )
    .await
    .expect("already accepted parent may extend while retiring");
    repo.db()
        .query(
            "UPDATE type::record('task','retiring-root:execute') SET state='completed' RETURN NONE",
        )
        .await
        .expect("task drained")
        .check()
        .expect("task statement");
    assert!(matches!(
        connect_registry::finish_retirement(
            &repo,
            &authority("native-fixture"),
            &receipt.registration_id,
            pending.revision
        )
        .await
        .expect("causal child drain check"),
        RetirementResult::Pending(_)
    ));
    repo.db()
        .query(
            "UPDATE type::record('job','accepted-causal-child') SET state='completed' RETURN NONE",
        )
        .await
        .expect("child drained")
        .check()
        .expect("child statement");
    let RetirementResult::Complete(complete) = connect_registry::finish_retirement(
        &repo,
        &authority("native-fixture"),
        &receipt.registration_id,
        pending.revision,
    )
    .await
    .expect("authoritative final retirement") else {
        panic!("all associated work is terminal");
    };
    assert!(complete.admission_fenced && complete.sessions_revoked && complete.active_work_drained);
    let replay = connect_registry::begin_retirement(
        &repo,
        &authority("native-fixture"),
        &receipt.registration_id,
        receipt.revision,
    )
    .await
    .expect("old-revision retirement reconciliation");
    assert_eq!(
        connect_registry::finish_retirement(
            &repo,
            &authority("native-fixture"),
            &receipt.registration_id,
            replay.revision
        )
        .await
        .expect("stable completed receipt"),
        RetirementResult::Complete(complete)
    );
    assert!(
        jobs::create_job(
            repo.db(),
            &NewJob {
                id: "too-late-child",
                kind: "task",
                operation: "fixture_child",
                parent: Some("retiring-root"),
                created_at: Utc::now()
            },
            &[]
        )
        .await
        .is_err(),
        "a drained receipt fences subsequent causal child creation"
    );
    assert!(
        connect_registry::ensure(
            &repo,
            &authority("native-fixture"),
            &RegistrationEnsureRequest::new(
                "old-marker-retry".into(),
                "192.0.2.42".parse().expect("node IP"),
                WorkspaceName::new("research").expect("child name"),
                "fixture-installation-node-a".into(),
                "fixture-workspace-old-research".into()
            )
        )
        .await
        .is_err(),
        "old marker cannot revive tombstone"
    );
    let fresh = register(&repo, "node-a", "research", "new-research").await;
    assert_ne!(fresh.workspace_id, receipt.workspace_id);
    assert_ne!(fresh.registration_id, receipt.registration_id);
    assert_eq!(fresh.connect_id, receipt.connect_id);
    assert_eq!(
        state(&repo, "connect_registration", &receipt.registration_id)
            .await
            .as_deref(),
        Some("retired")
    );
    assert_eq!(
        state(&repo, "connect_workspace", &receipt.workspace_id)
            .await
            .as_deref(),
        Some("retired")
    );
}
