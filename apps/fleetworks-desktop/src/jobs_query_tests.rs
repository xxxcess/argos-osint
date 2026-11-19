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
    assert!(state.scoped_jobs.dirty && !state.scoped_jobs.can_load_more());
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
    state.request_jobs(JobsEvent::More);
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
    assert_eq!(state.scoped_jobs.loaded_pages(), 2);
    assert_eq!(state.scoped_jobs.rows().len(), 2);
    assert_eq!(
        state
            .scoped_jobs
            .page
            .as_ref()
            .expect("head page")
            .matched_total,
        Some(3),
        "a tail page never replaces the observed total or turns it into zero"
    );
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
    for key in ["jobs-query-controls", "jobs-query-apply"] {
        assert!(
            runtime.find(key).is_none(),
            "retired query control {key} cannot reappear"
        );
    }
    for key in ["jobs-page-next", "jobs-page-previous"] {
        assert!(
            runtime.find(key).is_none(),
            "retired page control {key} cannot reappear"
        );
    }
    assert!(
        runtime.find("jobs-load-more").is_some(),
        "a head with a continuation offers one load-more control"
    );
    assert!(runtime.find("activity-job-fixture-job-2").is_some());
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

#[tokio::test(flavor = "current_thread")]
async fn load_more_appends_at_the_tail_while_rows_stay_visible() {
    let mut state = registered_app();
    state.page = crate::app::Page::Jobs;
    state.request_jobs(JobsEvent::Apply);
    let first = state
        .scoped_jobs
        .active_ticket()
        .expect("first registered page request")
        .clone();
    state.complete_scoped_jobs(first.clone(), Ok(page(&first.binding, 1, Some(2))));
    let mut runtime = Runtime::new(
        state,
        crate::app::view,
        (1180.0, 780.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    let mut clock = 0.0;
    runtime.frame(clock);
    assert!(runtime.find("activity-job-fixture-job-1").is_some());

    runtime.update(|_, tx| tx.scroll_into_view("jobs-load-more"));
    clock += 1.0 / 60.0;
    runtime.frame(clock);
    let (x, y) = runtime
        .find("jobs-load-more")
        .expect("visible load-more button");
    assert!(
        (0.0..1180.0).contains(&x) && (50.0..780.0).contains(&y),
        "load-more must be inside the Jobs scroll viewport"
    );
    runtime.pointer_down(x, y);
    runtime.pointer_up(x, y);
    clock += 1.0 / 60.0;
    runtime.frame(clock);
    let next = runtime
        .state
        .scoped_jobs
        .active_ticket()
        .expect("actual tail request")
        .clone();
    assert_eq!(next.query.cursor.as_deref(), Some(cursor(2).as_str()));
    assert!(runtime.state.scoped_jobs.loading_more());
    assert!(
        matches!(runtime.state.backend.jobs, Load::Ready(_)),
        "a tail fetch never swaps the list for a skeleton"
    );
    assert!(runtime.find("activity-job-fixture-job-1").is_some());
    assert!(runtime.find("activity-job-fixture-job-2").is_none());

    let (release, response) = tokio::sync::oneshot::channel();
    let mock_gateway = tokio::spawn(async move { response.await.expect("mock page released") });
    assert!(
        release.send(page(&next.binding, 2, None)).is_ok(),
        "mock gateway must accept the second registered page"
    );
    let second_page = mock_gateway.await.expect("mock gateway task");
    runtime.update(move |state, _| state.complete_scoped_jobs(next, Ok(second_page)));
    clock += 1.0 / 60.0;
    runtime.frame(clock);
    assert!(runtime.find("activity-job-fixture-job-1").is_some());
    assert!(runtime.find("activity-job-fixture-job-2").is_some());
    assert!(
        runtime.find("jobs-load-more").is_none(),
        "no continuation left, so no control"
    );
    assert_eq!(runtime.state.scoped_jobs.loaded_pages(), 2);
    assert!(runtime
        .accessibility_json()
        .to_string()
        .contains("Synthetic eligible job 2"));
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
    assert!(jobs.page.is_none() && jobs.handle(JobsEvent::More, now()).read.is_none());
    assert!(jobs.handle(JobsEvent::Apply, now()).read.is_some());
    jobs.bind(Some(binding("b")), false);
    assert!(!jobs.connected() && jobs.active_ticket().is_none());
}

#[test]
fn tail_is_bounded_and_invalid_scope_or_shared_private_payload_clears_the_page() {
    let mut jobs = JobsQueryState::default();
    jobs.bind(Some(binding("a")), true);
    let first = jobs
        .handle(JobsEvent::Apply, now())
        .read
        .expect("first read");
    jobs.complete(&first, Ok(page(&binding("a"), 1, Some(2))));
    for number in 2..=HISTORY_LIMIT {
        let next = jobs
            .handle(JobsEvent::More, now())
            .read
            .expect("bounded tail");
        jobs.complete(&next, Ok(page(&binding("a"), number, Some(number + 1))));
    }
    assert_eq!(jobs.loaded_pages(), HISTORY_LIMIT);
    assert!(!jobs.can_load_more() && jobs.at_bound());
    assert!(jobs.handle(JobsEvent::More, now()).read.is_none());
    assert!(jobs.tail_cursor.as_deref().is_some_and(cursor_valid));
    let refresh = jobs.handle(JobsEvent::Apply, now()).read.expect("refresh");
    jobs.complete(&refresh, Ok(page(&binding("b"), 1, None)));
    assert_eq!(jobs.status, JobsLoad::Failed);
    assert!(jobs.page.is_none() && jobs.rows().is_empty() && jobs.tail_cursor.is_none());
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
    state.request_jobs(JobsEvent::More);
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
    state.request_jobs(JobsEvent::Apply);
    let pending = state
        .scoped_jobs
        .active_ticket()
        .expect("filter refresh")
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
        "an old refresh callback cannot replace the new query"
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

#[test]
fn discovery_poll_finds_new_work_from_an_empty_page_without_a_selection() {
    let mut state = registered_app();
    state.request_jobs(JobsEvent::Apply);
    let ticket = state
        .scoped_jobs
        .active_ticket()
        .expect("apply ticket")
        .clone();
    let mut empty = page(&binding("a"), 1, None);
    empty.rows.clear();
    state.complete_scoped_jobs(ticket, Ok(empty));
    assert!(!state.job_list_has_active());
    assert!(state.backend.selected_job.is_none());

    state.discover_registered_jobs(100.0);
    assert_eq!(state.jobs_discovery_at, 105.0, "discovery is rate limited");
    let ticket = state
        .scoped_jobs
        .active_ticket()
        .expect("discovery starts a read from an empty list")
        .clone();
    state.discover_registered_jobs(101.0);
    assert!(
        state.scoped_jobs.busy(),
        "a read in flight is never duplicated"
    );
    state.complete_scoped_jobs(ticket, Ok(page(&binding("a"), 1, None)));

    assert!(
        matches!(&state.backend.jobs, Load::Ready(jobs) if jobs.rows.iter().any(|row| row.id == "fixture-job-1")),
        "newly admitted work appears without any UI selection"
    );
    assert!(state.backend.selected_job.is_none());
}

#[test]
fn discovery_poll_needs_a_registration_and_drops_replies_from_a_prior_one() {
    let mut state = registered_app();
    state.registration.context = None;
    state.discover_registered_jobs(100.0);
    assert!(
        state.scoped_jobs.active_ticket().is_none(),
        "no registered workstation means no discovery read"
    );

    let mut state = registered_app();
    state.discover_registered_jobs(100.0);
    let ticket = state
        .scoped_jobs
        .active_ticket()
        .expect("discovery read")
        .clone();
    state.registration.context = Some(binding("b").context);
    state.sync_jobs_binding();
    state.complete_scoped_jobs(ticket, Ok(page(&binding("a"), 1, None)));
    assert!(
        !matches!(&state.backend.jobs, Load::Ready(jobs) if jobs.rows.iter().any(|row| row.id == "fixture-job-1")),
        "a delayed reply from the previous registration must not repopulate Jobs"
    );
}

#[test]
fn scheduler_discovers_new_work_while_a_completed_job_with_unavailable_logs_is_selected() {
    let mut state = registered_app();
    state.request_jobs(JobsEvent::Apply);
    let ticket = state
        .scoped_jobs
        .active_ticket()
        .expect("apply ticket")
        .clone();
    let mut finished = page(&binding("a"), 1, None);
    finished.rows[0].state = "completed".into();
    state.complete_scoped_jobs(ticket, Ok(finished));
    state.select_job("fixture-job-1");
    state.scoped_logs.status = crate::logs::LogStatus::Unavailable;
    state.scoped_logs.window_focused = true;
    state.page = crate::app::Page::Jobs;
    // Production scheduling: the fixture flag only exists to keep scenes offline.
    state.fixture = false;
    assert!(!state.job_list_has_active() && !state.selected_job_is_active());

    let mut runtime = Runtime::new(
        state,
        crate::app::view,
        (1180.0, 780.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    let before = runtime.state.scoped_jobs.reads_begun();
    runtime.update(|state, tx| {
        crate::app::poll_jobs(state, 100.0, tx);
    });
    assert_ne!(
        runtime.state.scoped_jobs.reads_begun(),
        before,
        "the list is re-read even though log recovery owns the active path"
    );
    assert_eq!(runtime.state.jobs_discovery_at, 105.0);
    assert_eq!(
        runtime.state.backend.selected_job.as_deref(),
        Some("fixture-job-1"),
        "discovery keeps the selection"
    );

    // Not due again before the interval ends.
    let again = runtime.state.scoped_jobs.reads_begun();
    runtime.update(|state, tx| {
        crate::app::poll_jobs(state, 102.0, tx);
    });
    assert_eq!(runtime.state.scoped_jobs.reads_begun(), again);
}

fn full_page(binding: &JobsBinding, number: usize, next: Option<usize>) -> JobsPage {
    let mut page = page(binding, number, next);
    let template = page.rows[0].clone();
    page.rows = (0..usize::from(PAGE_SIZE))
        .map(|index| {
            let mut row = template.clone();
            row.id = format!("job-{number}-{index}");
            row
        })
        .collect();
    page
}

fn loaded_jobs(next: Option<usize>) -> JobsQueryState {
    let mut jobs = JobsQueryState::default();
    jobs.bind(Some(binding("a")), true);
    let first = jobs
        .handle(JobsEvent::Apply, now())
        .read
        .expect("first read");
    jobs.complete(&first, Ok(page(&binding("a"), 1, next)));
    jobs
}

#[test]
fn tail_pages_append_without_duplicates_and_keep_the_head_order() {
    let mut jobs = loaded_jobs(Some(2));
    let tail = jobs.handle(JobsEvent::More, now()).read.expect("tail read");
    // The second page repeats the head row, as a refreshed head can shift the boundary.
    let mut reply = page(&binding("a"), 2, None);
    let repeated = jobs.page.as_ref().expect("head").rows[0].clone();
    reply.rows.insert(0, repeated);
    assert!(jobs.complete(&tail, Ok(reply)));
    let ids: Vec<String> = jobs.rows().into_iter().map(|row| row.id).collect();
    assert_eq!(ids, vec!["fixture-job-1", "fixture-job-2"]);
    assert!(!jobs.can_load_more(), "the last page ends continuation");
}

#[test]
fn row_cap_stops_loading_even_when_the_server_has_more() {
    let mut jobs = JobsQueryState::default();
    jobs.bind(Some(binding("a")), true);
    let first = jobs
        .handle(JobsEvent::Apply, now())
        .read
        .expect("first read");
    jobs.complete(&first, Ok(full_page(&binding("a"), 1, Some(2))));
    let pages = MAX_ROWS / usize::from(PAGE_SIZE);
    for number in 2..=pages {
        let tail = jobs.handle(JobsEvent::More, now()).read.expect("tail read");
        jobs.complete(
            &tail,
            Ok(full_page(&binding("a"), number, Some(number + 1))),
        );
    }
    assert_eq!(jobs.rows().len(), MAX_ROWS);
    assert!(!jobs.can_load_more() && jobs.at_bound());
    assert!(jobs.handle(JobsEvent::More, now()).read.is_none());
}

#[test]
fn tail_failure_keeps_every_row_and_retry_recovers() {
    let mut jobs = loaded_jobs(Some(2));
    let tail = jobs.handle(JobsEvent::More, now()).read.expect("tail read");
    assert!(jobs.loading_more() && !jobs.can_load_more());
    jobs.complete(&tail, Err(ClientError::Transport));
    assert_eq!(jobs.status, JobsLoad::Ready);
    assert!(jobs.tail_error().is_some() && !jobs.loading_more());
    assert_eq!(jobs.rows().len(), 1, "resolved rows stay visible");
    assert!(jobs.can_load_more(), "the recovery control is available");
    let retry = jobs.handle(JobsEvent::More, now()).read.expect("retry");
    assert!(jobs.tail_error().is_none(), "retrying clears the error");
    assert_eq!(retry.query.cursor.as_deref(), Some(cursor(2).as_str()));
    jobs.complete(&retry, Ok(page(&binding("a"), 2, None)));
    assert_eq!(jobs.rows().len(), 2);
}

#[test]
fn an_outdated_tail_reply_never_overwrites_newer_state() {
    let mut jobs = loaded_jobs(Some(2));
    let tail = jobs.handle(JobsEvent::More, now()).read.expect("tail read");
    // A newer query replaces the tail read before it answers.
    let newer = jobs
        .handle(JobsEvent::Apply, now())
        .read
        .expect("newer read");
    assert!(!jobs.complete(&tail, Ok(page(&binding("a"), 2, None))));
    assert!(jobs.page.is_none() && jobs.rows().is_empty());
    assert!(jobs.complete(&newer, Ok(page(&binding("a"), 7, Some(8)))));
    let ids: Vec<String> = jobs.rows().into_iter().map(|row| row.id).collect();
    assert_eq!(ids, vec!["fixture-job-7"]);
}

#[test]
fn a_scope_switch_clears_loaded_rows_and_ignores_the_old_tail_reply() {
    let mut jobs = loaded_jobs(Some(2));
    let tail = jobs.handle(JobsEvent::More, now()).read.expect("tail read");
    jobs.bind(Some(binding("b")), true);
    assert!(jobs.page.is_none() && jobs.rows().is_empty() && jobs.tail_cursor.is_none());
    assert!(!jobs.complete(&tail, Ok(page(&binding("a"), 2, None))));
    assert!(jobs.rows().is_empty() && jobs.loaded_pages() == 0);
}

#[test]
fn a_head_poll_refreshes_the_head_and_keeps_appended_rows() {
    let mut jobs = loaded_jobs(Some(2));
    let tail = jobs.handle(JobsEvent::More, now()).read.expect("tail read");
    jobs.complete(&tail, Ok(page(&binding("a"), 2, Some(3))));
    let poll = jobs.handle(JobsEvent::Poll, now()).read.expect("poll");
    assert!(jobs.loaded_pages() == 2 && jobs.status == JobsLoad::Ready);
    let mut refreshed = page(&binding("a"), 1, Some(2));
    refreshed.rows[0].state = "running".into();
    assert!(jobs.complete(&poll, Ok(refreshed)));
    let rows = jobs.rows();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].state, "running");
    assert_eq!(rows[1].id, "fixture-job-2");
    assert_eq!(
        jobs.tail_cursor.as_deref(),
        Some(cursor(3).as_str()),
        "a head poll keeps the tail cursor once a tail exists"
    );
}

#[test]
fn a_failed_poll_keeps_same_scope_rows_and_a_selection_survives_load_more() {
    let mut state = registered_app();
    state.request_jobs(JobsEvent::Apply);
    let first = state.scoped_jobs.active_ticket().expect("read").clone();
    state.complete_scoped_jobs(first.clone(), Ok(page(&first.binding, 1, Some(2))));
    state.backend.selected_job = Some("fixture-job-1".into());
    state.request_jobs(JobsEvent::More);
    assert_eq!(state.backend.selected_job.as_deref(), Some("fixture-job-1"));
    let tail = state.scoped_jobs.active_ticket().expect("tail").clone();
    state.complete_scoped_jobs(tail.clone(), Ok(page(&tail.binding, 2, Some(3))));
    assert_eq!(state.backend.selected_job.as_deref(), Some("fixture-job-1"));
    state.request_jobs(JobsEvent::Poll);
    let poll = state.scoped_jobs.active_ticket().expect("poll").clone();
    state.complete_scoped_jobs(poll, Err(ClientError::Transport));
    assert!(matches!(state.backend.jobs, crate::app::Load::Ready(_)));
    assert_eq!(state.scoped_jobs.rows().len(), 2);
    assert!(state.scoped_jobs.message.is_some(), "stale reason is kept");
}

fn state_with_jobs_head(next: Option<usize>) -> crate::app::State {
    let mut state = registered_app();
    state.request_jobs(JobsEvent::Apply);
    let head = state
        .scoped_jobs
        .active_ticket()
        .expect("newest read")
        .clone();
    state.complete_scoped_jobs(head.clone(), Ok(page(&head.binding, 1, next)));
    state
}

#[test]
fn near_end_loads_one_tail_only_when_the_sentinel_comes_on_screen() {
    let mut state = state_with_jobs_head(Some(2));
    state.jobs_near_end(false);
    assert!(state.scoped_jobs.active_ticket().is_none(), "off screen");
    state.jobs_near_end(true);
    assert!(state.scoped_jobs.loading_more(), "on screen loads the tail");
    let tail = state.scoped_jobs.active_ticket().expect("tail").clone();
    // Repeated visibility reports while the read is in flight add no second read.
    state.jobs_near_end(true);
    let still = state.scoped_jobs.active_ticket().expect("same read");
    assert_eq!(still.query.cursor, tail.query.cursor);
    state.complete_scoped_jobs(tail.clone(), Ok(page(&tail.binding, 2, None)));
    assert_eq!(state.scoped_jobs.rows().len(), 2);
    // The server's end stops it: no continuation, no read.
    state.jobs_near_end(true);
    assert!(state.scoped_jobs.active_ticket().is_none());
}

#[test]
fn near_end_pauses_after_a_failed_tail_until_the_explicit_retry() {
    let mut state = state_with_jobs_head(Some(2));
    state.jobs_near_end(true);
    let tail = state.scoped_jobs.active_ticket().expect("tail").clone();
    state.complete_scoped_jobs(tail, Err(ClientError::Transport));
    assert!(state.scoped_jobs.tail_error().is_some());
    state.jobs_near_end(true);
    assert!(
        state.scoped_jobs.active_ticket().is_none(),
        "a failed tail is not retried by scrolling"
    );
    state.request_jobs(JobsEvent::More);
    assert!(state.scoped_jobs.loading_more(), "Retry still works");
}

#[test]
fn jobs_nav_badge_counts_in_progress_buckets_omits_unknown_and_clears_on_authority_change() {
    let mut state = registered_app();
    // Before any page loads, count is unknown -> omitted (never "0").
    assert_eq!(state.scoped_jobs.in_progress_count(), None);
    assert_eq!(
        crate::app::sidebar_row_badge(&state, crate::app::Page::Jobs),
        None
    );

    // Page with running(2) + queued(3) + retrying(1) = 6, even though only 1 row is loaded.
    state.request_jobs(JobsEvent::Apply);
    let ticket = state.scoped_jobs.active_ticket().expect("ticket").clone();
    let mut reply = page(&ticket.binding, 1, None);
    reply.counts = Some(JobsDashboardCounts {
        active: 2,
        queued: 3,
        retrying: 1,
        failed: 4,
        completed_recent: 9,
    });
    state.complete_scoped_jobs(ticket, Ok(reply));
    assert_eq!(state.scoped_jobs.in_progress_count(), Some(6));
    assert_eq!(
        crate::app::sidebar_row_badge(&state, crate::app::Page::Jobs).as_deref(),
        Some("6")
    );

    // Unknown counts (None) omits the badge rather than showing "0".
    state.request_jobs(JobsEvent::Poll);
    let poll = state.scoped_jobs.active_ticket().expect("poll").clone();
    let mut unknown_reply = page(&poll.binding, 1, None);
    unknown_reply.counts = None;
    unknown_reply.observed_at = "2026-11-12T12:01:00Z".into();
    state.complete_scoped_jobs(poll, Ok(unknown_reply));
    assert_eq!(state.scoped_jobs.in_progress_count(), None);
    assert_eq!(
        crate::app::sidebar_row_badge(&state, crate::app::Page::Jobs),
        None
    );

    // Authority change clears the count.
    state.request_jobs(JobsEvent::Poll);
    let poll2 = state.scoped_jobs.active_ticket().expect("poll2").clone();
    let mut active_reply = page(&poll2.binding, 1, None);
    active_reply.counts = Some(JobsDashboardCounts {
        active: 1,
        queued: 0,
        retrying: 0,
        failed: 0,
        completed_recent: 0,
    });
    active_reply.observed_at = "2026-11-12T12:02:00Z".into();
    state.complete_scoped_jobs(poll2, Ok(active_reply));
    assert_eq!(
        crate::app::sidebar_row_badge(&state, crate::app::Page::Jobs).as_deref(),
        Some("1")
    );
    state.scoped_jobs.invalidate_origin();
    assert_eq!(state.scoped_jobs.in_progress_count(), None);
    assert_eq!(
        crate::app::sidebar_row_badge(&state, crate::app::Page::Jobs),
        None
    );
}

#[tokio::test(flavor = "current_thread")]
async fn actor_reindex_is_listed_and_updates_the_jobs_badge_from_workstation() {
    use crate::app;
    use crate::app::Page;
    use crate::connection::GatewayEffects;
    use fleetworks_contracts::Client;
    use std::sync::Arc;

    let mut state = registered_app();
    state.page = Page::Workstation;
    state.scoped_logs.window_focused = true;
    // Supply the production effects boundary without installing a UI handle so
    // this test can deliver each registered response deterministically.
    state.services = Some(Arc::new(GatewayEffects::new(
        tokio::runtime::Handle::current(),
    )));
    state.client = Some(Client::new("http://127.0.0.1:1", Some("synthetic-token")).unwrap());
    let mut runtime = Runtime::new(
        state,
        app::view,
        (1280.0, 800.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    runtime.frame(0.0);
    assert_eq!(app::sidebar_row_badge(&runtime.state, Page::Jobs), None);

    for (step, (job_state, active, queued, retrying)) in [
        ("queued", 0, 1, 0),
        ("running", 1, 0, 0),
        ("retry_scheduled", 0, 0, 1),
        ("completed", 0, 0, 0),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(runtime.state.page, Page::Workstation);
        let now = 100.0 + 16.0 * step as f64;
        runtime.update(|state, tx| {
            state.scoped_logs.window_focused = true;
            state.fixture = false;
            assert!(app::poll_jobs(state, now, tx));
            state.fixture = true;
        });
        let ticket = runtime
            .state
            .scoped_jobs
            .active_ticket()
            .expect("off-screen scheduler requests registered activity")
            .clone();
        let mut response = page(&ticket.binding, 1, None);
        response.rows[0].operation = "actor_reindex".into();
        response.rows[0].title = "Rebuild actor index".into();
        response.rows[0].state = job_state.into();
        response.rows[0].phase = job_state.into();
        response.counts = Some(JobsDashboardCounts {
            active,
            queued,
            retrying,
            failed: 0,
            completed_recent: u64::from(job_state == "completed"),
        });
        response.observed_at = format!("2026-11-12T12:00:0{step}Z");
        runtime.update(move |state, _| state.complete_scoped_jobs(ticket, Ok(response)));
        let expected_badge = (job_state != "completed").then_some("1");
        assert_eq!(
            app::sidebar_row_badge(&runtime.state, Page::Jobs).as_deref(),
            expected_badge,
            "badge while on Workstation for {job_state}",
        );
        assert_eq!(
            crate::views::active_actor_reindex(&runtime.state).map(|job| job.state.as_str()),
            (job_state != "completed").then_some(job_state),
        );

        runtime.update(|state, _| state.page = Page::Jobs);
        runtime.frame((2 * step + 1) as f64 / 60.0);
        assert!(
            runtime.find("activity-job-fixture-job-1").is_some(),
            "tracked actor reindex must be present in Jobs & Logs for {job_state}",
        );
        assert!(runtime
            .accessibility_json()
            .to_string()
            .contains("Rebuild actor index"));
        runtime.update(|state, _| state.page = Page::Workstation);
        runtime.frame((2 * step + 2) as f64 / 60.0);
    }
}

#[test]
fn selected_job_detail_is_not_overwritten_by_stale_list_refresh_and_tail_cursor_survives() {
    let mut state = state_with_jobs_head(Some(2));
    // Load tail page 2 so tail_cursor advances to cursor(3).
    state.request_jobs(JobsEvent::More);
    let tail = state.scoped_jobs.active_ticket().expect("tail").clone();
    state.complete_scoped_jobs(tail.clone(), Ok(page(&tail.binding, 2, Some(3))));
    assert_eq!(state.scoped_jobs.loaded_pages(), 2);

    // Select fixture-job-1 and give it a newer terminal detail observation.
    state.backend.selected_job = Some("fixture-job-1".into());
    let newer_detail: crate::backend::JobDetail = serde_json::from_value(json!({
        "job": {
            "id": "fixture-job-1",
            "app": "tools",
            "operation": "manual_tool",
            "title": "Synthetic eligible job 1",
            "state": "completed",
            "phase": "done",
            "progress_done": 100,
            "progress_total": 100,
            "error_summary": "",
            "private_details_available": true,
            "activity_actions_available": false,
            "status_only": false
        },
        "stages": [],
        "private_details_available": true,
        "activity_actions_available": false
    }))
    .expect("newer detail");
    state.backend.detail = Load::Ready(newer_detail);

    // A head poll returns an older "queued" row for fixture-job-1.
    state.request_jobs(JobsEvent::Poll);
    let poll = state.scoped_jobs.active_ticket().expect("poll").clone();
    let mut stale_row_page = page(&poll.binding, 1, Some(99));
    stale_row_page.observed_at = "2026-11-12T12:00:30Z".into();
    state.complete_scoped_jobs(poll, Ok(stale_row_page));

    // Selected detail must remain "completed" (not regressed to "queued").
    let Load::Ready(detail) = &state.backend.detail else {
        panic!("detail must remain ready");
    };
    assert_eq!(detail.job.state, "completed");
    assert_eq!(detail.job.progress_done, Some(100));

    // Tail cursor must still be cursor(3) (not reset to cursor(99)).
    state.request_jobs(JobsEvent::More);
    let next_tail = state
        .scoped_jobs
        .active_ticket()
        .expect("next tail")
        .clone();
    assert_eq!(next_tail.query.cursor, Some(cursor(3)));
}
