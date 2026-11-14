//! Synthetic same-shell operational Logs states; no gateway, home or clipboard.

use fleetworks_contracts::operational_logs::LogPage;
use fleetworks_contracts::ClientError;
use fleetworks_desktop::app::Intent;
use fleetworks_desktop::logs::LogAction;
use fleetworks_desktop::logs::LogReply;
use fleetworks_desktop::State;
use serde_json::json;

fn row(n: usize) -> serde_json::Value {
    json!({"id":format!("synthetic-operational-{n}"),"ts":"2026-11-12T12:00:00Z","severity":"info","app":"tools","event_type":"phase","message":format!("Synthetic authorized operational summary {n}"),"job_id":"synthetic-tools-job","task_id":"","attempt_id":"","run_id":""})
}
fn cursor(n: usize) -> String {
    format!("{n:064x}")
}
fn page(start: usize, n: usize, next: Option<String>, follow: Option<String>) -> LogPage {
    let rows: Vec<_> = (start..start + n).map(row).collect();
    serde_json::from_value(json!({"rows":rows,"counts":{"total":1000,"info":1000,"warn":0,"error":0,"debug":0},"apps":["brain","tools"],"apps_complete":false,"next_cursor":next,"follow_cursor":follow,"available":true,"private_logs_only":true,"shared_private_logs_available":false,"reason_code":null,"narrowing_hint":null,"has_more":next.is_some()})).expect("bounded synthetic wire page")
}
fn setup(state: &mut State) {
    super::jobs_ready(state);
    state.sync_log_binding();
    state.scoped_logs.window_focused = true;
    state.reveal_operational_logs();
}
pub fn loaded(state: &mut State) {
    setup(state);
    let ticket = state
        .scoped_logs
        .refresh()
        .expect("fixture registered scope");
    state.scoped_logs.complete(
        &ticket,
        Ok(LogReply::Page(page(0, 3, Some(cursor(1)), Some(cursor(2))))),
    );
}
pub fn partial_export(state: &mut State) {
    loaded(state);
    let ticket = state
        .scoped_logs
        .prepare_export()
        .expect("separate synthetic export");
    state.scoped_logs.complete(
        &ticket,
        Ok(LogReply::Export {
            bytes: format!("{}\n", row(0)).into_bytes(),
            next_cursor: Some(cursor(3)),
            apps_complete: false,
        }),
    );
}
pub fn display_cap(state: &mut State) {
    setup(state);
    let first = state.scoped_logs.refresh().expect("synthetic scoped read");
    state.scoped_logs.complete(
        &first,
        Ok(LogReply::Page(page(
            0,
            100,
            Some(cursor(1)),
            Some(cursor(20)),
        ))),
    );
    for index in 1..=4 {
        let ticket = state.scoped_logs.older().expect("synthetic bounded page");
        state.scoped_logs.complete(
            &ticket,
            Ok(LogReply::Page(page(
                index * 100,
                if index == 4 { 50 } else { 100 },
                Some(cursor(index + 1)),
                None,
            ))),
        );
    }
    state.scoped_logs.set_following(true);
    let follow = state
        .scoped_logs
        .follow()
        .expect("visible synthetic follow");
    state.scoped_logs.complete(
        &follow,
        Ok(LogReply::Page(page(500, 100, Some(cursor(21)), None))),
    );
}
pub fn cursor_changed(state: &mut State) {
    loaded(state);
    let ticket = state.scoped_logs.older().expect("synthetic expired cursor");
    state
        .scoped_logs
        .complete(&ticket, Err(ClientError::Status(409)));
}
pub fn cohort_unavailable(state: &mut State) {
    setup(state);
    let ticket = state.scoped_logs.refresh().expect("synthetic scope budget");
    let unavailable=serde_json::from_value(json!({"rows":[],"counts":null,"apps":[],"apps_complete":false,"next_cursor":null,"follow_cursor":null,"available":false,"private_logs_only":true,"shared_private_logs_available":false,"reason_code":"scope_exceeds_complete_log_cohort_limit","narrowing_hint":"Select a workspace or an attributed job","has_more":false})).expect("explicit synthetic unavailable");
    state
        .scoped_logs
        .complete(&ticket, Ok(LogReply::Page(unavailable)));
}
pub fn reply(state: &mut State, intent: &Intent) -> bool {
    let Intent::OperationalLogs { action } = intent else {
        return false;
    };
    let Some(ticket) = state.scoped_logs.active_ticket().cloned() else {
        return true;
    };
    let response = match action {
        LogAction::Export => LogReply::Export {
            bytes: format!("{}\n", row(10)).into_bytes(),
            next_cursor: None,
            apps_complete: false,
        },
        LogAction::Follow => LogReply::Page(page(20, 1, Some(cursor(30)), None)),
        LogAction::Older => LogReply::Page(page(3, 2, None, None)),
        _ => LogReply::Page(page(0, 3, Some(cursor(1)), Some(cursor(2)))),
    };
    state.complete_scoped_logs(ticket, Ok(response));
    true
}
