//! Synthetic state and the actual standalone Rune log panel; no live gateway,
//! home, credentials, clipboard or file dialogs are opened by these fixtures.

use super::*;
use crate::app::Load;
use fleetworks_contracts::connect::RegistrationReceipt;
use rune::Env;
use rune::Runtime;
use serde_json::json;

fn binding(node: &str) -> LogBinding {
    let receipt = RegistrationReceipt::new(
        "fixture-deployment".into(),
        format!("fixture-node-{node}"),
        format!("fixture-registration-{node}"),
        Some("192.0.2.42".parse().expect("synthetic IP")),
        1,
    );
    LogBinding {
        context: RegisteredContext::new(
            receipt,
            1,
            1,
            "fixture-config".into(),
            "fixture-access".into(),
        )
        .expect("fixture context"),
        connection_epoch: 1,
        origin_revision: 1,
    }
}
fn cursor(n: usize) -> String {
    format!("{n:064x}")
}
fn row(n: usize) -> LogEntry {
    serde_json::from_value(json!({"id":format!("event-{n}"),"ts":"2026-11-12T12:00:00Z","severity":"info","app":"tools","event_type":"progress","message":format!("Synthetic safe summary {n}"),"job_id":"fixture-job","task_id":"","attempt_id":"","run_id":""})).expect("safe wire row")
}
fn page(
    start: usize,
    length: usize,
    total: u64,
    next: Option<String>,
    follow: Option<String>,
) -> LogPage {
    let rows: Vec<_> = (start..start + length).map(row).collect();
    serde_json::from_value(json!({"rows":rows,"counts":{"total":total,"info":total,"error":0,"warn":0,"debug":0},"apps":["tools"],"apps_complete":true,"next_cursor":next,"follow_cursor":follow,"available":true,"private_logs_only":true,"shared_private_logs_available":true,"reason_code":null,"narrowing_hint":null,"has_more":next.is_some()})).expect("actual public page shape")
}
fn ready() -> LogState {
    let mut logs = LogState::default();
    logs.bind(Some(binding("a")), true);
    logs.set_visible(true);
    let ticket = logs.refresh().expect("registered read");
    assert!(logs.complete(
        &ticket,
        Ok(LogReply::Page(page(
            0,
            1,
            10,
            Some(cursor(1)),
            Some(cursor(2))
        )))
    ));
    logs
}
fn export_reply(n: usize, next: Option<String>) -> LogReply {
    let mut bytes = serde_json::to_vec(&row(n)).expect("safe export row");
    bytes.push(b'\n');
    LogReply::Export {
        bytes,
        next_cursor: next,
        apps_complete: true,
    }
}

#[test]
fn query_scopes_filters_and_cursor_continuation_never_supply_backend_authority() {
    let mut logs = LogState::default();
    logs.bind(Some(binding("a")), true);
    let whole = logs.refresh().expect("whole workstation");
    assert!(whole.query.job_id.is_none());
    assert!(serde_json::to_value(&whole.query)
        .expect("query")
        .get("workspace_id")
        .is_none());
    logs.select_target(LogTarget::SelectedWorkspace);
    let child = logs.refresh().expect("selected child");
    assert!(serde_json::to_value(&child.query)
        .expect("legacy selector cannot narrow current scope")
        .get("workspace_id")
        .is_none());
    logs.select_target(LogTarget::Job {
        id: "fixture-job".into(),
        descendants: true,
    });
    logs.draft.text = TextEdit::new("safe");
    logs.draft.since = TextEdit::new("2026-11-12T07:00:00-05:00");
    logs.draft.min_rank = Some(2);
    let descendants = logs.refresh().expect("scoped descendants");
    assert_eq!(descendants.query.job_id.as_deref(), Some("fixture-job"));
    assert_eq!(descendants.query.descendants, Some(true));
    assert_eq!(
        descendants.query.since.as_deref(),
        Some("2026-11-12T12:00:00+00:00")
    );
    assert!(logs.complete(
        &descendants,
        Ok(LogReply::Page(page(
            0,
            1,
            10,
            Some(cursor(3)),
            Some(cursor(4))
        )))
    ));
    let next = logs.older().expect("opaque continuation");
    let query = serde_json::to_value(next.query).expect("query");
    assert_eq!(
        query,
        json!({"cursor":cursor(3)}),
        "the server owns the cursor's exact scope and filters"
    );
    logs.draft.text = TextEdit::new("a".repeat(257));
    logs.draft_edited();
    assert!(logs.refresh().is_none());
    assert!(logs.rows.is_empty() && logs.counts.is_none());
}

#[test]
fn changed_context_auth_and_cohort_clear_sensitive_state_and_reject_queued_completions() {
    let mut logs = ready();
    let old = logs.older().expect("old queued read");
    logs.bind(Some(binding("b")), true);
    assert!(logs.rows.is_empty() && logs.export.bytes_for_save(true).is_none());
    assert!(!logs.complete(&old, Ok(LogReply::Page(page(1, 1, 10, None, None)))));
    let current = logs.refresh().expect("new binding");
    logs.complete(&current, Err(ClientError::Status(401)));
    logs.bind(Some(binding("b")), true);
    assert!(
        !logs.connected(),
        "reusing the rejected credential epoch cannot resurrect access"
    );
    let mut rotated = binding("b");
    rotated.connection_epoch = 2;
    logs.bind(Some(rotated), true);
    let current = logs.refresh().expect("explicit new credential epoch");
    logs.complete(
        &current,
        Ok(LogReply::Page(page(
            0,
            1,
            10,
            Some(cursor(1)),
            Some(cursor(2)),
        ))),
    );
    let changed = logs.older().expect("current cursor");
    logs.complete(&changed, Err(ClientError::Status(409)));
    assert_eq!(logs.status, LogStatus::RestartRequired);
    assert!(logs.rows.is_empty() && logs.counts.is_none() && logs.copy_loaded().is_none());
    assert!(logs.older().is_none() && logs.continue_export().is_none());
    assert!(
        logs.refresh().is_some(),
        "an explicit scoped refresh can begin a new complete cohort"
    );
}

#[test]
fn follow_only_runs_when_visible_and_late_hidden_reply_cannot_repopulate_rows() {
    let mut logs = ready();
    logs.set_following(true);
    let follow = logs.follow().expect("visible finite follow");
    assert_eq!(follow.query.wait_ms, Some(3000));
    logs.set_visible(false);
    assert!(!logs.following && !logs.busy());
    assert!(!logs.complete(
        &follow,
        Ok(LogReply::Page(page(20, 1, 11, Some(cursor(3)), None)))
    ));
    assert_eq!(logs.rows[0].id, "event-0");
    assert!(logs.follow().is_none());
    logs.set_visible(true);
    assert!(
        logs.follow().is_none(),
        "visibility alone does not silently restart follow"
    );
}

#[test]
fn display_limit_keeps_received_overflow_cursor_unadvanced_and_offers_separate_export() {
    let mut logs = LogState::default();
    logs.bind(Some(binding("a")), true);
    logs.set_visible(true);
    let first = logs.refresh().expect("first page");
    logs.complete(
        &first,
        Ok(LogReply::Page(page(
            0,
            100,
            1000,
            Some(cursor(1)),
            Some(cursor(100)),
        ))),
    );
    for n in 1..=4 {
        let ticket = logs.older().expect("bounded older page");
        logs.complete(
            &ticket,
            Ok(LogReply::Page(page(
                n * 100,
                if n == 4 { 50 } else { 100 },
                1000,
                Some(cursor(n + 1)),
                None,
            ))),
        );
    }
    assert_eq!(logs.rows.len(), 450);
    let old_cursor = logs.follow_cursor.clone();
    logs.set_following(true);
    let ticket = logs.follow().expect("last permitted follow");
    logs.complete(
        &ticket,
        Ok(LogReply::Page(page(
            500,
            100,
            1100,
            Some(cursor(101)),
            None,
        ))),
    );
    assert_eq!(logs.status, LogStatus::DisplayLimit);
    assert_eq!(
        logs.rows.len(),
        450,
        "the client never silently drops a suffix and advances past it"
    );
    assert_eq!(logs.follow_cursor, old_cursor);
    assert!(!logs.following && !logs.can_load_older() && !logs.can_follow());
    assert!(logs.can_export());
    let copied = logs.copy_loaded().expect("explicit loaded-subset copy");
    assert!(!copied.complete_eligible_scope);
    assert_eq!(copied.rows, 450);
}

#[test]
fn export_is_explicitly_partial_until_last_page_and_bounded_or_private_data_never_claims_complete()
{
    let mut logs = ready();
    let first = logs.prepare_export().expect("separate export snapshot");
    logs.complete(&first, Ok(export_reply(1, Some(cursor(2)))));
    assert!(!logs.export.complete);
    assert!(logs.export.bytes_for_save(false).is_none());
    assert!(
        logs.export.bytes_for_save(true).is_some(),
        "the explicit partial-save control can export a labelled subset"
    );
    let next = logs.continue_export().expect("complete next export page");
    assert_eq!(next.query.cursor, Some(cursor(2)));
    logs.complete(&next, Ok(export_reply(2, None)));
    assert!(logs.export.complete && logs.export.bytes_for_save(false).is_some());
    assert_eq!(logs.export.rows(), 2);
    let first = logs.prepare_export().expect("bounded export");
    logs.complete(&first, Ok(export_reply(0, Some(cursor(1)))));
    for n in 1..EXPORT_PAGES {
        let next = logs.continue_export().expect("bounded page");
        logs.complete(&next, Ok(export_reply(n, Some(cursor(n + 1)))));
    }
    assert!(logs.export.bounded && !logs.export.complete && !logs.export.has_more());
    assert_eq!(logs.export.rows(), EXPORT_PAGES);
    assert!(logs.export.bytes_for_save(false).is_none());
    let first = logs.prepare_export().expect("new exact export");
    let mut raw = serde_json::to_value(row(1)).expect("row");
    raw["details"] = json!("never exported");
    logs.complete(
        &first,
        Ok(LogReply::Export {
            bytes: format!("{raw}\n").into_bytes(),
            next_cursor: None,
            apps_complete: true,
        }),
    );
    assert!(logs.export.bytes_for_save(true).is_none());
    assert!(
        logs.rows.is_empty(),
        "an invalid private payload clears the display too"
    );
}

#[test]
fn unavailable_counts_and_incomplete_facets_remain_distinct_from_zero_or_complete() {
    let mut logs = ready();
    let refresh = logs.refresh().expect("current scope");
    let mut partial = page(0, 1, 1, None, Some(cursor(1)));
    partial.apps_complete = false;
    partial.shared_private_logs_available = false;
    logs.complete(&refresh, Ok(LogReply::Page(partial)));
    assert!(!logs.apps_complete && !logs.shared_private_logs_available);
    assert!(
        logs.copy_loaded()
            .expect("safe loaded copy")
            .excludes_shared_private
    );
    let refresh = logs.refresh().expect("explicit refresh");
    let unavailable=serde_json::from_value(json!({"rows":[],"counts":null,"apps":[],"apps_complete":false,"next_cursor":null,"follow_cursor":null,"available":false,"private_logs_only":true,"shared_private_logs_available":false,"reason_code":"scope_exceeds_complete_log_cohort_limit","narrowing_hint":"Select a child or attributed job","has_more":false})).expect("actual unavailable wire");
    logs.complete(&refresh, Ok(LogReply::Page(unavailable)));
    assert_eq!(logs.status, LogStatus::Unavailable);
    assert!(logs.counts.is_none() && logs.rows.is_empty() && !logs.can_export());
    assert!(logs
        .message
        .as_deref()
        .is_some_and(|message| message.contains("512")));
}

#[test]
fn canonical_export_accumulation_stays_within_two_mib_and_requires_partial_consent_at_bound() {
    let mut logs = ready();
    let mut ticket = logs.prepare_export().expect("bounded export");
    for page in 0..EXPORT_PAGES {
        let mut bytes = Vec::new();
        for index in 0..100 {
            let mut row = row(page * 100 + index);
            row.message = "🧪".repeat(1000);
            serde_json::to_writer(&mut bytes, &row).expect("canonical UTF-8 safe summary");
            bytes.push(b'\n');
        }
        assert!(
            bytes.len() < EXPORT_LIMIT,
            "individual pages fit the existing file adapter budget"
        );
        logs.complete(
            &ticket,
            Ok(LogReply::Export {
                bytes,
                next_cursor: Some(cursor(page + 1)),
                apps_complete: true,
            }),
        );
        assert!(logs.export.bytes.len() <= 2 * 1024 * 1024);
        if logs.export.bounded {
            break;
        }
        ticket = logs.continue_export().expect("next bounded export page");
    }
    assert!(logs.export.bounded && !logs.export.complete);
    assert!(logs.export.bytes_for_save(false).is_none());
    assert!(logs.export.bytes_for_save(true).is_some());
    assert!(logs
        .message
        .as_deref()
        .is_some_and(|message| message.contains("2 MiB")));
}

fn shell() -> crate::app::State {
    let mut state = crate::app::State::fixture();
    state.connection.status = crate::connection::ConnectionStatus::Connected;
    let mut capabilities = fleetworks_contracts::capabilities::Capabilities::default();
    capabilities.permissions = vec![
        fleetworks_contracts::context::Permission::ReadBackend,
        fleetworks_contracts::context::Permission::ReadWorkstationActivity,
    ];
    state.connection.capabilities = Some(capabilities);
    state.registration.context = Some(binding("a").context);
    state.scoped_logs.window_focused = true;
    state.navigate(crate::app::Page::Jobs);
    state.sync_log_binding();
    state.backend.selected_job = Some("fixture-job".into());
    let job: crate::backend::ActivityJob = serde_json::from_value(json!({"id":"fixture-job","app":"tools","title":"Synthetic selected job","operation":"tool_run","state":"running","phase":"running","progress_done":null,"progress_total":null,"error_summary":"","private_details_available":true,"activity_actions_available":false,"status_only":false})).expect("safe scoped job");
    state.backend.jobs = Load::Ready(crate::backend::ActivityJobs {
        rows: vec![job.clone()],
        activity_scope: fleetworks_contracts::context::ActivityScope::for_workstation(
            state.registration.context.as_ref().expect("context"),
        ),
        relation_counts_available: false,
        activity_actions_available: false,
    });
    state.backend.detail = Load::Ready(crate::backend::JobDetail {
        job,
        stages: Vec::new(),
        private_details_available: true,
        activity_actions_available: false,
        maintenance_result: None,
    });
    state.sync_log_visibility();
    state.intents.clear();
    state
}

#[test]
fn large_loaded_log_set_mounts_only_table_window_and_preserves_full_safe_selection_and_copy() {
    fn data(length: usize) -> crate::app::State {
        let mut state = shell();
        state.reveal_operational_logs();
        let first = state.scoped_logs.refresh().expect("registered first page");
        let n = length.min(100);
        state.scoped_logs.complete(
            &first,
            Ok(LogReply::Page(page(
                0,
                n,
                length as u64,
                (length > n).then(|| cursor(1)),
                Some(cursor(100)),
            ))),
        );
        let mut start = n;
        while start < length {
            let ticket = state
                .scoped_logs
                .older()
                .expect("complete loaded cursor page");
            let n = (length - start).min(100);
            state.scoped_logs.complete(
                &ticket,
                Ok(LogReply::Page(page(
                    start,
                    n,
                    length as u64,
                    (start + n < length).then(|| cursor(start + n)),
                    None,
                ))),
            );
            start += n;
        }
        state.scoped_logs.rows[0].message = "Full safe selected summary ".repeat(30);
        state
    }
    fn render(state: crate::app::State) -> Runtime<crate::app::State> {
        let mut runtime = Runtime::new(
            state,
            crate::app::view,
            (1180.0, 800.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        runtime.frame(0.0);
        runtime.update(|_, tx| tx.scroll_into_view("scoped-logs-table"));
        runtime.frame(1.0 / 60.0);
        runtime
    }
    fn mounted(runtime: &Runtime<crate::app::State>) -> usize {
        runtime
            .elements()
            .iter()
            .filter(|element| {
                element.role == "row"
                    && !element.exiting
                    && element.path.contains("scoped-logs-table/")
            })
            .count()
    }
    let small = render(data(45));
    let mut large = render(data(450));
    let small_rows = mounted(&small);
    let large_rows = mounted(&large);
    assert_eq!(
        large.state.scoped_logs.rows.len(),
        450,
        "virtualization never truncates authorized client data"
    );
    assert!(
        large_rows > 0 && large_rows <= 64 && large_rows <= small_rows + 16,
        "finite320pt table window: small={small_rows}, large={large_rows}"
    );
    assert!(
        large.stats.layout_nodes <= small.stats.layout_nodes + 128,
        "adding405rows must not add405rowlayouts: small={}, large={}",
        small.stats.layout_nodes,
        large.stats.layout_nodes
    );
    let copy = large
        .state
        .scoped_logs
        .copy_loaded()
        .expect("all loaded safe rows stay copyable");
    assert_eq!(copy.rows, 450);
    assert!(copy.complete_eligible_scope);
    assert_eq!(copy.text.lines().count(), 450);
    let (x, y) = large.find("event-0").expect("mounted first safe row");
    large.pointer_down(x, y);
    large.pointer_up(x, y);
    large.frame(2.0 / 60.0);
    assert_eq!(large.state.scoped_logs.table.cursor(), Some("event-0"));
    let detail = large
        .elements()
        .into_iter()
        .find(|element| element.key.as_deref() == Some("logs-selected-summary"))
        .expect("selected full safe summary");
    assert_eq!(
        detail.text.as_deref(),
        Some(large.state.scoped_logs.rows[0].message.as_str())
    );
}

fn fake_reply(state: &mut crate::app::State, intent: &crate::app::Intent) {
    use crate::app::Intent;
    use crate::backend::JobDetail;
    use crate::backend::ReadKind;
    use crate::backend::ReadResult;

    match intent {
        Intent::BackendRead {
            kind: ReadKind::JobDetail,
        } => {
            let Load::Ready(jobs) = &state.backend.jobs else {
                panic!("fixture detail requires the job list");
            };
            let job = jobs
                .rows
                .iter()
                .find(|job| Some(job.id.as_str()) == state.backend.selected_job.as_deref())
                .expect("selected fixture job")
                .clone();
            state.backend.revisions.insert(ReadKind::JobDetail, 1);
            state.complete_backend_read(
                ReadKind::JobDetail,
                1,
                Ok(ReadResult::JobDetail(Box::new(JobDetail {
                    job,
                    stages: Vec::new(),
                    private_details_available: true,
                    activity_actions_available: false,
                    maintenance_result: None,
                }))),
            );
        }
        Intent::OperationalLogs { action } => {
            let ticket = state
                .scoped_logs
                .active_ticket()
                .expect("same-handler ticket")
                .clone();
            let result = if *action == LogAction::Export {
                export_reply(1, Some(cursor(2)))
            } else {
                LogReply::Page(page(0, 1, 10, None, Some(cursor(2))))
            };
            state.complete_scoped_logs(ticket, Ok(result));
        }
        _ => {}
    }
}
#[test]
fn actual_shell_job_selection_automatically_reads_safe_logs_and_fences_hidden_replies() {
    let mut state = shell();
    state.back_to_jobs();
    state.fixture_effect = Some(fake_reply);
    let mut runtime = Runtime::new(
        state,
        crate::app::view,
        (1180.0, 800.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    runtime.frame(0.0);
    for key in [
        "t-logs",
        "t-jobs",
        "logs-refresh",
        "logs-copy-loaded",
        "logs-export-prepare",
        "logs-export-save",
    ] {
        assert!(
            runtime.find(key).is_none(),
            "retired toolbar/tab control {key} must not reappear"
        );
    }
    runtime.update(|_, tx| tx.scroll_into_view("activity-job-fixture-job"));
    runtime.frame(1.0 / 60.0);
    let (x, y) = runtime
        .find("activity-job-fixture-job")
        .expect("actual selected-job control");
    runtime.pointer_down(x, y);
    runtime.pointer_up(x, y);
    runtime.frame(2.0 / 60.0);
    assert_eq!(runtime.state.scoped_logs.rows[0].id, "event-0");
    assert!(
        matches!(&runtime.state.scoped_logs.target,LogTarget::Job { id, descendants:false } if id=="fixture-job")
    );
    assert!(runtime.state.intents.iter().any(|intent| matches!(
        intent,
        crate::app::Intent::OperationalLogs {
            action: LogAction::Refresh
        }
    )));
    assert!(!runtime.state.intents.iter().any(|intent| matches!(
        intent,
        crate::app::Intent::PostJobAction { .. }
            | crate::app::Intent::PostInteractive { .. }
            | crate::app::Intent::CopyLogSummaries
            | crate::app::Intent::SaveLogExport { .. }
    )));
    runtime.window_focus(true);
    runtime.frame(3.0 / 60.0);
    runtime.state.scoped_logs.set_following(true);
    runtime.window_focus(false);
    runtime.frame(4.0 / 60.0);
    assert!(!runtime.state.scoped_logs.following && !runtime.state.scoped_logs.visible);
    runtime.window_focus(true);
    runtime.frame(5.0 / 60.0);
    runtime.state.scoped_logs.set_following(true);
    let pending = runtime
        .state
        .scoped_logs
        .follow()
        .expect("visible finite follow");
    let (x, y) = runtime
        .find("nav-models")
        .expect("current sidebar navigation");
    runtime.pointer_down(x, y);
    runtime.pointer_up(x, y);
    runtime.frame(6.0 / 60.0);
    assert_eq!(runtime.state.page, crate::app::Page::Models);
    assert!(!runtime.state.scoped_logs.following && !runtime.state.scoped_logs.busy());
    runtime.state.complete_scoped_logs(
        pending,
        Ok(LogReply::Page(page(99, 1, 11, Some(cursor(4)), None))),
    );
    assert_eq!(
        runtime.state.scoped_logs.rows[0].id, "event-0",
        "hidden-view completion cannot repopulate safe summaries"
    );
}
#[test]
fn production_navigation_origin_and_disconnect_hooks_fence_log_reads_without_cancelling_jobs() {
    let mut state = shell();
    state.reveal_operational_logs();
    let ticket = state.scoped_logs.refresh().expect("current native scope");
    state.complete_scoped_logs(
        ticket,
        Ok(LogReply::Page(page(
            0,
            1,
            10,
            Some(cursor(1)),
            Some(cursor(2)),
        ))),
    );
    state.scoped_logs.set_following(true);
    let old = state.scoped_logs.follow().expect("visible follow");
    state.navigate(crate::app::Page::Brain);
    assert!(!state.scoped_logs.following && !state.scoped_logs.busy());
    state.complete_scoped_logs(
        old,
        Ok(LogReply::Page(page(9, 1, 11, Some(cursor(3)), None))),
    );
    assert_eq!(state.scoped_logs.rows.len(), 1);
    state.invalidate_origin();
    assert!(state.scoped_logs.rows.is_empty() && !state.scoped_logs.connected());
    state.registration.context = Some(binding("b").context);
    state.sync_log_binding();
    let old = state.scoped_logs.refresh().expect("new scoped read");
    state.disconnect();
    state.complete_scoped_logs(old, Ok(LogReply::Page(page(0, 1, 1, None, None))));
    assert!(state.scoped_logs.rows.is_empty() && state.scoped_logs.counts.is_none());
    assert!(
        !state.intents.iter().any(|intent| matches!(
            intent,
            crate::app::Intent::PostJobAction { .. } | crate::app::Intent::PostInteractive { .. }
        )),
        "local read lifecycle never submits cancel/retry or any replacement job"
    );
}

#[test]
fn compact_log_headers_fit_the_real_viewport_and_selection_reveals_full_summary() {
    let mut state = shell();
    state.reveal_operational_logs();
    let ticket = state.scoped_logs.refresh().expect("registered log read");
    state
        .scoped_logs
        .complete(&ticket, Ok(LogReply::Page(page(0, 1, 1, None, None))));
    state.scoped_logs.rows[0].message =
        "Full safe summary retained while the compact cell truncates ".repeat(8);
    let mut runtime = Runtime::new(
        state,
        crate::app::view,
        (640.0, 480.0),
        Env {
            system_dark: true,
            scale: 2.0,
        },
    );
    runtime.frame(0.0);
    runtime.update(|_, tx| tx.scroll_into_view("scoped-logs-table"));
    runtime.frame(1.0 / 60.0);
    let ([left, _, width, _], _) = runtime
        .presented("scoped-logs-table")
        .expect("real compact grid");
    let headers = runtime
        .elements()
        .into_iter()
        .filter(|element| {
            element.role == "columnheader" && element.path.contains("scoped-logs-table/")
        })
        .collect::<Vec<_>>();
    assert!(!headers.is_empty());
    for header in headers {
        assert!(
            header.rect[0] >= left - 0.5 && header.rect[0] + header.rect[2] <= left + width + 0.5,
            "{} header overflows actual compact grid: {:?}, grid left={left}, width={width}",
            header.label.as_deref().unwrap_or("unnamed"),
            header.rect
        );
    }
    let (x, y) = runtime.find("event-0").expect("real safe row");
    runtime.pointer_down(x, y);
    runtime.pointer_up(x, y);
    runtime.frame(2.0 / 60.0);
    let detail = runtime
        .elements()
        .into_iter()
        .find(|element| element.key.as_deref() == Some("logs-selected-summary"))
        .expect("full selected safe summary");
    assert_eq!(
        detail.text.as_deref(),
        Some(runtime.state.scoped_logs.rows[0].message.as_str())
    );
}

#[test]
fn transient_follow_error_preserves_validated_rows_and_resumes_on_job_sync() {
    let mut state = shell();
    let initial = state
        .scoped_logs
        .refresh()
        .expect("initial job operational logs ticket");
    state.complete_scoped_logs(
        initial,
        Ok(LogReply::Page(page(0, 1, 1, None, Some(cursor(1))))),
    );
    assert_eq!(state.scoped_logs.rows.len(), 1);
    assert!(state.scoped_logs.following);

    let follow = state.scoped_logs.follow().expect("follow ticket");
    state.complete_scoped_logs(follow, Err(fleetworks_contracts::ClientError::Timeout));
    assert_eq!(state.scoped_logs.rows.len(), 1);
    assert_eq!(state.scoped_logs.status, LogStatus::Unavailable);
    assert!(!state.scoped_logs.following);

    // Syncing after job progress should resume following without clearing retained validated rows.
    state.sync_operational_logs_with_selected_job(true);
    assert!(state.scoped_logs.following);
    assert_eq!(state.scoped_logs.rows.len(), 1);
}

#[test]
fn completed_job_stops_live_and_keeps_a_late_summary_while_focused() {
    use crate::backend::ReadKind;
    use crate::backend::ReadResult;

    let mut state = shell();
    state.selected_log_job_changed("fixture-job");
    let initial = state
        .scoped_logs
        .active_ticket()
        .expect("job-scoped initial read")
        .clone();
    state.complete_scoped_logs(
        initial,
        Ok(LogReply::Page(page(0, 1, 1, None, Some(cursor(1))))),
    );
    assert!(state.poll_scoped_logs());
    let live_read = state
        .scoped_logs
        .active_ticket()
        .expect("in-flight live follow")
        .clone();
    assert_eq!(live_read.action, LogAction::Follow);
    assert_eq!(live_read.query.cursor, Some(cursor(1)));

    let mut runtime = Runtime::new(
        state,
        crate::app::view,
        (1180.0, 800.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    let shows_live = |runtime: &Runtime<crate::app::State>| {
        runtime.elements().iter().any(|element| {
            !element.exiting
                && element.path.contains("logs-status/")
                && element.text.as_deref() == Some("Live")
        })
    };
    runtime.frame(0.0);
    assert!(shows_live(&runtime));
    let sent = runtime.state.intents.len();

    runtime.update(|state, _| {
        let Load::Ready(detail) = &state.backend.detail else {
            panic!("expected the selected running job");
        };
        let mut completed = detail.clone();
        completed.job.state = "completed".into();
        completed.job.phase = "Completed".into();
        state.backend.revisions.insert(ReadKind::JobDetail, 1);
        state.complete_backend_read(
            ReadKind::JobDetail,
            1,
            Ok(ReadResult::JobDetail(Box::new(completed))),
        );
    });
    runtime.frame(1.0 / 60.0);
    assert!(runtime.state.scoped_logs.window_focused && runtime.state.scoped_logs.visible);
    assert!(!runtime.state.scoped_logs.following && !shows_live(&runtime));
    assert_eq!(runtime.state.scoped_logs.rows[0].id, "event-0");
    assert!(runtime.find("logs-status").is_none());

    runtime.update(move |state, _| {
        assert!(!state.poll_scoped_logs());
        // A previously started long poll may deliver a final summary.
        let mut last = page(1, 1, 2, Some(cursor(2)), None);
        last.has_more = false;
        state.complete_scoped_logs(live_read, Ok(LogReply::Page(last)));
    });
    runtime.update(|_, tx| tx.scroll_into_view("scoped-logs-table"));
    runtime.frame(2.0 / 60.0);
    assert!(runtime.state.scoped_logs.window_focused && runtime.state.scoped_logs.visible);
    assert_eq!(runtime.state.scoped_logs.status, LogStatus::Ready);
    assert!(!runtime.state.scoped_logs.following && !runtime.state.scoped_logs.busy());
    assert_eq!(runtime.state.scoped_logs.rows[0].id, "event-1");
    assert_eq!(runtime.state.scoped_logs.rows[1].id, "event-0");
    assert!(
        runtime.find("event-1").is_some(),
        "the final summary is shown"
    );
    assert!(runtime.find("logs-status").is_none());
    runtime.update(|state, _| {
        assert!(!state.poll_scoped_logs());
    });
    assert_eq!(runtime.state.intents.len(), sent);
}

#[test]
fn opening_a_terminal_job_reads_a_snapshot_without_starting_live_follow() {
    let mut state = shell();
    let Load::Ready(jobs) = &mut state.backend.jobs else {
        panic!("expected a loaded job list");
    };
    jobs.rows[0].state = "completed".into();
    state.back_to_jobs();
    state.select_job("fixture-job");
    let initial = state
        .scoped_logs
        .active_ticket()
        .expect("initial snapshot read")
        .clone();
    state.complete_scoped_logs(
        initial,
        Ok(LogReply::Page(page(0, 1, 1, None, Some(cursor(1))))),
    );
    assert_eq!(state.scoped_logs.rows[0].id, "event-0");
    assert!(state.scoped_logs.visible && !state.scoped_logs.following);
    assert!(!state.poll_scoped_logs());
}

#[test]
fn job_detail_status_and_progress_updates_synchronize_logs_and_poll_preserves_selection() {
    let mut state = shell();
    state.scoped_logs.follow_cursor = Some(cursor(1));
    state.scoped_logs.following = false;
    state.scoped_logs.status = LogStatus::Ready;
    let mut updated_job: crate::backend::ActivityJob = serde_json::from_value(json!({
        "id": "fixture-job",
        "app": "tools",
        "title": "Synthetic selected job",
        "operation": "tool_run",
        "state": "running",
        "phase": "seeding_actors",
        "progress_done": 250,
        "progress_total": 1000,
        "error_summary": "",
        "private_details_available": true,
        "activity_actions_available": false,
        "status_only": false
    }))
    .expect("updated job");
    state.backend_read(crate::backend::ReadRequest::JobDetail {
        id: "fixture-job".into(),
    });
    let revision = state
        .backend
        .revisions
        .get(&crate::backend::ReadKind::JobDetail)
        .copied()
        .unwrap_or(0);
    let detail = crate::backend::JobDetail {
        job: updated_job.clone(),
        stages: Vec::new(),
        private_details_available: true,
        activity_actions_available: false,
        maintenance_result: None,
    };
    state.complete_backend_read(
        crate::backend::ReadKind::JobDetail,
        revision,
        Ok(crate::backend::ReadResult::JobDetail(Box::new(detail))),
    );
    assert!(
        state.scoped_logs.following || state.scoped_logs.active_ticket().is_some(),
        "progress transition must synchronize/resume operational log polling"
    );
    if let Load::Ready(jobs) = &state.backend.jobs {
        assert_eq!(jobs.rows[0].progress_done, Some(250));
        assert_eq!(jobs.rows[0].phase, "seeding_actors");
    } else {
        panic!("jobs list should remain ready and reflect detail progress");
    }

    // Subsequent JobDetail transition into Failed must preserve error_summary cleanly.
    updated_job.state = "failed".into();
    updated_job.phase = "Failed".into();
    updated_job.error_summary = "Actor seed catalog missing column".into();
    state.backend_read(crate::backend::ReadRequest::JobDetail {
        id: "fixture-job".into(),
    });
    let revision = state
        .backend
        .revisions
        .get(&crate::backend::ReadKind::JobDetail)
        .copied()
        .unwrap_or(0);
    let failed_detail = crate::backend::JobDetail {
        job: updated_job,
        stages: Vec::new(),
        private_details_available: true,
        activity_actions_available: false,
        maintenance_result: None,
    };
    state.complete_backend_read(
        crate::backend::ReadKind::JobDetail,
        revision,
        Ok(crate::backend::ReadResult::JobDetail(Box::new(
            failed_detail,
        ))),
    );
    assert_eq!(
        state.backend.selected_job.as_deref(),
        Some("fixture-job"),
        "syncing detail updates must preserve current job selection"
    );
    if let Load::Ready(jobs) = &state.backend.jobs {
        assert_eq!(jobs.rows[0].state, "failed");
        assert_eq!(
            jobs.rows[0].error_summary,
            "Actor seed catalog missing column"
        );
    }
}

#[test]
fn unavailable_logs_hide_empty_summary_and_render_worker_sync_status() {
    let mut state = shell();
    let ticket = state.scoped_logs.refresh().expect("initial log ticket");
    state.complete_scoped_logs(ticket, Err(fleetworks_contracts::ClientError::Transport));
    assert_eq!(state.scoped_logs.status, LogStatus::Unavailable);
    let mut runtime = Runtime::new(
        state,
        crate::app::view,
        (1180.0, 800.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    runtime.frame(0.0);
    let tree = runtime.accessibility_json().to_string();
    assert!(
        !tree.contains("No safe operational summaries are available for this job."),
        "unavailable state must not contradict itself with empty-summary text"
    );
    assert!(tree.contains("Refresh logs"));
    assert!(tree.contains("running"));
}

#[test]
fn brain_debounce_and_first_fetch_share_one_results_skeleton() {
    let mut runtime = Runtime::new(
        shell(),
        crate::app::view,
        (1180.0, 800.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    runtime.update(|state, _| {
        state.navigate(crate::app::Page::Brain);
        state.brain.results = Load::Pending;
    });
    runtime.frame(0.0);
    assert!(
        runtime.find("search-loading").is_some(),
        "debounce shows the skeleton"
    );
    assert!(
        runtime.find("search-pending").is_none(),
        "no debounce narration strip"
    );
    let debounce_size = runtime.find("search-loading");

    runtime.update(|state, _| state.brain.results = Load::Loading);
    runtime.frame(1.0 / 60.0);
    assert!(runtime.find("search-loading").is_some());
    assert_eq!(
        runtime.find("search-loading"),
        debounce_size,
        "the first fetch does not move or replace the skeleton"
    );
}

#[test]
fn loading_list_areas_render_shape_accurate_shimmering_skeletons_with_reduced_motion_fallback() {
    let mut state = shell();
    state.backend.jobs = Load::Loading;
    state.backend.detail = Load::Loading;
    let mut runtime = Runtime::new(
        state,
        crate::app::view,
        (1180.0, 800.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    runtime.frame(0.0);
    assert!(runtime.find("jobs-loading").is_none());
    assert!(runtime.find("job-detail-loading").is_some());
    assert!(runtime.find("skeleton-shimmer").is_some());
    runtime.update(|state, _| {
        state.back_to_jobs();
    });
    runtime.frame(1.0 / 60.0);
    assert!(runtime.find("jobs-loading").is_some());
    assert!(runtime.find("job-detail-loading").is_none());

    runtime.update(|state, _| {
        state.navigate(crate::app::Page::Workstation);
        state.workstation_status_checks_open = true;
        state.readiness = Load::Loading;
    });
    runtime.frame(1.0 / 60.0);
    assert!(runtime.find("readiness-loading").is_some());

    runtime.update(|state, _| {
        state.navigate(crate::app::Page::Brain);
        state.brain.results = Load::Loading;
    });
    runtime.frame(2.0 / 60.0);
    assert!(runtime.find("search-loading").is_some());
    assert!(runtime.find("dossier-loading").is_none());

    runtime.update(|state, _| {
        state.brain.results = Load::Ready(crate::tests::brain_search_response_fixture());
        state.toggle_actor(0);
    });
    runtime.frame(3.0 / 60.0);
    assert!(runtime.find("actor-hit-first").is_some());
    assert!(runtime.find("dossier-loading").is_some());

    runtime.update(|state, _| {
        state.navigate(crate::app::Page::Models);
        state.backend.models = Load::Loading;
        state.accessibility.fixture_override = Some(crate::accessibility::FixtureAccessibility {
            reduced_motion: true,
            ..crate::accessibility::FixtureAccessibility::default()
        });
    });
    runtime.frame(4.0 / 60.0);
    assert!(runtime.find("models-loading").is_some());
    assert!(
        runtime.find("skeleton-shimmer").is_none(),
        "reduced-motion mode disables shimmering skeleton sweep layers"
    );
}

fn simulated_log_row(id: &str, severity: &str, event_type: &str, message: &str) -> LogEntry {
    serde_json::from_value(json!({
        "id": id,
        "ts": "2026-11-14T21:35:34Z",
        "severity": severity,
        "app": "native-worker",
        "event_type": event_type,
        "message": message,
        "job_id": "fixture-job",
        "task_id": "",
        "attempt_id": "",
        "run_id": ""
    }))
    .expect("simulated wire log entry")
}

async fn spawn_simulated_log_gateway(
    responses: Vec<(&'static str, serde_json::Value)>,
) -> (String, tokio::task::JoinHandle<Vec<String>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback simulated log gateway");
    let origin = format!("http://{}", listener.local_addr().expect("local addr"));
    let task = tokio::spawn(async move {
        let mut recorded_requests = Vec::new();
        for (status, payload) in responses {
            let (mut socket, _) = listener.accept().await.expect("accept client request");
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0_u8; 1];
                let read = socket.read(&mut byte).await.expect("read request byte");
                assert!(
                    read == 1,
                    "unexpected EOF reading simulated gateway request"
                );
                request.push(byte[0]);
                assert!(
                    request.len() <= 16 * 1024,
                    "bounded simulated request header"
                );
            }
            let text = String::from_utf8(request).expect("utf8 HTTP request");
            let lower = text.to_ascii_lowercase();
            assert!(
                lower.contains("\r\nauthorization: bearer fixture-token\r\n"),
                "native client must send registered bearer credential"
            );
            assert!(
                lower.contains("\r\nx-fleetworks-registration:"),
                "native client must bind log reads to registered workstation header"
            );
            recorded_requests.push(text.lines().next().unwrap_or_default().to_owned());
            let body = serde_json::to_vec(&payload).expect("serialize simulated log payload");
            let header = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket
                .write_all(header.as_bytes())
                .await
                .expect("write response headers");
            socket.write_all(&body).await.expect("write response body");
        }
        recorded_requests
    });
    (origin, task)
}

#[tokio::test(flavor = "current_thread")]
async fn simulated_backend_log_stream_exercises_follow_warnings_and_error_recovery_in_native_runtime(
) {
    let initial_payload = json!({
        "rows": [
            simulated_log_row(
                "sim-warn-paced",
                "warn",
                "rebuild_http",
                "Actor index write paced (429 write_paced); deferred pending batch",
            ),
            simulated_log_row(
                "sim-info-start",
                "info",
                "startup",
                "Worker claimed Actor reindex (attempt 1)",
            ),
        ],
        "counts": {"total": 2, "info": 1, "error": 0, "warn": 1, "debug": 0},
        "apps": ["native-worker"],
        "apps_complete": true,
        "next_cursor": cursor(2),
        "follow_cursor": cursor(1),
        "available": true,
        "private_logs_only": true,
        "shared_private_logs_available": true,
        "reason_code": null,
        "narrowing_hint": null,
        "has_more": true
    });
    let follow_error_payload = json!({
        "rows": [
            simulated_log_row(
                "sim-error-batch",
                "error",
                "rebuild_acknowledge",
                "Actor index batch rejected (correction_required=true)",
            ),
        ],
        "counts": {"total": 3, "info": 1, "error": 1, "warn": 1, "debug": 0},
        "apps": ["native-worker"],
        "apps_complete": true,
        "next_cursor": cursor(2),
        "follow_cursor": cursor(3),
        "available": true,
        "private_logs_only": true,
        "shared_private_logs_available": true,
        "reason_code": null,
        "narrowing_hint": null,
        "has_more": true
    });
    let (origin, gateway) = spawn_simulated_log_gateway(vec![
        ("200 OK", initial_payload),
        (
            "503 Service Unavailable",
            json!({"error": "temporary_backpressure"}),
        ),
        ("200 OK", follow_error_payload),
    ])
    .await;

    let mut state = shell();
    state.reveal_operational_logs();
    let client = Client::new(&origin, Some("fixture-token")).expect("loopback client");
    let refresh_ticket = state.scoped_logs.refresh().expect("initial log ticket");
    let initial_reply = execute(client.clone(), &refresh_ticket).await;

    let mut runtime = Runtime::new(
        state,
        crate::app::view,
        (1180.0, 800.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    let mut clock = 0.0;
    runtime.update(move |state, _| state.complete_scoped_logs(refresh_ticket, initial_reply));
    runtime.frame(clock);
    assert_eq!(runtime.state.scoped_logs.rows.len(), 2);
    assert_eq!(
        runtime
            .state
            .scoped_logs
            .counts
            .as_ref()
            .map(|c| (c.warn, c.info)),
        Some((1, 1))
    );

    let follow_ticket = runtime
        .state
        .scoped_logs
        .follow()
        .expect("follow ticket after initial page");
    let transient_reply = execute(client.clone(), &follow_ticket).await;
    assert!(matches!(transient_reply, Err(ClientError::Status(503))));
    runtime.update(move |state, _| state.complete_scoped_logs(follow_ticket, transient_reply));
    clock += 1.0 / 60.0;
    runtime.frame(clock);
    assert_eq!(runtime.state.scoped_logs.status, LogStatus::Unavailable);
    assert_eq!(
        runtime.state.scoped_logs.rows.len(),
        2,
        "transient 503 during follow must preserve validated info/warn rows"
    );

    runtime.update(|state, _| state.sync_operational_logs_with_selected_job(true));
    let resumed_ticket = runtime
        .state
        .scoped_logs
        .follow()
        .expect("resumed follow ticket after job progress");
    let recovered_reply = execute(client, &resumed_ticket).await;
    runtime.update(move |state, _| state.complete_scoped_logs(resumed_ticket, recovered_reply));
    clock += 1.0 / 60.0;
    runtime.frame(clock);

    assert_eq!(runtime.state.scoped_logs.status, LogStatus::Ready);
    assert_eq!(runtime.state.scoped_logs.rows.len(), 3);
    assert_eq!(
        runtime.state.scoped_logs.counts.as_ref().map(|c| c.error),
        Some(1)
    );
    let tree = runtime.accessibility_json().to_string();
    assert!(tree.contains("429 write_paced"));
    assert!(tree.contains("correction_required=true"));

    let requests = gateway.await.expect("gateway completed");
    assert_eq!(requests.len(), 3);
    assert!(requests[0].starts_with("GET /api/v1/logs?"));
    assert!(requests[1].starts_with("GET /api/v1/logs/follow?"));
    assert!(requests[2].starts_with("GET /api/v1/logs/follow?"));
}

#[tokio::test(flavor = "current_thread")]
async fn simulated_backend_logs_reject_malformed_cursor_over_wire() {
    let malformed = json!({
        "rows": [simulated_log_row("sim-bad-cursor", "error", "fault", "Bad cursor probe")],
        "counts": {"total": 1, "info": 0, "error": 1, "warn": 0, "debug": 0},
        "apps": ["native-worker"],
        "apps_complete": true,
        "next_cursor": "../not-hex-cursor",
        "follow_cursor": null,
        "available": true,
        "private_logs_only": true,
        "shared_private_logs_available": true,
        "reason_code": null,
        "narrowing_hint": null,
        "has_more": true
    });
    let (origin, gateway) = spawn_simulated_log_gateway(vec![("200 OK", malformed)]).await;
    let mut state = shell();
    state.reveal_operational_logs();
    let client = Client::new(&origin, Some("fixture-token")).expect("loopback client");
    let ticket = state.scoped_logs.refresh().expect("initial log ticket");
    let reply = execute(client, &ticket).await;
    assert!(
        reply.is_ok(),
        "mock HTTP payload deserializes at transport layer"
    );
    state.complete_scoped_logs(ticket, reply);
    assert_eq!(
        state.scoped_logs.status,
        LogStatus::Unavailable,
        "LogState::complete must reject non-hex continuation tokens from untrusted wire payloads"
    );
    assert!(state.scoped_logs.rows.is_empty() && state.scoped_logs.next_cursor.is_none());
    gateway.await.expect("gateway completed");
}

#[tokio::test(flavor = "current_thread")]
async fn simulated_actor_reindex_post_reconcile_job_projection_and_readiness_transition() {
    use fleetworks_contracts::connect::RegistrationReceipt;
    use fleetworks_contracts::context::{
        JobOrigin, JobRequestAssociation, MutationContext, OriginSource, RegisteredContext,
        WorkstationRequestDetailQuery,
    };
    use fleetworks_contracts::readiness::{
        ActorReindexResult, MaintenanceRequest, ReadinessCheck, ReadinessCheckId, ReadinessState,
        WorkstationReadiness, ACTOR_REINDEX_OPERATION, WORKSTATION_READINESS_PATH,
    };
    use fleetworks_contracts::search::ACTOR_REINDEX_PATH;

    let receipt = RegistrationReceipt::new(
        "fixture-deployment".into(),
        "fixture-node-reindex".into(),
        "fixture-registration-reindex".into(),
        Some("192.0.2.42".parse().unwrap()),
        1,
    );
    let context = RegisteredContext::new(
        receipt.clone(),
        1,
        1,
        "fixture-config".into(),
        "fixture-access".into(),
    )
    .unwrap();
    let req = MaintenanceRequest::for_reindex(
        "sim-reindex-req-1".into(),
        MutationContext::new(&context).with_configuration_revision(1),
    )
    .unwrap();
    let expected_origin = JobOrigin::new(
        &context,
        OriginSource::Native,
        "sim-reindex-req-1".into(),
        "2026-11-14T22:10:00Z".into(),
        1,
        Vec::new(),
    );

    let assoc_json = serde_json::to_value(JobRequestAssociation::new(
        "sim-reindex-job".into(),
        expected_origin,
        "2026-11-14T22:10:01Z".into(),
    ))
    .unwrap();

    let jobs_json = json!({
        "rows": [{
            "id": "sim-reindex-job",
            "app": "worker",
            "operation": "actor_reindex",
            "title": "Build actor index",
            "state": "running",
            "phase": "scanning",
            "progress_done": 64,
            "progress_total": null,
            "error_summary": "",
            "private_details_available": true,
            "activity_actions_available": false,
            "status_only": false
        }],
        "activity_scope": fleetworks_contracts::context::ActivityScope::for_workstation(&context),
        "relation_counts_available": false,
        "activity_actions_available": false,
        "counts": {
            "active": 1, "queued": 0, "retrying": 0, "failed": 0, "completed_recent": 0
        },
        "dashboard_counts_scope": "unfiltered_top_level_workstation_activity",
        "matched_total": 1,
        "observed_at": "2026-11-14T22:10:02Z",
        "apps": ["worker"],
        "apps_complete": true,
        "has_more": false,
        "page_cursor": cursor(1),
        "next_cursor": null
    });

    let typed_result = ActorReindexResult::new(
        Some("sim-reindex-job".into()),
        Some("gen-sim-7".into()),
        Some(512),
        512,
        true,
    );
    let detail_json = json!({
        "job": {
            "id": "sim-reindex-job",
            "app": "worker",
            "operation": "actor_reindex",
            "title": "Build actor index",
            "state": "completed",
            "phase": "completed",
            "progress_done": 512,
            "progress_total": 512,
            "error_summary": "",
            "private_details_available": true,
            "activity_actions_available": false,
            "status_only": false
        },
        "stages": [],
        "private_details_available": true,
        "activity_actions_available": false,
        "maintenance_result": typed_result
    });

    let mut state = shell();
    if let Some(capabilities) = &mut state.connection.capabilities {
        capabilities.deployment_id = Some("fixture-deployment".into());
        let mut search = fleetworks_contracts::capabilities::SearchCapabilities::default();
        search.actors = Some(Default::default());
        capabilities.search = Some(search);
    }
    let deployment_id = state.connection.deployment_label().to_owned();
    let mut ready_check = ReadinessCheck::new(
        ReadinessCheckId::ActorSearch,
        ReadinessState::Ready,
        "ready".into(),
        "Actor search index generation gen-sim-7 is serving.".into(),
        "2026-11-14T22:10:05Z".into(),
    );
    ready_check.fresh_for_ms = 30_000;
    let mut seed_check = ReadinessCheck::new(
        ReadinessCheckId::ActorCatalog,
        ReadinessState::Ready,
        "seeded".into(),
        "Actor catalog has 512 actors.".into(),
        "2026-11-14T22:10:05Z".into(),
    );
    seed_check.fresh_for_ms = 30_000;
    let mut readiness = WorkstationReadiness::new(
        deployment_id,
        "2026-11-14T22:10:05Z".into(),
        vec![seed_check, ready_check],
    );
    readiness.registration = Some(receipt);
    let readiness_json = serde_json::to_value(&readiness).unwrap();

    let (origin, gateway) = spawn_simulated_log_gateway(vec![
        (
            "503 Service Unavailable",
            json!({"error": "temporary_backpressure"}),
        ),
        ("200 OK", assoc_json),
        ("200 OK", jobs_json),
        ("200 OK", detail_json),
        ("200 OK", readiness_json),
    ])
    .await;
    let client = fleetworks_contracts::Client::new(&origin, Some("fixture-token"))
        .unwrap()
        .with_registered_context(&context)
        .unwrap();

    // 1. Uncertain 503 on POST /api/v1/search/actors/reindex
    let post_res: Result<fleetworks_contracts::readiness::MaintenanceReceipt, _> =
        client.post(ACTOR_REINDEX_PATH, &req).await;
    assert!(post_res.is_err());

    // 2. Reconcile exact request via GET /api/v1/workstation/requests/detail
    let query = WorkstationRequestDetailQuery::new(
        req.request_id.clone(),
        ACTOR_REINDEX_OPERATION.into(),
        Some(context.registration_id.clone()),
    );
    let assoc = client.request_detail(&query).await.expect("reconciled job");
    assert_eq!(assoc.job_id, "sim-reindex-job");

    // 3. Jobs list + 4. JobDetail with ActorReindexResult + 5. WorkstationReadiness
    state.page = crate::Page::Jobs;
    state.connection.status = crate::connection::ConnectionStatus::Connected;
    state.registration.context = Some(context);
    state.sync_jobs_binding();
    state.request_jobs(crate::jobs_query::JobsEvent::Apply);
    let ticket = state.scoped_jobs.active_ticket().unwrap().clone();
    let jobs_reply = crate::jobs_query::read(&client, &ticket).await;
    if let Err(ref err) = jobs_reply {
        panic!("jobs_reply failed: {err:?}");
    }
    let detail_reply: Result<crate::backend::JobDetail, _> = client
        .get_with_query(
            "/api/v1/jobs/detail",
            &serde_json::json!({ "id": "sim-reindex-job", "limit": 100 }),
        )
        .await;
    assert!(
        detail_reply.is_ok(),
        "detail_reply failed: {detail_reply:?}"
    );
    let readiness_reply: Result<WorkstationReadiness, _> =
        client.get(WORKSTATION_READINESS_PATH).await;
    assert!(
        readiness_reply.is_ok(),
        "readiness_reply failed: {readiness_reply:?}"
    );

    let mut runtime = Runtime::new(
        state,
        crate::app::view,
        (1280.0, 800.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    let mut clock = 0.0;
    runtime.frame(clock);
    runtime.update(move |state, _| {
        state.complete_scoped_jobs(ticket, jobs_reply);
        state.backend.selected_job = Some("sim-reindex-job".into());
        state
            .backend
            .revisions
            .insert(crate::backend::ReadKind::JobDetail, 1);
        state.complete_backend_read(
            crate::backend::ReadKind::JobDetail,
            1,
            detail_reply
                .map(|d| crate::backend::ReadResult::JobDetail(Box::new(d)))
                .map_err(|e| e.to_string()),
        );
        state.complete_readiness(readiness_reply.map_err(|e| e.to_string()));
    });
    clock += 1.0 / 60.0;
    runtime.frame(clock);
    let jobs_tree = runtime.accessibility_json().to_string();
    for expected in [
        "Original job",
        "sim-reindex-job",
        "Index generation",
        "gen-sim-7",
        "Scanned actors",
        "512",
        "Reader visible",
    ] {
        assert!(
            jobs_tree.contains(expected),
            "Job detail must display {expected}"
        );
    }

    runtime.update(|state, _| state.page = crate::Page::Workstation);
    clock += 1.0 / 60.0;
    runtime.frame(clock);
    let ws_tree = runtime.accessibility_json().to_string();
    assert!(ws_tree.contains("Ready"));
    assert!(runtime.find("workstation-search-actors").is_some());

    let requests = gateway.await.expect("gateway completed");
    assert_eq!(requests.len(), 5);
    assert!(requests[0].starts_with("POST "));
    assert!(requests[1].starts_with("GET "));
    assert!(requests[2].starts_with("GET "));
    assert!(requests[3].starts_with("GET "));
    assert!(requests[4].starts_with("GET "));
}

#[test]
fn jobs_list_groups_newest_first_and_slides_to_scoped_logs_with_focus_return() {
    for (size, reduced_motion) in [((640.0, 480.0), true), ((1700.0, 1100.0), false)] {
        let mut state = shell();
        state.back_to_jobs();
        state.fixture_effect = Some(fake_reply);
        state.accessibility.fixture_override = Some(crate::accessibility::FixtureAccessibility {
            reduced_motion,
            ..Default::default()
        });
        let Load::Ready(jobs) = &mut state.backend.jobs else {
            panic!("expected ready job fixtures");
        };
        let job = jobs.rows[0].clone();
        jobs.rows.extend(
            [
                ("job-done", "completed"),
                ("job-failed", "failed"),
                ("job-queued", "queued"),
                ("job-retry", "retry_scheduled"),
                ("job-running", "running"),
                ("job-cancelled", "cancelled"),
            ]
            .map(|(id, status)| crate::backend::ActivityJob {
                id: id.into(),
                state: status.into(),
                ..job.clone()
            }),
        );
        let mut runtime = Runtime::new(
            state,
            crate::app::view,
            size,
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        runtime.frame(0.5);
        if !reduced_motion {
            let elements = runtime.elements();
            let sequence = [
                "jobs-group-latest",
                "activity-job-fixture-job",
                "jobs-group-running",
                "activity-job-job-running",
                "jobs-group-queued",
                "activity-job-job-queued",
                "activity-job-job-retry",
                "jobs-group-failed",
                "activity-job-job-failed",
                "jobs-group-completed",
                "activity-job-job-done",
                "jobs-group-other",
                "activity-job-job-cancelled",
            ];
            let positions = sequence.map(|key| {
                elements
                    .iter()
                    .find(|element| element.key.as_deref() == Some(key))
                    .unwrap_or_else(|| panic!("expected {key}"))
                    .rect[1]
            });
            assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
        }
        let (x, y) = runtime
            .find("activity-job-fixture-job")
            .expect("latest job");
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(0.0);
        assert!(runtime.find("activity-job-fixture-job").is_none());
        assert!(runtime.find("jobs-back-to-list").is_some());
        let entering = runtime.presented("jobs-detail-view").expect("logs view").0;
        runtime.frame(0.5);
        let settled = runtime.presented("jobs-detail-view").expect("logs view").0;
        if reduced_motion {
            assert_eq!(entering, settled);
        } else {
            assert!(entering[0] > settled[0] + 1.0);
        }
        assert_eq!(runtime.state.scoped_logs.rows[0].id, "event-0");
        assert!(runtime.find("scoped-logs-table").is_some());
        let pending = runtime
            .state
            .scoped_logs
            .follow()
            .expect("in-flight log read");

        let (x, y) = runtime.find("jobs-back-to-list").expect("Back to jobs");
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(0.5);
        assert!(runtime.find("jobs-detail-view").is_none());
        assert!(runtime.state.backend.selected_job.is_none());
        assert!(!runtime.state.scoped_logs.visible);
        runtime.update(|state, _| {
            state.complete_scoped_logs(pending, Ok(LogReply::Page(page(2, 1, 10, None, None))));
        });
        runtime.frame(0.0);
        assert!(runtime.state.scoped_logs.rows.is_empty());
        assert!(runtime.elements().iter().any(|element| {
            element.key.as_deref() == Some("activity-job-fixture-job") && element.focused
        }));
    }
}

#[test]
fn older_logs_load_by_themselves_only_while_nothing_pauses_them() {
    let mut logs = ready();
    assert!(logs.auto_load_older(), "a continuation and no message");
    let ticket = logs.older().expect("older read");
    assert!(
        !logs.auto_load_older(),
        "no second read while one is in flight"
    );
    assert!(logs.complete(
        &ticket,
        Ok(LogReply::Page(page(
            1,
            1,
            10,
            Some(cursor(3)),
            Some(cursor(4))
        )))
    ));
    assert!(logs.auto_load_older(), "the next page is eligible");
    logs.message = Some("Operational log follow was interrupted.".into());
    assert!(!logs.auto_load_older(), "a message keeps loading manual");
    logs.message = None;
    let last = logs.older().expect("last older read");
    assert!(logs.complete(
        &last,
        Ok(LogReply::Page(page(2, 1, 10, None, Some(cursor(5)))))
    ));
    assert!(!logs.auto_load_older(), "no continuation, no auto-load");
}

#[test]
fn only_a_fresh_read_is_unresolved_while_follow_and_older_keep_the_resolved_status() {
    let mut logs = ready();
    logs.set_following(true);
    let follow = logs.follow().expect("follow read");
    assert!(logs.busy());
    assert_eq!(
        logs.status,
        LogStatus::Ready,
        "a follow poll must not turn resolved rows or a resolved empty result back into loading"
    );
    logs.cancel_requests();
    assert_eq!(logs.status, LogStatus::Ready);
    assert!(!logs.complete(&follow, Ok(LogReply::Page(page(1, 0, 10, None, None)))));

    let same_query = logs.refresh().expect("same-query refresh");
    assert_eq!(
        logs.status,
        LogStatus::Ready,
        "a same-query refresh preserves resolved rows without flashing back to loading"
    );
    assert!(logs.busy());
    drop(same_query);

    logs.draft.min_rank = Some(2);
    let fresh = logs.refresh().expect("changed-query read");
    assert_eq!(
        logs.status,
        LogStatus::Loading,
        "a fresh query read is unresolved"
    );
    assert!(logs.busy());
    drop(fresh);
}

fn follow_with(logs: &mut LogState, start: usize, length: usize, next: usize) {
    logs.set_following(true);
    let ticket = logs.follow().expect("follow read");
    assert!(logs.complete(
        &ticket,
        Ok(LogReply::Page(page(
            start,
            length,
            30,
            Some(cursor(next)),
            None
        )))
    ));
}

#[test]
fn unread_counts_followed_rows_only_while_the_reader_is_away_from_the_newest() {
    let mut logs = ready();
    follow_with(&mut logs, 10, 2, 3);
    assert_eq!(logs.unread(), 0, "at the newest rows nothing is unread");

    assert!(logs.set_away(true), "scrolling away is a change");
    assert!(!logs.set_away(true), "a repeated frame changes nothing");
    follow_with(&mut logs, 20, 3, 4);
    assert_eq!(logs.unread(), 3);
    follow_with(&mut logs, 30, 1, 5);
    assert_eq!(logs.unread(), 4, "arrivals accumulate");
    assert_eq!(
        logs.rows[0].id, "event-30",
        "newest first; rows were prepended"
    );
}

#[test]
fn unread_clears_on_reaching_the_newest_rows_and_on_the_jump_control() {
    let mut logs = ready();
    logs.set_away(true);
    follow_with(&mut logs, 10, 2, 3);
    assert_eq!(logs.unread(), 2);
    assert!(logs.set_away(false), "reaching the newest rows");
    assert_eq!(logs.unread(), 0);
    follow_with(&mut logs, 20, 1, 4);
    assert_eq!(logs.unread(), 0, "following at the newest rows stays read");

    logs.set_away(true);
    follow_with(&mut logs, 30, 2, 5);
    assert_eq!(logs.unread(), 2);
    assert_eq!(logs.jump_to_newest().as_deref(), Some("event-31"));
    assert_eq!(logs.unread(), 0, "the jump control clears the count");
    assert!(
        logs.set_away(true),
        "the jump left the away state, so a later scroll counts again"
    );
}

#[test]
fn unread_never_counts_a_row_twice_or_a_refresh() {
    let mut logs = ready();
    logs.set_away(true);
    // The follow reply repeats the row already loaded and adds one new row.
    follow_with(&mut logs, 0, 2, 3);
    assert_eq!(
        logs.unread(),
        1,
        "only the row new to the loaded set counts"
    );

    // A same-query refresh preserves unread IDs while away and never counts
    // duplicate or older rows appearing after already-known rows.
    let refresh = logs.refresh().expect("refresh read");
    assert_eq!(
        logs.unread(),
        1,
        "starting a same-query refresh preserves unread IDs before the response arrives"
    );
    assert!(logs.complete(
        &refresh,
        Ok(LogReply::Page(page(
            0,
            3,
            30,
            Some(cursor(4)),
            Some(cursor(5))
        )))
    ));
    assert_eq!(
        logs.unread(),
        1,
        "refresh keeps existing unread IDs without counting duplicates or older rows"
    );
    assert!(logs.set_away(false));
    assert_eq!(logs.unread(), 0);
}

#[test]
fn transient_refresh_and_older_errors_preserve_loaded_page_viewport_and_unread_ids() {
    let mut logs = ready();
    logs.table.select("event-0");
    logs.set_away(true);
    follow_with(&mut logs, 10, 2, 3);
    assert_eq!(logs.rows.len(), 3);
    assert_eq!(logs.unread(), 2);

    // Transient Refresh error keeps loaded rows, viewport cursor, and unread IDs.
    let refresh = logs.refresh().expect("same-query refresh");
    assert_eq!(logs.rows.len(), 3);
    assert!(logs.complete(&refresh, Err(ClientError::Status(503))));
    assert_eq!(logs.status, LogStatus::Unavailable);
    assert_eq!(logs.rows.len(), 3);
    assert_eq!(logs.table.cursor(), Some("event-0"));
    assert_eq!(logs.unread(), 2);
    assert!(logs.available());

    // Transient Older error also preserves loaded rows, viewport cursor, and unread IDs.
    let older = logs.older().expect("older page retry");
    assert!(logs.complete(&older, Err(ClientError::Timeout)));
    assert_eq!(logs.status, LogStatus::Unavailable);
    assert_eq!(logs.rows.len(), 3);
    assert_eq!(logs.table.cursor(), Some("event-0"));
    assert_eq!(logs.unread(), 2);

    // Job/scope change clears rows, viewport cursor, and unread IDs.
    logs.select_target(LogTarget::Job {
        id: "other-job".into(),
        descendants: false,
    });
    assert!(logs.rows.is_empty());
    assert!(logs.table.cursor().is_none());
    assert_eq!(logs.unread(), 0);
}

#[test]
fn same_query_refresh_preserves_unread_ids_and_counts_only_new_newer_rows() {
    let mut logs = ready();
    logs.set_away(true);
    follow_with(&mut logs, 10, 1, 3);
    assert_eq!(logs.unread(), 1);

    // Loading an older page never increments unread.
    let older = logs.older().expect("older page");
    assert!(logs.complete(
        &older,
        Ok(LogReply::Page(page(100, 2, 20, Some(cursor(4)), None)))
    ));
    assert_eq!(logs.unread(), 1);

    // Same-query refresh catch-up with two brand-new newer rows at the head
    // before already-known rows counts only those two new newer IDs plus the preserved unread ID.
    let refresh = logs.refresh().expect("catch-up refresh");
    let catchup: LogPage = serde_json::from_value(json!({
        "rows": [row(30), row(29), row(10), row(0)],
        "counts": {"total": 22, "info": 22, "error": 0, "warn": 0, "debug": 0},
        "apps": ["tools"],
        "apps_complete": true,
        "next_cursor": cursor(5),
        "follow_cursor": cursor(6),
        "available": true,
        "private_logs_only": true,
        "shared_private_logs_available": true,
        "reason_code": null,
        "narrowing_hint": null,
        "has_more": true
    }))
    .expect("catch-up page");
    assert!(logs.complete(&refresh, Ok(LogReply::Page(catchup))));
    assert_eq!(logs.unread(), 3);

    // Re-refreshing the exact same head page never double-counts.
    let repeat = logs.refresh().expect("repeat refresh");
    let same_head: LogPage = serde_json::from_value(json!({
        "rows": [row(30), row(29), row(10), row(0)],
        "counts": {"total": 22, "info": 22, "error": 0, "warn": 0, "debug": 0},
        "apps": ["tools"],
        "apps_complete": true,
        "next_cursor": cursor(5),
        "follow_cursor": cursor(6),
        "available": true,
        "private_logs_only": true,
        "shared_private_logs_available": true,
        "reason_code": null,
        "narrowing_hint": null,
        "has_more": true
    }))
    .expect("same head page");
    assert!(logs.complete(&repeat, Ok(LogReply::Page(same_head))));
    assert_eq!(logs.unread(), 3);

    // Reaching the newest row clears unread.
    assert_eq!(logs.jump_to_newest().as_deref(), Some("event-30"));
    assert_eq!(logs.unread(), 0);
}

#[test]
fn unread_resets_on_a_scope_or_backend_switch() {
    let mut logs = ready();
    logs.set_away(true);
    follow_with(&mut logs, 10, 2, 3);
    assert_eq!(logs.unread(), 2);
    logs.bind(Some(binding("b")), true);
    assert_eq!(logs.unread(), 0, "another scope starts with nothing unread");
    assert!(
        logs.set_away(true),
        "and the away state was reset with it, so scrolling away is a change again"
    );
    logs.bind(None, false);
    assert_eq!(logs.unread(), 0, "a backend switch keeps nothing");
}
