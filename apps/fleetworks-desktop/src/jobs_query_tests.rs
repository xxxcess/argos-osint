//! Synthetic effects over the production Jobs query controller, adapter and current shell.
//! No gateway, user home, credential, clipboard or backend job is touched.

use crate::app::Load;
use fleetworks_contracts::connect::RegistrationReceipt;
use rune::Env;
use rune::Runtime;
use serde_json::json;

use super::*;

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-12T12:00:00Z")
        .expect("fixed fixture clock")
        .with_timezone(&Utc)
}
fn binding(node: &str) -> JobsBinding {
    let receipt = RegistrationReceipt::new(
        "fixture-deployment".into(),
        format!("fixture-node-{node}"),
        format!("fixture-registration-{node}"),
        Some("192.0.2.42".parse().expect("synthetic IP")),
        1,
    );
    JobsBinding {
        context: RegisteredContext::new(
            receipt,
            1,
            1,
            "fixture-config".into(),
            "fixture-access".into(),
        )
        .expect("synthetic registered context"),
        connection_epoch: 1,
        origin_revision: 1,
    }
}
fn cursor(index: usize) -> String {
    format!("{index:064x}")
}
fn page(binding: &JobsBinding, number: usize, next: Option<usize>) -> JobsPage {
    let row = serde_json::from_value(json!({"id":format!("fixture-job-{number}"),"app":"tools","operation":"manual_tool","title":format!("Synthetic eligible job {number}"),"state":"queued","phase":"queued","progress_done":null,"progress_total":null,"error_summary":"","private_details_available":true,"activity_actions_available":false,"status_only":false})).expect("safe synthetic activity row");
    JobsPage {
        rows: vec![row],
        activity_scope: ActivityScope::for_workstation(&binding.context),
        relation_counts_available: false,
        activity_actions_available: false,
        counts: Some(JobsDashboardCounts {
            active: 700,
            queued: 3,
            retrying: 0,
            failed: 1,
            completed_recent: 9,
        }),
        dashboard_counts_scope: "unfiltered_top_level_workstation_activity".into(),
        matched_total: Some(3),
        observed_at: "2026-11-12T12:00:00Z".into(),
        apps: vec!["tools".into()],
        apps_complete: false,
        has_more: next.is_some(),
        page_cursor: cursor(number),
        next_cursor: next.map(cursor),
    }
}

#[test]
fn registered_handler_applies_filters_and_pins_requests_while_drafts_change() {
    use crate::app::Intent;
    use crate::backend::ReadKind;
    let mut state = registered_app();
    state.backend.selected_job = Some("fixture-selected-job".into());
    state.scoped_jobs.draft.status = JobsStatusFilter::Failed;
    state.scoped_jobs.draft.time = JobsTime::Hour;
    state.scoped_jobs.draft.app = TextEdit::new("tools");
    state.scoped_jobs.draft.operation = TextEdit::new("manual_tool");
    state.scoped_jobs.draft.kind = TextEdit::new("task");
    state.scoped_jobs.draft.correlation_id = TextEdit::new("fixture-correlation");
    state.scoped_jobs.draft.text = TextEdit::new("needle");
    state.scoped_jobs.draft_edited();
    assert!(
        state.scoped_jobs.active_ticket().is_none(),
        "draft edits alone issue no read"
    );
    state.request_jobs(JobsEvent::Apply);
    assert!(state.backend.selected_job.is_none() && state.job_actions.confirmation.is_none());
    let first = state
        .scoped_jobs
        .active_ticket()
        .expect("actual registered adapter request")
        .clone();
    assert_eq!(
        state
            .intents
            .iter()
            .filter(|intent| matches!(
                intent,
                Intent::BackendRead {
                    kind: ReadKind::Jobs
                }
            ))
            .count(),
        1
    );
    assert_eq!(
        serde_json::to_value(&first.query).expect("safe query wire"),
        json!({"sort":"newest","limit":50,"status":"failed","app":"tools","operation":"manual_tool","kind":"task","correlation_id":"fixture-correlation","text":"needle","since":"2026-11-12T11:00:00Z"})
    );
    state.scoped_jobs.draft.text = TextEdit::new("needle edited");
    state.scoped_jobs.draft_edited();
    assert!(state.scoped_jobs.dirty && !state.scoped_jobs.can_next());
    assert_eq!(first.query.text.as_deref(), Some("needle"));
    state.complete_scoped_jobs(first.clone(), Ok(page(&first.binding, 1, Some(2))));
    assert!(state.scoped_jobs.page.is_some() && state.scoped_jobs.dirty);
}

#[test]
fn registered_paging_uses_server_tokens_and_keeps_dashboard_counts_separate() {
    let mut state = registered_app();
    state.request_jobs(JobsEvent::Apply);
    let first = state
        .scoped_jobs
        .active_ticket()
        .expect("newest registered read")
        .clone();
    state.complete_scoped_jobs(first.clone(), Ok(page(&first.binding, 1, Some(2))));
    let observed = state
        .scoped_jobs
        .page
        .as_ref()
        .expect("validated current page");
    assert_eq!(observed.matched_total, Some(3));
    assert_eq!(
        observed.counts.as_ref().expect("separate dashboard").active,
        700
    );
    assert!(
        !observed.apps_complete,
        "incomplete facets cannot claim completeness"
    );
    state.request_jobs(JobsEvent::Next);
    let next = state
        .scoped_jobs
        .active_ticket()
        .expect("actual continuation")
        .clone();
    assert_eq!(
        serde_json::to_value(&next.query).expect("cursor-only continuation"),
        json!({"sort":"newest","cursor":cursor(2)})
    );
    let mut reply = page(&next.binding, 2, Some(3));
    reply.matched_total = None;
    state.complete_scoped_jobs(next, Ok(reply));
    assert_eq!(state.scoped_jobs.page_number(), 2);
    assert!(
        state
            .scoped_jobs
            .page
            .as_ref()
            .expect("second page")
            .matched_total
            .is_none(),
        "unavailable total never becomes zero or a dashboard bucket"
    );
    state.request_jobs(JobsEvent::Previous);
    let previous = state
        .scoped_jobs
        .active_ticket()
        .expect("server-issued previous token")
        .clone();
    assert_eq!(
        serde_json::to_value(&previous.query).expect("fixed first-page cursor"),
        json!({"sort":"newest","cursor":cursor(1)})
    );
    state.complete_scoped_jobs(previous.clone(), Ok(page(&previous.binding, 1, Some(2))));
    assert_eq!(state.scoped_jobs.page_number(), 1);
    assert!(!state.scoped_jobs.can_previous());
    state.page = crate::app::Page::Jobs;
    let mut runtime = Runtime::new(
        state,
        crate::app::view,
        (1180.0, 780.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    runtime.frame(0.0);
    assert!(
        runtime.find("activity-job-fixture-job-1").is_some(),
        "validated current rows reach the production list"
    );
    for key in [
        "jobs-query-controls",
        "jobs-query-apply",
        "jobs-page-next",
        "jobs-page-previous",
    ] {
        assert!(
            runtime.find(key).is_none(),
            "retired query/pager control {key} cannot reappear"
        );
    }
    fn announces_dashboard_bucket(node: &serde_json::Value) -> bool {
        ["name", "value", "description"].into_iter().any(|field| {
            node[field]
                .as_str()
                .is_some_and(|text| text.contains("700"))
        }) || node["children"]
            .as_array()
            .is_some_and(|children| children.iter().any(announces_dashboard_bucket))
    }
    let tree = runtime.accessibility_json();
    assert!(
        runtime.elements().iter().any(|element| {
            element
                .label
                .as_deref()
                .or(element.text.as_deref())
                .is_some_and(|text| text.contains("Synthetic eligible job 1"))
        }),
        "the production list must announce the validated current job"
    );
    // Bounds may legitimately contain 700.0; only announced content can
    // mislabel the unfiltered dashboard bucket as a matching-job total.
    assert!(
        !announces_dashboard_bucket(&tree),
        "the current list cannot mislabel dashboard buckets as matching totals: {tree}"
    );
}

#[test]
fn stale_query_context_auth_and_expired_cursor_never_repopulate_private_rows() {
    let mut jobs = JobsQueryState::default();
    jobs.bind(Some(binding("a")), true);
    let old = jobs.handle(JobsEvent::Apply, now()).read.expect("old read");
    jobs.draft.text = TextEdit::new("new query");
    jobs.draft_edited();
    let newer = jobs
        .handle(JobsEvent::Apply, now())
        .read
        .expect("new query");
    assert!(!jobs.complete(&old, Ok(page(&binding("a"), 1, None))));
    jobs.bind(Some(binding("b")), true);
    assert!(!jobs.complete(&newer, Ok(page(&binding("a"), 1, None))));
    assert!(jobs.page.is_none());
    let current = jobs
        .handle(JobsEvent::Apply, now())
        .read
        .expect("new origin");
    jobs.complete(&current, Err(ClientError::Status(403)));
    jobs.bind(Some(binding("b")), true);
    assert!(!jobs.connected() && jobs.page.is_none());
    let mut rotated = binding("b");
    rotated.connection_epoch += 1;
    jobs.bind(Some(rotated), true);
    jobs.draft.text = TextEdit::new("x".repeat(257));
    assert!(jobs.handle(JobsEvent::Apply, now()).read.is_none());
    jobs.draft.selected_workspace_only = true;
    let reset = jobs
        .handle(JobsEvent::Clear, now())
        .read
        .expect("explicit reset");
    assert!(serde_json::to_value(&reset.query)
        .expect("query")
        .get("workspace_id")
        .is_none());
    assert!(
        reset.query.text.is_none(),
        "Clear cannot change registered workstation scope"
    );
    jobs.complete(&reset, Err(ClientError::Status(409)));
    assert_eq!(jobs.status, JobsLoad::RestartRequired);
    assert!(jobs.page.is_none() && jobs.handle(JobsEvent::Next, now()).read.is_none());
    assert!(jobs.handle(JobsEvent::Apply, now()).read.is_some());
    jobs.bind(Some(binding("b")), false);
    assert!(!jobs.connected() && jobs.active_ticket().is_none());
}

#[test]
fn history_is_bounded_and_invalid_scope_or_shared_private_payload_clears_the_page() {
    let mut jobs = JobsQueryState::default();
    jobs.bind(Some(binding("a")), true);
    let first = jobs
        .handle(JobsEvent::Apply, now())
        .read
        .expect("first read");
    jobs.complete(&first, Ok(page(&binding("a"), 1, Some(2))));
    for number in 2..=HISTORY_LIMIT {
        let next = jobs
            .handle(JobsEvent::Next, now())
            .read
            .expect("bounded next");
        jobs.complete(&next, Ok(page(&binding("a"), number, Some(number + 1))));
    }
    assert_eq!(jobs.history.len(), HISTORY_LIMIT);
    assert_eq!(jobs.page_number(), HISTORY_LIMIT);
    assert!(jobs.page.as_ref().expect("last loaded page").has_more);
    assert!(!jobs.can_next() && jobs.can_previous());
    assert!(jobs.history.iter().all(|token| cursor_valid(token)));
    let refresh = jobs.handle(JobsEvent::Apply, now()).read.expect("refresh");
    jobs.complete(&refresh, Ok(page(&binding("b"), 1, None)));
    assert_eq!(jobs.status, JobsLoad::Failed);
    assert!(jobs.page.is_none() && jobs.history.is_empty());
    let current = jobs
        .handle(JobsEvent::Apply, now())
        .read
        .expect("new query");
    let mut private_shared = page(&binding("a"), 1, None);
    private_shared.rows[0].private_details_available = false;
    private_shared.rows[0].status_only = true;
    assert!(jobs.complete(&current, Ok(private_shared)));
    assert!(
        jobs.page.is_none(),
        "a shared row cannot carry a private title or phase"
    );
}

fn registered_app() -> crate::app::State {
    use crate::app::State;
    use crate::connection::ConnectionStatus;
    use fleetworks_contracts::capabilities::Capabilities;
    use fleetworks_contracts::context::Permission;

    let mut state = State::fixture();
    state.connection.status = ConnectionStatus::Connected;
    let mut capabilities = Capabilities::default();
    capabilities
        .permissions
        .extend([Permission::ReadWorkstationActivity, Permission::RetryJob]);
    state.connection.capabilities = Some(capabilities);
    state.registration.context = Some(binding("a").context);
    state.sync_jobs_binding();
    state
}

#[test]
fn app_adapter_preserves_logs_drafts_but_drops_pages_on_permission_and_context_changes() {
    use crate::app::Load;

    let mut state = registered_app();
    state.scoped_logs.draft.text = TextEdit::new("keep independent log draft");
    state.scoped_jobs.draft.kind = TextEdit::new("task");
    state.request_jobs(JobsEvent::Apply);
    let first = state
        .scoped_jobs
        .active_ticket()
        .expect("registered newest read")
        .clone();
    assert_eq!(first.query.limit, Some(50));
    state.complete_scoped_jobs(first.clone(), Ok(page(&first.binding, 1, Some(2))));
    assert!(matches!(state.backend.jobs, Load::Ready(_)));
    assert_eq!(
        state.scoped_logs.draft.text.text,
        "keep independent log draft"
    );
    state.request_jobs(JobsEvent::Next);
    let pending = state
        .scoped_jobs
        .active_ticket()
        .expect("pending continuation")
        .clone();
    state.registration.context = Some(binding("b").context);
    state.sync_jobs_binding();
    state.complete_scoped_jobs(pending.clone(), Ok(page(&pending.binding, 2, None)));
    assert!(state.scoped_jobs.page.is_none() && matches!(state.backend.jobs, Load::Idle));
    state.request_jobs(JobsEvent::Apply);
    let old = state
        .scoped_jobs
        .active_ticket()
        .expect("current pending query")
        .clone();
    state
        .connection
        .capabilities
        .as_mut()
        .expect("synthetic grant")
        .permissions
        .clear();
    state.sync_jobs_binding();
    state.complete_scoped_jobs(old.clone(), Ok(page(&old.binding, 1, None)));
    assert!(state.backend.selected_job.is_none() && matches!(state.backend.jobs, Load::Idle));
    assert!(!state.scoped_jobs.connected() && state.scoped_jobs.page.is_none());
}

#[test]
fn paged_rows_route_to_exact_details_and_preview_is_invalidated_before_page_or_filter_change() {
    use crate::app::Intent;
    use crate::app::Load;
    use crate::backend::JobDetail;
    use crate::backend::ReadKind;
    use crate::job_actions::JobAction;

    let mut state = registered_app();
    state.request_jobs(JobsEvent::Apply);
    let ticket = state
        .scoped_jobs
        .active_ticket()
        .expect("current query")
        .clone();
    let mut reply = page(&ticket.binding, 1, Some(2));
    let job = &mut reply.rows[0];
    job.app = "atlas".into();
    job.operation = "atlas_cycle".into();
    job.state = "failed".into();
    job.phase = "failed".into();
    job.can_retry = true;
    job.activity_actions_available = true;
    reply.activity_actions_available = true;
    let job = reply.rows[0].clone();
    state.complete_scoped_jobs(ticket, Ok(reply));
    state.intents.clear();
    state.select_job("outside-current-page");
    assert!(state.backend.selected_job.is_none() && state.intents.is_empty());
    state.select_job(&job.id);
    assert_eq!(
        state.intents,
        vec![Intent::BackendRead {
            kind: ReadKind::JobDetail
        }]
    );
    state.backend.detail = Load::Ready(JobDetail {
        job,
        stages: Vec::new(),
        private_details_available: true,
        activity_actions_available: true,
        maintenance_result: None,
    });
    state.preview_job_action(JobAction::Retry);
    assert!(state.job_actions.confirmation.is_some());
    state.request_jobs(JobsEvent::Next);
    let pending = state
        .scoped_jobs
        .active_ticket()
        .expect("next page")
        .clone();
    assert!(state.job_actions.confirmation.is_none() && state.backend.selected_job.is_none());
    state.intents.clear();
    state.open_job("exact-authorized-history-link");
    let current = state
        .scoped_jobs
        .active_ticket()
        .expect("newest refresh")
        .clone();
    state.complete_scoped_jobs(pending.clone(), Ok(page(&pending.binding, 2, None)));
    assert!(
        matches!(state.backend.jobs, Load::Loading),
        "an old next-page callback cannot replace the new query"
    );
    state.complete_scoped_jobs(current.clone(), Ok(page(&current.binding, 1, None)));
    assert_eq!(
        state.backend.selected_job.as_deref(),
        Some("exact-authorized-history-link"),
        "a list page cannot replace a cross-page detail target"
    );
    assert_eq!(
        state.intents,
        vec![
            Intent::BackendRead {
                kind: ReadKind::Jobs
            },
            Intent::BackendRead {
                kind: ReadKind::JobDetail
            }
        ]
    );
}

#[test]
fn background_poll_preserves_ready_page_and_selected_job_while_updating_progress() {
    let mut state = registered_app();
    state.request_jobs(JobsEvent::Apply);
    let ticket = state
        .scoped_jobs
        .active_ticket()
        .expect("apply ticket")
        .clone();
    state.complete_scoped_jobs(ticket, Ok(page(&binding("a"), 1, None)));
    state.select_job("fixture-job-1");
    assert_eq!(state.backend.selected_job.as_deref(), Some("fixture-job-1"));

    state.request_jobs(JobsEvent::Poll);
    assert_eq!(
        state.backend.selected_job.as_deref(),
        Some("fixture-job-1"),
        "periodic background poll must preserve selected job and ready rows"
    );
    assert!(matches!(state.backend.jobs, Load::Ready(_)));

    let poll_ticket = state
        .scoped_jobs
        .active_ticket()
        .expect("poll ticket")
        .clone();
    state.complete_scoped_jobs(poll_ticket, Err(ClientError::Timeout));
    assert!(
        matches!(state.backend.jobs, Load::Ready(_)),
        "transient poll failure must keep existing validated job rows instead of wiping the list"
    );
}
