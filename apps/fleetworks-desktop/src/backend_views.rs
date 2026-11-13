//! Allowlisted backend operation cards and workstation-scoped activity.

use fleetworks_contracts::context::Permission;
use fleetworks_contracts::readiness::MaintenanceOperation;
use rune::El;
use rune::ScrollSpec;
use rune::TextEdit;
use rune_kit::Button;
use rune_kit::ButtonVariant;
use rune_kit::Cell;
use rune_kit::Dialog;
use rune_kit::Kit;
use rune_kit::Table;
use rune_kit::TableColumn;
use rune_kit::TextInput;

use crate::app::Load;
use crate::app::Page;
use crate::app::State;
use crate::submission::AcceptedReceipt;
use crate::submission::SubmissionKind;
use crate::submission::SubmissionPhase;
use crate::views::heading;
use crate::views::line;
use crate::views::quiet;
use crate::views::scroll;

fn card(body: El) -> El {
    crate::material::matte_card(body.padding(16.0).gap(8.0).w_full())
}

fn load_message<T>(load: &Load<T>, empty: &str, kit: &Kit) -> Option<El> {
    match load {
        Load::Idle => Some(quiet(empty.to_owned(), kit)),
        Load::Pending | Load::Loading => {
            Some(quiet("Loading from the selected gateway…", kit).role("status"))
        }
        Load::Failed(error) => Some(line(error.clone(), kit).role("alert")),
        Load::Ready(_) => None,
    }
}

pub(crate) fn page(state: &State, kit: &Kit, layers: &mut Vec<El>, log_width: f32) -> El {
    match state.page {
        Page::Seed => scroll(maintenance(state, kit), "seed-body"),
        Page::Atlas => atlas(state, kit),
        Page::Tools => tools(state, kit, layers),
        Page::Models => models(state, kit, layers),
        Page::Jobs => jobs(state, kit, log_width),
        _ => rune::col(),
    }
}

fn atlas(state: &State, kit: &Kit) -> El {
    let lane = state.submissions.lane(SubmissionKind::Atlas);
    let reason = state.submission_disabled_reason(SubmissionKind::Atlas);
    let unresolved = lane.attempt.is_some() && lane.receipt.is_none();
    let mut action = rune::col().child(heading("News Cycles", kit)).child(
        Button::new(if unresolved {
            "Resolve pending cycle"
        } else {
            "New Cycle"
        })
        .variant(ButtonVariant::Submit)
        .disabled(
            lane.busy()
                || if unresolved {
                    !state.permission(Permission::ReadWorkstationActivity)
                } else {
                    reason.is_some()
                },
        )
        .on_press(|state: &mut State| {
            if state
                .submissions
                .lane(SubmissionKind::Atlas)
                .attempt
                .is_some()
                && state
                    .submissions
                    .lane(SubmissionKind::Atlas)
                    .receipt
                    .is_none()
            {
                state.resolve_submission(SubmissionKind::Atlas);
            } else {
                state.preview_atlas_cycle();
            }
        })
        .build(kit)
        .key("atlas-submit-preview"),
    );
    if let Some(reason) = reason {
        action = action.child(quiet(reason, kit));
    }
    if let Some(snapshot) = &lane.confirmation {
        action = action.child(rune::col().gap(8.0).key("atlas-submit-confirmation")
            .child(heading("Confirm New Cycle", kit))
            .child(quiet(format!("Deployment {} · configuration revision {}", snapshot.context.deployment_id, snapshot.configuration_revision), kit))
            .child(quiet("Queue one collection cycle. Acceptance means queued; only the gateway can report completion.", kit))
            .child(Button::new("Confirm New Cycle").variant(ButtonVariant::Outline)
                .on_press(|state: &mut State| state.confirm_submission(SubmissionKind::Atlas)).build(kit).key("atlas-submit-confirm"))
            .child(Button::new("Keep current cycles").variant(ButtonVariant::Ghost)
                .on_press(|state: &mut State| state.submissions.lane_mut(SubmissionKind::Atlas).confirmation = None).build(kit)));
    }
    let status = match lane.phase {
        SubmissionPhase::Preparing => Some("Preparing the durable cycle request…"),
        SubmissionPhase::Posting => Some("Waiting for the gateway acceptance receipt…"),
        SubmissionPhase::OutcomeUnknown => Some("Delivery is uncertain. The original request is retained; resolving it does not submit a replacement cycle."),
        _ => None,
    };
    if let Some(status) = status {
        action = action.child(quiet(status, kit).role("status"));
    }
    if let Some(AcceptedReceipt::Atlas(receipt)) = &lane.receipt {
        action = action.child(
            line(
                format!(
                    "Accepted cycle {} · job {}. Completion pending.",
                    receipt.run_id, receipt.job_id
                ),
                kit,
            )
            .role("status"),
        );
    }
    if let Some(message) = load_message(&lane.resolution, "", kit) {
        if !matches!(lane.resolution, Load::Idle) {
            action = action.child(message);
        }
    }
    if let Some(message) = &lane.message {
        action = action.child(line(message.clone(), kit).role("status"));
    }
    let mut history = rune::col().child(heading("Cycle history", kit));
    if let Some(message) = load_message(
        &state.cycles.history,
        "Cycle history has not been observed.",
        kit,
    ) {
        history = history.child(message);
    }
    if let Load::Ready(cycles) = &state.cycles.history {
        if cycles.rows.is_empty() {
            history = history.child(quiet(
                "No cycles are associated with this workstation.",
                kit,
            ));
        }
        for cycle in &cycles.rows {
            let id = cycle.job_id.clone();
            let selected = state.cycles.selected.as_deref() == Some(cycle.job_id.as_str());
            let mut row = rune::col()
                .key(format!("cycle-{}", cycle.job_id))
                .child(
                    Button::new(format!("{} · {}", cycle.created_at, cycle.state))
                        .variant(if selected {
                            ButtonVariant::Outline
                        } else {
                            ButtonVariant::Ghost
                        })
                        .on_press(move |state: &mut State| state.select_cycle(&id))
                        .build(kit)
                        .key(format!("cycle-select-{}", cycle.job_id)),
                )
                .child(quiet(
                    format!(
                        "Run {} · articles {}",
                        cycle.run_id,
                        cycle
                            .article_count
                            .map_or("unavailable".into(), |count| count.to_string())
                    ),
                    kit,
                ));
            if let Some(finished) = &cycle.finished_at {
                row = row.child(quiet(format!("Finished {finished}"), kit));
            }
            if selected {
                row = row.child(
                    Button::new("Delete cycle history…")
                        .variant(ButtonVariant::Ghost)
                        .disabled(state.cycle_deletion_disabled_reason().is_some())
                        .on_press(|state: &mut State| state.preview_delete_cycle())
                        .build(kit)
                        .key("cycle-delete-preview"),
                );
                if let Some(reason) = state.cycle_deletion_disabled_reason() {
                    row = row.child(quiet(reason, kit));
                }
            }
            history = history.child(card(row));
        }
    }
    if let Some(id) = &state.cycles.confirmation {
        history = history.child(rune::col().key("cycle-delete-confirmation")
            .child(heading("Delete this cycle from workstation history?", kit))
            .child(quiet(format!("Job {id}. Shared articles, evidence and durable job audit remain intact. This does not cancel the cycle."), kit))
            .child(Button::new("Delete from history").variant(ButtonVariant::Outline)
                .disabled(matches!(state.cycles.deletion, Load::Loading))
                .on_press(|state: &mut State| state.confirm_delete_cycle()).build(kit).key("cycle-delete-confirm"))
            .child(Button::new("Keep cycle history").variant(ButtonVariant::Ghost)
                .on_press(|state: &mut State| state.cycles.confirmation = None).build(kit)));
    }
    if let Some(message) = load_message(&state.cycles.deletion, "", kit) {
        if !matches!(state.cycles.deletion, Load::Idle) {
            history = history.child(message);
        }
    }
    if matches!(state.cycles.deletion, Load::Failed(_)) {
        history = history.child(
            Button::new("Resolve pending deletion")
                .variant(ButtonVariant::Outline)
                .on_press(|state: &mut State| state.confirm_delete_cycle())
                .build(kit)
                .key("cycle-delete-resolve"),
        );
    }
    if let Load::Ready(receipt) = &state.cycles.deletion {
        history = history.child(quiet(
            if receipt.hidden {
                "Cycle hidden from this workstation's history."
            } else {
                "The gateway did not report the cycle as hidden."
            },
            kit,
        ));
    }
    if let Some(message) = &state.cycles.message {
        history = history.child(line(message.clone(), kit).role("status"));
    }
    scroll(
        rune::col().child(card(action)).child(card(history)),
        "atlas-body",
    )
}

fn tools(state: &State, kit: &Kit, layers: &mut Vec<El>) -> El {
    let mut body = rune::col().child(heading("OSINT Tools", kit));
    if let Some(message) = load_message(
        &state.backend.tools,
        "Tool availability has not been observed.",
        kit,
    ) {
        body = body.child(message);
    }
    if let Load::Ready(catalog) = &state.backend.tools {
        if catalog.tools.is_empty() {
            body = body.child(quiet("No services are advertised by this gateway.", kit));
        }
        for tool in &catalog.tools {
            body = body.child(card(
                rune::col()
                    .key(format!("tool-{}", tool.id))
                    .child(heading(tool.name.clone(), kit))
                    .child(quiet(tool.category.clone(), kit))
                    .child(line(tool.description.clone(), kit))
                    .child(quiet(tool.restrictions.clone(), kit)),
            ));
        }
    }
    if let Some(message) = load_message(
        &state.backend.profile,
        "Credential availability has not been observed.",
        kit,
    ) {
        body = body.child(message);
    }
    if let Load::Ready(profile) = &state.backend.profile {
        for credential in &profile.tool_credentials {
            let provider = credential.provider.clone();
            body = body.child(card(
                rune::col()
                    .key(format!("tool-provider-{}", credential.provider))
                    .child(heading(credential.provider.clone(), kit))
                    .child(quiet(
                        format!(
                            "Primary key {} · fallback key {}",
                            if credential.primary_present {
                                "configured"
                            } else {
                                "not configured"
                            },
                            if credential.fallback_present {
                                "configured"
                            } else {
                                "not configured"
                            }
                        ),
                        kit,
                    ))
                    .child(
                        Button::new("Configure API keys")
                            .variant(ButtonVariant::Outline)
                            .disabled(!state.permission(Permission::ManageConfiguration))
                            .on_press(move |state: &mut State| {
                                state.configuration.tool_provider = TextEdit::new(&provider);
                                state.configuration.tool_primary.replace_text("");
                                state.configuration.tool_fallback.replace_text("");
                                state.configuration.clear_tool_primary = false;
                                state.configuration.clear_tool_fallback = false;
                                state.configuration_edited();
                                state.tool_key_dialog = true;
                            })
                            .build(kit)
                            .key(format!("tool-key-open-{}", credential.provider)),
                    ),
            ));
        }
    }
    Dialog::new("tool-api-key", format!("{} API keys", state.configuration.tool_provider.text))
        .open(state.tool_key_dialog)
        .description("Keys remain masked and are sent only after confirmation. Blank fields preserve saved keys; clearing a slot requires its explicit Clear action.")
        .width(480.0).click_outside(false)
        .on_close(|state: &mut State| {
            state.tool_key_dialog = false;
            state.configuration.tool_primary.replace_text("");
            state.configuration.tool_fallback.replace_text("");
            state.configuration.clear_tool_primary = false;
            state.configuration.clear_tool_fallback = false;
            state.configuration_edited();
        })
        .child(rune::col().grow(1.0).min_h(0.0).scroll(ScrollSpec { stick_bottom: false }).padding(16.0).gap(10.0)
            .child(input("Primary API key", &state.configuration.tool_primary, |state| &mut state.configuration.tool_primary, true, "tool-key-primary", kit))
            .child(input("Fallback API key", &state.configuration.tool_fallback, |state| &mut state.configuration.tool_fallback, true, "tool-key-fallback", kit))
            .child(Button::new(if state.configuration.clear_tool_primary { "Keep saved primary key" } else { "Clear primary key…" })
                .variant(ButtonVariant::Ghost)
                .on_press(|state: &mut State| {
                    state.configuration.clear_tool_primary = !state.configuration.clear_tool_primary;
                    state.configuration.tool_primary.replace_text("");
                    state.configuration_edited();
                }).build(kit).key("tool-key-clear-primary"))
            .child(Button::new(if state.configuration.clear_tool_fallback { "Keep saved fallback key" } else { "Clear fallback key…" })
                .variant(ButtonVariant::Ghost)
                .on_press(|state: &mut State| {
                    state.configuration.clear_tool_fallback = !state.configuration.clear_tool_fallback;
                    state.configuration.tool_fallback.replace_text("");
                    state.configuration_edited();
                }).build(kit).key("tool-key-clear-fallback"))
            .child(Button::new("Review key replacement")
                .disabled(!state.permission(Permission::ManageConfiguration) || matches!(state.configuration.result, Load::Loading))
                .on_press(|state: &mut State| state.preview_tool_credentials()).build(kit).key("tool-credentials-preview"))
            .child(configuration_status(state, kit)))
        .build(kit, layers);
    scroll(body, "tools-body")
}

fn input(
    label: &str,
    edit: &TextEdit,
    getter: for<'a> fn(&'a mut State) -> &'a mut TextEdit,
    secure: bool,
    key: &str,
    kit: &Kit,
) -> El {
    let field = TextInput::new(edit)
        .label(label.to_owned())
        .width(None)
        .on_edit(getter)
        .on_change(|state: &mut State, _| state.configuration_edited());
    let field = if secure { field.secure() } else { field };
    rune::col()
        .gap(4.0)
        .child(quiet(label.to_owned(), kit))
        .child(field.build(kit).key(key.to_owned()))
}

fn models(state: &State, kit: &Kit, layers: &mut Vec<El>) -> El {
    let mut body = rune::col()
        .child(heading("Model Assignments", kit))
        .child(heading("Providers", kit));
    if let Some(message) = load_message(
        &state.backend.profile,
        "Provider metadata has not been observed.",
        kit,
    ) {
        body = body.child(message);
    }
    if let Load::Ready(profile) = &state.backend.profile {
        if profile.accounts.is_empty() {
            body = body.child(quiet(
                "No provider accounts are configured on this gateway.",
                kit,
            ));
        }
        for account in &profile.accounts {
            let id = account.id.clone();
            let editing = state.configuration.account_id.text == account.id;
            let mut row = rune::col()
                .key(format!("provider-account-{}", account.id))
                .child(heading(
                    format!("{} · {}", account.provider, account.id),
                    kit,
                ))
                .child(quiet(
                    if account.has_api_key {
                        "API key configured"
                    } else {
                        "API key unavailable"
                    },
                    kit,
                ));
            if editing {
                row = row
                    .child(input(
                        "API key",
                        &state.configuration.account_api_key,
                        |state| &mut state.configuration.account_api_key,
                        true,
                        "account-api-key",
                        kit,
                    ))
                    .child(
                        Button::new("Review key change")
                            .variant(ButtonVariant::Outline)
                            .disabled(
                                !state.permission(Permission::ManageConfiguration)
                                    || matches!(state.configuration.result, Load::Loading),
                            )
                            .on_press(|state: &mut State| state.preview_provider_account())
                            .build(kit)
                            .key("account-preview"),
                    )
                    .child(
                        Button::new("Clear key…")
                            .variant(ButtonVariant::Ghost)
                            .disabled(!state.permission(Permission::ManageConfiguration))
                            .on_press(|state: &mut State| {
                                state.configuration.account_api_key.replace_text("");
                                state.configuration.preserve_account_key = false;
                                state.configuration_edited();
                                state.preview_provider_account();
                            })
                            .build(kit)
                            .key("account-clear-key"),
                    );
            } else {
                row = row.child(
                    Button::new("Configure key")
                        .variant(ButtonVariant::Outline)
                        .disabled(!state.permission(Permission::ManageConfiguration))
                        .on_press(move |state: &mut State| state.edit_provider_account(&id))
                        .build(kit)
                        .key(format!("account-key-edit-{}", account.id)),
                );
            }
            body = body.child(card(row));
        }
    }
    body = body.child(heading("Role assignments", kit));
    if let Some(message) = load_message(
        &state.backend.models,
        "Role assignments have not been observed.",
        kit,
    ) {
        body = body.child(message);
    }
    if let Load::Ready(defaults) = &state.backend.models {
        body = body.child(quiet(
            format!("Configuration revision {}", defaults.configuration_revision),
            kit,
        ));
        for role in &defaults.roles {
            let edited = role.clone();
            let mut row = rune::col()
                .key(format!("model-role-{role}"))
                .child(heading(role.clone(), kit));
            if state.configuration.role.text == *role {
                row = row.child(crate::configuration_views::role_row_editor(
                    state, kit, layers,
                ));
            } else {
                if let Some(assignment) = defaults.assignment(role) {
                    row = row.child(line(assignment.primary.label(), kit));
                    for (index, fallback) in assignment.fallbacks.iter().enumerate() {
                        row = row.child(quiet(
                            format!("Fallback {}: {}", index + 1, fallback.label()),
                            kit,
                        ));
                    }
                } else {
                    row = row.child(quiet("Unconfigured", kit));
                }
                row = row.child(
                    Button::new("Edit assignment")
                        .variant(ButtonVariant::Ghost)
                        .disabled(!state.permission(Permission::ManageConfiguration))
                        .on_press(move |state: &mut State| state.edit_model_role(&edited))
                        .build(kit)
                        .key(format!("model-role-edit-{role}")),
                );
            }
            body = body.child(card(row));
        }
    }
    body = body.child(configuration_status(state, kit));
    scroll(body, "models-body")
}

fn configuration_status(state: &State, kit: &Kit) -> El {
    let mut body = rune::col().gap(8.0);
    if let Some(confirmation) = &state.configuration.confirmation {
        body = body
            .key("configuration-confirmation")
            .child(quiet(confirmation.change.summary(), kit))
            .child(
                Button::new("Confirm change")
                    .variant(ButtonVariant::Outline)
                    .on_press(|state: &mut State| state.confirm_configuration())
                    .build(kit)
                    .key("configuration-confirm"),
            )
            .child(
                Button::new("Keep current configuration")
                    .variant(ButtonVariant::Ghost)
                    .on_press(|state: &mut State| state.configuration.confirmation = None)
                    .build(kit)
                    .key("configuration-cancel"),
            );
    }
    if let Some(message) = load_message(&state.configuration.result, "", kit) {
        if !matches!(state.configuration.result, Load::Idle) {
            body = body.child(message);
        }
    }
    if let Load::Ready(saved) = &state.configuration.result {
        body = body.child(
            quiet(
                format!(
                    "Configuration saved at revision {}",
                    saved.configuration_revision
                ),
                kit,
            )
            .role("status"),
        );
    }
    if let Some(message) = &state.configuration.message {
        body = body.child(line(message.clone(), kit).role("status"));
    }
    body
}

fn jobs(state: &State, kit: &Kit, log_width: f32) -> El {
    let mut body = rune::col().child(heading("Jobs & Logs", kit));
    if state.registration.context.is_none() {
        return scroll(body.child(quiet("Workstation registration is required to read activity. No global jobs or logs are displayed.", kit)), "jobs-body");
    }
    if let Some(message) = load_message(
        &state.backend.jobs,
        "Workstation activity has not been observed.",
        kit,
    ) {
        body = body.child(message);
    }
    if let Load::Ready(jobs) = &state.backend.jobs {
        if jobs.rows.is_empty() {
            body = body.child(quiet("No jobs are associated with this workstation.", kit));
        }
        for job in jobs.rows.iter().take(100) {
            let id = job.id.clone();
            body = body.child(
                Button::new(format!(
                    "{} · {} · {}{}",
                    job.app,
                    if job.title.is_empty() {
                        &job.operation
                    } else {
                        &job.title
                    },
                    job.state,
                    if job.status_only {
                        " · status only"
                    } else {
                        ""
                    }
                ))
                .variant(
                    if state.backend.selected_job.as_deref() == Some(job.id.as_str()) {
                        ButtonVariant::Outline
                    } else {
                        ButtonVariant::Ghost
                    },
                )
                .on_press(move |state: &mut State| state.select_job(&id))
                .build(kit)
                .key(format!("activity-job-{}", job.id)),
            );
        }
    }
    if let Some(message) = load_message(
        &state.backend.detail,
        "Select a job to view its safe operational summaries.",
        kit,
    ) {
        body = body.child(message);
    }
    if let Load::Ready(detail) = &state.backend.detail {
        let mut details = rune::col()
            .child(heading(
                if detail.job.title.is_empty() {
                    detail.job.operation.clone()
                } else {
                    detail.job.title.clone()
                },
                kit,
            ))
            .child(quiet(
                format!(
                    "{} · {} · {}",
                    detail.job.id, detail.job.state, detail.job.phase
                ),
                kit,
            ));
        if let (Some(done), Some(total)) = (detail.job.progress_done, detail.job.progress_total) {
            details = details.child(quiet(format!("Progress {done} / {total}"), kit));
        }
        if !detail.private_details_available
            || !detail.job.private_details_available
            || detail.job.status_only
        {
            details = details.child(quiet(
                "Private details and logs are unavailable for this shared job.",
                kit,
            ));
        } else {
            if !detail.job.error_summary.is_empty() {
                details = details.child(line(detail.job.error_summary.clone(), kit));
            }
            if let Some(result) = &detail.maintenance_result {
                details = details.child(result.view(kit));
            }
            details = details.child(heading("Operational logs", kit));
            if let Some(message) = &state.scoped_logs.message {
                details = details.child(line(message.clone(), kit).role("status"));
            }
            if state.scoped_logs.busy() {
                details =
                    details.child(quiet("Reading safe operational summaries…", kit).role("status"));
            }
            if state.scoped_logs.rows.is_empty() && !state.scoped_logs.busy() {
                details = details.child(quiet(
                    "No safe operational summaries are available for this job.",
                    kit,
                ));
            }
            details = details.child(safe_log_table(state, kit, log_width));
        }
        body = body.child(card(details));
    }
    scroll(body, "jobs-body")
}

fn safe_log_table(state: &State, kit: &Kit, available_width: f32) -> El {
    let mut table = Table::new(
        |state: &State| &state.scoped_logs.table,
        |state: &mut State| &mut state.scoped_logs.table,
        |state: &State| state.scoped_logs.rows.len(),
        |state: &State, index| state.scoped_logs.rows[index].id.as_str().into(),
    )
    .name("Loaded safe operational log summaries");
    // The actual table budget includes page/card padding and sidebar resizing.
    // At very narrow widths keep the summary first; full timestamp/severity
    // remain in the read-only selected-row detail below the virtual viewport.
    if available_width >= 320.0 {
        let timestamp_width = if available_width < 500.0 {
            112.0
        } else {
            176.0
        };
        table = table.column(
            TableColumn::new("timestamp", "Timestamp")
                .width(timestamp_width)
                .min(timestamp_width)
                .resizable(false),
        );
    }
    if available_width >= 180.0 {
        table = table.column(
            TableColumn::new("severity", "Severity")
                .width(64.0)
                .min(64.0)
                .resizable(false),
        );
    }
    let table = table
        .column(
            TableColumn::new("message", "Safe summary")
                .flex(1.0)
                .min(0.0)
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
    let mut body = rune::col().gap(8.0).child(table);
    if let Some(selected) = state
        .scoped_logs
        .table
        .cursor()
        .and_then(|id| state.scoped_logs.rows.iter().find(|row| row.id == id))
    {
        body = body.child(
            rune::col()
                .gap(4.0)
                .key("logs-selected-detail")
                .child(heading("Selected safe log summary", kit))
                .child(quiet(
                    format!("{} · {} · {}", selected.ts, selected.severity, selected.app),
                    kit,
                ))
                .child(line(selected.message.clone(), kit).key("logs-selected-summary")),
        );
    }
    body
}

pub(crate) fn maintenance(state: &State, kit: &Kit) -> El {
    let mut body = rune::col().child(heading("DB Seed & Train", kit));
    for (label, operation, key, explanation) in [
        (
            "Seed Actors",
            MaintenanceOperation::SeedActors,
            "maintenance-seed",
            "Seed the configured actor catalog into the shared backend.",
        ),
        (
            "Build Search Index",
            MaintenanceOperation::ReindexActors,
            "maintenance-reindex",
            "Build the actor-search projection from the backend catalog.",
        ),
        (
            "Train Classifier — Natural",
            MaintenanceOperation::TrainClassifier,
            "maintenance-train",
            "Train a candidate from the configured Natural dataset. Training does not activate it.",
        ),
    ] {
        let reason = state.maintenance_disabled_reason(operation);
        let mut operation_card = rune::col()
            .child(heading(label, kit))
            .child(quiet(explanation, kit))
            .child(
                Button::new(label)
                    .variant(ButtonVariant::Outline)
                    .disabled(reason.is_some())
                    .on_press(move |state: &mut State| state.preview_maintenance(operation))
                    .build(kit)
                    .key(key),
            );
        if let Some(reason) = reason {
            operation_card = operation_card.child(quiet(reason, kit));
        }
        body = body.child(card(operation_card));
    }
    if let Some(confirmation) = &state.maintenance.confirmation {
        body = body.child(card(
            rune::col()
                .key("maintenance-confirmation")
                .child(heading(
                    format!("Confirm {:?}", confirmation.operation),
                    kit,
                ))
                .child(quiet(
                    format!(
                        "Deployment {} · configuration revision {}",
                        confirmation.context.deployment_id, confirmation.configuration_revision
                    ),
                    kit,
                ))
                .child(quiet(
                    confirmation.dataset.as_ref().map_or(
                        "No dataset is required for this operation.".into(),
                        |dataset| {
                            format!(
                                "Configured dataset {} · SHA-256 {}",
                                dataset.id, dataset.sha256
                            )
                        },
                    ),
                    kit,
                ))
                .child(
                    Button::new("Confirm operation")
                        .variant(ButtonVariant::Outline)
                        .on_press(|state: &mut State| state.confirm_maintenance())
                        .build(kit)
                        .key("maintenance-confirm"),
                )
                .child(
                    Button::new("Keep current backend state")
                        .variant(ButtonVariant::Ghost)
                        .on_press(|state: &mut State| state.maintenance.confirmation = None)
                        .build(kit),
                ),
        ));
    }
    if let Some(message) = load_message(&state.maintenance.receipt, "", kit) {
        if !matches!(state.maintenance.receipt, Load::Idle) {
            body = body.child(message);
        }
    }
    if let Load::Ready(receipt) = &state.maintenance.receipt {
        body = body.child(
            line(
                format!("Accepted job {} · completion pending", receipt.job_id),
                kit,
            )
            .role("status"),
        );
    }
    if let Some(message) = &state.maintenance.message {
        body = body.child(line(message.clone(), kit).role("status"));
    }
    body
}
