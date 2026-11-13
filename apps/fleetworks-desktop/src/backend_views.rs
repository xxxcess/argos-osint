//! Allowlisted backend operation cards and workstation-scoped activity.

use fleetworks_contracts::context::Permission;
use fleetworks_contracts::readiness::DatasetKind;
use fleetworks_contracts::readiness::MaintenanceOperation;
use fleetworks_contracts::readiness::ReadinessCheckId;
use fleetworks_contracts::readiness::ReadinessState;
use rune::Cx;
use rune::El;
use rune::ScrollSpec;
use rune::TextEdit;
use rune_kit::Cell;
use rune_kit::Dialog;
use rune_kit::Kit;
use rune_kit::Progress;
use rune_kit::ProgressCircle;
use rune_kit::RollingText;
use rune_kit::SegmentedControl;
use rune_kit::Table;
use rune_kit::TableColumn;
use rune_kit::TextInput;

use crate::accessibility::ViewPreferences;
use crate::app::Load;
use crate::app::ModelsTab;
use crate::app::Page;
use crate::app::SeedStage;
use crate::app::State;
use crate::icons;
use crate::material;
use crate::material::MetalFinish;
use crate::submission::AcceptedReceipt;
use crate::submission::SubmissionKind;
use crate::submission::SubmissionPhase;
use crate::theme;
use crate::theme::StatusTone;
use crate::ui_components;
use crate::ui_components::ActionButton;
use crate::views::heading;
use crate::views::line;
use crate::views::quiet;
use crate::views::scroll;

fn card(state: &State, preferences: ViewPreferences, tone: StatusTone, body: El) -> El {
    let pointer_ref = (!preferences.matte()).then_some(&state.pointer_offset);
    let shadow_ref = (!preferences.matte()).then_some(&state.shadow_offset);
    material::luminous_card(
        body.padding(16.0).gap(10.0).w_full(),
        tone,
        false,
        preferences,
        pointer_ref,
        shadow_ref,
    )
}

fn load_message<T>(
    load: &Load<T>,
    empty: &str,
    preferences: ViewPreferences,
    kit: &Kit,
) -> Option<El> {
    match load {
        Load::Idle => Some(quiet(empty.to_owned(), kit)),
        Load::Pending | Load::Loading => Some(
            ui_components::async_status_strip(
                "backend-loading",
                "Syncing with gateway",
                "Loading from the selected gateway…",
                true,
                StatusTone::Accent,
                preferences,
                kit,
            )
            .role("status"),
        ),
        Load::Failed(error) => Some(line(error.clone(), kit).role("alert")),
        Load::Ready(_) => None,
    }
}

fn state_tone(value: &str) -> StatusTone {
    let lower = value.to_ascii_lowercase();
    if lower.contains("succeed") || lower.contains("ready") || lower.contains("done") {
        StatusTone::Ok
    } else if lower.contains("run") || lower.contains("pend") || lower.contains("queue") {
        StatusTone::Info
    } else if lower.contains("fail") || lower.contains("err") {
        StatusTone::Err
    } else {
        StatusTone::Neutral
    }
}

pub(crate) fn page(
    state: &State,
    cx: &Cx<State>,
    kit: &Kit,
    layers: &mut Vec<El>,
    log_width: f32,
) -> El {
    let preferences = state
        .accessibility
        .for_view(state.fixture, cx.is_reduced_motion());
    match state.page {
        Page::Seed => scroll(maintenance(state, preferences, kit, layers), "seed-body"),
        Page::Atlas => atlas(state, preferences, kit, layers),
        Page::Tools => tools(state, preferences, kit, layers),
        Page::Models => models(state, preferences, kit, layers),
        Page::Jobs => jobs(state, preferences, kit, layers, log_width),
        _ => rune::col(),
    }
}

fn atlas(state: &State, preferences: ViewPreferences, kit: &Kit, layers: &mut Vec<El>) -> El {
    let pointer_ref = (!preferences.matte()).then_some(&state.pointer_offset);
    let shadow_ref = (!preferences.matte()).then_some(&state.shadow_offset);
    let lane = state.submissions.lane(SubmissionKind::Atlas);
    let reason = state.submission_disabled_reason(SubmissionKind::Atlas);
    let unresolved = lane.attempt.is_some() && lane.receipt.is_none();

    let count_badge = match &state.cycles.history {
        Load::Ready(cycles) => Some(ui_components::status_pill(
            format!("{} cycles", cycles.rows.len()),
            None,
            StatusTone::Info,
            kit,
        )),
        _ => None,
    };

    let trigger = ActionButton::new(if unresolved {
        "Resolve pending cycle"
    } else {
        "New Cycle"
    })
    .key("atlas-submit-preview")
    .icon(if unresolved {
        &icons::REFRESH
    } else {
        &icons::PLUS
    })
    .finish(MetalFinish::GoldCta)
    .busy(lane.busy())
    .disabled(if unresolved {
        !state.permission(Permission::ReadWorkstationActivity)
    } else {
        reason.is_some()
    })
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
    .build(kit, preferences, pointer_ref);

    let mut action = rune::col().child(ui_components::section_header(
        &icons::ATLAS,
        "News Cycles",
        Some("Queue incremental news collection or inspect prior runs"),
        count_badge,
        Some(trigger),
        kit,
    ));

    if let Some(reason) = reason {
        action = action.child(quiet(reason, kit));
    }
    if let Some(snapshot) = &lane.confirmation {
        action = action.child(
            rune::col()
                .gap(8.0)
                .padding(12.0)
                .radius(8.0)
                .fill(theme::hex(theme::SURFACE_RAISED))
                .border(1.0, theme::hex(theme::ACCENT_DEEP))
                .key("atlas-submit-confirmation")
                .child(heading("Confirm New Cycle", kit))
                .child(quiet(
                    format!(
                        "Deployment {} · configuration revision {}",
                        snapshot.context.deployment_id, snapshot.configuration_revision
                    ),
                    kit,
                ))
                .child(quiet(
                    "Queue one collection cycle. Acceptance means queued; only the gateway can report completion.",
                    kit,
                ))
                .child(
                    rune::row()
                        .gap(8.0)
                        .child(
                            ActionButton::new("Confirm New Cycle")
                                .key("atlas-submit-confirm")
                                .icon(&icons::ATLAS)
                                .finish(MetalFinish::GoldCta)
                                .on_press(|state: &mut State| {
                                    state.confirm_submission(SubmissionKind::Atlas)
                                })
                                .build(kit, preferences, pointer_ref),
                        )
                        .child(
                            ActionButton::new("Keep current cycles")
                                .finish(MetalFinish::Gunmetal)
                                .on_press(|state: &mut State| {
                                    state.submissions.lane_mut(SubmissionKind::Atlas).confirmation =
                                        None
                                })
                                .build(kit, preferences, pointer_ref),
                        ),
                ),
        );
    }
    let status = match lane.phase {
        SubmissionPhase::Preparing => Some("Preparing the durable cycle request…"),
        SubmissionPhase::Posting => Some("Waiting for the gateway acceptance receipt…"),
        SubmissionPhase::OutcomeUnknown => Some(
            "Delivery is uncertain. The original request is retained; resolving it does not submit a replacement cycle.",
        ),
        _ => None,
    };
    if let Some(status) = status {
        action = action.child(ui_components::async_status_strip(
            "atlas-phase-status",
            "Submitting cycle",
            status,
            true,
            StatusTone::Accent,
            preferences,
            kit,
        ));
    }
    if let Some(AcceptedReceipt::Atlas(receipt)) = &lane.receipt {
        action = action.child(
            ui_components::async_status_strip(
                "atlas-receipt",
                format!("Accepted cycle {}", receipt.run_id),
                format!("Job {} · Completion pending", receipt.job_id),
                false,
                StatusTone::Ok,
                preferences,
                kit,
            )
            .role("status"),
        );
    }
    if !matches!(lane.resolution, Load::Idle) {
        if let Some(message) = load_message(&lane.resolution, "", preferences, kit) {
            action = action.child(message);
        }
    }
    if let Some(message) = &lane.message {
        action = action.child(line(message.clone(), kit).role("status"));
    }

    let mut history = rune::col().child(ui_components::section_header(
        &icons::JOBS,
        "Cycle history",
        Some("Select any cycle to expand run metadata or history removal"),
        None,
        None,
        kit,
    ));
    if let Some(message) = load_message(
        &state.cycles.history,
        "Cycle history has not been observed.",
        preferences,
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
            let modal_id = cycle.job_id.clone();
            let selected = state.cycles.selected.as_deref() == Some(cycle.job_id.as_str());
            let tone = state_tone(&cycle.state);

            let mut row_content = rune::col().padding(10.0).gap(6.0).child(
                rune::row()
                    .w_full()
                    .center_y()
                    .gap(8.0)
                    .child(ui_components::status_dot(tone, preferences))
                    .child(
                        ActionButton::new(format!("{} · {}", cycle.created_at, cycle.state))
                            .key(format!("cycle-select-{}", cycle.job_id))
                            .finish(if selected {
                                MetalFinish::GoldCta
                            } else {
                                MetalFinish::ObsidianPill
                            })
                            .on_press(move |state: &mut State| state.select_cycle(&id))
                            .build(kit, preferences, pointer_ref),
                    )
                    .child(rune::spacer())
                    .child(ui_components::status_pill(
                        cycle.state.clone(),
                        None,
                        tone,
                        kit,
                    )),
            );
            if selected {
                row_content = row_content
                    .child(quiet(
                        format!(
                            "Run {} · articles {}",
                            cycle.run_id,
                            cycle
                                .article_count
                                .map_or("unavailable".into(), |count| count.to_string())
                        ),
                        kit,
                    ))
                    .children(
                        cycle
                            .finished_at
                            .as_ref()
                            .map(|finished| quiet(format!("Finished {finished}"), kit)),
                    )
                    .child(
                        rune::row()
                            .gap(8.0)
                            .child(
                                ActionButton::new("Inspect summary")
                                    .finish(MetalFinish::Gunmetal)
                                    .on_press(move |state: &mut State| {
                                        state.selected_cycle_id = Some(modal_id.clone());
                                    })
                                    .build(kit, preferences, pointer_ref),
                            )
                            .child(
                                ActionButton::new("Delete cycle history…")
                                    .key("cycle-delete-preview")
                                    .icon(&icons::TRASH)
                                    .finish(MetalFinish::CrimsonDanger)
                                    .disabled(state.cycle_deletion_disabled_reason().is_some())
                                    .on_press(|state: &mut State| state.preview_delete_cycle())
                                    .build(kit, preferences, pointer_ref),
                            ),
                    );
                if let Some(reason) = state.cycle_deletion_disabled_reason() {
                    row_content = row_content.child(quiet(reason, kit));
                }
            }
            history = history.child(
                material::interactive_tile(
                    row_content,
                    selected,
                    tone,
                    preferences,
                    pointer_ref,
                    shadow_ref,
                )
                .key(format!("cycle-{}", cycle.job_id)),
            );
        }

        if let Some(active_id) = &state.selected_cycle_id {
            if let Some(cycle) = cycles.rows.iter().find(|c| c.job_id == *active_id) {
                let detail = rune::col()
                    .gap(8.0)
                    .child(line(format!("Run ID: {}", cycle.run_id), kit))
                    .child(line(format!("Job ID: {}", cycle.job_id), kit))
                    .child(quiet(format!("Created: {}", cycle.created_at), kit))
                    .children(
                        cycle
                            .finished_at
                            .as_ref()
                            .map(|finished| quiet(format!("Finished: {finished}"), kit)),
                    )
                    .child(quiet(
                        format!(
                            "Articles collected: {}",
                            cycle
                                .article_count
                                .map_or("unavailable".into(), |count| count.to_string())
                        ),
                        kit,
                    ));
                ui_components::modal_prompt(
                    "cycle-inspector",
                    format!("Cycle {}", cycle.run_id),
                    format!("State: {}", cycle.state),
                    true,
                    460.0,
                    |state: &mut State| state.selected_cycle_id = None,
                    detail,
                    Some(
                        ActionButton::new("Close")
                            .finish(MetalFinish::GoldCta)
                            .on_press(|state: &mut State| state.selected_cycle_id = None)
                            .build(kit, preferences, pointer_ref),
                    ),
                    kit,
                    layers,
                );
            }
        }
    }
    if let Some(id) = &state.cycles.confirmation {
        history = history.child(
            rune::col()
                .padding(12.0)
                .gap(8.0)
                .radius(8.0)
                .fill(theme::hex(theme::STATUS_ERR_BG))
                .border(1.0, theme::hex(theme::STATUS_ERR_BORDER))
                .key("cycle-delete-confirmation")
                .child(heading("Delete this cycle from workstation history?", kit))
                .child(quiet(
                    format!(
                        "Job {id}. Shared articles, evidence and durable job audit remain intact. This does not cancel the cycle."
                    ),
                    kit,
                ))
                .child(
                    rune::row()
                        .gap(8.0)
                        .child(
                            ActionButton::new("Delete from history")
                                .key("cycle-delete-confirm")
                                .icon(&icons::TRASH)
                                .finish(MetalFinish::CrimsonDanger)
                                .busy(matches!(state.cycles.deletion, Load::Loading))
                                .on_press(|state: &mut State| state.confirm_delete_cycle())
                                .build(kit, preferences, pointer_ref),
                        )
                        .child(
                            ActionButton::new("Keep cycle history")
                                .finish(MetalFinish::Gunmetal)
                                .on_press(|state: &mut State| state.cycles.confirmation = None)
                                .build(kit, preferences, pointer_ref),
                        ),
                ),
        );
    }
    if !matches!(state.cycles.deletion, Load::Idle) {
        if let Some(message) = load_message(&state.cycles.deletion, "", preferences, kit) {
            history = history.child(message);
        }
    }
    if matches!(state.cycles.deletion, Load::Failed(_)) {
        history = history.child(
            ActionButton::new("Resolve pending deletion")
                .key("cycle-delete-resolve")
                .finish(MetalFinish::Gunmetal)
                .on_press(|state: &mut State| state.confirm_delete_cycle())
                .build(kit, preferences, pointer_ref),
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
        rune::col()
            .gap(14.0)
            .child(card(state, preferences, StatusTone::Accent, action))
            .child(card(state, preferences, StatusTone::Info, history)),
        "atlas-body",
    )
}

fn tools(state: &State, preferences: ViewPreferences, kit: &Kit, layers: &mut Vec<El>) -> El {
    let pointer_ref = (!preferences.matte()).then_some(&state.pointer_offset);
    let shadow_ref = (!preferences.matte()).then_some(&state.shadow_offset);
    let mut body = rune::col().gap(12.0).child(ui_components::section_header(
        &icons::TOOLS,
        "OSINT Tools",
        Some("Inspect gateway-advertised services & configure masked provider keys"),
        None,
        None,
        kit,
    ));
    if let Some(message) = load_message(
        &state.backend.tools,
        "Tool availability has not been observed.",
        preferences,
        kit,
    ) {
        body = body.child(message);
    }
    if let Load::Ready(catalog) = &state.backend.tools {
        if catalog.tools.is_empty() {
            body = body.child(quiet("No services are advertised by this gateway.", kit));
        }
        for tool in &catalog.tools {
            body = body.child(material::luminous_card(
                rune::col()
                    .padding(14.0)
                    .gap(6.0)
                    .key(format!("tool-{}", tool.id))
                    .child(
                        rune::row()
                            .w_full()
                            .center_y()
                            .gap(8.0)
                            .child(
                                rune::icon(&icons::TOOLS, 15.0)
                                    .color(theme::hex(theme::ENTITY_TECH))
                                    .a11y_hidden(),
                            )
                            .child(heading(tool.name.clone(), kit))
                            .child(rune::spacer())
                            .child(ui_components::status_pill(
                                tool.category.clone(),
                                None,
                                StatusTone::Cyan,
                                kit,
                            )),
                    )
                    .child(line(tool.description.clone(), kit))
                    .child(quiet(tool.restrictions.clone(), kit)),
                StatusTone::Cyan,
                false,
                preferences,
                pointer_ref,
                shadow_ref,
            ));
        }
    }
    if let Some(message) = load_message(
        &state.backend.profile,
        "Credential availability has not been observed.",
        preferences,
        kit,
    ) {
        body = body.child(message);
    }
    if let Load::Ready(profile) = &state.backend.profile {
        for credential in &profile.tool_credentials {
            let provider = credential.provider.clone();
            let configured = credential.primary_present || credential.fallback_present;
            let tone = if configured {
                StatusTone::Ok
            } else {
                StatusTone::Warn
            };
            body = body.child(material::interactive_tile(
                rune::row()
                    .padding(14.0)
                    .gap(10.0)
                    .center_y()
                    .key(format!("tool-provider-{}", credential.provider))
                    .child(ui_components::status_dot(tone, preferences))
                    .child(
                        rune::col()
                            .grow(1.0)
                            .gap(4.0)
                            .child(heading(credential.provider.clone(), kit))
                            .child(
                                rune::row()
                                    .gap(6.0)
                                    .child(ui_components::status_pill(
                                        if credential.primary_present {
                                            "Primary configured"
                                        } else {
                                            "Primary empty"
                                        },
                                        Some(&icons::KEY),
                                        if credential.primary_present {
                                            StatusTone::Ok
                                        } else {
                                            StatusTone::Neutral
                                        },
                                        kit,
                                    ))
                                    .child(ui_components::status_pill(
                                        if credential.fallback_present {
                                            "Fallback configured"
                                        } else {
                                            "Fallback empty"
                                        },
                                        None,
                                        if credential.fallback_present {
                                            StatusTone::Info
                                        } else {
                                            StatusTone::Neutral
                                        },
                                        kit,
                                    )),
                            ),
                    )
                    .child(
                        ActionButton::new("Configure API keys")
                            .key(format!("tool-key-open-{}", credential.provider))
                            .icon(&icons::KEY)
                            .finish(MetalFinish::Gunmetal)
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
                            .build(kit, preferences, pointer_ref),
                    ),
                false,
                tone,
                preferences,
                pointer_ref,
                shadow_ref,
            ));
        }
    }
    Dialog::new(
        "tool-api-key",
        format!("{} API keys", state.configuration.tool_provider.text),
    )
    .open(state.tool_key_dialog)
    .width(480.0)
    .click_outside(false)
    .on_close(|state: &mut State| {
        state.tool_key_dialog = false;
        state.configuration.tool_primary.replace_text("");
        state.configuration.tool_fallback.replace_text("");
        state.configuration.clear_tool_primary = false;
        state.configuration.clear_tool_fallback = false;
        state.configuration_edited();
    })
    .child(
        rune::col()
            .grow(1.0)
            .min_h(0.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .padding(16.0)
            .gap(12.0)
            .child(quiet(
                "Keys remain masked and are sent only after confirmation. Blank fields preserve saved keys; clearing a slot requires its explicit toggle.",
                kit,
            ))
            .child(input(
                "Primary API key",
                &state.configuration.tool_primary,
                |state| &mut state.configuration.tool_primary,
                true,
                "tool-key-primary",
                kit,
            ))
            .child(input(
                "Fallback API key",
                &state.configuration.tool_fallback,
                |state| &mut state.configuration.tool_fallback,
                true,
                "tool-key-fallback",
                kit,
            ))
            .child(ui_components::inline_toggle_row(
                "tool-key-clear-primary",
                state.configuration.clear_tool_primary,
                "Clear primary key…",
                Some("When enabled, confirming removes the saved primary slot"),
                false,
                kit,
                |state: &mut State| {
                    state.configuration.clear_tool_primary =
                        !state.configuration.clear_tool_primary;
                    state.configuration.tool_primary.replace_text("");
                    state.configuration_edited();
                },
            ))
            .child(ui_components::inline_toggle_row(
                "tool-key-clear-fallback",
                state.configuration.clear_tool_fallback,
                "Clear fallback key…",
                Some("When enabled, confirming removes the saved fallback slot"),
                false,
                kit,
                |state: &mut State| {
                    state.configuration.clear_tool_fallback =
                        !state.configuration.clear_tool_fallback;
                    state.configuration.tool_fallback.replace_text("");
                    state.configuration_edited();
                },
            ))
            .child(
                ActionButton::new("Review key replacement")
                    .key("tool-credentials-preview")
                    .icon(&icons::SHIELD_CHECK)
                    .finish(MetalFinish::GoldCta)
                    .busy(matches!(state.configuration.result, Load::Loading))
                    .disabled(!state.permission(Permission::ManageConfiguration))
                    .on_press(|state: &mut State| state.preview_tool_credentials())
                    .build(kit, preferences, pointer_ref),
            )
            .child(configuration_status(state, preferences, kit)),
    )
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

fn models(state: &State, preferences: ViewPreferences, kit: &Kit, layers: &mut Vec<El>) -> El {
    let pointer_ref = (!preferences.matte()).then_some(&state.pointer_offset);
    let shadow_ref = (!preferences.matte()).then_some(&state.shadow_offset);
    let active_tab = if !state.configuration.account_id.text.is_empty() {
        ModelsTab::Providers
    } else {
        state.models_tab
    };
    let tab_idx = match active_tab {
        ModelsTab::Roles => 0,
        ModelsTab::Providers => 1,
    };
    let tabs = SegmentedControl::new(&["Role Assignments", "Providers & Keys"], Some(tab_idx))
        .label("Model settings view")
        .fit()
        .on_pick(|state: &mut State, index, _| {
            state.models_tab = if index == 1 {
                ModelsTab::Providers
            } else {
                ModelsTab::Roles
            };
        })
        .build(kit);

    let mut body = rune::col().gap(12.0).child(ui_components::section_header(
        &icons::MODELS,
        "Model Assignments",
        Some("Switch between routing roles and provider account credentials"),
        None,
        Some(tabs),
        kit,
    ));

    match active_tab {
        ModelsTab::Providers => {
            body = body.child(heading("Providers", kit));
            if let Some(message) = load_message(
                &state.backend.profile,
                "Provider metadata has not been observed.",
                preferences,
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
                    let tone = if account.has_api_key {
                        StatusTone::Ok
                    } else {
                        StatusTone::Warn
                    };
                    let mut row = rune::col()
                        .gap(10.0)
                        .key(format!("provider-account-{}", account.id))
                        .child(
                            rune::row()
                                .w_full()
                                .center_y()
                                .gap(8.0)
                                .child(ui_components::status_dot(tone, preferences))
                                .child(heading(
                                    format!("{} · {}", account.provider, account.id),
                                    kit,
                                ))
                                .child(rune::spacer())
                                .child(ui_components::status_pill(
                                    if account.has_api_key {
                                        "API key configured"
                                    } else {
                                        "API key unavailable"
                                    },
                                    Some(&icons::KEY),
                                    tone,
                                    kit,
                                )),
                        );
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
                            .child(ui_components::inline_toggle_row(
                                "account-preserve-key",
                                state.configuration.preserve_account_key,
                                "Preserve saved key if input blank",
                                None,
                                false,
                                kit,
                                |state: &mut State| {
                                    state.configuration.preserve_account_key =
                                        !state.configuration.preserve_account_key;
                                    state.configuration_edited();
                                },
                            ))
                            .child(
                                rune::row()
                                    .gap(8.0)
                                    .child(
                                        ActionButton::new("Review key change")
                                            .key("account-preview")
                                            .icon(&icons::SHIELD_CHECK)
                                            .finish(MetalFinish::GoldCta)
                                            .busy(matches!(
                                                state.configuration.result,
                                                Load::Loading
                                            ))
                                            .disabled(
                                                !state.permission(Permission::ManageConfiguration),
                                            )
                                            .on_press(|state: &mut State| {
                                                state.preview_provider_account()
                                            })
                                            .build(kit, preferences, pointer_ref),
                                    )
                                    .child(
                                        ActionButton::new("Clear key…")
                                            .key("account-clear-key")
                                            .icon(&icons::TRASH)
                                            .finish(MetalFinish::CrimsonDanger)
                                            .disabled(
                                                !state.permission(Permission::ManageConfiguration),
                                            )
                                            .on_press(|state: &mut State| {
                                                state
                                                    .configuration
                                                    .account_api_key
                                                    .replace_text("");
                                                state.configuration.preserve_account_key = false;
                                                state.configuration_edited();
                                                state.preview_provider_account();
                                            })
                                            .build(kit, preferences, pointer_ref),
                                    ),
                            );
                    } else {
                        row = row.child(
                            ActionButton::new("Configure key")
                                .key(format!("account-key-edit-{}", account.id))
                                .icon(&icons::KEY)
                                .finish(MetalFinish::Gunmetal)
                                .disabled(!state.permission(Permission::ManageConfiguration))
                                .on_press(move |state: &mut State| state.edit_provider_account(&id))
                                .build(kit, preferences, pointer_ref),
                        );
                    }
                    body = body.child(card(state, preferences, tone, row));
                }
            }
        }
        ModelsTab::Roles => {
            body = body.child(heading("Role assignments", kit));
            if let Some(message) = load_message(
                &state.backend.models,
                "Role assignments have not been observed.",
                preferences,
                kit,
            ) {
                body = body.child(message);
            }
            if let Load::Ready(defaults) = &state.backend.models {
                body = body.child(ui_components::status_pill(
                    format!("Configuration revision {}", defaults.configuration_revision),
                    Some(&icons::SHIELD_CHECK),
                    StatusTone::Info,
                    kit,
                ));
                for role in &defaults.roles {
                    let edited = role.clone();
                    let active = state.configuration.role.text == *role;
                    let mut row = rune::col()
                        .gap(8.0)
                        .key(format!("model-role-{role}"))
                        .child(
                            rune::row()
                                .w_full()
                                .center_y()
                                .gap(8.0)
                                .child(ui_components::status_dot(
                                    if active {
                                        StatusTone::Accent
                                    } else {
                                        StatusTone::Info
                                    },
                                    preferences,
                                ))
                                .child(heading(role.clone(), kit))
                                .child(rune::spacer())
                                .children(active.then(|| {
                                    ui_components::status_pill(
                                        "Editing",
                                        None,
                                        StatusTone::Accent,
                                        kit,
                                    )
                                })),
                        );
                    if active {
                        row = row.child(crate::configuration_views::role_row_editor(
                            state, kit, layers,
                        ));
                    } else {
                        if let Some(assignment) = defaults.assignment(role) {
                            row = row.child(line(assignment.primary.label(), kit));
                            if !assignment.fallbacks.is_empty() {
                                row = row.child(quiet(
                                    format!(
                                        "{} configured fallback(s)",
                                        assignment.fallbacks.len()
                                    ),
                                    kit,
                                ));
                            }
                        } else {
                            row = row.child(quiet("Unconfigured", kit));
                        }
                        row = row.child(
                            ActionButton::new("Edit assignment")
                                .key(format!("model-role-edit-{role}"))
                                .icon(&icons::MODELS)
                                .finish(MetalFinish::Gunmetal)
                                .disabled(!state.permission(Permission::ManageConfiguration))
                                .on_press(move |state: &mut State| state.edit_model_role(&edited))
                                .build(kit, preferences, pointer_ref),
                        );
                    }
                    body = body.child(material::interactive_tile(
                        row.padding(14.0),
                        active,
                        if active {
                            StatusTone::Accent
                        } else {
                            StatusTone::Info
                        },
                        preferences,
                        pointer_ref,
                        shadow_ref,
                    ));
                }
            }
        }
    }
    body = body.child(configuration_status(state, preferences, kit));
    scroll(body, "models-body")
}

fn configuration_status(state: &State, preferences: ViewPreferences, kit: &Kit) -> El {
    let pointer_ref = (!preferences.matte()).then_some(&state.pointer_offset);
    let mut body = rune::col().gap(8.0);
    if let Some(confirmation) = &state.configuration.confirmation {
        body = body
            .key("configuration-confirmation")
            .padding(12.0)
            .radius(8.0)
            .fill(theme::hex(theme::SELECTION_SURFACE))
            .border(1.0, theme::hex(theme::ACCENT))
            .child(heading("Confirm configuration update", kit))
            .child(quiet(confirmation.change.summary(), kit))
            .child(
                rune::row()
                    .gap(8.0)
                    .child(
                        ActionButton::new("Confirm change")
                            .key("configuration-confirm")
                            .icon(&icons::SHIELD_CHECK)
                            .finish(MetalFinish::GoldCta)
                            .on_press(|state: &mut State| state.confirm_configuration())
                            .build(kit, preferences, pointer_ref),
                    )
                    .child(
                        ActionButton::new("Keep current configuration")
                            .key("configuration-cancel")
                            .finish(MetalFinish::Gunmetal)
                            .on_press(|state: &mut State| state.configuration.confirmation = None)
                            .build(kit, preferences, pointer_ref),
                    ),
            );
    }
    if !matches!(state.configuration.result, Load::Idle) {
        if let Some(message) = load_message(&state.configuration.result, "", preferences, kit) {
            body = body.child(message);
        }
    }
    if let Load::Ready(saved) = &state.configuration.result {
        body = body.child(
            ui_components::async_status_strip(
                "config-saved",
                "Configuration saved",
                format!("Revision {}", saved.configuration_revision),
                false,
                StatusTone::Ok,
                preferences,
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

fn jobs(
    state: &State,
    preferences: ViewPreferences,
    kit: &Kit,
    layers: &mut Vec<El>,
    log_width: f32,
) -> El {
    let pointer_ref = (!preferences.matte()).then_some(&state.pointer_offset);
    let shadow_ref = (!preferences.matte()).then_some(&state.shadow_offset);
    let job_count = match &state.backend.jobs {
        Load::Ready(jobs) => Some(jobs.rows.len()),
        _ => None,
    };
    let count_pill = job_count.map(|n| {
        rune::row().gap(6.0).center_y().child(
            RollingText::new("jobs-total-count", format!("{n} jobs"))
                .value(n as f64)
                .font(kit.ty.small_medium)
                .color(theme::hex(theme::ACCENT_HOVER))
                .build(kit),
        )
    });
    let mut body = rune::col().gap(12.0).child(ui_components::section_header(
        &icons::JOBS,
        "Jobs & Logs",
        Some("Workstation-scoped activity stream and bounded operational summaries"),
        count_pill,
        None,
        kit,
    ));
    if state.registration.context.is_none() {
        return scroll(
            body.child(ui_components::async_status_strip(
                "jobs-unregistered",
                "Workstation registration required",
                "Workstation registration is required to read activity. No global jobs or logs are displayed.",
                false,
                StatusTone::Warn,
                preferences,
                kit,
            )),
            "jobs-body",
        );
    }
    if let Some(message) = load_message(
        &state.backend.jobs,
        "Workstation activity has not been observed.",
        preferences,
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
            let selected = state.backend.selected_job.as_deref() == Some(job.id.as_str());
            let tone = state_tone(&job.state);
            let title = format!(
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
            );
            let tile_content = rune::row()
                .w_full()
                .padding(10.0)
                .gap(10.0)
                .center_y()
                .child(ui_components::status_dot(tone, preferences))
                .child(
                    rune::col()
                        .grow(1.0)
                        .gap(2.0)
                        .child(
                            rune::text(title.clone())
                                .font(kit.ty.ui_medium)
                                .color(theme::hex(theme::TEXT_PRIMARY)),
                        )
                        .child(quiet(job.id.clone(), kit)),
                )
                .child(ui_components::status_pill(
                    job.state.clone(),
                    None,
                    tone,
                    kit,
                ));
            body = body.child(
                material::interactive_tile(
                    tile_content,
                    selected,
                    tone,
                    preferences,
                    pointer_ref,
                    shadow_ref,
                )
                .key(format!("activity-job-{}", job.id))
                .role("button")
                .label(title)
                .focusable(true)
                .on_press(move |state: &mut State| state.select_job(&id)),
            );
        }
    }
    if let Some(message) = load_message(
        &state.backend.detail,
        "Select a job to view its safe operational summaries.",
        preferences,
        kit,
    ) {
        body = body.child(message);
    }
    if let Load::Ready(detail) = &state.backend.detail {
        let detail_title = if detail.job.title.is_empty() {
            detail.job.operation.clone()
        } else {
            detail.job.title.clone()
        };
        let detail_id = detail.job.id.clone();
        let tone = state_tone(&detail.job.state);
        let mut details = rune::col().gap(10.0).child(ui_components::section_header(
            &icons::JOBS,
            detail_title.clone(),
            Some(&format!(
                "{} · {} · {}",
                detail.job.id, detail.job.state, detail.job.phase
            )),
            Some(ui_components::status_pill(
                detail.job.state.clone(),
                None,
                tone,
                kit,
            )),
            Some(
                ActionButton::new("Pop out summary")
                    .finish(MetalFinish::ObsidianPill)
                    .on_press(move |state: &mut State| {
                        state.selected_job_id = Some(detail_id.clone());
                    })
                    .build(kit, preferences, pointer_ref),
            ),
            kit,
        ));
        if let (Some(done), Some(total)) = (detail.job.progress_done, detail.job.progress_total) {
            let frac = if total > 0 {
                (done as f32 / total as f32).clamp(0.0, 1.0)
            } else {
                0.0
            };
            details = details.child(
                rune::row()
                    .w_full()
                    .center_y()
                    .gap(10.0)
                    .child(
                        ProgressCircle::new(frac)
                            .size(24.0)
                            .label("Job progress")
                            .build(kit),
                    )
                    .child(
                        Progress::new(Some(frac))
                            .label(format!("Progress {done} / {total}"))
                            .show_value()
                            .build(kit)
                            .grow(1.0),
                    ),
            );
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
                details = details.child(line(detail.job.error_summary.clone(), kit).role("alert"));
            }
            if let Some(result) = &detail.maintenance_result {
                details = details.child(result.view(kit));
            }
            details = details.child(heading("Operational logs", kit));
            if let Some(message) = &state.scoped_logs.message {
                details = details.child(line(message.clone(), kit).role("status"));
            }
            if state.scoped_logs.busy() {
                details = details.child(ui_components::async_status_strip(
                    "logs-loading",
                    "Reading safe operational summaries…",
                    "Streaming bounded log slice from gateway",
                    true,
                    StatusTone::Info,
                    preferences,
                    kit,
                ));
            }
            if state.scoped_logs.rows.is_empty() && !state.scoped_logs.busy() {
                details = details.child(quiet(
                    "No safe operational summaries are available for this job.",
                    kit,
                ));
            }
            details = details.child(safe_log_table(state, kit, log_width));
        }
        body = body.child(card(state, preferences, tone, details));

        if state.selected_job_id.as_deref() == Some(detail.job.id.as_str()) {
            let modal_body = rune::col()
                .gap(8.0)
                .child(line(format!("State: {}", detail.job.state), kit))
                .child(line(format!("Phase: {}", detail.job.phase), kit))
                .children(
                    (!detail.job.error_summary.is_empty())
                        .then(|| line(detail.job.error_summary.clone(), kit).role("alert")),
                );
            ui_components::modal_prompt(
                "job-operational-summary",
                detail_title,
                format!("Job {}", detail.job.id),
                true,
                460.0,
                |state: &mut State| state.selected_job_id = None,
                modal_body,
                Some(
                    ActionButton::new("Close")
                        .finish(MetalFinish::GoldCta)
                        .on_press(|state: &mut State| state.selected_job_id = None)
                        .build(kit, preferences, pointer_ref),
                ),
                kit,
                layers,
            );
        }
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
                .width(92.0)
                .min(88.0)
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

pub(crate) fn maintenance(
    state: &State,
    preferences: ViewPreferences,
    kit: &Kit,
    layers: &mut Vec<El>,
) -> El {
    let pointer_ref = (!preferences.matte()).then_some(&state.pointer_offset);
    let shadow_ref = (!preferences.matte()).then_some(&state.shadow_offset);

    let active_step = match state.seed_stage {
        SeedStage::Seed => 0,
        SeedStage::Classifier => 1,
        SeedStage::Maintenance => 2,
    };
    let refresh_btn = ActionButton::new("Refresh checks")
        .key("maintenance-refresh")
        .icon(&icons::REFRESH)
        .finish(MetalFinish::Gunmetal)
        .busy(matches!(state.readiness, Load::Pending | Load::Loading))
        .disabled(!state.connected())
        .on_press(|state: &mut State| state.refresh_page())
        .build(kit, preferences, pointer_ref);

    let stage_picker = SegmentedControl::new(
        &["1. Seed Actors", "2. Train Classifier", "3. Search Index"],
        Some(active_step),
    )
    .label("Maintenance stage")
    .fit()
    .on_pick(|state: &mut State, idx, _| {
        state.seed_stage = match idx {
            1 => SeedStage::Classifier,
            2 => SeedStage::Maintenance,
            _ => SeedStage::Seed,
        };
    })
    .build(kit);

    let (gateway_ready, datastore_ready) = match &state.readiness {
        Load::Ready(readiness) => (
            readiness
                .checks
                .iter()
                .any(|c| c.id == ReadinessCheckId::Gateway && c.state == ReadinessState::Ready),
            readiness
                .checks
                .iter()
                .any(|c| c.id == ReadinessCheckId::Datastore && c.state == ReadinessState::Ready),
        ),
        _ => (false, false),
    };
    let dataset_uploaded = matches!(state.maintenance.upload, Load::Ready(_));

    let checklist = card(
        state,
        preferences,
        if gateway_ready && datastore_ready {
            StatusTone::Ok
        } else {
            StatusTone::Accent
        },
        rune::col()
            .child(ui_components::section_header(
                &icons::SHIELD_CHECK,
                "Pre-flight Checklist",
                Some("Live readiness requirements before backend mutations"),
                None,
                None,
                kit,
            ))
            .child(ui_components::checklist_row(
                "preflight-gateway",
                gateway_ready,
                matches!(state.readiness, Load::Pending | Load::Loading),
                "Gateway health verified",
                "Required before streaming actor or training CSV datasets",
                true,
                StatusTone::Ok,
                kit,
                |_: &mut State| {},
            ))
            .child(ui_components::checklist_row(
                "preflight-datastore",
                datastore_ready,
                false,
                "Datastore online",
                "Backing actor catalog & index storage healthy",
                true,
                StatusTone::Info,
                kit,
                |_: &mut State| {},
            ))
            .child(ui_components::checklist_row(
                "preflight-dataset",
                dataset_uploaded,
                matches!(state.maintenance.upload, Load::Loading),
                "CSV dataset validated locally",
                "Selected CSV passed header and size checks",
                true,
                StatusTone::Accent,
                kit,
                |_: &mut State| {},
            )),
    );

    let mut body = rune::col()
        .gap(12.0)
        .child(ui_components::section_header(
            &icons::SEED,
            "DB Seed & Train",
            Some("Choose stage or invoke directly below"),
            None,
            Some(refresh_btn),
            kit,
        ))
        .child(ui_components::stage_stepper(
            "seed-stage-stepper",
            &["Seed Actors", "Train Classifier", "Search Index"],
            active_step,
            preferences,
            kit,
        ))
        .child(stage_picker)
        .child(checklist);

    match &state.maintenance.upload {
        Load::Loading => {
            body = body.child(ui_components::async_status_strip(
                "maintenance-upload-loading",
                "Streaming dataset",
                "Checking CSV columns locally, then streaming to the backend…",
                true,
                StatusTone::Accent,
                preferences,
                kit,
            ));
        }
        Load::Failed(error) => body = body.child(line(error.clone(), kit).role("alert")),
        Load::Ready(upload) => {
            let name = state
                .maintenance
                .local_dataset
                .as_ref()
                .and_then(|selection| selection.path.file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Selected CSV".into());
            body = body.child(ui_components::status_pill(
                format!("{name} · {} validated bytes", upload.dataset.size_bytes),
                Some(&icons::SEED),
                StatusTone::Ok,
                kit,
            ));
        }
        _ => {}
    }

    for (stage_match, label, operation, key, explanation, glyph, finish) in [
        (
            SeedStage::Seed,
            "Seed Actors",
            MaintenanceOperation::SeedActors,
            "maintenance-seed",
            "Choose a CSV to add or update actors. Existing actors outside this CSV are kept.",
            &icons::SEED,
            MetalFinish::GoldCta,
        ),
        (
            SeedStage::Maintenance,
            "Build Search Index",
            MaintenanceOperation::ReindexActors,
            "maintenance-reindex",
            "Build the actor-search projection from the backend catalog.",
            &icons::BRAIN,
            MetalFinish::Gunmetal,
        ),
        (
            SeedStage::Classifier,
            "Train Classifier — Natural",
            MaintenanceOperation::TrainClassifier,
            "maintenance-train",
            "Choose a training CSV for a Natural candidate. Training does not activate it.",
            &icons::MODELS,
            MetalFinish::Gunmetal,
        ),
    ] {
        let reason = state.maintenance_disabled_reason(operation);
        let focused = state.seed_stage == stage_match;
        let mut operation_card = rune::col().padding(14.0).gap(8.0).child(
            rune::row()
                .w_full()
                .center_y()
                .gap(8.0)
                .child(
                    rune::icon(glyph, 15.0)
                        .color(theme::hex(if focused {
                            theme::ACCENT_HOVER
                        } else {
                            theme::TEXT_MUTED
                        }))
                        .a11y_hidden(),
                )
                .child(heading(label, kit))
                .child(rune::spacer())
                .child(
                    ActionButton::new(label)
                        .key(key)
                        .icon(glyph)
                        .finish(finish)
                        .disabled(reason.is_some())
                        .on_press_tx(move |state: &mut State, tx| {
                            state.seed_stage = stage_match;
                            match operation {
                                MaintenanceOperation::SeedActors => {
                                    state.choose_dataset(DatasetKind::ActorCatalog, tx)
                                }
                                MaintenanceOperation::TrainClassifier => {
                                    state.choose_dataset(DatasetKind::ClassifierTraining, tx)
                                }
                                _ => state.preview_maintenance(operation),
                            }
                        })
                        .build(kit, preferences, pointer_ref),
                ),
        );
        if focused {
            operation_card = operation_card.child(quiet(explanation, kit));
            if let Some(reason) = reason {
                operation_card = operation_card.child(quiet(reason, kit));
            }
        }
        body = body.child(material::interactive_tile(
            operation_card,
            focused,
            if focused {
                StatusTone::Accent
            } else {
                StatusTone::Neutral
            },
            preferences,
            pointer_ref,
            shadow_ref,
        ));
    }

    if let Some(confirmation) = &state.maintenance.confirmation {
        body = body.child(card(
            state,
            preferences,
            StatusTone::Accent,
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
                                "Selected dataset {} · SHA-256 {}",
                                dataset.id, dataset.sha256
                            )
                        },
                    ),
                    kit,
                ))
                .child(
                    rune::row()
                        .gap(8.0)
                        .child(
                            ActionButton::new("Confirm operation")
                                .key("maintenance-confirm")
                                .icon(&icons::SHIELD_CHECK)
                                .finish(MetalFinish::GoldCta)
                                .on_press(|state: &mut State| state.confirm_maintenance())
                                .build(kit, preferences, pointer_ref),
                        )
                        .child(
                            ActionButton::new("Keep current backend state")
                                .finish(MetalFinish::Gunmetal)
                                .on_press(|state: &mut State| state.maintenance.confirmation = None)
                                .build(kit, preferences, pointer_ref),
                        ),
                ),
        ));
    }
    if !matches!(state.maintenance.receipt, Load::Idle) {
        if let Some(message) = load_message(&state.maintenance.receipt, "", preferences, kit) {
            body = body.child(message);
        }
    }
    if let Load::Ready(receipt) = &state.maintenance.receipt {
        body = body.child(
            ui_components::async_status_strip(
                "maintenance-accepted",
                "Operation queued",
                format!("Accepted job {} · Completion pending", receipt.job_id),
                false,
                StatusTone::Ok,
                preferences,
                kit,
            )
            .role("status"),
        );
    }
    if let Some(message) = &state.maintenance.message {
        body = body.child(line(message.clone(), kit).role("status"));
    }
    let _ = layers;
    body
}
