//! Actual scoped action transactions and retirement fences; no operator bypass.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::storage::connect_registry;
use argos_osint_core::storage::connect_registry::RegistryAuthority;
use argos_osint_core::storage::job_actions;
use argos_osint_core::storage::job_actions::Action;
use argos_osint_core::storage::job_context::InteractiveAdmission;
use argos_osint_core::storage::job_dispatch;
use argos_osint_core::storage::jobs;
use argos_osint_core::storage::jobs::NewJob;
use argos_osint_core::storage::jobs::NewTask;
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::storage::setup_jobs;
use chrono::Utc;
use fleetworks_contracts::connect::RegistrationEnsureRequest;
use fleetworks_contracts::connect::RegistrationReceipt;
use fleetworks_contracts::context::JobOrigin;
use fleetworks_contracts::context::OriginSource;
use fleetworks_contracts::context::RegisteredContext;
use fleetworks_contracts::readiness::MaintenanceOperation;
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

async fn register(
    repo: &Repository,
    node: &str,
    registration_attempt: &str,
) -> RegistrationReceipt {
    let ip = if node == "node-a" {
        "192.0.2.42"
    } else {
        "198.51.100.42"
    };
    connect_registry::ensure(
        repo,
        &authority("native-fixture"),
        &RegistrationEnsureRequest::new(
            format!("ensure-{node}-{registration_attempt}"),
            Some(ip.parse().expect("synthetic node IP")),
            format!("fixture-installation-{node}"),
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
    let receipt = register(&repo, "node-a", "first-registration").await;
    let creator = context(&repo, &receipt).await;
    let other_receipt = register(&repo, "node-b", "other-registration").await;
    let other = context(&repo, &other_receipt).await;
    assert_ne!(creator.workstation_id, other.workstation_id);
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
    let mut response=repo.db().query("SELECT VALUE request_id FROM job_action WHERE job_id='queued-owned' LIMIT 2; SELECT VALUE origin_json FROM job_origin WHERE job_id='queued-owned'; SELECT VALUE [scope_id,deployment_id,registration_id,workstation_id,origin_json] FROM job_request WHERE job_id='queued-owned' LIMIT 2;")
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
    let receipt = register(&repo, "node-a", "first-registration").await;
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
async fn accepted_causal_work_survives_registration_replacement_but_node_revocation_fences_children(
) {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "workstation_causal_fence").await;
    initialize(&repo).await;
    let receipt = register(&repo, "node-a", "first-registration").await;
    let context = context(&repo, &receipt).await;
    enqueue(
        &repo,
        &native(&context, "parent-request"),
        "accepted-parent",
    )
    .await;
    // A migration retires the former registration, not the workstation lifecycle.
    repo.db()
        .query("UPDATE $record SET state='legacy_read_only',revision+=1 RETURN NONE")
        .bind((
            "record",
            surrealdb::types::RecordId::new(
                "connect_registration",
                receipt.registration_id.clone(),
            ),
        ))
        .await
        .expect("legacy fence")
        .check()
        .expect("legacy state");
    jobs::create_job(
        repo.db(),
        &NewJob {
            id: "accepted-child",
            kind: "task",
            operation: "fixture_child",
            parent: Some("accepted-parent"),
            created_at: Utc::now(),
        },
        &[],
    )
    .await
    .expect("accepted chain continues under workstation fence");
    let origin = argos_osint_core::storage::job_context::origin_for_job(&repo, "accepted-child")
        .await
        .expect("origin lookup")
        .expect("causal origin");
    assert_eq!(origin.registration_id, receipt.registration_id);
    assert_eq!(origin.workstation_id, receipt.workstation_id);
    repo.db()
        .query("UPDATE $node SET state='revoked',revision+=1 RETURN NONE")
        .bind((
            "node",
            surrealdb::types::RecordId::new("connect_workstation", receipt.workstation_id),
        ))
        .await
        .expect("node fence")
        .check()
        .expect("node state");
    assert!(jobs::create_job(
        repo.db(),
        &NewJob {
            id: "denied-child",
            kind: "task",
            operation: "fixture_child",
            parent: Some("accepted-parent"),
            created_at: Utc::now()
        },
        &[]
    )
    .await
    .is_err());
}

#[tokio::test]
async fn failed_actor_seed_retries_and_respects_active_resource_owner() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "retry_actor_seed").await;
    initialize(&repo).await;
    repo.db()
        .query(setup_jobs::SCHEMA)
        .await
        .expect("setup schema")
        .check()
        .expect("setup schema statements");
    job_dispatch::enable_pool(repo.db(), "actor_seed")
        .await
        .expect("enable actor_seed pool");
    setup_jobs::enable_operation(repo.db(), MaintenanceOperation::SeedActors, true)
        .await
        .expect("enable actor_seed operation");

    let receipt = register(&repo, "node-a", "seed-retry-reg").await;
    let ctx = context(&repo, &receipt).await;
    let now = Utc::now();
    for (job_id, req_id) in [("seed-failed", "seed-req-1"), ("seed-other", "seed-req-2")] {
        let admission = native(&ctx, req_id);
        job_dispatch::submit_interactive(
            repo.db(),
            &NewJob {
                id: job_id,
                kind: "maintenance",
                operation: "actor_seed",
                parent: None,
                created_at: now,
            },
            &NewTask {
                id: &format!("{job_id}-execute"),
                job: job_id,
                pool: "actor_seed",
                operation: "actor_seed",
                priority: 0,
                deps: Vec::new(),
                input_hash: "seed-payload-hash",
                created_at: now,
            },
            &admission,
        )
        .await
        .expect("submit actor_seed fixture");
        repo.db()
            .query("CREATE $op SET operation='actor_seed', payload_json='{}', payload_hash='seed-payload-hash', scope_id=$scope, updated_at=time::now() RETURN NONE")
            .bind(("op", surrealdb::types::RecordId::new("setup_operation", job_id)))
            .bind(("scope", authority("native-fixture").scope_id().to_owned()))
            .await
            .expect("setup operation record")
            .check()
            .expect("setup operation statement");
    }

    // Fail `seed-failed`, and assign `setup_resource:actor_catalog` to active `seed-other`.
    repo.db()
        .query(
            "UPDATE job:`seed-failed` SET state='failed', error_category='transient', error_summary='seed failed' RETURN NONE;
             UPDATE task:`seed-failed-execute` SET state='failed', attempts=1 RETURN NONE;
             UPSERT setup_resource:actor_catalog SET revision=1, job_id='seed-other', payload_hash='other-hash', scope_id='fixture-scope' RETURN NONE;",
        )
        .await
        .expect("mark seed failed with active resource holder")
        .check()
        .expect("state update");

    let blocked =
        job_actions::availability(&repo, "fixture-scope", &ctx, &["seed-failed".to_owned()])
            .await
            .expect("availability while active");
    assert_eq!(blocked.len(), 1);
    assert!(!blocked[0].can_retry);
    assert!(job_actions::perform(
        &repo,
        &native(&ctx, "retry-while-busy"),
        Action::Retry,
        "seed-failed"
    )
    .await
    .is_err());

    // Once `seed-other` also fails, `seed-failed` becomes retryable and rebinds the resource.
    repo.db()
        .query(
            "UPDATE job:`seed-other` SET state='failed' RETURN NONE;
             UPDATE task:`seed-other-execute` SET state='failed', attempts=1 RETURN NONE;",
        )
        .await
        .expect("fail other seed job")
        .check()
        .expect("fail other seed statement");

    let ready =
        job_actions::availability(&repo, "fixture-scope", &ctx, &["seed-failed".to_owned()])
            .await
            .expect("availability after conflict clears");
    assert_eq!(ready.len(), 1);
    assert!(ready[0].can_retry);

    let retried = job_actions::perform(
        &repo,
        &native(&ctx, "retry-seed-once"),
        Action::Retry,
        "seed-failed",
    )
    .await
    .expect("retry failed actor_seed");
    assert_eq!(retried.action, Action::Retry);
    assert_eq!(retried.state, "queued");
    assert_eq!(retried.affected_tasks, 1);

    let mut obs = repo
        .db()
        .query(
            "RETURN type::record('job','seed-failed').state;
             RETURN type::record('task','seed-failed-execute').state;
             RETURN setup_resource:actor_catalog.job_id;",
        )
        .await
        .expect("observe retried seed")
        .check()
        .expect("observe retried seed statements");
    assert_eq!(
        obs.take::<Option<String>>(0).unwrap().as_deref(),
        Some("queued")
    );
    assert_eq!(
        obs.take::<Option<String>>(1).unwrap().as_deref(),
        Some("queued")
    );
    assert_eq!(
        obs.take::<Option<String>>(2).unwrap().as_deref(),
        Some("seed-failed")
    );
}
