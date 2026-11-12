//! Actual scoped membership plus real operational SQLite counts/pages.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::storage::bootstrap;
use argos_osint_core::storage::connect_registry;
use argos_osint_core::storage::connect_registry::RegistryAuthority;
use argos_osint_core::storage::job_context::InteractiveAdmission;
use argos_osint_core::storage::job_dispatch;
use argos_osint_core::storage::jobs;
use argos_osint_core::storage::jobs::NewJob;
use argos_osint_core::storage::jobs::NewTask;
use argos_osint_core::storage::jobs_client;
use argos_osint_core::storage::operational_logs;
use argos_osint_core::storage::operational_logs::CohortPlan;
use argos_osint_core::storage::operational_logs::CompleteLogCohort;
use argos_osint_core::storage::operational_logs::LogDirection;
use argos_osint_core::storage::operational_logs::LogFilters;
use argos_osint_core::storage::operational_logs::LogSelection;
use argos_osint_core::storage::operational_logs::OperationalLogStore;
use argos_osint_core::storage::repo::Repository;
use chrono::Utc;
use fleetworks_contracts::connect::RegistrationEnsureRequest;
use fleetworks_contracts::connect::WorkspaceName;
use fleetworks_contracts::context::ActivityScope;
use fleetworks_contracts::context::JobOrigin;
use fleetworks_contracts::context::OriginSource;
use fleetworks_contracts::context::RegisteredContext;
use serde_json::json;
use support::Server;

fn authority() -> RegistryAuthority {
    RegistryAuthority::new("logs-scope", "native", "logs-access")
        .expect("explicit fixture authority")
}
async fn context(repo: &Repository, node: &str, child: &str) -> RegisteredContext {
    let receipt = connect_registry::ensure(
        repo,
        &authority(),
        &RegistrationEnsureRequest::new(
            format!("ensure-{node}-{child}"),
            if node == "a" {
                "192.0.2.42"
            } else {
                "198.51.100.42"
            }
            .parse()
            .expect("synthetic IP"),
            WorkspaceName::new(child).expect("child"),
            format!("fixture-installation-{node}"),
            format!("fixture-folder-{node}-{child}"),
        ),
    )
    .await
    .expect("actual registration")
    .receipt;
    connect_registry::registration_context(
        repo,
        &authority(),
        &receipt.registration_id,
        receipt.revision,
    )
    .await
    .expect("actual context")
}
async fn enqueue(repo: &Repository, context: &RegisteredContext, id: &str, request: &str) {
    let capture = InteractiveAdmission::native(
        authority(),
        context.clone(),
        JobOrigin::new(
            context,
            OriginSource::Native,
            request.into(),
            Utc::now().to_rfc3339(),
            0,
            Vec::new(),
        ),
    )
    .expect("fixture server admission");
    let task = format!("{id}-execute");
    let now = Utc::now();
    job_dispatch::submit_interactive(
        repo.db(),
        &NewJob {
            id,
            kind: "task",
            operation: "logs_fixture",
            parent: None,
            created_at: now,
        },
        &NewTask {
            id: &task,
            job: id,
            pool: "logs_fixture",
            operation: "logs_fixture",
            priority: 100,
            deps: Vec::new(),
            input_hash: "same-fixture-input",
            created_at: now,
        },
        &capture,
    )
    .await
    .expect("actual attributed job acceptance");
}
fn complete(plan: CohortPlan) -> CompleteLogCohort {
    let CohortPlan::Complete(cohort) = plan else {
        panic!("complete authorized cohort expected");
    };
    cohort
}

#[tokio::test]
async fn operational_rows_counts_and_descendants_are_filtered_before_limits_and_cap_never_claims_completeness(
) {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("fixture", "operational_log_scope").await;
    bootstrap::ensure_network_identity(repo.db())
        .await
        .expect("actual identity");
    connect_registry::initialize_control(repo.db())
        .await
        .expect("control");
    job_dispatch::enable_pool(repo.db(), "logs_fixture")
        .await
        .expect("explicit fixture consumer");
    let a = context(&repo, "a", "research").await;
    let sibling = context(&repo, "a", "analysis").await;
    let b = context(&repo, "b", "research").await;
    enqueue(&repo, &a, "a-root", "a-root-request").await;
    enqueue(&repo, &sibling, "a-sibling", "a-sibling-request").await;
    enqueue(&repo, &b, "b-root", "b-root-request").await;
    enqueue(&repo, &a, "shared-root", "shared-a").await;
    enqueue(&repo, &b, "shared-root", "shared-b").await;
    for id in ["a-child", "unattributed-child"] {
        jobs::create_job(
            repo.db(),
            &NewJob {
                id,
                kind: "task",
                operation: "logs_child",
                parent: Some("a-root"),
                created_at: Utc::now(),
            },
            &[],
        )
        .await
        .expect("real causal child");
    }
    // Deliberately remove only this fixture's attribution: parent links alone
    // must never grant operational log visibility to historical children.
    repo.db()
        .query("DELETE $record RETURN NONE")
        .bind((
            "record",
            surrealdb::types::RecordId::new("job_origin", "unattributed-child"),
        ))
        .await
        .expect("controlled legacy child")
        .check()
        .expect("legacy fixture statement");
    jobs::create_job(
        repo.db(),
        &NewJob {
            id: "legacy-root",
            kind: "task",
            operation: "legacy",
            parent: None,
            created_at: Utc::now(),
        },
        &[],
    )
    .await
    .expect("unassigned legacy job");
    jobs::create_job(
        repo.db(),
        &NewJob {
            id: "system-feed",
            kind: "service",
            operation: "api_server",
            parent: None,
            created_at: Utc::now(),
        },
        &[],
    )
    .await
    .expect("system service");
    let temporary = tempfile::tempdir().expect("temporary operational store");
    let store = OperationalLogStore::open(temporary.path().join("logs.sqlite3"))
        .await
        .expect("actual Jobs SQLite");
    jobs_client::install_local(store.clone()).expect("fixture's exclusive log owner");
    for (id, job, message) in [
        ("root-log", "a-root", "allowed root"),
        ("foreign-log", "b-root", "foreign private"),
        ("system-log", "system-feed", "daemon private"),
        ("child-log", "a-child", "allowed causal"),
        (
            "legacy-child-log",
            "unattributed-child",
            "legacy child private",
        ),
        ("shared-log", "shared-root", "mixed private"),
        ("sibling-log", "a-sibling", "allowed sibling"),
        ("legacy-log", "legacy-root", "legacy root private"),
    ] {
        store.log_append(json!({"id":id,"ts":Utc::now().to_rfc3339(),"job_id":job,"app":"tools","severity":"info","message":message,"details":"hidden prompt"})).await.expect("real operational log append");
    }
    let scope = ActivityScope::for_workstation(&a);
    let whole = complete(
        operational_logs::plan(&repo, "logs-scope", &scope, &LogSelection::workstation())
            .await
            .expect("whole workstation authority"),
    );
    assert_eq!(whole.len(), 3);
    assert!(whole.excluded_shared());
    let page = jobs_client::scoped_logs::page(
        &whole,
        LogFilters::default(),
        LogDirection::Older,
        None,
        None,
        1,
    )
    .await
    .expect("actual scoped SQL page");
    assert_eq!(page.counts.total, 3);
    assert_eq!(page.rows[0].row.job_id, "a-sibling");
    assert!(page.has_more);
    let next = jobs_client::scoped_logs::page(
        &whole,
        LogFilters::default(),
        LogDirection::Older,
        Some(page.upper_seq),
        page.next_boundary,
        100,
    )
    .await
    .expect("complete continuation");
    assert_eq!(next.rows.len(), 2);
    assert!(next
        .rows
        .iter()
        .all(|row| ["a-root", "a-child"].contains(&row.row.job_id.as_str())));
    assert!(!next.has_more);
    let child_scope = ActivityScope::for_selected_workspace(&a);
    let selected = complete(
        operational_logs::plan(
            &repo,
            "logs-scope",
            &child_scope,
            &LogSelection::job("a-root".into(), true).expect("root selector"),
        )
        .await
        .expect("authorized descendants"),
    );
    assert_eq!(selected.len(), 2);
    let exact = complete(
        operational_logs::plan(
            &repo,
            "logs-scope",
            &child_scope,
            &LogSelection::job("a-root".into(), false).expect("exact selector"),
        )
        .await
        .expect("exact job"),
    );
    assert_eq!(exact.len(), 1);
    assert!(matches!(
        operational_logs::plan(
            &repo,
            "logs-scope",
            &scope,
            &LogSelection::job("shared-root".into(), true).expect("shared selector")
        )
        .await
        .expect("shared status member"),
        CohortPlan::SharedPrivateUnavailable
    ));
    assert!(operational_logs::plan(
        &repo,
        "logs-scope",
        &scope,
        &LogSelection::job("b-root".into(), true).expect("guessed foreign selector")
    )
    .await
    .is_err());
    let mut foreign = scope.clone();
    foreign.deployment_id = "other-deployment".into();
    let empty = complete(
        operational_logs::plan(&repo, "logs-scope", &foreign, &LogSelection::workstation())
            .await
            .expect("strict deployment filter"),
    );
    assert!(empty.is_empty());
    assert_eq!(
        jobs_client::scoped_logs::page(
            &empty,
            LogFilters::default(),
            LogDirection::Older,
            None,
            None,
            100
        )
        .await
        .expect("empty local result")
        .counts
        .total,
        0
    );
    for index in 0..=operational_logs::COHORT_LIMIT {
        enqueue(
            &repo,
            &a,
            &format!("cap-job-{index}"),
            &format!("cap-request-{index}"),
        )
        .await;
    }
    assert!(
        matches!(
            operational_logs::plan(&repo, "logs-scope", &scope, &LogSelection::workstation())
                .await
                .expect("bounded complete cohort"),
            CohortPlan::TooLarge
        ),
        "above cap never issues an incomplete global cursor"
    );
    let narrow = complete(
        operational_logs::plan(
            &repo,
            "logs-scope",
            &ActivityScope::for_selected_workspace(&sibling),
            &LogSelection::workstation(),
        )
        .await
        .expect("narrowing retains complete semantics"),
    );
    assert_eq!(narrow.len(), 1);
}
