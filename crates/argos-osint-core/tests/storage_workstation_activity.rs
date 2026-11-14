//! Requester membership must filter every activity read before pagination.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::jobs_view::JobFilter;
use argos_osint_core::storage::bootstrap;
use argos_osint_core::storage::connect_registry;
use argos_osint_core::storage::connect_registry::RegistryAuthority;
use argos_osint_core::storage::job_context::InteractiveAdmission;
use argos_osint_core::storage::job_dispatch;
use argos_osint_core::storage::jobs;
use argos_osint_core::storage::jobs::NewJob;
use argos_osint_core::storage::jobs::NewTask;
use argos_osint_core::storage::jobs_view as view;
use argos_osint_core::storage::repo::Repository;
use chrono::Utc;
use fleetworks_contracts::connect::RegistrationEnsureRequest;
use fleetworks_contracts::context::ActivityScope;
use fleetworks_contracts::context::JobOrigin;
use fleetworks_contracts::context::OriginSource;
use fleetworks_contracts::context::RegisteredContext;
use support::Server;
use surrealdb::engine::any::Any;
use surrealdb::Surreal;

fn authority() -> RegistryAuthority {
    RegistryAuthority::new("scope-a", "fixture-native", "fixture-access")
        .expect("fixture authority")
}

async fn register(repo: &Repository, node: &str, child: &str) -> RegisteredContext {
    bootstrap::ensure_network_identity(repo.db())
        .await
        .expect("explicit fixture bootstrap identity");
    connect_registry::initialize_control(repo.db())
        .await
        .expect("explicit fixture control");
    let ip = if node == "node-a" {
        "192.0.2.42"
    } else {
        "198.51.100.42"
    };
    let receipt = connect_registry::ensure(
        repo,
        &authority(),
        &RegistrationEnsureRequest::new(
            format!("ensure-{node}-{child}"),
            Some(ip.parse().expect("synthetic IP")),
            format!("installation-{node}"),
        ),
    )
    .await
    .expect("actual fixture registration")
    .receipt;
    connect_registry::registration_context(
        repo,
        &authority(),
        &receipt.registration_id,
        receipt.revision,
    )
    .await
    .expect("actual current registration context")
}

fn scope(context: &RegisteredContext, child: bool) -> ActivityScope {
    let _ = child;
    ActivityScope::for_workstation(context)
}

async fn job(db: &Surreal<Any>, id: &str, parent: Option<&str>, kind: &str) {
    jobs::create_job(
        db,
        &NewJob {
            id,
            parent,
            kind,
            operation: "summaries",
            created_at: Utc::now(),
        },
        &[],
    )
    .await
    .expect("fixture job");
    db.query("UPDATE $job SET state='queued', app='intel', title=$title, error_summary=$error, provider='private-provider', result_ref='private-result' RETURN NONE")
        .bind(("job", surrealdb::types::RecordId::new("job", id.to_owned())))
        .bind(("title", format!("private-title-{id}"))).bind(("error", format!("private-error-{id}")))
        .await.expect("fixture metadata").check().expect("fixture metadata statement");
}

async fn associate(
    repo: &Repository,
    context: &RegisteredContext,
    id: &str,
    key: &str,
    kind: &str,
) {
    let origin = JobOrigin::new(
        context,
        OriginSource::Native,
        key.into(),
        Utc::now().to_rfc3339(),
        1,
        Vec::new(),
    );
    let admission = InteractiveAdmission::native(authority(), context.clone(), origin)
        .expect("trusted fixture adapter");
    job_dispatch::enable_pool(repo.db(), "activity_fixture")
        .await
        .expect("explicit fixture consumer");
    let task_id = format!("{id}:execute");
    job_dispatch::submit_interactive(
        repo.db(),
        &NewJob {
            id,
            parent: None,
            kind,
            operation: "summaries",
            created_at: Utc::now(),
        },
        &NewTask {
            id: &task_id,
            job: id,
            pool: "activity_fixture",
            operation: "summaries",
            input_hash: "fixture-input",
            priority: 0,
            deps: Vec::new(),
            created_at: Utc::now(),
        },
        &admission,
    )
    .await
    .expect("actual proof-fenced job/task/outbox admission");
    repo.db().query("UPDATE $job SET app='intel', title=$title, error_summary=$error, provider='private-provider', result_ref='private-result' RETURN NONE")
        .bind(("job", surrealdb::types::RecordId::new("job", id.to_owned())))
        .bind(("title", format!("private-title-{id}"))).bind(("error", format!("private-error-{id}")))
        .await.expect("fixture metadata").check().expect("fixture metadata statement");
}

#[tokio::test]
async fn node_child_deployment_and_causal_membership_filter_before_limits_and_facets() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "workstation_activity_scope").await;
    let db = repo.db();
    let first = register(&repo, "node-a", "research").await;
    let second = register(&repo, "node-a", "analysis").await;
    assert_eq!(first.workstation_id, second.workstation_id);
    assert_eq!(first.registration_id, second.registration_id);
    let foreign = register(&repo, "node-b", "research").await;
    let ordering_epoch = Utc::now();
    for (id, context) in [
        ("owned-a", &first),
        ("owned-b", &second),
        ("other-node", &foreign),
    ] {
        associate(&repo, context, id, id, "task").await;
    }
    job(db, "other-deployment", None, "task").await;
    // Adversarial foreign-deployment metadata must not grant membership.
    db.query("CREATE job_origin:other_deployment SET job_id='other-deployment', root_id='other-deployment', scope_id='scope-a', deployment_id='foreign-deployment', workstation_id=$node, workspace_id=$child, origin_json='{}'; CREATE job_request:other_deployment SET request_key='foreign-deployment', operation='summaries', job_id='other-deployment', scope_id='scope-a', deployment_id='foreign-deployment', registration_id=$registration, workstation_id=$node, workspace_id=$child, requested_at=$now, origin_json='{}';")
        .bind(("node",first.workstation_id.clone())).bind(("child",String::from("legacy-child")))
        .bind(("registration",first.registration_id.clone())).bind(("now",Utc::now().to_rfc3339()))
        .await.expect("adversarial foreign fixture").check().expect("foreign fixture statement");
    job(db, "legacy", None, "task").await;
    associate(&repo, &first, "system", "service-request", "service").await;
    // The production child path copies origin under the real registry fence.
    job(db, "causal-child", Some("owned-a"), "task").await;
    job(db, "parent-link-only", None, "task").await;
    db.query("UPDATE $child SET parent=$parent,root=$parent RETURN NONE")
        .bind((
            "child",
            surrealdb::types::RecordId::new("job", "parent-link-only"),
        ))
        .bind(("parent", surrealdb::types::RecordId::new("job", "owned-a")))
        .await
        .expect("adversarial parent-only fixture")
        .check()
        .expect("parent fixture statement");
    db.query("FOR $i IN 0..205 { LET $job = type::record('job', string::concat('foreign-', <string>$i)); CREATE $job SET root=$job, kind='task', operation='summaries', state='running', app='foreign-only', created_at=time::now(), updated_at=<datetime>$foreign_updated; };")
        .bind(("foreign_updated", (ordering_epoch + chrono::Duration::seconds(1)).to_rfc3339()))
        .await.expect("crowded foreign jobs").check().expect("crowded foreign statements");
    // Preserve genuine admission/origins; pin only mutable ordering metadata.
    // Tied owned rows use ascending key, while the newer foreign crowd must be
    // excluded before either the large or one-row LIMIT is applied.
    db.query("UPDATE job SET updated_at=<datetime>$owned_updated WHERE id IN [$first,$second] RETURN NONE")
        .bind(("owned_updated", ordering_epoch.to_rfc3339()))
        .bind(("first", surrealdb::types::RecordId::new("job", "owned-a")))
        .bind(("second", surrealdb::types::RecordId::new("job", "owned-b")))
        .await.expect("controlled tied ordering after real admission").check().expect("ordering statement");
    let node = scope(&first, false);
    let rows = view::list_activity_jobs(db, "scope-a", &node, &JobFilter::default(), 200)
        .await
        .expect("node activity");
    let ids: std::collections::BTreeSet<_> = rows.iter().map(|row| row.job.id.as_str()).collect();
    assert_eq!(ids, ["owned-a", "owned-b"].into_iter().collect());
    let small_page = view::list_activity_jobs(db, "scope-a", &node, &JobFilter::default(), 1)
        .await
        .expect("small page");
    assert_eq!(small_page.len(), 1);
    assert_eq!(
        small_page[0].job.id, "owned-a",
        "ascending key breaks equal rank/update ties after membership filtering"
    );
    let child = scope(&first, true);
    assert_eq!(
        view::list_activity_jobs(db, "scope-a", &child, &JobFilter::default(), 100)
            .await
            .expect("child page")[0]
            .job
            .id,
        "owned-a"
    );
    assert_eq!(
        view::activity_job_counts(db, "scope-a", &node)
            .await
            .expect("scoped counts")
            .queued,
        2
    );
    assert_eq!(
        view::activity_job_apps(db, "scope-a", &node)
            .await
            .expect("scoped app facets"),
        vec!["intel"]
    );
    assert!(
        view::get_activity_job(db, "scope-a", &child, "causal-child")
            .await
            .expect("causal child")
            .is_some()
    );
    for id in [
        "other-node",
        "other-deployment",
        "legacy",
        "system",
        "parent-link-only",
    ] {
        assert!(
            view::get_activity_job(db, "scope-a", &child, id)
                .await
                .expect("guessed ID check")
                .is_none(),
            "unauthorized guessed ID {id}"
        );
    }
    assert!(
        view::list_activity_jobs(db, "different-scope", &node, &JobFilter::default(), 100)
            .await
            .expect("other authority")
            .is_empty()
    );
}

#[tokio::test]
async fn shared_reuse_preserves_creator_but_private_fields_and_search_stay_unavailable() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "workstation_activity_shared").await;
    let db = repo.db();
    let first = register(&repo, "node-a", "research").await;
    let other = register(&repo, "node-b", "analysis").await;
    associate(&repo, &first, "shared", "original", "task").await;
    let first_scope = scope(&first, true);
    let before = view::get_activity_job(db, "scope-a", &first_scope, "shared")
        .await
        .expect("original job")
        .expect("member");
    assert!(before.private_details_available);
    assert_eq!(before.job.title, "private-title-shared");
    associate(&repo, &other, "shared", "reuse-other-node", "task").await;
    for selected in [first_scope, scope(&other, true)] {
        let row = view::get_activity_job(db, "scope-a", &selected, "shared")
            .await
            .expect("shared job")
            .expect("genuine requester");
        assert!(
            row.status_only && !row.private_details_available && !row.activity_actions_available
        );
        assert_eq!(row.job.id, "shared");
        assert_eq!(row.job.state, "queued");
        assert!(!row.job.cancellable);
        let json = serde_json::to_string(&row).expect("row wire serialization");
        for private in [
            "private-title",
            "private-error",
            "private-provider",
            "private-result",
        ] {
            assert!(
                !json.contains(private),
                "private value escaped status projection"
            );
        }
        let filter = JobFilter {
            text: "private-title-shared".into(),
            ..Default::default()
        };
        assert!(
            view::list_activity_jobs(db, "scope-a", &selected, &filter, 100)
                .await
                .expect("private search oracle check")
                .is_empty()
        );
    }
    let mut result=db.query("RETURN job_origin:shared.workstation_id; SELECT VALUE workstation_id FROM job_request WHERE job_id='shared';")
        .await.expect("creator audit").check().expect("creator audit statements");
    assert_eq!(
        result
            .take::<Option<String>>(0)
            .expect("saved creator")
            .as_deref(),
        Some(first.workstation_id.as_str())
    );
    let mut requesters = result.take::<Vec<String>>(1).expect("request associations");
    let mut expected = vec![first.workstation_id.clone(), other.workstation_id.clone()];
    requesters.sort();
    expected.sort();
    assert_eq!(requesters, expected);
}
