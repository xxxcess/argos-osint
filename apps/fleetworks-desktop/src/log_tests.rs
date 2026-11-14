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
    let crate::app::Intent::OperationalLogs { action } = intent else {
        return;
    };
    let ticket = state
        .scoped_logs
        .active_ticket()
        .expect("same-handler ticket")
        .clone();
    let result = if *action == LogAction::Export {
        export_reply(1, Some(cursor(2)))
    } else {
        LogReply::Page(page(0, 1, 10, Some(cursor(1)), Some(cursor(2))))
    };
    state.complete_scoped_logs(ticket, Ok(result));
}
#[test]
fn actual_shell_job_selection_automatically_reads_safe_logs_and_fences_hidden_replies() {
    let mut state = shell();
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
    state.navigate(crate::app::Page::Atlas);
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
fn loading_list_areas_render_shape_accurate_shimmering_skeletons_with_reduced_motion_fallback() {
    let mut state = shell();
    state.host_metrics = Load::Loading;
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
    assert!(runtime.find("jobs-loading").is_some());
    assert!(runtime.find("job-detail-loading").is_some());
    assert!(runtime.find("skeleton-shimmer").is_some());

    runtime.update(|state, _| state.navigate(crate::app::Page::Workstation));
    runtime.frame(1.0 / 60.0);
    assert!(runtime.find("host-metrics-loading").is_some());

    runtime.update(|state, _| {
        state.navigate(crate::app::Page::Brain);
        state.brain.results = Load::Loading;
        state.brain.showing_detail = true;
        state.brain.dossier = Load::Loading;
    });
    runtime.frame(2.0 / 60.0);
    assert!(runtime.find("search-loading").is_some());
    assert!(runtime.find("dossier-loading").is_some());

    runtime.update(|state, _| {
        state.navigate(crate::app::Page::Models);
        state.backend.models = Load::Loading;
        state.accessibility.fixture_override = Some(crate::accessibility::FixtureAccessibility {
            reduced_motion: true,
            ..crate::accessibility::FixtureAccessibility::default()
        });
    });
    runtime.frame(3.0 / 60.0);
    assert!(runtime.find("models-loading").is_some());
    assert!(
        runtime.find("skeleton-shimmer").is_none(),
        "reduced-motion mode disables shimmering skeleton sweep layers"
    );
}
