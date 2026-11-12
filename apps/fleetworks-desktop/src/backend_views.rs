//! Simple endpoint-backed lists, setup forms and scoped activity.

use fleetworks_contracts::context::Permission;
use fleetworks_contracts::readiness::DatasetKind;
use fleetworks_contracts::readiness::MaintenanceOperation;
use rune::El;
use rune_kit::Button;
use rune_kit::ButtonVariant;
use rune_kit::Kit;
use rune_kit::TextArea;
use rune_kit::TextInput;

use crate::app::Load;
use crate::app::Page;
use crate::app::State;
use crate::maintenance::dataset_kind_label;
use crate::views::heading;
use crate::views::line;
use crate::views::quiet;
use crate::views::scroll;

fn refresh(state: &State, label: &str, kit: &Kit) -> El {
    Button::new(label.to_owned())
        .disabled(!state.connected())
        .on_press(|state: &mut State| state.refresh_page())
        .build(kit)
        .key("backend-refresh")
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

pub(crate) fn page(state: &State, kit: &Kit) -> El {
    match state.page {
        Page::Atlas => atlas(state, kit),
        Page::Tools => tools(state, kit),
        Page::Models => models(state, kit),
        Page::Profile => profile(state, kit),
        Page::Jobs => jobs(state, kit),
        _ => rune::col(),
    }
}

fn atlas(state: &State, kit: &Kit) -> El {
    let mut body = rune::col().child(heading("Collection and articles", kit))
        .child(quiet("Articles belong to the selected backend. Read a bounded calendar day and category; collection is a separate confirmed job.", kit))
        .child(TextInput::new(&state.backend.article_day).label("Article day (UTC)").placeholder("YYYY-MM-DD").width(None)
            .on_edit(|state: &mut State| &mut state.backend.article_day).build(kit).key("article-day"))
        .child(TextInput::new(&state.backend.article_category).label("Category").placeholder("all").width(None)
            .on_edit(|state: &mut State| &mut state.backend.article_category).build(kit).key("article-category"))
        .child(TextInput::new(&state.backend.article_search).label("Search these articles").width(None)
            .on_edit(|state: &mut State| &mut state.backend.article_search)
            .on_submit(|state: &mut State, _| state.refresh_page()).build(kit).key("article-search"))
        .child(refresh(state, "Refresh articles", kit));
    if let Some(message) = load_message(
        &state.backend.articles,
        "Choose a day and refresh to read articles.",
        kit,
    ) {
        body = body.child(message);
    }
    if let Load::Ready(page) = &state.backend.articles {
        body = body.child(quiet(
            format!("{} articles · revision {}", page.rows.len(), page.revision),
            kit,
        ));
        if page.rows.is_empty() {
            body = body.child(quiet(
                "No articles match this page of the selected day and category.",
                kit,
            ));
        }
        for article in &page.rows {
            body = body.child(
                rune::col()
                    .gap(5.0)
                    .py(8.0)
                    .key(format!("article-{}", article.id))
                    .child(heading(article.title.clone(), kit))
                    .child(line(article.description.clone(), kit))
                    .child(quiet(
                        format!(
                            "{} · {} · {}",
                            article.source_name,
                            article.published_at,
                            article.categories.join(", ")
                        ),
                        kit,
                    )),
            );
        }
        body = body.child(
            Button::new("Next article page")
                .disabled(page.next_cursor.is_none())
                .on_press(|state: &mut State| state.next_articles())
                .build(kit)
                .key("articles-next"),
        );
    }
    body = body.child(crate::submission_views::atlas(state, kit));
    scroll(body, "atlas-body")
}

fn tools(state: &State, kit: &Kit) -> El {
    let mut body = rune::col().child(heading("Tool catalog", kit))
        .child(quiet("Read catalog entries independently of provider credential administration. Execution needs an authorized registered origin and a durable acceptance contract.", kit))
        .child(refresh(state, "Refresh tool catalog", kit));
    if let Some(message) = load_message(
        &state.backend.tools,
        "Refresh to read the gateway's tool catalog.",
        kit,
    ) {
        body = body.child(message);
    }
    if let Load::Ready(catalog) = &state.backend.tools {
        for tool in &catalog.tools {
            let id = tool.id.clone();
            body = body.child(
                Button::new(format!("{} · {}", tool.name, tool.category))
                    .variant(
                        if state.backend.selected_tool.as_deref() == Some(&tool.id) {
                            ButtonVariant::Submit
                        } else {
                            ButtonVariant::Outline
                        },
                    )
                    .on_press(move |state: &mut State| {
                        state.backend.selected_tool = Some(id.clone());
                        state.backend.tool_inputs = rune::TextEdit::new("{}");
                        state.interactive_draft_edited(crate::submission::SubmissionKind::Tool);
                    })
                    .build(kit)
                    .key(format!("tool-{}", tool.id)),
            );
        }
        if let Some(tool) = state
            .backend
            .selected_tool
            .as_ref()
            .and_then(|id| catalog.tools.iter().find(|tool| &tool.id == id))
        {
            body = body
                .child(heading(tool.name.clone(), kit))
                .child(line(tool.description.clone(), kit))
                .child(quiet(
                    format!(
                        "Inputs: {} · timeout {}s",
                        tool.inputs.join(", "),
                        tool.timeout_seconds
                    ),
                    kit,
                ))
                .child(quiet(tool.restrictions.clone(), kit))
                .child(
                    TextArea::new(&state.backend.tool_inputs)
                        .label("Tool inputs (JSON)")
                        .rows(4)
                        .width(None)
                        .on_edit(|state: &mut State| &mut state.backend.tool_inputs)
                        .on_change(|state: &mut State, _| {
                            state.interactive_draft_edited(crate::submission::SubmissionKind::Tool)
                        })
                        .build(kit)
                        .key("tool-inputs"),
                );
        }
    }
    body = body.child(crate::submission_views::tools(state, kit));
    scroll(body, "tools-body")
}

fn models(state: &State, kit: &Kit) -> El {
    let mut body = rune::col().child(heading("Backend model roles", kit))
        .child(quiet("Primary and ordered fallback routes are backend-wide. A new workspace uses these same role defaults.", kit))
        .child(refresh(state, "Refresh model roles", kit));
    if let Some(message) = load_message(
        &state.backend.models,
        "Refresh to inspect the current canonical role assignments.",
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
            let edited_role = role.clone();
            let assignment = defaults.assignment(role);
            let mut row = rune::col()
                .gap(5.0)
                .py(8.0)
                .key(format!("model-role-{role}"))
                .child(heading(role.clone(), kit));
            if let Some(assignment) = assignment {
                row = row.child(line(
                    format!("Primary: {}", assignment.primary.label()),
                    kit,
                ));
                for (index, route) in assignment.fallbacks.iter().enumerate() {
                    row = row.child(quiet(
                        format!("Fallback {}: {}", index + 1, route.label()),
                        kit,
                    ));
                }
            } else {
                row = row.child(quiet(
                    "Unconfigured — assign an explicit primary route",
                    kit,
                ));
            }
            body = body.child(
                row.child(
                    Button::new("Edit role")
                        .variant(ButtonVariant::Ghost)
                        .disabled(!state.permission(Permission::ManageConfiguration))
                        .on_press_tx(move |state: &mut State, tx| {
                            state.edit_model_role(&edited_role);
                            tx.scroll_into_view("model-role-name");
                        })
                        .build(kit)
                        .key("model-role-edit"),
                ),
            );
        }
    }
    body = body.child(crate::configuration_views::model_editor(state, kit));
    scroll(body, "models-body")
}

fn profile(state: &State, kit: &Kit) -> El {
    let mut body = rune::col().child(heading("Backend profile and provider accounts", kit))
        .child(quiet("The backend owns credentials and role routing. This view reads credential presence and opaque references; secret values are never returned.", kit))
        .child(refresh(state, "Refresh backend profile", kit));
    if let Some(message) = load_message(
        &state.backend.profile,
        "Refresh to inspect this backend's current configuration format and account metadata.",
        kit,
    ) {
        body = body.child(message);
    }
    if let Load::Ready(profile) = &state.backend.profile {
        body = body.child(quiet(
            format!(
                "Configuration revision {} · format {} · portable schema {}",
                profile.configuration_revision,
                profile.configuration_format,
                profile.portable_schema_version
            ),
            kit,
        ));
        if profile.setup_required {
            body = body.child(line("Backend setup required: enter current provider accounts and explicit model roles.", kit).role("status"));
        }
        for account in &profile.accounts {
            let edit_id = account.id.clone();
            let delete_id = account.id.clone();
            body = body.child(
                rune::col()
                    .gap(5.0)
                    .py(8.0)
                    .key(format!("provider-account-{}", account.id))
                    .child(heading(
                        format!("{} / {}", account.provider, account.id),
                        kit,
                    ))
                    .child(line(
                        format!("{} · {}", account.base_url, account.model),
                        kit,
                    ))
                    .child(quiet(
                        if account.has_api_key {
                            "API key configured"
                        } else {
                            "API key not configured"
                        },
                        kit,
                    ))
                    .child(
                        rune::row()
                            .gap(8.0)
                            .child(
                                Button::new("Edit account")
                                    .variant(ButtonVariant::Ghost)
                                    .disabled(!state.permission(Permission::ManageConfiguration))
                                    .on_press_tx(move |state: &mut State, tx| {
                                        state.edit_provider_account(&edit_id);
                                        tx.scroll_into_view("account-id");
                                    })
                                    .build(kit)
                                    .key("account-edit"),
                            )
                            .child(
                                Button::new("Delete account…")
                                    .variant(ButtonVariant::Outline)
                                    .disabled(!state.permission(Permission::ManageConfiguration))
                                    .on_press_tx(move |state: &mut State, tx| {
                                        state.preview_delete_account(&delete_id);
                                        tx.scroll_into_view("configuration-confirmation");
                                    })
                                    .build(kit)
                                    .key("account-delete"),
                            ),
                    ),
            );
        }
        for credential in &profile.tool_credentials {
            let provider = credential.provider.clone();
            body = body.child(quiet(
                format!(
                    "{} tool credentials · primary {} · fallback {}",
                    credential.provider,
                    if credential.primary_present {
                        "configured"
                    } else {
                        "empty"
                    },
                    if credential.fallback_present {
                        "configured"
                    } else {
                        "empty"
                    }
                ),
                kit,
            ));
            body = body.child(
                Button::new("Edit these tool credential slots")
                    .variant(ButtonVariant::Ghost)
                    .disabled(!state.permission(Permission::ManageConfiguration))
                    .on_press(move |state: &mut State| {
                        state.configuration.tool_provider = rune::TextEdit::new(&provider);
                        state.configuration.tool_primary.replace_text("");
                        state.configuration.tool_fallback.replace_text("");
                        state.configuration_edited();
                    })
                    .build(kit)
                    .key(format!("tool-credential-edit-{}", credential.provider)),
            );
        }
    }
    body = body
        .child(crate::configuration_views::profile_editor(state, kit))
        .child(crate::configuration_views::profile_transfer(state, kit));
    scroll(body, "profile-body")
}

fn jobs(state: &State, kit: &Kit) -> El {
    let registered = state.registration.context.is_some();
    let mut body = rune::col().child(heading("Workstation jobs and selected-job logs", kit))
        .child(quiet("Only jobs attributed or associated with this registered workstation are requested. Shared jobs may expose status while private details and logs remain unavailable.", kit))
        .child(rune::row().gap(8.0)
            .child(Button::new("This workstation").variant(if !state.backend.selected_workspace_only { ButtonVariant::Submit } else { ButtonVariant::Outline }).disabled(!registered)
                .on_press(|state: &mut State| state.set_activity_workspace_filter(false)).build(kit).key("jobs-node-scope"))
            .child(Button::new("Selected workspace").variant(if state.backend.selected_workspace_only { ButtonVariant::Submit } else { ButtonVariant::Outline }).disabled(!registered)
                .on_press(|state: &mut State| state.set_activity_workspace_filter(true)).build(kit).key("jobs-workspace-scope")))
        .child(Button::new("Refresh activity").disabled(!registered || !state.permission(Permission::ReadWorkstationActivity))
            .on_press(|state: &mut State| state.refresh_page()).build(kit).key("jobs-refresh"));
    if let Some(context) = &state.registration.context {
        body = body.child(line(
            format!(
                "{} · {}",
                context.workstation_ip,
                if state.backend.selected_workspace_only {
                    context.workspace_name.as_str()
                } else {
                    "all attributed children on this node"
                }
            ),
            kit,
        ));
    } else {
        body = body.child(quiet(
            "Register an owned workspace to establish this workstation's activity boundary.",
            kit,
        ));
    }
    if let Some(message) = load_message(&state.backend.jobs, "Activity has not been loaded.", kit) {
        body = body.child(message);
    }
    if let Load::Ready(jobs) = &state.backend.jobs {
        if jobs.rows.is_empty() {
            body = body.child(quiet(
                "No attributed jobs match this workstation scope.",
                kit,
            ));
        }
        for job in &jobs.rows {
            let id = job.id.clone();
            body = body.child(
                Button::new(format!(
                    "{} · {} · {}{}",
                    job.app,
                    job.title,
                    job.state,
                    if job.status_only {
                        " · status only"
                    } else {
                        ""
                    }
                ))
                .variant(if state.backend.selected_job.as_deref() == Some(&job.id) {
                    ButtonVariant::Submit
                } else {
                    ButtonVariant::Outline
                })
                .on_press(move |state: &mut State| state.select_job(&id))
                .build(kit)
                .key(format!("activity-job-{}", job.id)),
            );
        }
    }
    if let Some(message) = load_message(
        &state.backend.detail,
        "Select an attributed job to inspect its current lifecycle.",
        kit,
    ) {
        body = body.child(message);
    }
    if let Load::Ready(detail) = &state.backend.detail {
        body = body
            .child(heading(detail.job.title.clone(), kit))
            .child(line(
                format!(
                    "{} · {} · {}",
                    detail.job.id, detail.job.state, detail.job.phase
                ),
                kit,
            ));
        if let (Some(done), Some(total)) = (detail.job.progress_done, detail.job.progress_total) {
            body = body.child(quiet(format!("Progress {done} / {total}"), kit));
        }
        if detail.private_details_available
            && detail.job.private_details_available
            && !detail.job.status_only
        {
            if !detail.job.error_summary.is_empty() {
                body = body.child(line(detail.job.error_summary.clone(), kit));
            }
            for stage in &detail.stages {
                body = body.child(quiet(format!("{} · {}", stage.operation, stage.state), kit));
            }
            if let Some(result) = &detail.maintenance_result {
                body = body.child(result.view(kit));
                if result.operation == "classifier_train"
                    && detail.job.operation == "classifier_train"
                    && matches!(detail.job.state.as_str(), "completed" | "succeeded")
                {
                    if let Some(candidate) = &result.candidate_model_id {
                        let candidate = candidate.clone();
                        let training = detail.job.id.clone();
                        body = body.child(
                            Button::new("Inspect this activation candidate")
                                .disabled(
                                    !state.permission(Permission::MaintainBackend)
                                        || !state.permission(Permission::ReadWorkstationActivity),
                                )
                                .on_press(move |state: &mut State| {
                                    state.inspect_training_candidate(&training, &candidate)
                                })
                                .build(kit)
                                .key("training-inspect-candidate"),
                        );
                    }
                }
            }
        } else {
            body = body.child(quiet(
                "Private details are unavailable for this shared job.",
                kit,
            ));
        }
        body = body
            .child(
                Button::new("Refresh selected-job logs")
                    .disabled(!detail.private_details_available || detail.job.status_only)
                    .on_press(|state: &mut State| state.refresh_job_logs())
                    .build(kit)
                    .key("job-logs-refresh"),
            )
            .child(crate::job_action_views::actions(state, kit));
    }
    if let Some(message) = load_message(
        &state.backend.logs,
        "Selected-job logs have not been loaded.",
        kit,
    ) {
        body = body.child(message);
    }
    if let Load::Ready(logs) = &state.backend.logs {
        if !logs.available {
            body = body.child(quiet(
                logs.reason
                    .clone()
                    .unwrap_or_else(|| "Private logs are unavailable".into()),
                kit,
            ));
        } else if logs.rows.is_empty() {
            body = body.child(quiet("No logs were returned for this authorized job.", kit));
        } else {
            for row in &logs.rows {
                body = body.child(line(
                    format!("{} · {} · {}", row.ts, row.severity, row.message),
                    kit,
                ));
            }
        }
    }
    scroll(body, "jobs-body")
}

pub(crate) fn maintenance(state: &State, kit: &Kit) -> El {
    let can_upload = state.permission(Permission::MaintainBackend);
    let mut body = rune::col().gap(10.0).child(heading("Backend datasets", kit))
        .child(quiet("Select CSV bytes on this Mac or an existing opaque backend dataset. The backend validates kind, schema, byte count and hash. Local paths are never sent as a dataset reference.", kit))
        .child(rune::row().gap(8.0)
            .child(Button::new("Choose actor catalog CSV").disabled(!can_upload).on_press_tx(|state: &mut State, tx| state.choose_dataset(DatasetKind::ActorCatalog, tx)).build(kit).key("dataset-choose-actors"))
            .child(Button::new("Choose Natural training CSV").disabled(!can_upload).on_press_tx(|state: &mut State, tx| state.choose_dataset(DatasetKind::ClassifierTraining, tx)).build(kit).key("dataset-choose-training")));
    if let Some(selection) = &state.maintenance.local_dataset {
        body = body
            .child(line(
                format!(
                    "Selected {}: {} · limit {} MiB",
                    dataset_kind_label(selection.kind),
                    selection
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy(),
                    crate::maintenance::dataset_limit(selection.kind) / 1024 / 1024
                ),
                kit,
            ))
            .child(
                Button::new("Upload selected CSV bytes")
                    .disabled(!can_upload || matches!(state.maintenance.upload, Load::Loading))
                    .on_press(|state: &mut State| state.upload_selected_dataset())
                    .build(kit)
                    .key("dataset-upload"),
            );
    }
    if let Some(message) = load_message(
        &state.maintenance.upload,
        "No local dataset upload has been performed.",
        kit,
    ) {
        body = body.child(message);
    }
    if let Load::Ready(upload) = &state.maintenance.upload {
        body = body.child(line(
            format!(
                "Validated backend dataset {} · {} bytes · expires in {} seconds",
                upload.dataset.id, upload.dataset.size_bytes, upload.expires_in_seconds
            ),
            kit,
        ));
    }
    body = body.child(
        Button::new("Refresh backend dataset inventory")
            .disabled(!state.permission(Permission::ReadBackend))
            .on_press(|state: &mut State| state.refresh_datasets())
            .build(kit)
            .key("datasets-refresh"),
    );
    if let Some(message) = load_message(
        &state.maintenance.datasets,
        "Dataset inventory has not been read.",
        kit,
    ) {
        body = body.child(message);
    }
    if let Load::Ready(list) = &state.maintenance.datasets {
        for dataset in &list.rows {
            let id = dataset.id.clone();
            body = body.child(
                Button::new(format!(
                    "{} · {} · {} bytes{}",
                    dataset.id,
                    dataset_kind_label(dataset.kind),
                    dataset.size_bytes,
                    if state.maintenance.selected_dataset.as_ref() == Some(dataset) {
                        " · selected"
                    } else {
                        ""
                    }
                ))
                .on_press(move |state: &mut State| state.select_backend_dataset(&id))
                .build(kit)
                .key(format!("dataset-{}", dataset.id)),
            );
        }
        if list.truncated {
            body = body.child(quiet(
                "Inventory was bounded by the backend; only the returned datasets are selectable.",
                kit,
            ));
        }
    }
    for (label, operation, key) in [
        (
            "Seed Actors",
            MaintenanceOperation::SeedActors,
            "maintenance-seed",
        ),
        (
            "Train Classifier — Natural (optional)",
            MaintenanceOperation::TrainClassifier,
            "maintenance-train",
        ),
        (
            "Rebuild Actor Index",
            MaintenanceOperation::ReindexActors,
            "maintenance-reindex",
        ),
    ] {
        let reason = state.maintenance_disabled_reason(operation);
        body = body.child(
            Button::new(label)
                .disabled(reason.is_some())
                .on_press_tx(move |state: &mut State, tx| {
                    state.preview_maintenance(operation);
                    tx.scroll_into_view("maintenance-confirmation");
                })
                .build(kit)
                .key(key),
        );
        if let Some(reason) = reason {
            body = body.child(quiet(reason, kit));
        }
    }
    if let Some(confirmation) = &state.maintenance.confirmation {
        body = body.child(rune::col().gap(8.0).key("maintenance-confirmation")
            .child(heading(format!("Confirm {:?}", confirmation.operation), kit))
            .child(line(format!("Origin {} · backend {} · configuration revision {}", confirmation.context.connect_id, confirmation.context.deployment_id, confirmation.configuration_revision), kit))
            .child(quiet(confirmation.dataset.as_ref().map_or("No uploaded dataset is used by this rebuild.".into(), |dataset| format!("Dataset {} · SHA-256 {} · {} bytes", dataset.id, dataset.sha256, dataset.size_bytes)), kit))
            .child(quiet("This changes a shared backend resource. Acceptance returns a durable job receipt; it does not mean completion. Natural training never activates a candidate automatically.", kit))
            .child(Button::new("Confirm backend operation").on_press(|state: &mut State| state.confirm_maintenance()).build(kit).key("maintenance-confirm"))
            .child(Button::new("Cancel preview").variant(ButtonVariant::Ghost).on_press(|state: &mut State| state.maintenance.confirmation = None).build(kit)));
    }
    if let Some(message) = load_message(
        &state.maintenance.receipt,
        "No maintenance job has been accepted in this context.",
        kit,
    ) {
        body = body.child(message);
    }
    if let Load::Ready(receipt) = &state.maintenance.receipt {
        let id = receipt.job_id.clone();
        body = body
            .child(line(
                format!(
                    "{} job {} · accepted, completion pending",
                    if receipt.reused {
                        "Associated with existing"
                    } else {
                        "Created"
                    },
                    receipt.job_id
                ),
                kit,
            ))
            .child(
                Button::new("View this workstation job")
                    .disabled(state.registration.context.is_none())
                    .on_press(move |state: &mut State| state.open_job(&id))
                    .build(kit)
                    .key("maintenance-view-job"),
            );
    }
    body = body.child(crate::classifier_activation_views::panel(state, kit));
    if let Some(message) = &state.maintenance.message {
        body = body.child(line(message.clone(), kit).role("status"));
    }
    body
}
