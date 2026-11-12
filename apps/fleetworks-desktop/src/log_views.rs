//! Shared native log panel. Integration supplies a state lens and effect
//! dispatcher; this view never reaches a backend or changes a durable job.

use rune::El;
use rune::Tx;
use rune_kit::Button;
use rune_kit::ButtonVariant;
use rune_kit::Cell;
use rune_kit::Checkbox;
use rune_kit::Kit;
use rune_kit::Table;
use rune_kit::TableColumn;
use rune_kit::TextInput;

use crate::app::State;
use crate::logs::LogState;
use crate::logs::LogStatus;
use crate::logs::LogTarget;
use crate::views::heading;
use crate::views::line;
use crate::views::quiet;

#[derive(Clone)]
pub enum LogEvent {
    Select(LogTarget),
    Refresh,
    Older,
    ToggleFollow,
    CopyLoaded,
    PrepareExport,
    ContinueExport,
    SaveExport { allow_partial: bool },
}

/// Use the same function for production and isolated Rune Runtime fixtures.
/// Visibility is supplied by the app/window lifecycle, never inferred here.
pub fn panel<S: 'static>(
    logs: &LogState,
    selected_job: Option<&str>,
    kit: &Kit,
    lens: for<'a> fn(&'a mut S) -> &'a mut LogState,
    dispatch: fn(&mut S, LogEvent, &mut Tx),
) -> El {
    let mut body = rune::col().gap(8.0).key("scoped-operational-logs")
        .child(heading("Workstation operational logs", kit))
        .child(quiet("Private summaries are authorized by this workstation's durable request and causal associations. Shared private, system and unassigned logs are excluded.", kit));
    let Some(origin) = logs.origin_label() else {
        body = body.child(quiet(
            "Connect and register a workspace with workstation activity permission to view logs.",
            kit,
        ));
        if let Some(message) = &logs.message {
            body = body.child(
                line(message.clone(), kit)
                    .role("alert")
                    .key("logs-authority-message"),
            );
        }
        return body;
    };
    body = body
        .child(quiet(format!("Current request origin: {origin}"), kit))
        .child(
            Button::new("Whole workstation")
                .variant(ButtonVariant::Ghost)
                .on_press_tx(move |state: &mut S, tx| {
                    dispatch(state, LogEvent::Select(LogTarget::Workstation), tx)
                })
                .build(kit)
                .key("logs-scope-workstation"),
        )
        .child(
            Button::new("Selected workspace")
                .variant(ButtonVariant::Ghost)
                .on_press_tx(move |state: &mut S, tx| {
                    dispatch(state, LogEvent::Select(LogTarget::SelectedWorkspace), tx)
                })
                .build(kit)
                .key("logs-scope-workspace"),
        );
    if let Some(job) = selected_job {
        for (descendants, label, key) in [
            (false, "Selected job", "logs-scope-job"),
            (
                true,
                "Selected job and descendants",
                "logs-scope-descendants",
            ),
        ] {
            let id = job.to_owned();
            body = body.child(
                Button::new(label)
                    .variant(ButtonVariant::Ghost)
                    .on_press_tx(move |state: &mut S, tx| {
                        dispatch(
                            state,
                            LogEvent::Select(LogTarget::Job {
                                id: id.clone(),
                                descendants,
                            }),
                            tx,
                        )
                    })
                    .build(kit)
                    .key(key),
            );
        }
    }
    let target = match &logs.target {
        LogTarget::Workstation if logs.draft.selected_workspace_only => {
            "Registered workstation, narrowed to the selected workspace".into()
        }
        LogTarget::Workstation => "Whole registered workstation".into(),
        LogTarget::SelectedWorkspace => "Selected registered workspace".into(),
        LogTarget::Job { id, descendants } => format!(
            "Job {id}{}",
            if *descendants {
                " and authorized descendants"
            } else {
                ""
            }
        ),
    };
    body = body
        .child(line(target, kit))
        .child(
            Checkbox::new(logs.draft.selected_workspace_only)
                .label("Narrow to the selected workspace")
                .disabled(logs.target == LogTarget::SelectedWorkspace)
                .on_toggle(move |state: &mut S| {
                    let logs = lens(state);
                    logs.draft.selected_workspace_only = !logs.draft.selected_workspace_only;
                    logs.draft_edited();
                })
                .build(kit)
                .key("logs-filter-workspace"),
        )
        .child(quiet("Summary search", kit).key("logs-label-summary"))
        .child(
            TextInput::new(&logs.draft.text)
                .label("Summary search")
                .width(None)
                .on_edit(move |state: &mut S| &mut lens(state).draft.text)
                .on_change(move |state: &mut S, _| lens(state).draft_edited())
                .build(kit)
                .key("logs-filter-text"),
        )
        .child(quiet("App (optional)", kit).key("logs-label-app"))
        .child(
            TextInput::new(&logs.draft.app)
                .label("App (optional)")
                .width(None)
                .on_edit(move |state: &mut S| &mut lens(state).draft.app)
                .on_change(move |state: &mut S, _| lens(state).draft_edited())
                .build(kit)
                .key("logs-filter-app"),
        )
        .child(quiet("Since (UTC/RFC3339, optional)", kit).key("logs-label-since"))
        .child(
            TextInput::new(&logs.draft.since)
                .label("Since (UTC/RFC3339, optional)")
                .width(None)
                .on_edit(move |state: &mut S| &mut lens(state).draft.since)
                .on_change(move |state: &mut S, _| lens(state).draft_edited())
                .build(kit)
                .key("logs-filter-since"),
        );
    for (rank, label, key) in [
        (None, "All severities", "logs-severity-all"),
        (Some(1), "Info and above", "logs-severity-info"),
        (Some(2), "Warnings and errors", "logs-severity-warn"),
        (Some(3), "Errors only", "logs-severity-error"),
    ] {
        body = body.child(
            Button::new(label)
                .variant(ButtonVariant::Ghost)
                .on_press(move |state: &mut S| {
                    let logs = lens(state);
                    logs.draft.min_rank = rank;
                    logs.draft_edited();
                })
                .build(kit)
                .key(key),
        );
    }
    body = body.child(
        Button::new("Apply filters / refresh logs")
            .disabled(logs.busy())
            .on_press_tx(move |state: &mut S, tx| dispatch(state, LogEvent::Refresh, tx))
            .build(kit)
            .key("logs-refresh"),
    );
    if logs.status == LogStatus::Loading {
        body =
            body.child(quiet("Checking current scope and operational logs…", kit).role("status"));
    }
    if let Some(message) = &logs.message {
        body = body.child(
            line(message.clone(), kit)
                .role(
                    if matches!(
                        logs.status,
                        LogStatus::Unavailable | LogStatus::RestartRequired
                    ) {
                        "alert"
                    } else {
                        "status"
                    },
                )
                .key("logs-status-message"),
        );
    }
    if let Some(counts) = &logs.counts {
        body=body.child(quiet(format!("{} loaded of {} eligible private summaries · {} errors · {} warnings · {} info · {} debug",logs.rows.len(),counts.total,counts.error,counts.warn,counts.info,counts.debug),kit).key("logs-scoped-counts"));
        let facet_label = if logs.apps_complete {
            "Apps"
        } else {
            "Apps (first 32; additional scoped apps exist)"
        };
        body = body.child(
            quiet(format!("{facet_label}: {}", logs.apps.join(", ")), kit).key("logs-app-facets"),
        );
        if logs.available() && !logs.shared_private_logs_available {
            body = body.child(
                quiet(
                    "Mixed-origin shared private logs are unavailable in this scope.",
                    kit,
                )
                .key("logs-shared-private-excluded"),
            );
        }
    }
    body=body.child(Button::new("Load older summaries").disabled(!logs.can_load_older())
        .on_press_tx(move |state:&mut S,tx|dispatch(state,LogEvent::Older,tx)).build(kit).key("logs-older"))
        .child(Button::new(if logs.following {"Pause visible log follow"} else {"Follow while visible"})
            .disabled(!logs.following && !logs.can_follow())
            .on_press_tx(move |state:&mut S,tx|dispatch(state,LogEvent::ToggleFollow,tx)).build(kit).key("logs-follow"))
        .child(quiet("Follow pauses when this panel is hidden or this window loses focus. Closing this client only stops its reads.",kit))
        .child(Button::new("Copy loaded safe summaries").disabled(!logs.can_copy_loaded())
            .on_press_tx(move |state:&mut S,tx|dispatch(state,LogEvent::CopyLoaded,tx)).build(kit).key("logs-copy-loaded"))
        .child(Button::new("Prepare paginated log export").disabled(!logs.can_export())
            .on_press_tx(move |state:&mut S,tx|dispatch(state,LogEvent::PrepareExport,tx)).build(kit).key("logs-export-prepare"));
    if logs.export.prepared() || logs.export.complete {
        body = body.child(
            quiet(
                format!(
                    "{} {} eligible private summaries prepared.{}",
                    if logs.export.complete {
                        "Complete snapshot:"
                    } else {
                        "Partial snapshot:"
                    },
                    logs.export.rows(),
                    if logs.export.apps_complete {
                        ""
                    } else {
                        " The app facet list is incomplete."
                    }
                ),
                kit,
            )
            .key("logs-export-status"),
        );
        if !logs.export.complete {
            body = body.child(
                Button::new("Continue export page")
                    .disabled(logs.busy() || !logs.export.has_more())
                    .on_press_tx(move |state: &mut S, tx| {
                        dispatch(state, LogEvent::ContinueExport, tx)
                    })
                    .build(kit)
                    .key("logs-export-continue"),
            );
        }
        body = body.child(
            Button::new(if logs.export.complete {
                "Save complete eligible snapshot…"
            } else {
                "Save explicitly partial snapshot…"
            })
            .disabled(logs.busy() || logs.export.rows() == 0)
            .on_press_tx(move |state: &mut S, tx| {
                let allow_partial = !lens(state).export.complete;
                dispatch(state, LogEvent::SaveExport { allow_partial }, tx);
            })
            .build(kit)
            .key("logs-export-save"),
        );
    }
    if logs.available() && logs.rows.is_empty() {
        body = body.child(
            quiet(
                "No operational summaries match this authorized scope and filters.",
                kit,
            )
            .key("logs-empty"),
        );
    }
    body
}

/// Only the finite table viewport materializes rows. A selected row's complete
/// safe message remains selectable below it; copy/export retains all loaded
/// safe rows independently of the virtualized paint window.
pub(crate) fn loaded_rows(state: &State, kit: &Kit) -> El {
    if state.scoped_logs.rows.is_empty() {
        return rune::el();
    }
    let table = Table::new(
        |state: &State| &state.scoped_logs.table,
        |state: &mut State| &mut state.scoped_logs.table,
        |state: &State| state.scoped_logs.rows.len(),
        |state: &State, index| state.scoped_logs.rows[index].id.as_str().into(),
    )
    .name("Loaded safe operational log summaries")
    .column(
        TableColumn::new("timestamp", "Timestamp")
            .width(176.0)
            .min(144.0)
            .resizable(false),
    )
    .column(
        TableColumn::new("severity", "Severity")
            .width(80.0)
            .min(64.0)
            .resizable(false),
    )
    .column(
        TableColumn::new("message", "Safe summary")
            .flex(1.0)
            .min(120.0)
            .resizable(false),
    )
    .cell(|state: &State, index, column| {
        let row = &state.scoped_logs.rows[index];
        match column {
            "timestamp" => Cell::text(row.ts.clone()).mono().muted(),
            "severity" => Cell::text(row.severity.clone()),
            _ => Cell::text(row.message.clone()),
        }
    })
    .build(state, kit)
    .w_full()
    .h(320.0)
    .min_h(320.0)
    .grow(0.0)
    .fill(kit.c.surface_base);
    let mut body=rune::col().gap(8.0).child(quiet("Select a row for its full safe summary; copy/export includes the loaded subset, not just visible rows.",kit)).child(table);
    if let Some(selected) = state
        .scoped_logs
        .table
        .cursor()
        .and_then(|id| state.scoped_logs.rows.iter().find(|row| row.id == id))
    {
        let id = selected.job_id.clone();
        body = body.child(
            rune::col()
                .gap(4.0)
                .key("logs-selected-detail")
                .child(heading("Selected safe log summary", kit))
                .child(quiet(
                    format!(
                        "{} · {} · {} · {}",
                        selected.ts, selected.severity, selected.app, selected.job_id
                    ),
                    kit,
                ))
                .child(line(selected.message.clone(), kit).key("logs-selected-summary"))
                .child(
                    Button::new("View attributed job")
                        .variant(ButtonVariant::Ghost)
                        .on_press(move |state: &mut State| state.open_job(&id))
                        .build(kit)
                        .key("logs-selected-view-job"),
                ),
        );
    }
    body
}
