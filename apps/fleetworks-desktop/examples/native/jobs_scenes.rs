//! Synthetic responses through the real registered Jobs query handlers.

use chrono::DateTime;
use chrono::Utc;
use fleetworks_contracts::context::ActivityScope;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::ClientError;
use fleetworks_desktop::app::Intent;
use fleetworks_desktop::app::Load;
use fleetworks_desktop::backend::ActivityJob;
use fleetworks_desktop::backend::JobDetail;
use fleetworks_desktop::backend::ReadKind;
use fleetworks_desktop::jobs_query::JobsDashboardCounts;
use fleetworks_desktop::jobs_query::JobsEvent;
use fleetworks_desktop::jobs_query::JobsPage;
use fleetworks_desktop::jobs_query::JobsStatusFilter;
use fleetworks_desktop::jobs_query::JobsTicket;
use fleetworks_desktop::jobs_query::JobsTime;
use fleetworks_desktop::Page;
use fleetworks_desktop::State;
use rune::TextEdit;

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-12T12:00:00Z")
        .expect("synthetic clock")
        .with_timezone(&Utc)
}
fn cursor(number: usize) -> String {
    format!("{number:064x}")
}
fn row(id: &str, app: &str, operation: &str, state: &str) -> ActivityJob {
    ActivityJob {
        id: id.into(),
        app: app.into(),
        title: "Synthetic eligible work".into(),
        operation: operation.into(),
        state: state.into(),
        phase: state.into(),
        progress_done: None,
        progress_total: None,
        error_summary: String::new(),
        private_details_available: true,
        activity_actions_available: false,
        status_only: false,
        can_cancel: false,
        can_retry: false,
        action_reason_code: "not_offered".into(),
    }
}
fn response(
    ticket: &JobsTicket,
    rows: Vec<ActivityJob>,
    number: usize,
    total: Option<u64>,
    next: Option<usize>,
) -> JobsPage {
    let scope = ActivityScope::for_workstation(&ticket.binding.context);
    JobsPage {
        apps: rows
            .iter()
            .map(|row| row.app.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect(),
        rows,
        activity_scope: scope,
        relation_counts_available: false,
        activity_actions_available: false,
        counts: Some(JobsDashboardCounts {
            active: 7,
            queued: 12,
            retrying: 1,
            failed: 2,
            completed_recent: 31,
        }),
        dashboard_counts_scope: "unfiltered_top_level_workstation_activity".into(),
        matched_total: total,
        observed_at: "2026-11-12T12:00:00Z".into(),
        apps_complete: true,
        has_more: next.is_some(),
        page_cursor: cursor(number),
        next_cursor: next.map(cursor),
    }
}

/// Initial scene data still enters through the production controller/completion.
pub fn install(state: &mut State, rows: Vec<ActivityJob>) {
    state.sync_jobs_binding();
    let ticket = state
        .scoped_jobs
        .handle(JobsEvent::Apply, now())
        .read
        .expect("synthetic registered query");
    let count = rows.len() as u64;
    let page = response(&ticket, rows, 1, Some(count), None);
    state.complete_scoped_jobs(ticket, Ok(page));
}

fn setup(state: &mut State) {
    super::registered(state);
    state.page = Page::Jobs;
    state.sync_jobs_binding();
}
pub fn filters_ready(state: &mut State) {
    setup(state);
    state.scoped_jobs.draft.status = JobsStatusFilter::Failed;
    state.scoped_jobs.draft.app = TextEdit::new("tools");
    state.scoped_jobs.draft.operation = TextEdit::new("manual_tool");
    state.scoped_jobs.draft.kind = TextEdit::new("task");
    state.scoped_jobs.draft.correlation_id = TextEdit::new("synthetic-correlation");
    state.scoped_jobs.draft.text = TextEdit::new("Synthetic");
    state.scoped_jobs.draft.time = JobsTime::Day;
    state.request_jobs(JobsEvent::Apply);
}
pub fn page_two(state: &mut State) {
    setup(state);
    state.request_jobs(JobsEvent::Apply);
    state.request_jobs(JobsEvent::Next);
}
pub fn no_matches(state: &mut State) {
    setup(state);
    state.scoped_jobs.draft.text = TextEdit::new("no-match");
    state.request_jobs(JobsEvent::Apply);
}
pub fn expired(state: &mut State) {
    setup(state);
    state.request_jobs(JobsEvent::Apply);
    state.fixture_effect = None;
    state.request_jobs(JobsEvent::Next);
    let ticket = state
        .scoped_jobs
        .active_ticket()
        .expect("synthetic pending continuation")
        .clone();
    state.complete_scoped_jobs(ticket, Err(ClientError::Status(409)));
}
pub fn permission_required(state: &mut State) {
    setup(state);
    state
        .connection
        .capabilities
        .as_mut()
        .expect("synthetic metadata")
        .permissions
        .retain(|permission| *permission != Permission::ReadWorkstationActivity);
    state.sync_jobs_binding();
    state.request_jobs(JobsEvent::Apply);
}
pub fn loading(state: &mut State) {
    setup(state);
    state.fixture_effect = None;
    state.request_jobs(JobsEvent::Apply);
}
pub fn error(state: &mut State) {
    loading(state);
    let ticket = state
        .scoped_jobs
        .active_ticket()
        .expect("synthetic pending read")
        .clone();
    state.complete_scoped_jobs(ticket, Err(ClientError::Transport));
}
pub fn status_only(state: &mut State) {
    setup(state);
    state.fixture_effect = None;
    state.request_jobs(JobsEvent::Apply);
    let ticket = state
        .scoped_jobs
        .active_ticket()
        .expect("synthetic pending read")
        .clone();
    let mut shared = row("synthetic-shared-job", "atlas", "atlas_cycle", "running");
    shared.title.clear();
    shared.phase.clear();
    shared.private_details_available = false;
    shared.status_only = true;
    let page = response(&ticket, vec![shared], 1, Some(1), None);
    state.complete_scoped_jobs(ticket, Ok(page));
}

pub fn reply(state: &mut State, intent: &Intent) -> bool {
    if matches!(
        intent,
        Intent::BackendRead {
            kind: ReadKind::JobDetail
        }
    ) {
        let Some(id) = &state.backend.selected_job else {
            return true;
        };
        let job = match &state.backend.jobs {
            Load::Ready(page) => page.rows.iter().find(|row| &row.id == id).cloned(),
            _ => None,
        };
        state.backend.detail = match job {
            Some(job) => Load::Ready(JobDetail {
                private_details_available: job.private_details_available,
                activity_actions_available: job.activity_actions_available,
                job,
                stages: Vec::new(),
                maintenance_result: None,
            }),
            None => Load::Failed("This synthetic job is outside the fixture response.".into()),
        };
        return true;
    }
    if !matches!(
        intent,
        Intent::BackendRead {
            kind: ReadKind::Jobs
        }
    ) {
        return false;
    }
    let Some(ticket) = state.scoped_jobs.active_ticket().cloned() else {
        return true;
    };
    let query = state
        .scoped_jobs
        .applied_query()
        .expect("pinned synthetic filter snapshot")
        .clone();
    let number = match ticket.query.cursor.as_deref() {
        None => 1,
        Some(value) if value == cursor(1) => 1,
        Some(value) if value == cursor(2) => 2,
        Some(_) => {
            state.complete_scoped_jobs(ticket, Err(ClientError::Status(409)));
            return true;
        }
    };
    let empty = query.text.as_deref() == Some("no-match");
    let status = match query.status.unwrap_or_default() {
        JobsStatusFilter::Failed => "failed",
        JobsStatusFilter::Completed => "completed",
        JobsStatusFilter::Stopped => "cancelled",
        JobsStatusFilter::Retrying => "retry_scheduled",
        JobsStatusFilter::Active => "running",
        JobsStatusFilter::All => "queued",
    };
    let app = query.app.as_deref().unwrap_or("atlas");
    let operation = query.operation.as_deref().unwrap_or("atlas_cycle");
    let rows = if empty {
        Vec::new()
    } else if number == 1 {
        let parent = row("synthetic-query-job", app, operation, status);
        let mut child = row("synthetic-query-causal-child", app, operation, status);
        child.title = "Synthetic causal child stage".into();
        vec![parent, child]
    } else {
        vec![row("synthetic-query-next-job", app, operation, status)]
    };
    let next = (!empty && number == 1).then_some(2);
    let mut page = response(&ticket, rows, number, Some(if empty { 0 } else { 3 }), next);
    page.apps_complete = empty || number == 2;
    state.complete_scoped_jobs(ticket, Ok(page));
    true
}
