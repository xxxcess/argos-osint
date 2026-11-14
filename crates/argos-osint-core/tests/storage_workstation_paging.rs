//! Real registered scope, private predicates and creation-keyset paging.
#![cfg(feature = "surreal")]

mod support;

use std::collections::BTreeSet;

use argos_osint_core::jobs_view::JobStatusFilter;
use argos_osint_core::storage::bootstrap;
use argos_osint_core::storage::connect_registry;
use argos_osint_core::storage::connect_registry::RegistryAuthority;
use argos_osint_core::storage::job_context::InteractiveAdmission;
use argos_osint_core::storage::job_dispatch;
use argos_osint_core::storage::jobs;
use argos_osint_core::storage::jobs::NewJob;
use argos_osint_core::storage::jobs::NewTask;
use argos_osint_core::storage::jobs_paging;
use argos_osint_core::storage::jobs_paging::CreationWindow;
use argos_osint_core::storage::jobs_paging::Filters;
use argos_osint_core::storage::jobs_view;
use argos_osint_core::storage::jobs_view::JobMeta;
use argos_osint_core::storage::repo::Repository;
use chrono::DateTime;
use chrono::Duration;
use chrono::Utc;
use fleetworks_contracts::connect::RegistrationEnsureRequest;
use fleetworks_contracts::context::ActivityScope;
use fleetworks_contracts::context::JobOrigin;
use fleetworks_contracts::context::OriginSource;
use fleetworks_contracts::context::RegisteredContext;
use support::Server;
use surrealdb::types::RecordId;

const SCOPE: &str = "workstation-paging-scope";
const POOL: &str = "workstation_paging_fixture";

fn authority() -> RegistryAuthority {
    RegistryAuthority::new(SCOPE, "paging-native", "paging-access")
        .expect("explicit synthetic server authority")
}

async fn initialize(repo: &Repository) {
    bootstrap::ensure_network_identity(repo.db())
        .await
        .expect("actual deployment bootstrap identity");
    connect_registry::initialize_control(repo.db())
        .await
        .expect("actual registry control");
    job_dispatch::ensure_queue_transport(repo.db(), POOL)
        .await
        .expect("actual queue transport policy");
    job_dispatch::enable_pool(repo.db(), POOL)
        .await
        .expect("explicit fixture consumer pool");
}

async fn register(repo: &Repository, node: &str, child: &str) -> RegisteredContext {
    let ip = if node == "a" {
        "192.0.2.71"
    } else {
        "198.51.100.71"
    };
    let receipt = connect_registry::ensure(
        repo,
        &authority(),
        &RegistrationEnsureRequest::new(
            format!("ensure-{node}-{child}"),
            Some(ip.parse().expect("synthetic IP")),
            format!("paging-installation-{node}"),
        ),
    )
    .await
    .expect("actual registry acceptance")
    .receipt;
    connect_registry::registration_context(
        repo,
        &authority(),
        &receipt.registration_id,
        receipt.revision,
    )
    .await
    .expect("actual current registered context")
}

async fn associate(
    repo: &Repository,
    context: &RegisteredContext,
    id: &str,
    request_id: &str,
    kind: &str,
    created_at: DateTime<Utc>,
) {
    let admission = InteractiveAdmission::native(
        authority(),
        context.clone(),
        JobOrigin::new(
            context,
            OriginSource::Native,
            request_id.into(),
            Utc::now().to_rfc3339(),
            0,
            Vec::new(),
        ),
    )
    .expect("server-derived admission from actual registration");
    let task = format!("{id}:execute");
    job_dispatch::submit_interactive(
        repo.db(),
        &NewJob {
            id,
            parent: None,
            kind,
            operation: POOL,
            created_at,
        },
        &NewTask {
            id: &task,
            job: id,
            pool: POOL,
            operation: POOL,
            input_hash: "paging-fixture-input",
            priority: 0,
            deps: Vec::new(),
            created_at,
        },
        &admission,
    )
    .await
    .expect("real proof-fenced Job/task/outbox admission");
}

async fn metadata(repo: &Repository, id: &str, app: &str, correlation: &str) {
    jobs_view::set_job_meta(
        repo.db(),
        id,
        &JobMeta {
            app: Some(app.into()),
            title: Some(format!("private-title-{id}")),
            error_summary: Some(format!("private-error-{id}")),
            provider: Some("private-provider".into()),
            result_ref: Some("private-result".into()),
            correlation_id: Some(correlation.into()),
            ..Default::default()
        },
    )
    .await
    .expect("actual dashboard metadata writer");
}

fn ids(page: &jobs_paging::Page) -> Vec<String> {
    page.rows.iter().map(|row| row.job.id.clone()).collect()
}

#[tokio::test]
async fn registered_scope_precedes_window_counts_facets_and_newest_tied_keysets() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("paging", "workstation_creation_scope").await;
    initialize(&repo).await;
    let first = register(&repo, "a", "research").await;
    let sibling = register(&repo, "a", "analysis").await;
    let foreign = register(&repo, "b", "research").await;
    let observed = Utc::now();
    let base = observed - Duration::minutes(1);
    for (id, context, offset, app) in [
        ("newest", &first, 3, "atlas"),
        ("tie-a", &first, 2, "tools"),
        ("tie-b", &sibling, 2, "tools"),
        ("foreign", &foreign, 10, "foreign-only"),
    ] {
        associate(
            &repo,
            context,
            id,
            id,
            "task",
            base + Duration::seconds(offset),
        )
        .await;
        metadata(&repo, id, app, "scope-fixture").await;
    }
    associate(&repo, &first, "system", "system", "service", base).await;
    jobs::create_job(
        repo.db(),
        &NewJob {
            id: "causal-child",
            parent: Some("newest"),
            kind: "task",
            operation: POOL,
            created_at: base + Duration::seconds(1),
        },
        &[],
    )
    .await
    .expect("production causal-origin propagation");
    metadata(&repo, "causal-child", "tools", "scope-fixture").await;
    jobs::create_job(
        repo.db(),
        &NewJob {
            id: "parent-only",
            parent: None,
            kind: "task",
            operation: POOL,
            created_at: base,
        },
        &[],
    )
    .await
    .expect("unattributed adversarial child");
    repo.db().query("UPDATE $child SET parent=$parent,root=$parent RETURN NONE; FOR $i IN 0..205 { LET $job=type::record('job',string::concat('unassigned-',<string>$i)); CREATE $job SET root=$job,kind='task',operation='paging_unassigned',state='queued',app='unassigned-only',created_at=<datetime>$fixture_created,updated_at=<datetime>$fixture_created; };")
        .bind(("child", RecordId::new("job", "parent-only"))).bind(("parent", RecordId::new("job", "newest")))
        .bind(("fixture_created", (base+Duration::seconds(15)).to_rfc3339()))
        .await.expect("bounded adversarial unassigned/parent-only rows").check().expect("adversarial fixture statements");
    associate(
        &repo,
        &first,
        "expired",
        "expired",
        "task",
        observed - Duration::hours(73),
    )
    .await;
    let window = CreationWindow::at(observed);
    let scope = ActivityScope::for_workstation(&first);
    let filters = Filters::default();
    let first_page = jobs_paging::page(repo.db(), SCOPE, &scope, &filters, &window, None, 2)
        .await
        .expect("actual paged scoped read");
    assert_eq!(ids(&first_page), ["newest", "tie-a"]);
    assert_eq!(
        first_page.matched_total,
        Some(4),
        "real causal children join the new paged cohort"
    );
    assert_eq!(first_page.apps, ["atlas", "tools"]);
    assert!(first_page.apps_complete && first_page.has_more);
    let anchor = first_page
        .next_anchor
        .as_ref()
        .expect("genuine next keyset");
    associate(
        &repo,
        &first,
        "post-snapshot",
        "post-snapshot",
        "task",
        observed + Duration::seconds(1),
    )
    .await;
    let second_page =
        jobs_paging::page(repo.db(), SCOPE, &scope, &filters, &window, Some(anchor), 2)
            .await
            .expect("fixed window continuation");
    assert_eq!(
        ids(&second_page),
        ["tie-b", "causal-child"],
        "equal creation times use ID ascending without duplicates"
    );
    assert_eq!(second_page.matched_total, Some(4));
    assert!(!second_page.has_more && second_page.next_anchor.is_none());
    let child_scope = scope.clone();
    // Version two has no child narrowing.
    let child_page = jobs_paging::page(repo.db(), SCOPE, &child_scope, &filters, &window, None, 20)
        .await
        .expect("child narrows the actual node cohort");
    assert_eq!(
        ids(&child_page),
        ids(&first_page)
            .into_iter()
            .chain(ids(&second_page))
            .collect::<Vec<_>>()
    );
    let wrong = jobs_paging::page(
        repo.db(),
        "wrong-authority",
        &scope,
        &filters,
        &window,
        None,
        20,
    )
    .await
    .expect("non-enumerating other authority scope");
    assert!(wrong.rows.is_empty() && wrong.apps.is_empty());
    assert_eq!(wrong.matched_total, Some(0));
}

#[tokio::test]
async fn exact_kind_correlation_and_filtered_facets_never_probe_shared_private_fields() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("paging", "workstation_private_filters").await;
    initialize(&repo).await;
    let first = register(&repo, "a", "research").await;
    let foreign = register(&repo, "b", "analysis").await;
    let observed = Utc::now();
    let created = observed - Duration::minutes(1);
    for (id, kind, correlation) in [
        ("private-good", "task", "case-17"),
        ("near-correlation", "task", "case-17-extra"),
        ("near-kind", "task-extra", "case-17"),
        ("shared", "task", "case-17"),
    ] {
        associate(&repo, &first, id, id, kind, created).await;
        metadata(&repo, id, "tools", correlation).await;
        jobs::finalize_job(repo.db(), id, None, "completed", Utc::now())
            .await
            .expect("actual terminal-state writer");
    }
    associate(
        &repo,
        &foreign,
        "shared",
        "shared-other-node",
        "task",
        created,
    )
    .await;
    let scope = ActivityScope::for_workstation(&first);
    let window = CreationWindow::at(observed);
    let mut filters = Filters::default();
    filters.status = JobStatusFilter::Completed;
    filters.app = "tools".into();
    filters.operation = POOL.into();
    filters.kind = "task".into();
    filters.correlation_id = "case-17".into();
    let filtered = jobs_paging::page(repo.db(), SCOPE, &scope, &filters, &window, None, 1)
        .await
        .expect("all exact predicates precede counts/facets/LIMIT");
    assert_eq!(ids(&filtered), ["private-good"]);
    assert_eq!(filtered.matched_total, Some(1));
    assert_eq!(filtered.apps, ["tools"]);
    assert!(filtered.apps_complete && !filtered.has_more);
    let unfiltered = jobs_paging::page(
        repo.db(),
        SCOPE,
        &scope,
        &Filters::default(),
        &window,
        None,
        20,
    )
    .await
    .expect("shared status-only membership remains visible");
    let shared = unfiltered
        .rows
        .iter()
        .find(|row| row.job.id == "shared")
        .expect("genuine shared requester association");
    assert!(shared.status_only && !shared.private_details_available);
    let encoded = serde_json::to_string(shared).expect("safe row serialization");
    for private in [
        "private-title",
        "private-error",
        "private-provider",
        "private-result",
        "case-17",
    ] {
        assert!(
            !encoded.contains(private),
            "shared status projection leaked {private}"
        );
    }
    let mut private_text = Filters::default();
    private_text.text = "private-title-shared".into();
    let no_oracle = jobs_paging::page(repo.db(), SCOPE, &scope, &private_text, &window, None, 20)
        .await
        .expect("private text oracle is guarded");
    assert!(no_oracle.rows.is_empty() && no_oracle.apps.is_empty());
    assert_eq!(no_oracle.matched_total, Some(0));
    for index in 0..33 {
        let id = format!("facet-{index:02}");
        associate(&repo, &first, &id, &id, "task", created).await;
        metadata(&repo, &id, &format!("facet-app-{index:02}"), "facet-cohort").await;
    }
    let mut facet_filter = Filters::default();
    facet_filter.correlation_id = "facet-cohort".into();
    let facets = jobs_paging::page(repo.db(), SCOPE, &scope, &facet_filter, &window, None, 1)
        .await
        .expect("bounded filtered app facets are independent of one-row page");
    assert_eq!(facets.rows.len(), 1);
    assert_eq!(facets.matched_total, Some(33));
    assert_eq!(facets.apps.len(), 32);
    assert!(
        !facets.apps_complete,
        "the 33rd facet proves the bounded facet list is incomplete"
    );
    assert_eq!(facets.apps.iter().collect::<BTreeSet<_>>().len(), 32);
}
