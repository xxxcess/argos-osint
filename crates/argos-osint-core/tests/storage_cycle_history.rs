//! Real history membership and tombstones preserve shared collection work.
#![cfg(feature = "surreal")]
mod support;

use argos_osint_core::storage::bootstrap;
use argos_osint_core::storage::connect_registry;
use argos_osint_core::storage::connect_registry::RegistryAuthority;
use argos_osint_core::storage::cycle_history;
use argos_osint_core::storage::job_context::InteractiveAdmission;
use argos_osint_core::storage::job_dispatch;
use argos_osint_core::storage::jobs::NewJob;
use argos_osint_core::storage::jobs::NewTask;
use argos_osint_core::storage::repo::Repository;
use chrono::Utc;
use fleetworks_contracts::connect::RegistrationEnsureRequest;
use fleetworks_contracts::context::ActivityScope;
use fleetworks_contracts::context::JobOrigin;
use fleetworks_contracts::context::MutationContext;
use fleetworks_contracts::context::OriginSource;
use fleetworks_contracts::context::RegisteredContext;
use fleetworks_contracts::cycle_history::DeleteCycleRequest;
use fleetworks_contracts::AtlasCycleReceipt;

fn authority() -> RegistryAuthority {
    RegistryAuthority::new("scope-a", "fixture-native", "fixture-access").expect("authority")
}
async fn register(repo: &Repository, node: &str) -> RegisteredContext {
    let receipt = connect_registry::ensure(
        repo,
        &authority(),
        &RegistrationEnsureRequest::new(
            format!("ensure-{node}"),
            None,
            format!("installation-{node}"),
        ),
    )
    .await
    .expect("registration")
    .receipt;
    connect_registry::registration_context(
        repo,
        &authority(),
        &receipt.registration_id,
        receipt.revision,
    )
    .await
    .expect("context")
}
fn admission(context: &RegisteredContext, request: &str) -> InteractiveAdmission {
    InteractiveAdmission::native(
        authority(),
        context.clone(),
        JobOrigin::new(
            context,
            OriginSource::Native,
            request.into(),
            Utc::now().to_rfc3339(),
            1,
            vec![],
        ),
    )
    .expect("admission")
}

#[tokio::test]
async fn cycle_history_tombstone_is_private_revisioned_idempotent_and_keeps_audit() {
    let Some(server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("argos", "cycle_history").await;
    bootstrap::ensure_network_identity(repo.db())
        .await
        .expect("database identity");
    connect_registry::initialize_control(repo.db())
        .await
        .expect("control");
    let one = register(&repo, "one").await;
    let other = register(&repo, "other").await;
    let cycle = AtlasCycleReceipt::from_request_id("fixture-cycle").expect("cycle");
    let now = Utc::now();
    let task = cycle.task_id();
    job_dispatch::enable_pool(repo.db(), job_dispatch::ATLAS_POOL)
        .await
        .expect("pool");
    job_dispatch::submit_interactive(
        repo.db(),
        &NewJob {
            id: &cycle.job_id,
            kind: "task",
            operation: fleetworks_contracts::ATLAS_CYCLE_OPERATION,
            parent: None,
            created_at: now,
        },
        &NewTask {
            id: &task,
            job: &cycle.job_id,
            pool: job_dispatch::ATLAS_POOL,
            operation: fleetworks_contracts::ATLAS_CYCLE_OPERATION,
            priority: 100,
            deps: vec![],
            input_hash: &cycle.run_id,
            created_at: now,
        },
        &admission(&one, "collect-once"),
    )
    .await
    .expect("durable collection");
    let scope = ActivityScope::for_workstation(&one);
    let before = cycle_history::list(&repo, "scope-a", &scope, 20)
        .await
        .expect("history");
    assert_eq!(before.rows.len(), 1);
    assert_eq!(before.rows[0].job_id, cycle.job_id);
    assert!(before.rows[0].article_count.is_none());
    assert!(cycle_history::list(
        &repo,
        "scope-a",
        &ActivityScope::for_workstation(&other),
        20
    )
    .await
    .expect("other history")
    .rows
    .is_empty());
    let denied = DeleteCycleRequest::new(
        "foreign-delete".into(),
        MutationContext::new(&other).with_configuration_revision(1),
        cycle.job_id.clone(),
        1,
    );
    assert!(
        cycle_history::delete(&repo, &admission(&other, "foreign-delete"), &denied)
            .await
            .is_err()
    );
    let request = DeleteCycleRequest::new(
        "delete-once".into(),
        MutationContext::new(&one).with_configuration_revision(1),
        cycle.job_id.clone(),
        1,
    );
    let proof = admission(&one, "delete-once");
    let receipt = cycle_history::delete(&repo, &proof, &request)
        .await
        .expect("tombstone");
    assert!(receipt.hidden);
    assert_eq!(receipt.revision, 2);
    assert_eq!(
        cycle_history::delete(&repo, &proof, &request)
            .await
            .expect("uncertain exact replay"),
        receipt
    );
    assert!(cycle_history::list(&repo, "scope-a", &scope, 20)
        .await
        .expect("hidden history")
        .rows
        .is_empty());
    let accepted = argos_osint_core::storage::job_context::accepted_request(
        &repo,
        &authority(),
        &one,
        &one.registration_id,
        cycle_history::DELETE_OPERATION,
        "delete-once",
    )
    .await
    .expect("read-only reconciliation")
    .expect("accepted deletion");
    assert_eq!(accepted.job_id, cycle.job_id);
    let mut check = repo
        .db()
        .query("RETURN record::exists($job); RETURN record::exists($origin);")
        .bind((
            "job",
            surrealdb::types::RecordId::new("job", cycle.job_id.clone()),
        ))
        .bind((
            "origin",
            surrealdb::types::RecordId::new("job_origin", cycle.job_id.clone()),
        ))
        .bind(("id", cycle.job_id))
        .bind(("operation", fleetworks_contracts::ATLAS_CYCLE_OPERATION))
        .await
        .expect("audit")
        .check()
        .expect("audit statements");
    assert_eq!(
        check.take::<Option<bool>>(0).expect("job remains"),
        Some(true)
    );
    assert_eq!(
        check.take::<Option<bool>>(1).expect("origin remains"),
        Some(true)
    );
}
