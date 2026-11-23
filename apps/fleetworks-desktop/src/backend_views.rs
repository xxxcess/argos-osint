//! Allowlisted backend operation cards and workstation-scoped activity.

use fleetworks_contracts::context::Permission;
use fleetworks_contracts::readiness::AtlasKeyPresence;
use fleetworks_contracts::readiness::DatasetKind;
use fleetworks_contracts::readiness::MaintenanceOperation;
use fleetworks_contracts::readiness::ReadinessState;
use rune::Cx;
use rune::El;
use rune::Key;
use rune::ScrollSpec;
use rune::TextEdit;
use rune_kit::Cell;
use rune_kit::Kit;
use rune_kit::Progress;
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
use crate::backend::ReadRequest;
use crate::icons;
use crate::job_actions::JobAction;
use crate::logs::LogStatus;
use crate::maintenance;
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
        body.padding(12.0).gap(8.0).w_full(),
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

fn list_load_or_skeleton<T>(
    load: &Load<T>,
    empty: &str,
    skeleton: El,
    preferences: ViewPreferences,
    kit: &Kit,
) -> Option<El> {
    match load {
        Load::Pending | Load::Loading => Some(skeleton.role("status")),
        _ => load_message(load, empty, preferences, kit),
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

fn job_worker_sync_summary(job: &crate::backend::ActivityJob) -> String {
    let mut parts = Vec::new();
    if !job.state.is_empty() {
        parts.push(job.state.clone());
    }
    if !job.phase.is_empty() && !job.phase.eq_ignore_ascii_case(&job.state) {
        parts.push(job.phase.clone());
    }
    let progress = match (job.progress_done, job.progress_total) {
        (Some(done), Some(total)) if total > 0 => Some(format!("{done} / {total}")),
        (Some(done), _) => Some(format!("{done}")),
        _ => None,
    };
    let mut summary = parts.join(" · ");
    if let Some(progress) = progress {
        if summary.is_empty() {
            summary = progress;
        } else {
            summary.push_str(&format!(" ({progress})"));
        }
    }
    summary
}

fn job_error_summary(job: &crate::backend::ActivityJob) -> &str {
    if job.operation != fleetworks_contracts::ATLAS_CYCLE_OPERATION {
        return &job.error_summary;
    }
    match job.error_summary.as_str() {
        "atlas_gnews_key_missing" => "GNews API key missing. Save a key in Tool Box.",
        "atlas_newsdata_key_missing" => "NewsData.io API key missing. Save a key in Tool Box.",
        "atlas_newsapi_key_missing" => "NewsAPI API key missing. Save a key in Tool Box.",
        "atlas_currents_key_missing" => "Currents API key missing. Save a key in Tool Box.",
        "atlas_classifier_unavailable" | "atlas_classifier_service_unavailable" => {
            "Rust-BERT worker unavailable. Check the Distributed gateway."
        }
        "atlas_classifier_model_not_configured" => "Rust-BERT checkpoint is not configured.",
        "atlas_classifier_timeout" => "Rust-BERT worker timed out.",
        "atlas_classifier_busy" => "Rust-BERT worker is busy.",
        _ => &job.error_summary,
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
        Page::Tools => tools(state, cx, preferences, kit, layers),
        Page::Models => models(state, cx, preferences, kit, layers),
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

    let mut action = rune::col().gap(8.0).child(ui_components::section_header(
        &icons::ATLAS,
        "News Cycles",
        None,
        count_badge,
        None,
        kit,
    ));

    if state.atlas_advertised() {
        action = action.child(atlas_prerequisites(state, preferences, kit));
        tool_key_dialog(state, preferences, kit, layers);
    }
    if let Some(reason) = &reason {
        action = action.child(quiet(reason.as_str(), kit));
    }
    if state.is_local_backend() {
        action = action.child(
            ActionButton::new("Set up Distributed")
                .key("atlas-open-distributed-setup")
                .on_press_tx(|state: &mut State, tx| {
                    state.open_connection_settings();
                    tx.scroll_into_view("switch-remote-gateway");
                    tx.focus("switch-remote-gateway", tx.by_keys());
                })
                .build(kit, preferences, None),
        );
    }
    action = action.child(rune::row().child(trigger));
    if !lane.busy() {
        if let Some(snapshot) = &lane.confirmation {
            action = action.child(material::glass_panel(
                rune::col()
                    .gap(8.0)
                    .padding(12.0)
                    .radius(8.0)
                    .key("atlas-submit-confirmation")
                    .child(heading("Confirm New Cycle", kit))
                    .child(quiet(
                        format!("Configuration revision {}", snapshot.configuration_revision),
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
                                    .finish(MetalFinish::FracturedConfirm)
                                    .disabled(reason.is_some())
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
                preferences,
            ));
        }
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
        let matched_cycle = match &state.cycles.history {
            Load::Ready(cycles) => cycles
                .rows
                .iter()
                .find(|cycle| cycle.job_id == receipt.job_id || cycle.run_id == receipt.run_id),
            _ => None,
        };
        let (receipt_title, receipt_detail, receipt_tone) = match matched_cycle {
            Some(cycle) if !crate::backend::job_is_active(&cycle.state) => {
                let detail = match (cycle.article_count, &cycle.finished_at) {
                    (Some(count), _) => format!("Job {} · {count} articles", receipt.job_id),
                    (None, Some(finished)) => {
                        format!("Job {} · Finished {finished}", receipt.job_id)
                    }
                    (None, None) => format!("Job {} · {}", receipt.job_id, cycle.state),
                };
                (
                    format!("Cycle {} · {}", receipt.run_id, cycle.state),
                    detail,
                    state_tone(&cycle.state),
                )
            }
            Some(cycle) => (
                format!("Cycle {} · {}", receipt.run_id, cycle.state),
                format!("Job {} · Completion pending", receipt.job_id),
                state_tone(&cycle.state),
            ),
            None => (
                format!("Accepted cycle {}", receipt.run_id),
                format!("Job {} · Completion pending", receipt.job_id),
                StatusTone::Ok,
            ),
        };
        action = action.child(
            ui_components::async_status_strip(
                "atlas-receipt",
                receipt_title,
                receipt_detail,
                false,
                receipt_tone,
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

    let cycles_unsynced = state.cycles.read_error.as_ref().map(|error| {
        crate::views::unsynced_glyph(
            "cycles-unsynced",
            format!("Cycle history unsynced · {error}"),
            kit,
        )
    });
    let cycles_retry = (state.cycles.read_error.is_some()
        || matches!(state.cycles.history, Load::Failed(_)))
    .then(|| {
        ActionButton::new("Retry")
            .key("cycles-history-retry")
            .icon(&icons::REFRESH)
            .finish(MetalFinish::Gunmetal)
            .busy(state.cycles.busy())
            .disabled(!state.connected() || state.cycles.busy())
            .on_press(|state: &mut State| state.refresh_cycles())
            .build(kit, preferences, pointer_ref)
    });
    let mut history = rune::col().child(ui_components::section_header(
        &icons::JOBS,
        "Cycle history",
        Some("Select any cycle to expand run metadata or history removal"),
        cycles_unsynced,
        cycles_retry,
        kit,
    ));
    if let Some(message) = list_load_or_skeleton(
        &state.cycles.history,
        "Cycle history has not been observed.",
        ui_components::skeleton_cycle_history(preferences, kit),
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
        history = history.child(cycles_tail(
            state,
            cycles.rows.len(),
            kit,
            preferences,
            pointer_ref,
        ));

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
                                .finish(MetalFinish::FracturedDanger)
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

pub(crate) fn switch_to_distributed(state: &State, preferences: ViewPreferences, kit: &Kit) -> El {
    ActionButton::new("Switch to Distributed")
        .key("switch-remote-gateway")
        .disabled(
            state.connection.status == crate::connection::ConnectionStatus::Connecting
                || state.local_backend.stopping,
        )
        .on_press_tx(State::open_atlas_distributed_setup)
        .build(kit, preferences, None)
}

fn atlas_prerequisites(state: &State, preferences: ViewPreferences, kit: &Kit) -> El {
    let preflight = state.atlas_preflight();
    let mut checks = rune::col()
        .key("atlas-prerequisites")
        .gap(4.0)
        .w_full()
        .child(ui_components::section_header(
            &icons::SHIELD_CHECK,
            "Cycle prerequisites",
            None,
            None,
            Some(
                rune::row()
                    .gap(6.0)
                    .center_y()
                    .child(
                        ActionButton::new("Configure API keys")
                            .key("atlas-configure-provider-keys")
                            .icon(&icons::KEY)
                            .finish(MetalFinish::ObsidianPill)
                            .disabled(!can_configure_atlas_keys(state))
                            .on_press(|state: &mut State| {
                                if !can_configure_atlas_keys(state) {
                                    return;
                                }
                                reset_tool_key_editor(state);
                                state.tool_key_dialog = true;
                                state.submissions.atlas.confirmation = None;
                                state.backend_read(ReadRequest::Profile);
                            })
                            .build(kit, preferences, None),
                    )
                    .child(
                        ActionButton::new("Refresh checks")
                            .key("atlas-preflight-refresh")
                            .icon(&icons::REFRESH)
                            .finish(MetalFinish::ObsidianPill)
                            .disabled(
                                !state.permission(Permission::ReadBackend)
                                    || state.readiness_pending(),
                            )
                            .on_press(|state: &mut State| state.refresh_readiness())
                            .build(kit, preferences, None),
                    ),
            ),
            kit,
        ));

    for (id, label, observed, ready_label) in [
        (
            "atlas-worker",
            "Atlas worker",
            preflight.map(|report| report.atlas_worker),
            "Available",
        ),
        (
            "rust-bert",
            "Rust-BERT worker and checkpoint",
            preflight.map(|report| report.rust_bert),
            if preflight.is_some_and(|report| report.rust_bert_verified) {
                "Verified"
            } else {
                "Checkpoint available"
            },
        ),
    ] {
        let (status, tone) = match observed {
            Some(ReadinessState::Ready) => (ready_label, StatusTone::Ok),
            Some(ReadinessState::Running) => ("Checking", StatusTone::Info),
            Some(ReadinessState::NotReady | ReadinessState::Degraded) => {
                ("Not ready", StatusTone::Warn)
            }
            Some(ReadinessState::Unavailable | ReadinessState::NotApplicable) => {
                ("Unavailable", StatusTone::Neutral)
            }
            _ => ("Unchecked", StatusTone::Neutral),
        };
        checks = checks.child(atlas_check_row(id, label, status, tone, None, kit));
    }

    let keys = state.atlas_key_states();
    let mut have_saved_key = false;
    for (provider, label, presence) in keys {
        let (status, tone) = atlas_key_status(presence);
        have_saved_key |= presence == AtlasKeyPresence::Saved;
        let action = (presence == AtlasKeyPresence::Missing).then(|| {
            ActionButton::new("Tool Box")
                .key(format!("atlas-key-open-{provider}"))
                .finish(MetalFinish::ObsidianPill)
                .on_press(move |state: &mut State| {
                    state.navigate(Page::Tools);
                    open_tool_credential_editor(state, provider);
                })
                .build(kit, preferences, None)
                .label(format!("Configure the {label} API key in Tool Box"))
        });
        checks = checks.child(atlas_check_row(provider, label, status, tone, action, kit));
    }
    if have_saved_key {
        checks = checks.child(quiet("Saved keys still need provider authentication.", kit));
    }
    checks
}

fn atlas_key_status(presence: AtlasKeyPresence) -> (&'static str, StatusTone) {
    match presence {
        AtlasKeyPresence::Missing => ("Missing key", StatusTone::Warn),
        AtlasKeyPresence::Saved => ("Key saved", StatusTone::Neutral),
        _ => ("Unchecked", StatusTone::Neutral),
    }
}

fn atlas_check_row(
    id: &str,
    name: &str,
    status: &str,
    tone: StatusTone,
    action: Option<El>,
    kit: &Kit,
) -> El {
    rune::row()
        .key(format!("atlas-check-{id}"))
        .w_full()
        .min_w(0.0)
        .flex_wrap()
        .center_y()
        .gap(6.0)
        .child(line(name, kit).grow(1.0).min_w(0.0))
        .child(ui_components::status_pill(status, None, tone, kit))
        .children(action)
}

fn can_configure_atlas_keys(state: &State) -> bool {
    state.page == Page::Atlas
        && state.atlas_advertised()
        && state.connected()
        && state.registration.context.is_some()
        && state.permission(Permission::ManageConfiguration)
        && !matches!(state.configuration.result, Load::Loading)
}

fn current_tool_credential_slot<'a>(
    state: &'a State,
    provider: &str,
) -> Option<&'a crate::backend::ToolCredentialMetadata> {
    let Load::Ready(profile) = &state.backend.profile else {
        return None;
    };
    if state.configuration_revision() != Some(profile.configuration_revision) {
        return None;
    }
    profile
        .tool_credentials
        .iter()
        .find(|slot| slot.provider == provider)
}

fn reset_tool_key_editor(state: &mut State) {
    state.configuration.tool_provider.replace_text("");
    state.clear_tool_credential_draft();
    state.configuration_edited();
    if !matches!(state.configuration.result, Load::Loading) {
        state.configuration.result = Load::Idle;
        state.configuration.message = None;
    }
}

fn open_tool_credential_editor(state: &mut State, provider: &str) {
    let available = match state.page {
        Page::Tools => state.permission(Permission::ManageConfiguration),
        Page::Atlas => {
            can_configure_atlas_keys(state)
                && current_tool_credential_slot(state, provider).is_some()
        }
        _ => false,
    };
    if !available {
        return;
    }
    reset_tool_key_editor(state);
    state.configuration.tool_provider = TextEdit::new(provider);
    state.tool_key_dialog = true;
}

fn tools(
    state: &State,
    cx: &Cx<State>,
    preferences: ViewPreferences,
    kit: &Kit,
    layers: &mut Vec<El>,
) -> El {
    let width = cx.layout_size("page-content").map_or(0.0, |size| size[0]);
    let separate_overview = width >= 760.0;
    let title = ui_components::section_header(&icons::TOOLS, "Tool Box", None, None, None, kit);
    let catalog = tool_catalog(state, width >= 1100.0, preferences, kit);
    let overview = tools_overview(state, preferences, kit);
    tool_key_dialog(state, preferences, kit, layers);
    if separate_overview {
        rune::row()
            .key("tool-box-layout")
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(
                rune::col()
                    .grow(1.0)
                    .h_full()
                    .min_w(0.0)
                    .min_h(0.0)
                    .child(scroll(
                        rune::col().child(title).child(catalog),
                        "tools-body",
                    )),
            )
            .child(
                rune::col()
                    .w(300.0)
                    .h_full()
                    .min_h(0.0)
                    .border_sides([1.0, 0.0, 0.0, 0.0], theme::hex(theme::BORDER_SUBTLE))
                    .child(scroll(overview, "tool-box-overview-scroll")),
            )
    } else {
        scroll(
            rune::col().child(title).child(overview).child(catalog),
            "tools-body",
        )
    }
}

fn tool_catalog(state: &State, wide: bool, preferences: ViewPreferences, kit: &Kit) -> El {
    let mut body = rune::col().w_full().min_w(0.0).gap(10.0);
    if let Some(message) = list_load_or_skeleton(
        &state.backend.tools,
        "Tool availability has not been observed.",
        ui_components::skeleton_tool_catalog(preferences, kit),
        preferences,
        kit,
    ) {
        body = body.child(message);
    }
    if let Load::Ready(catalog) = &state.backend.tools {
        if catalog.tools.is_empty() {
            body = body.child(quiet("No services are advertised by this gateway.", kit));
        }
        let mut grid = if wide {
            rune::row().flex_wrap()
        } else {
            rune::col()
        }
        .w_full()
        .gap(10.0);
        for tool in &catalog.tools {
            let tile = material::canvas_tile(
                rune::col()
                    .min_w(0.0)
                    .padding(10.0)
                    .gap(6.0)
                    .key(format!("tool-{}", tool.id))
                    .child(
                        rune::row()
                            .w_full()
                            .flex_wrap()
                            .center_y()
                            .gap(8.0)
                            .child(
                                rune::icon(&icons::TOOLS, 15.0)
                                    .color(theme::hex(theme::ENTITY_TECH))
                                    .a11y_hidden(),
                            )
                            .child(heading(tool.name.clone(), kit).min_w(0.0).shrink())
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
                false,
                preferences,
            );
            grid = grid.child(if wide { tile.w_frac(0.48) } else { tile });
        }
        if !catalog.tools.is_empty() {
            body = body.child(grid);
        }
    }
    body
}

fn tools_overview(state: &State, preferences: ViewPreferences, kit: &Kit) -> El {
    let pointer_ref = (!preferences.matte()).then_some(&state.pointer_offset);
    let mut body = rune::col()
        .key("tool-box-overview")
        .w_full()
        .min_w(0.0)
        .gap(10.0)
        .child(
            heading("Overview", kit)
                .role("heading")
                .key("tool-box-overview-title"),
        )
        .child(quiet("Provider API keys", kit));
    if let Some(message) = list_load_or_skeleton(
        &state.backend.profile,
        "Credential availability has not been observed.",
        ui_components::skeleton_tool_credentials(preferences, kit),
        preferences,
        kit,
    ) {
        body = body.child(message);
    }
    if let Load::Ready(profile) = &state.backend.profile {
        if profile.tool_credentials.is_empty() {
            body = body.child(quiet("No tool credential slots are configured.", kit));
        }
        for credential in &profile.tool_credentials {
            let provider = credential.provider.clone();
            let configured = credential.primary_present || credential.fallback_present;
            let tone = if configured {
                StatusTone::Ok
            } else {
                StatusTone::Warn
            };
            body = body.child(material::canvas_tile(
                rune::col()
                    .w_full()
                    .min_w(0.0)
                    .padding(10.0)
                    .gap(8.0)
                    .key(format!("tool-provider-{}", credential.provider))
                    .child(
                        rune::row()
                            .center_y()
                            .gap(8.0)
                            .child(ui_components::status_dot(tone, preferences))
                            .child(heading(credential.provider.clone(), kit).min_w(0.0)),
                    )
                    .child(
                        rune::row()
                            .flex_wrap()
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
                    )
                    .child(
                        ActionButton::new("Configure API keys")
                            .key(format!("tool-key-open-{}", credential.provider))
                            .icon(&icons::KEY)
                            .finish(MetalFinish::Gunmetal)
                            .disabled(!state.permission(Permission::ManageConfiguration))
                            .on_press(move |state: &mut State| {
                                open_tool_credential_editor(state, &provider);
                            })
                            .build(kit, preferences, pointer_ref)
                            .label(format!("Configure API keys for {}", credential.provider)),
                    ),
                false,
                preferences,
            ));
        }
    }
    body
}

fn tool_key_dialog(state: &State, preferences: ViewPreferences, kit: &Kit, layers: &mut Vec<El>) {
    if !state.tool_key_dialog {
        return;
    }
    let atlas = state.page == Page::Atlas;
    let provider = state.configuration.tool_provider.text.as_str();
    let editing = !provider.is_empty();
    let title = if atlas {
        "Atlas provider keys".into()
    } else {
        format!("{provider} API keys")
    };
    let editor = if atlas && !editing {
        atlas_key_sources(state, preferences, kit)
    } else {
        tool_key_editor(state, atlas, provider, preferences, kit)
    };
    layers.extend(ui_components::glass_dialog(
        "tool-api-key",
        &title,
        480.0,
        rune::col()
            .key(if editing {
                "provider-key-editor"
            } else {
                "atlas-key-sources"
            })
            .w_full()
            .max_h(316.0)
            .min_h(0.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .padding(16.0)
            .gap(12.0)
            .child(editor)
            .child(configuration_status(state, preferences, kit)),
        false,
        |state: &mut State| {
            state.tool_key_dialog = false;
            reset_tool_key_editor(state);
        },
        preferences,
        kit,
    ));
}

fn atlas_key_sources(state: &State, preferences: ViewPreferences, kit: &Kit) -> El {
    let mut body = rune::col().gap(6.0).child(quiet(
        "Save a primary or fallback key for each source. Saved keys still need provider authentication.",
        kit,
    ));
    for (provider, label, presence) in state.atlas_key_states() {
        let (status, tone) = atlas_key_status(presence);
        let editor = ActionButton::new("Edit key")
            .key(format!("atlas-key-edit-{provider}"))
            .finish(MetalFinish::ObsidianPill)
            .disabled(
                !can_configure_atlas_keys(state)
                    || current_tool_credential_slot(state, provider).is_none(),
            )
            .on_press(move |state: &mut State| open_tool_credential_editor(state, provider))
            .build(kit, preferences, None)
            .label(format!("Edit {label} API keys"));
        body = body.child(atlas_check_row(
            &format!("source-{provider}"),
            label,
            status,
            tone,
            Some(editor),
            kit,
        ));
    }
    if let Some(message) = load_message(
        &state.backend.profile,
        "Saved provider slots have not been loaded.",
        preferences,
        kit,
    ) {
        body = body.child(message);
    }
    if !matches!(state.backend.profile, Load::Loading | Load::Pending)
        && state
            .atlas_key_states()
            .iter()
            .any(|(provider, _, _)| current_tool_credential_slot(state, provider).is_none())
    {
        body = body.child(
            ActionButton::new("Refresh provider slots")
                .key("atlas-key-refresh-slots")
                .on_press(|state: &mut State| state.backend_read(ReadRequest::Profile))
                .build(kit, preferences, None),
        );
    }
    body
}

fn tool_key_editor(
    state: &State,
    atlas: bool,
    provider: &str,
    preferences: ViewPreferences,
    kit: &Kit,
) -> El {
    let pointer_ref = (!preferences.matte()).then_some(&state.pointer_offset);
    let changed = !state.configuration.tool_primary.text.is_empty()
        || !state.configuration.tool_fallback.text.is_empty()
        || state.configuration.clear_tool_primary
        || state.configuration.clear_tool_fallback;
    let mut body = rune::col().gap(12.0);
    if atlas {
        let provider_name = state
            .atlas_key_states()
            .into_iter()
            .find_map(|(id, name, _)| (id == provider).then_some(name))
            .unwrap_or(provider);
        body = body.child(
            rune::row()
                .w_full()
                .center_y()
                .flex_wrap()
                .gap(8.0)
                .child(heading(provider_name, kit).grow(1.0))
                .child(
                    ActionButton::new("All providers")
                        .key("atlas-key-back")
                        .finish(MetalFinish::ObsidianPill)
                        .disabled(matches!(state.configuration.result, Load::Loading))
                        .on_press(|state: &mut State| reset_tool_key_editor(state))
                        .build(kit, preferences, None),
                ),
        );
    }
    body.child(quiet(
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
                ActionButton::new(if atlas { "Review API keys" } else { "Review key replacement" })
                    .key("tool-credentials-preview")
                    .icon(&icons::SHIELD_CHECK)
                    .finish(MetalFinish::GoldCta)
                    .busy(matches!(state.configuration.result, Load::Loading))
                    .disabled(
                        !state.permission(Permission::ManageConfiguration)
                            || (atlas && (!changed || current_tool_credential_slot(state, provider).is_none())),
                    )
                    .on_press(|state: &mut State| state.preview_tool_credentials())
                    .build(kit, preferences, pointer_ref),
            )
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

fn models(
    state: &State,
    cx: &Cx<State>,
    preferences: ViewPreferences,
    kit: &Kit,
    layers: &mut Vec<El>,
) -> El {
    let pointer_ref = (!preferences.matte()).then_some(&state.pointer_offset);
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
            if let Some(message) = list_load_or_skeleton(
                &state.backend.profile,
                "Provider metadata has not been observed.",
                ui_components::skeleton_provider_accounts(preferences, kit),
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
            if let Some(message) = list_load_or_skeleton(
                &state.backend.models,
                "Role assignments have not been observed.",
                ui_components::skeleton_model_roles(preferences, kit),
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
                let wide = state.configuration.role.text.is_empty()
                    && cx
                        .layout_size("page-content")
                        .is_some_and(|size| size[0] >= 780.0);
                let mut grid = if wide {
                    rune::row().flex_wrap()
                } else {
                    rune::col()
                }
                .w_full()
                .gap(10.0);
                for role in &defaults.roles {
                    let edited = role.clone();
                    let active = state.configuration.role.text == *role;
                    let mut row = rune::col()
                        .min_w(0.0)
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
                                .child(heading(role.clone(), kit).min_w(0.0).shrink())
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
                    let tile = material::canvas_tile(row.padding(10.0), active, preferences);
                    grid = grid.child(if wide { tile.w_frac(0.48) } else { tile });
                }
                if !defaults.roles.is_empty() {
                    body = body.child(grid);
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
                            .finish(MetalFinish::FracturedConfirm)
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

fn truncate_id(value: &str, max_chars: usize) -> String {
    let total = value.chars().count();
    if total <= max_chars || max_chars < 6 {
        return value.to_owned();
    }
    let keep = (max_chars - 1) / 2;
    let tail_keep = max_chars - 1 - keep;
    let head: String = value.chars().take(keep).collect();
    let tail: String = value.chars().skip(total - tail_keep).collect();
    format!("{head}…{tail}")
}

fn job_count_text(count: usize) -> String {
    if count == 1 {
        "1 job".to_owned()
    } else {
        format!("{count} jobs")
    }
}

/// Jobs & Logs list width, gap and the page width from which the list sits beside the detail.
const JOBS_LIST_WIDTH: f32 = 300.0;
const JOBS_GAP: f32 = 16.0;
const JOBS_SIDE_BY_SIDE_WIDTH: f32 = 940.0;

fn jobs(
    state: &State,
    preferences: ViewPreferences,
    kit: &Kit,
    layers: &mut Vec<El>,
    log_width: f32,
) -> El {
    let pointer_ref = (!preferences.matte()).then_some(&state.pointer_offset);
    let shadow_ref = (!preferences.matte()).then_some(&state.shadow_offset);
    let narrow = log_width < 660.0;
    let header_count = match (&state.scoped_jobs.page, state.scoped_jobs.status) {
        (Some(page), crate::jobs_query::JobsLoad::Ready) => {
            if let Some(total) = page.matched_total {
                let label = if total == 1 {
                    "1 match".to_owned()
                } else {
                    format!("{total} matches")
                };
                Some((total, label))
            } else {
                let loaded = state.scoped_jobs.row_count() as u64;
                Some((loaded, format!("{loaded} loaded")))
            }
        }
        _ => match &state.backend.jobs {
            Load::Ready(jobs) => Some((jobs.rows.len() as u64, job_count_text(jobs.rows.len()))),
            _ => None,
        },
    };
    let jobs_unsynced = (state.scoped_jobs.status == crate::jobs_query::JobsLoad::Ready)
        .then_some(state.scoped_jobs.message.as_ref())
        .flatten()
        .map(|error| {
            crate::views::unsynced_glyph(
                "jobs-unsynced",
                format!("Jobs list unsynced · {error}"),
                kit,
            )
        });
    let count_pill = if header_count.is_some() || jobs_unsynced.is_some() {
        let mut slot = rune::row().gap(6.0).center_y();
        if let Some((n, label)) = header_count {
            slot = slot.child(
                RollingText::new("jobs-total-count", label)
                    .value(n as f64)
                    .font(kit.ty.small_medium)
                    .color(theme::hex(theme::ACCENT_HOVER))
                    .build(kit),
            );
        }
        if let Some(glyph) = jobs_unsynced {
            slot = slot.child(glyph);
        }
        Some(slot)
    } else {
        None
    };
    let mut body = rune::col().gap(12.0).child(ui_components::section_header(
        &icons::JOBS,
        "Jobs & Logs",
        None,
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
    // Wide enough for a ~300 pt list beside a detail pane that still has room for the log table.
    let side_by_side = log_width >= JOBS_SIDE_BY_SIDE_WIDTH;
    let detail_width = if side_by_side {
        log_width - JOBS_LIST_WIDTH - JOBS_GAP
    } else {
        log_width
    };
    let mut list_col = rune::col().gap(12.0);
    let mut detail_col = rune::col().gap(12.0);
    let show_list = !narrow || state.backend.selected_job.is_none();
    let show_detail = !narrow || state.backend.selected_job.is_some();
    if show_list {
        if let Some(message) = list_load_or_skeleton(
            &state.backend.jobs,
            "Workstation activity has not been observed.",
            ui_components::skeleton_job_list(preferences, kit),
            preferences,
            kit,
        ) {
            list_col = list_col.child(message);
        }
        if let Load::Ready(jobs) = &state.backend.jobs {
            if jobs.rows.is_empty() {
                list_col =
                    list_col.child(quiet("No jobs are associated with this workstation.", kit));
            }
            for job in jobs.rows.iter() {
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
                    .padding(7.0)
                    .gap(8.0)
                    .center_y()
                    .child(ui_components::status_dot(tone, preferences))
                    .child(
                        rune::col()
                            .grow(1.0)
                            .min_w(0.0)
                            .gap(1.0)
                            .child(
                                rune::text(title.clone())
                                    .font(kit.ty.ui_medium)
                                    .color(theme::hex(theme::TEXT_PRIMARY))
                                    .wrap(),
                            )
                            .child(quiet(truncate_id(&job.id, 24), kit).label(job.id.clone())),
                    )
                    .child(ui_components::status_pill(
                        job.state.clone(),
                        None,
                        tone,
                        kit,
                    ));
                list_col = list_col.child(
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
            list_col = list_col.child(jobs_tail(
                state,
                jobs.rows.len(),
                kit,
                preferences,
                pointer_ref,
            ));
        }
    }
    if show_detail {
        if narrow && state.backend.selected_job.is_some() {
            detail_col = detail_col.child(
                ActionButton::new("Back to jobs")
                    .key("jobs-back-to-list")
                    .finish(MetalFinish::Gunmetal)
                    .on_press(|state: &mut State| {
                        state.backend.selected_job = None;
                        state.backend.detail = Load::Idle;
                    })
                    .build(kit, preferences, pointer_ref),
            );
        }
        if let Some(message) = list_load_or_skeleton(
            &state.backend.detail,
            "Select a job to view its safe operational summaries.",
            ui_components::skeleton_job_detail(preferences, kit),
            preferences,
            kit,
        ) {
            detail_col = detail_col.child(message);
        }
    }
    if show_detail {
        if let Load::Ready(detail) = &state.backend.detail {
            let detail_title = if detail.job.title.is_empty() {
                detail.job.operation.clone()
            } else {
                detail.job.title.clone()
            };
            let detail_id = detail.job.id.clone();
            let tone = state_tone(&detail.job.state);
            let error_summary = job_error_summary(&detail.job);
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
            if let Some((done, total)) = detail
                .job
                .progress_done
                .zip(detail.job.progress_total)
                .filter(|(done, total)| *done >= 0 && *total > 0)
            {
                let frac = (done.min(total) as f32 / total as f32).clamp(0.0, 1.0);
                let prefix = if detail.job.phase.is_empty() {
                    "Progress"
                } else {
                    detail.job.phase.as_str()
                };
                details = details.child(
                    Progress::new(Some(frac))
                        .label(format!("{prefix} · {done} / {total}"))
                        .show_value()
                        .build(kit)
                        .w_full(),
                );
            } else {
                let phase = detail.job.phase.trim();
                let known_done = detail.job.progress_done.filter(|done| *done >= 0);
                let progress_label = match (phase.is_empty(), known_done) {
                    (false, Some(done)) => format!("{phase} · {done} processed"),
                    (true, Some(done)) => format!("{done} processed"),
                    (false, None) => format!("Phase: {phase}"),
                    (true, None) => detail.job.state.clone(),
                };
                if crate::backend::job_is_active(&detail.job.state)
                    || known_done.is_some()
                    || !phase.is_empty()
                {
                    details = details.child(quiet(progress_label, kit).role("status"));
                }
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
                if !error_summary.is_empty() {
                    details = details.child(line(error_summary, kit).role("alert"));
                }
                if detail.job.state == "failed" || detail.job.can_retry {
                    let retry_reason = state.job_action_disabled_reason(JobAction::Retry);
                    let retry_busy = matches!(state.job_actions.response, Load::Loading);
                    details = details.child(
                        rune::row().w_full().center_y().gap(8.0).child(
                            ActionButton::new("Retry job")
                                .key("job-retry-preview")
                                .icon(&icons::REFRESH)
                                .finish(MetalFinish::GoldCta)
                                .status(
                                    if retry_busy {
                                        "Retrying…"
                                    } else if retry_reason.is_none() {
                                        "Ready"
                                    } else {
                                        "Unavailable"
                                    },
                                    if retry_busy {
                                        StatusTone::Accent
                                    } else if retry_reason.is_none() {
                                        StatusTone::Ok
                                    } else {
                                        StatusTone::Warn
                                    },
                                )
                                .busy(retry_busy)
                                .disabled(retry_reason.is_some())
                                .on_press(|state: &mut State| {
                                    state.preview_job_action(JobAction::Retry);
                                })
                                .build(kit, preferences, pointer_ref),
                        ),
                    );
                    if let Some(reason) = retry_reason {
                        details = details.child(quiet(reason, kit));
                    }
                }
                if let Some(confirmation) =
                    state.job_actions.confirmation.as_ref().filter(|item| {
                        item.action == JobAction::Retry && item.job_id == detail.job.id
                    })
                {
                    let confirming = matches!(state.job_actions.response, Load::Loading);
                    let confirm_button = ActionButton::new("Confirm retry")
                        .key("job-action-confirm")
                        .icon(&icons::SHIELD_CHECK)
                        .finish(MetalFinish::FracturedConfirm)
                        .status(
                            if confirming { "Queuing…" } else { "Ready" },
                            if confirming {
                                StatusTone::Accent
                            } else {
                                StatusTone::Ok
                            },
                        )
                        .busy(confirming)
                        .disabled(confirming)
                        .on_press(|state: &mut State| state.confirm_job_action())
                        .build(kit, preferences, pointer_ref);
                    let cancel_button = ActionButton::new("Keep current job state")
                        .finish(MetalFinish::Gunmetal)
                        .on_press(|state: &mut State| {
                            state.job_actions.confirmation = None;
                        })
                        .build(kit, preferences, pointer_ref);
                    let action_buttons = if detail_width < 420.0 {
                        rune::col()
                            .gap(8.0)
                            .child(confirm_button)
                            .child(cancel_button)
                    } else {
                        rune::row()
                            .gap(8.0)
                            .child(confirm_button)
                            .child(cancel_button)
                    };
                    details = details.child(card(
                        state,
                        preferences,
                        StatusTone::Accent,
                        rune::col()
                            .key("job-action-confirmation")
                            .gap(8.0)
                            .child(heading(
                                format!("Confirm retry for {}", confirmation.title),
                                kit,
                            ))
                            .child(quiet(
                                format!(
                                    "{} · re-queues the failed task with a fresh request identity.",
                                    confirmation.job_id
                                ),
                                kit,
                            ))
                            .child(action_buttons),
                    ));
                }
                let matching_attempt =
                    state.job_actions.attempt.as_ref().is_some_and(|(item, _)| {
                        item.action == JobAction::Retry && item.job_id == detail.job.id
                    });
                match &state.job_actions.response {
                    Load::Loading if matching_attempt => {
                        details = details.child(ui_components::async_status_strip(
                            "job-action-loading",
                            "Submitting retry…",
                            "Sending scoped retry trigger to backend gateway",
                            true,
                            StatusTone::Accent,
                            preferences,
                            kit,
                        ));
                    }
                    Load::Ready(receipt)
                        if receipt.receipt.job_id == detail.job.id
                            && receipt.receipt.action == JobAction::Retry =>
                    {
                        details = details.child(
                            ui_components::async_status_strip(
                                "job-action-receipt",
                                "Retry queued",
                                format!(
                                    "Action {} · state {}",
                                    receipt.receipt.action_id, receipt.receipt.state
                                ),
                                false,
                                StatusTone::Ok,
                                preferences,
                                kit,
                            )
                            .role("status"),
                        );
                    }
                    Load::Failed(error) if matching_attempt => {
                        details = details.child(line(error.clone(), kit).role("alert")).child(
                            ActionButton::new("Retry exact request")
                                .key("job-action-retry-exact")
                                .icon(&icons::REFRESH)
                                .finish(MetalFinish::GoldCta)
                                .on_press(|state: &mut State| state.retry_exact_job_action())
                                .build(kit, preferences, pointer_ref),
                        );
                    }
                    Load::Failed(error) if state.job_actions.attempt.is_none() => {
                        details = details.child(line(error.clone(), kit).role("alert"));
                    }
                    _ => {}
                }
                if let Some(message) = &state.job_actions.message {
                    details = details.child(line(message.clone(), kit).role("status"));
                }
                if let Some(result) = &detail.maintenance_result {
                    details = details.child(result.view(kit));
                }
                let worker_summary = job_worker_sync_summary(&detail.job);
                details = details.child(
                    rune::row()
                        .w_full()
                        .center_y()
                        .gap(8.0)
                        .child(heading("Operational logs", kit).grow(1.0))
                        .child(ui_components::status_pill(
                            if worker_summary.is_empty() {
                                detail.job.state.clone()
                            } else {
                                worker_summary.clone()
                            },
                            None,
                            tone,
                            kit,
                        )),
                );
                // One status strip with one key. Its title follows the job and the live toggle, never
                // the individual request, so follow polls do not swap text or resize the section.
                let job_active = crate::backend::job_is_active(&detail.job.state)
                    && state.scoped_logs.status != LogStatus::Unavailable;
                let strip = if state.scoped_logs.following {
                    Some(("Live", format!("Worker: {worker_summary}"), tone))
                } else if job_active {
                    Some((
                        "Job active",
                        format!("Phase: {worker_summary}"),
                        StatusTone::Info,
                    ))
                } else if state.scoped_logs.busy() {
                    Some((
                        "Reading summaries",
                        format!("Worker: {worker_summary}"),
                        StatusTone::Info,
                    ))
                } else {
                    None
                };
                if let Some((strip_title, strip_detail, strip_tone)) = strip {
                    details = details.child(ui_components::async_status_strip(
                        "logs-status",
                        strip_title,
                        strip_detail,
                        true,
                        strip_tone,
                        preferences,
                        kit,
                    ));
                }
                if let Some(message) = &state.scoped_logs.message {
                    details = details.child(line(message.clone(), kit).role("status"));
                    if !state.scoped_logs.busy()
                        && matches!(
                            state.scoped_logs.status,
                            LogStatus::Unavailable | LogStatus::RestartRequired
                        )
                    {
                        details = details.child(
                            ActionButton::new("Refresh logs")
                                .finish(MetalFinish::ObsidianPill)
                                .on_press(|state: &mut State| state.refresh_operational_logs())
                                .build(kit, preferences, pointer_ref),
                        );
                    }
                }
                // A read that already resolved stays resolved while a follow or older read runs.
                let resolved = matches!(
                    state.scoped_logs.status,
                    LogStatus::Ready | LogStatus::DisplayLimit
                );
                if state.scoped_logs.rows.is_empty() && (!state.scoped_logs.busy() || resolved) {
                    // Three different states: not read yet, read with no events, and a failed read.
                    // A failed read shows its own message and Refresh button above.
                    match state.scoped_logs.status {
                        LogStatus::Idle => {
                            details = details
                                .child(quiet("Logs for this job have not been read yet.", kit));
                        }
                        LogStatus::Ready | LogStatus::DisplayLimit => {
                            details = details.child(quiet(
                                "No safe operational summaries are available for this job.",
                                kit,
                            ));
                        }
                        _ => {}
                    }
                }
                if state.scoped_logs.rows.is_empty() && state.scoped_logs.busy() && !resolved {
                    details = details.child(ui_components::skeleton_log_table(preferences, kit));
                } else if !state.scoped_logs.rows.is_empty()
                    || matches!(
                        state.scoped_logs.status,
                        LogStatus::Ready | LogStatus::DisplayLimit
                    )
                {
                    details = details.child(safe_log_table(
                        state,
                        kit,
                        detail_width,
                        preferences,
                        pointer_ref,
                    ));
                    if state.scoped_logs.can_load_older() {
                        details = details.child(
                            ActionButton::new("Load older summaries")
                                .key("logs-load-older")
                                .finish(MetalFinish::ObsidianPill)
                                .on_press_tx(|state: &mut State, tx| {
                                    state.log_event(crate::logs::LogEvent::Older, tx)
                                })
                                .build(kit, preferences, pointer_ref),
                        );
                        if state.scoped_logs.auto_load_older() {
                            // A one-pixel sentinel, new for every loaded count: coming on screen
                            // loads one older page, and a page that leaves it on screen loads
                            // another.
                            details = details.child(
                                rune::el()
                                    .key(format!("logs-near-end-{}", state.scoped_logs.rows.len()))
                                    .size(1.0, 1.0)
                                    .a11y_hidden()
                                    .on_visible(|state: &mut State, on_screen, tx| {
                                        state.logs_near_end(on_screen, tx)
                                    }),
                            );
                        }
                    }
                }
            }
            detail_col = detail_col.child(card(state, preferences, tone, details));

            if state.selected_job_id.as_deref() == Some(detail.job.id.as_str()) {
                let modal_body = rune::col()
                    .gap(8.0)
                    .child(line(format!("State: {}", detail.job.state), kit))
                    .child(line(format!("Phase: {}", detail.job.phase), kit))
                    .children(
                        (!error_summary.is_empty()).then(|| line(error_summary, kit).role("alert")),
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
    }
    body = if side_by_side && show_list && show_detail {
        // A paged list beside the detail keeps the selection in view while it refreshes.
        body.child(
            rune::row()
                .key("jobs-workbench")
                .w_full()
                .gap(JOBS_GAP)
                .child(list_col.w(JOBS_LIST_WIDTH).shrink())
                .child(detail_col.grow(1.0).min_w(0.0)),
        )
    } else {
        body.child(list_col).child(detail_col)
    };
    scroll(body, "jobs-body")
}

/// The newest log row is at the top of the table. Its drawn position against the table's own
/// top tells whether the reader has scrolled away from it, from the same layout the renderer
/// draws, with no view commit while the answer is unchanged. shortcut: the signal comes from the
/// newest row's own drawn rect, so a jump that unmounts it without it first moving off the top
/// (dragging the scroll bar far) keeps the previous answer until it mounts again.
fn safe_log_table(
    state: &State,
    kit: &Kit,
    available_width: f32,
    preferences: ViewPreferences,
    pointer_ref: Option<&rune::Live<[f32; 2]>>,
) -> El {
    let style = rune_kit::TableStyle::from_kit(kit);
    let table_rect = rune::Live::new([0.0_f32; 4]);
    let newest_rect = table_rect.clone();
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
        .cell(move |state: &State, index, column| {
            let row = &state.scoped_logs.rows[index];
            match column {
                // The newest row's own text reports where it is drawn. It repeats the kit's text
                // cell (font, color, truncation) so the row looks as every other row does, and
                // adds no column or hidden element for assistive technology to meet.
                "message" if index == 0 => Cell::el(
                    rune::text(row.message.clone())
                        .font(style.font)
                        .color(style.text)
                        .truncate()
                        .shrink()
                        .min_w(0.0)
                        .help_when_truncated(row.message.clone())
                        .on_drawn_layout_tx({
                            let table_rect = newest_rect.clone();
                            move |state: &mut State, rect: [f32; 4], tx| {
                                // The newest row's centre when the reader is at the top.
                                let at_top = table_rect.get()[1]
                                    + style.header_height
                                    + style.row_height * 0.5;
                                // Away once it has moved up by half a row.
                                let away =
                                    rect[1] + rect[3] * 0.5 < at_top - style.row_height * 0.5;
                                if !state.scoped_logs.set_away(away) {
                                    tx.live_only();
                                }
                            }
                        }),
                ),
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
        .fill(kit.c.surface_base)
        .drawn_rect_into(&table_rect);
    let mut body = rune::col().gap(8.0);
    let unread = state.scoped_logs.unread();
    // The marker row is always present, empty at zero, so the table never shifts when the count
    // first rises. The count is text and the control a labelled button, so nothing depends on
    // color. The jump is the runtime's own scroll request, which lands at once under reduced
    // motion.
    let mut marker = rune::row()
        .key("logs-unread")
        .w_full()
        .min_h(44.0)
        .gap(8.0)
        .center_y();
    if unread > 0 {
        marker = marker
            .child(ui_components::status_pill(
                format!(
                    "{unread} new log {}",
                    if unread == 1 { "line" } else { "lines" }
                ),
                None,
                StatusTone::Info,
                kit,
            ))
            .child(
                ActionButton::new("Jump to newest")
                    .key("logs-jump-newest")
                    .finish(MetalFinish::ObsidianPill)
                    .on_press_tx(|state: &mut State, tx| {
                        if let Some(id) = state.scoped_logs.jump_to_newest() {
                            tx.scroll_into_view_in(crate::logs::TABLE_KEY, id);
                        }
                    })
                    .build(kit, preferences, pointer_ref),
            );
    }
    body = body.child(marker);
    body = body.child(table);
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

fn maintenance_preflight_status(
    state: &State,
    operation: MaintenanceOperation,
    preflight: &crate::maintenance::MaintenancePreflight,
) -> (&'static str, StatusTone, bool) {
    if matches!(state.maintenance.upload, Load::Loading)
        && operation == MaintenanceOperation::SeedActors
    {
        return ("Uploading…", StatusTone::Accent, true);
    }
    let queuing_for_op = (matches!(state.maintenance.receipt, Load::Loading)
        || state.maintenance.busy())
        && (state.maintenance.active_operation == Some(operation)
            || state.maintenance.pending_preview == Some(operation)
            || state
                .maintenance
                .confirmation
                .as_ref()
                .is_some_and(|c| c.operation == operation));
    if queuing_for_op {
        return ("Queuing…", StatusTone::Accent, true);
    }
    if matches!(state.readiness, Load::Pending | Load::Loading) {
        return ("Checking…", StatusTone::Info, true);
    }
    let (label, tone) = preflight.status_label();
    (label, tone, false)
}

fn operation_key(operation: MaintenanceOperation) -> &'static str {
    match operation {
        MaintenanceOperation::SeedActors => "seed",
        MaintenanceOperation::ReindexActors => "reindex",
        _ => "other",
    }
}

fn operation_job_card_status(
    state: &State,
    operation: MaintenanceOperation,
    preferences: ViewPreferences,
    kit: &Kit,
    pointer_ref: Option<&rune::Live<[f32; 2]>>,
) -> Option<El> {
    let record = state.maintenance.operation_job(operation)?;
    let job_id = record.job_id.clone();
    let short_id = truncate_id(&job_id, 24);
    let fallback_job = match (&state.backend.detail, &state.backend.jobs) {
        (Load::Ready(detail), _) if detail.job.id == job_id => {
            Some((&detail.job, detail.maintenance_result.as_ref()))
        }
        (_, Load::Ready(jobs)) => jobs
            .rows
            .iter()
            .find(|job| job.id == job_id)
            .map(|job| (job, None)),
        _ => None,
    };
    let observed = record
        .detail
        .as_ref()
        .map(|detail| (&detail.job, detail.maintenance_result.as_ref()))
        .or(fallback_job);

    let (title, detail_text, tone, active_progress) = match observed {
        Some((job, _)) if crate::backend::job_is_active(&job.state) => {
            let stage_title = match job.state.as_str() {
                "running" => "Running",
                "retry_scheduled" => "Retrying",
                _ => "Queued",
            };
            let phase = job.phase.trim();
            let count_text = match (
                job.progress_done.filter(|d| *d >= 0),
                job.progress_total.filter(|t| *t > 0),
            ) {
                (Some(done), Some(total)) => Some(format!("{done} / {total}")),
                (Some(done), None) => Some(format!("{done} processed")),
                _ => None,
            };
            let mut parts = vec![format!("Job {short_id}")];
            if !phase.is_empty() {
                parts.push(phase.to_owned());
            }
            if let Some(counts) = count_text {
                parts.push(counts);
            }
            let frac = job
                .progress_done
                .zip(job.progress_total)
                .filter(|(done, total)| *done >= 0 && *total > 0)
                .map(|(done, total)| (done.min(total) as f32 / total as f32).clamp(0.0, 1.0));
            (
                stage_title.to_owned(),
                parts.join(" · "),
                state_tone(&job.state),
                frac,
            )
        }
        Some((job, result)) if job.state == "completed" => {
            let seed_count = if operation == MaintenanceOperation::SeedActors {
                let job_count =
                    job.progress_done
                        .zip(job.progress_total)
                        .and_then(|(done, total)| {
                            Some((u64::try_from(done).ok()?, u64::try_from(total).ok()?))
                        });
                result
                    .and_then(|result| result.processed.zip(result.total))
                    .or(job_count)
                    .filter(|(done, total)| *total > 0 && done <= total)
            } else {
                None
            };
            let outcome = match operation {
                MaintenanceOperation::SeedActors => seed_count.map_or_else(
                    || "Completed".to_owned(),
                    |(done, total)| format!("Completed ({done}/{total})"),
                ),
                MaintenanceOperation::ReindexActors => {
                    if result.and_then(|r| r.reader_visible) == Some(false) {
                        "Index built".to_owned()
                    } else {
                        "Search index ready".to_owned()
                    }
                }
                _ => "Completed".to_owned(),
            };
            let mut parts = vec![format!("Job {short_id}")];
            if operation == MaintenanceOperation::SeedActors {
                parts.insert(0, "Actors seeded".to_owned());
            }
            if let Some(res) = result {
                if let Some(processed) = res.processed.filter(|_| seed_count.is_none()) {
                    parts.push(res.total.map_or_else(
                        || format!("{processed} processed"),
                        |total| format!("{processed} / {total} processed"),
                    ));
                } else if let Some(scanned) = res.scanned {
                    parts.push(res.total.map_or_else(
                        || format!("{scanned} scanned"),
                        |total| format!("{scanned} / {total} scanned"),
                    ));
                }
                if let Some(candidate) = res
                    .candidate_model_id
                    .as_deref()
                    .filter(|id| !id.is_empty())
                {
                    parts.push(format!("Candidate {}", truncate_id(candidate, 18)));
                }
            }
            (outcome, parts.join(" · "), StatusTone::Ok, None)
        }
        Some((job, _)) => {
            let summary = if job.error_summary.is_empty() {
                format!("Job {short_id} · {}", job.state)
            } else {
                format!("Job {short_id} · {}", job.error_summary)
            };
            (
                format!("Job {}", job.state),
                summary,
                state_tone(&job.state),
                None,
            )
        }
        None => {
            let title = if record.reused {
                "Already running"
            } else {
                "Queued"
            };
            let detail = if record.reused {
                format!("Job {short_id} is already active on the backend · Completion pending")
            } else {
                format!("Accepted job {short_id} · Completion pending")
            };
            (title.to_owned(), detail, StatusTone::Ok, None)
        }
    };

    let op_key = operation_key(operation);
    let mut strip = rune::col()
        .key(format!("maintenance-op-job-{op_key}"))
        .gap(6.0)
        .child(
            ui_components::async_status_strip(
                format!("maintenance-op-strip-{op_key}"),
                title.clone(),
                detail_text,
                false,
                tone,
                preferences,
                kit,
            )
            .label(format!("{title}: Job {job_id}"))
            .role("status"),
        );
    if let Some(frac) = active_progress {
        strip = strip.child(
            Progress::new(Some(frac))
                .label(format!("{title} · Job {short_id}"))
                .build(kit)
                .w_full(),
        );
    }
    if let Some(error) = &record.poll_error {
        strip = strip.child(crate::views::unsynced_glyph(
            &format!("maintenance-op-unsynced-{op_key}"),
            format!("Job status unsynced · {error}"),
            kit,
        ));
    }
    strip = strip.child(
        ActionButton::new("View job")
            .key(format!("maintenance-view-job-{op_key}"))
            .icon(&icons::JOBS)
            .finish(MetalFinish::ObsidianPill)
            .on_press(move |state: &mut State| state.open_job(&job_id))
            .build(kit, preferences, pointer_ref),
    );
    Some(strip)
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
        SeedStage::Maintenance => 1,
    };
    let checking_readiness = matches!(state.readiness, Load::Pending | Load::Loading);
    let refresh_btn = ActionButton::new("Refresh checks")
        .key("maintenance-refresh")
        .icon(&icons::REFRESH)
        .finish(MetalFinish::Gunmetal)
        .status(
            if checking_readiness {
                "Checking…"
            } else {
                "Preflight"
            },
            if checking_readiness {
                StatusTone::Info
            } else {
                StatusTone::Neutral
            },
        )
        .busy(checking_readiness)
        .disabled(!state.connected())
        .on_press(|state: &mut State| state.refresh_page())
        .build(kit, preferences, pointer_ref);

    let stage_picker =
        SegmentedControl::new(&["1. Seed Actors", "2. Search Index"], Some(active_step))
            .label("Maintenance stage")
            .fit()
            .on_pick(|state: &mut State, idx, _| {
                state.seed_stage = match idx {
                    1 => SeedStage::Maintenance,
                    _ => SeedStage::Seed,
                };
            })
            .build(kit);

    let readiness_unsynced = state.readiness_error.as_ref().map(|error| {
        crate::views::unsynced_glyph(
            "maintenance-readiness-unsynced",
            format!("Readiness checks unsynced · {error}"),
            kit,
        )
    });
    let mut body = rune::col()
        .gap(12.0)
        .child(ui_components::section_header(
            &icons::SEED,
            "Actor Maintenance",
            Some("Seed the actor catalog, then make it searchable."),
            readiness_unsynced,
            Some(refresh_btn),
            kit,
        ))
        .child(stage_picker);

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
            "1. Seed Actors",
            MaintenanceOperation::SeedActors,
            maintenance::SEED_ACTION_KEY,
            "Use a CSV to add or update actors. Existing actors outside this CSV are kept.",
            &icons::SEED,
            MetalFinish::GoldCta,
        ),
        (
            SeedStage::Maintenance,
            "2. Build Search Index",
            MaintenanceOperation::ReindexActors,
            maintenance::REINDEX_ACTION_KEY,
            "Build the actor-search projection from the seeded backend catalog.",
            &icons::BRAIN,
            MetalFinish::Gunmetal,
        ),
    ] {
        let preflight = state.maintenance_preflight(operation);
        let reason = preflight.disabled_reason();
        let (preflight_label, preflight_tone, button_busy) =
            maintenance_preflight_status(state, operation, &preflight);
        let focused = state.seed_stage == stage_match;
        let dataset = match operation {
            MaintenanceOperation::SeedActors => Some(DatasetKind::ActorCatalog),
            _ => None,
        };
        let action_label = match dataset {
            Some(DatasetKind::ActorCatalog) if cfg!(target_os = "linux") => "Enter actor CSV path",
            _ => label,
        };
        let mut operation_card = rune::col().padding(10.0).gap(8.0).child(
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
                    ActionButton::new(action_label)
                        .key(key)
                        .icon(glyph)
                        .finish(finish)
                        .status(preflight_label, preflight_tone)
                        .busy(button_busy)
                        .disabled(reason.is_some())
                        .on_press_tx(move |state: &mut State, tx| {
                            state.seed_stage = stage_match;
                            match dataset {
                                Some(kind) if cfg!(target_os = "linux") => {
                                    state.enter_csv_path(kind, tx);
                                }
                                Some(kind) => state.choose_dataset(kind, tx),
                                None => state.preview_maintenance(operation),
                            }
                        })
                        .build(kit, preferences, pointer_ref),
                ),
        );
        if let Some(reason) = reason {
            operation_card = operation_card.child(quiet(reason, kit));
        } else if focused {
            operation_card = operation_card.child(quiet(explanation, kit));
        }
        if let Some(job_status) =
            operation_job_card_status(state, operation, preferences, kit, pointer_ref)
        {
            operation_card = operation_card.child(job_status);
        }
        if let Some(prompt) = &state.maintenance.csv_path_prompt {
            let matches_card = matches!(
                (operation, prompt.kind),
                (MaintenanceOperation::SeedActors, DatasetKind::ActorCatalog)
            );
            if matches_card {
                operation_card =
                    operation_card.child(csv_path_prompt(prompt, preferences, kit, pointer_ref));
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
            maintenance_confirmation_content(state, confirmation, preferences, kit),
        ));
    }
    if !matches!(state.maintenance.receipt, Load::Idle) {
        if let Some(message) = load_message(&state.maintenance.receipt, "", preferences, kit) {
            body = body.child(message);
        }
    }
    if let Some(unresolved) = unresolved_actor_reindex_banner(state, preferences, kit) {
        body = body.child(unresolved);
    }
    if let Some(message) = &state.maintenance.message {
        body = body.child(line(message.clone(), kit).role("status"));
    }
    let _ = layers;
    body
}

pub(crate) fn csv_path_prompt(
    prompt: &crate::maintenance::CsvPathPrompt,
    preferences: ViewPreferences,
    kit: &Kit,
    pointer_ref: Option<&rune::Live<[f32; 2]>>,
) -> El {
    let field = TextInput::new(&prompt.input)
        .label("Local CSV path")
        .placeholder("/home/you/actors.csv")
        .width(None)
        .on_edit(|state: &mut State| {
            &mut state
                .maintenance
                .csv_path_prompt
                .as_mut()
                .expect("path prompt is open while its field is shown")
                .input
        })
        .on_key(|state: &mut State, event, tx| {
            if event.key != Key::Escape {
                return false;
            }
            if !tx.key_repeat() {
                state.cancel_csv_path_prompt(tx);
            }
            true
        })
        .on_submit(|state: &mut State, _| state.submit_csv_path())
        .build(kit)
        .key(maintenance::CSV_PATH_KEY);
    let mut column = rune::col()
        .gap(8.0)
        .key("maintenance-csv-path-prompt")
        .child(quiet(
            "Local CSV path. The file is read on this computer and checked before upload. The path is not sent to the backend.",
            kit,
        ))
        .child(field);
    if let Some(error) = &prompt.error {
        column = column.child(line(error.clone(), kit).role("alert"));
    }
    let mut actions = rune::row()
        .gap(8.0)
        .child(
            ActionButton::new("Use this CSV")
                .key("maintenance-csv-path-submit")
                .finish(MetalFinish::Fractured)
                .on_press(|state: &mut State| state.submit_csv_path())
                .build(kit, preferences, pointer_ref),
        )
        .child(
            ActionButton::new("Cancel")
                .key("maintenance-csv-path-cancel")
                .finish(MetalFinish::Gunmetal)
                .on_press_tx(State::cancel_csv_path_prompt)
                .build(kit, preferences, pointer_ref),
        );
    if cfg!(target_os = "linux") {
        let kind = prompt.kind;
        actions = actions.child(
            ActionButton::new("Browse…")
                .key("maintenance-csv-path-browse")
                .finish(MetalFinish::Gunmetal)
                .on_press_tx(move |state: &mut State, tx| state.choose_dataset(kind, tx))
                .build(kit, preferences, pointer_ref),
        );
    }
    column.child(actions)
}

pub(crate) fn maintenance_confirmation_content(
    state: &State,
    confirmation: &crate::maintenance::MaintenanceConfirmation,
    preferences: ViewPreferences,
    kit: &Kit,
) -> El {
    let pointer_ref = (!preferences.matte()).then_some(&state.pointer_offset);
    let busy = state.maintenance.busy() || matches!(state.maintenance.receipt, Load::Loading);
    let current = state.maintenance_confirmation_matches(confirmation);
    let blocked = if current {
        state.maintenance_confirmation_preflight(confirmation.operation)
    } else {
        Some("The backend context or selected dataset changed. Cancel and review again.".into())
    };
    let refreshing = state.readiness_pending() || state.configuration_read_pending();
    let is_reindex = confirmation.operation == MaintenanceOperation::ReindexActors;
    let title = if is_reindex {
        "Build actor search index".to_owned()
    } else {
        format!("Confirm {:?}", confirmation.operation)
    };
    let confirm_label = if is_reindex {
        "Build index"
    } else {
        "Confirm operation"
    };
    let cancel_label = if is_reindex {
        "Cancel"
    } else {
        "Keep current backend state"
    };
    let mut content = rune::col()
        .gap(8.0)
        .key("maintenance-confirmation")
        .child(heading(title, kit))
        .child(quiet(
            format!(
                "Gateway {} · Deployment {} · Shared backend catalog · Revision {}",
                truncate_id(&state.connection.origin.text, 48),
                truncate_id(&confirmation.context.deployment_id, 28),
                confirmation.configuration_revision
            ),
            kit,
        )
        .label(format!(
            "Gateway {} Deployment {} Shared backend catalog Configuration revision {}",
            state.connection.origin.text,
            confirmation.context.deployment_id,
            confirmation.configuration_revision
        )))
        .child(quiet(
            confirmation.dataset.as_ref().map_or_else(
                || {
                    if is_reindex {
                        "Builds the shared backend-wide actor search index from the seeded actor catalog."
                            .into()
                    } else {
                        "No dataset is required for this operation.".into()
                    }
                },
                |dataset| {
                    format!(
                        "Selected dataset {} · SHA-256 {}",
                        truncate_id(&dataset.id, 24),
                        truncate_id(&dataset.sha256, 20)
                    )
                },
            ),
            kit,
        ));
    if let Some(reason) = &blocked {
        content = content.child(line(reason.clone(), kit).role("status"));
    }
    if blocked.is_some() && current && !busy {
        content = content.child(
            ActionButton::new("Refresh checks")
                .key("maintenance-confirm-refresh")
                .icon(&icons::REFRESH)
                .finish(MetalFinish::Gunmetal)
                .busy(refreshing)
                .disabled(!state.connected() || refreshing)
                .on_press(|state: &mut State| state.refresh_page())
                .build(kit, preferences, pointer_ref),
        );
    }
    content.child(
        rune::row()
            .gap(8.0)
            .child(
                ActionButton::new(confirm_label)
                    .key("maintenance-confirm")
                    .icon(&icons::SHIELD_CHECK)
                    .finish(MetalFinish::FracturedConfirm)
                    .status(
                        if busy {
                            "Working…"
                        } else if blocked.is_some() {
                            "Blocked"
                        } else {
                            "Ready"
                        },
                        if busy {
                            StatusTone::Accent
                        } else if blocked.is_some() {
                            StatusTone::Warn
                        } else {
                            StatusTone::Ok
                        },
                    )
                    .busy(busy)
                    .disabled(busy || blocked.is_some())
                    .on_press(|state: &mut State| state.confirm_maintenance())
                    .build(kit, preferences, pointer_ref),
            )
            .child(
                ActionButton::new(cancel_label)
                    .key("maintenance-cancel")
                    .finish(MetalFinish::Gunmetal)
                    .on_press_tx(State::dismiss_maintenance_confirmation)
                    .build(kit, preferences, pointer_ref),
            ),
    )
}

pub(crate) fn actor_index_receipt_banner(
    state: &State,
    preferences: ViewPreferences,
    kit: &Kit,
) -> Option<El> {
    let pointer_ref = (!preferences.matte()).then_some(&state.pointer_offset);
    if let Some(record) = state
        .maintenance
        .operation_job(MaintenanceOperation::ReindexActors)
    {
        let job_id = record.job_id.clone();
        let short_id = truncate_id(&job_id, 24);
        let (title, detail, tone) = match record.detail.as_ref() {
            Some(detail) if !crate::backend::job_is_active(&detail.job.state) => {
                let title = if detail.job.state == "completed" {
                    "Search index ready"
                } else {
                    detail.job.state.as_str()
                };
                (
                    title.to_owned(),
                    format!("Job {short_id}"),
                    state_tone(&detail.job.state),
                )
            }
            Some(detail) => (
                detail.job.state.clone(),
                format!("Job {short_id} · {}", detail.job.phase),
                state_tone(&detail.job.state),
            ),
            None => {
                let title = if record.reused {
                    "Already running"
                } else {
                    "Queued"
                };
                let detail = if record.reused {
                    format!("Job {short_id} is already active on the backend · Completion pending")
                } else {
                    format!("Accepted job {short_id} · Completion pending")
                };
                (title.to_owned(), detail, StatusTone::Ok)
            }
        };
        return Some(
            rune::col()
                .key("maintenance-accepted")
                .gap(8.0)
                .child(
                    ui_components::async_status_strip(
                        "maintenance-accepted-strip",
                        title.clone(),
                        detail,
                        false,
                        tone,
                        preferences,
                        kit,
                    )
                    .label(format!("{title}: Job {job_id}"))
                    .role("status"),
                )
                .child(
                    ActionButton::new("View job")
                        .key("maintenance-view-job")
                        .icon(&icons::JOBS)
                        .finish(MetalFinish::ObsidianPill)
                        .on_press(move |state: &mut State| state.open_job(&job_id))
                        .build(kit, preferences, pointer_ref),
                ),
        );
    }
    if let Load::Ready(receipt) = &state.maintenance.receipt {
        if receipt.operation != MaintenanceOperation::ReindexActors || receipt.already_completed {
            return None;
        }
        let title = if receipt.reused {
            "Already running"
        } else {
            "Queued"
        };
        let detail = if receipt.reused {
            format!(
                "Job {} is already active on the backend · Completion pending",
                truncate_id(&receipt.job_id, 24)
            )
        } else {
            format!(
                "Accepted job {} · Completion pending",
                truncate_id(&receipt.job_id, 24)
            )
        };
        let job_id = receipt.job_id.clone();
        return Some(
            rune::col()
                .key("maintenance-accepted")
                .gap(8.0)
                .child(
                    ui_components::async_status_strip(
                        "maintenance-accepted-strip",
                        title,
                        detail,
                        false,
                        StatusTone::Ok,
                        preferences,
                        kit,
                    )
                    .label(format!("{title}: Job {}", receipt.job_id))
                    .role("status"),
                )
                .child(
                    ActionButton::new("View job")
                        .key("maintenance-view-job")
                        .icon(&icons::JOBS)
                        .finish(MetalFinish::ObsidianPill)
                        .on_press(move |state: &mut State| state.open_job(&job_id))
                        .build(kit, preferences, pointer_ref),
                ),
        );
    }
    if let Some(association) = &state.maintenance.reindex_association {
        let job_id = association.job_id.clone();
        return Some(
            rune::col()
                .key("maintenance-accepted")
                .gap(8.0)
                .child(
                    ui_components::async_status_strip(
                        "maintenance-reconciled-strip",
                        "Already running",
                        format!(
                            "Recovered job {} for previous request",
                            truncate_id(&job_id, 24)
                        ),
                        false,
                        StatusTone::Ok,
                        preferences,
                        kit,
                    )
                    .label(format!("Recovered job {}", association.job_id))
                    .role("status"),
                )
                .child(
                    ActionButton::new("View job")
                        .key("maintenance-view-job")
                        .icon(&icons::JOBS)
                        .finish(MetalFinish::ObsidianPill)
                        .on_press(move |state: &mut State| state.open_job(&job_id))
                        .build(kit, preferences, pointer_ref),
                ),
        );
    }
    None
}

pub(crate) fn unresolved_actor_reindex_banner(
    state: &State,
    preferences: ViewPreferences,
    kit: &Kit,
) -> Option<El> {
    let pending = state.maintenance.pending_reindex.as_ref()?;
    let pointer_ref = (!preferences.matte()).then_some(&state.pointer_offset);
    let can_replay = state.pending_actor_reindex_matches_current() && !state.maintenance.busy();
    Some(card(
        state,
        preferences,
        StatusTone::Warn,
        rune::col()
            .gap(8.0)
            .key("reindex-unresolved-attempt")
            .child(heading("Actor index request unresolved", kit))
            .child(
                quiet(
                    format!(
                        "Request {} did not receive a confirmed receipt. Inspect request status before starting another build.",
                        truncate_id(&pending.request.request_id, 24)
                    ),
                    kit,
                )
                .label(format!(
                    "Unresolved actor index request {}",
                    pending.request.request_id
                )),
            )
            .child(
                rune::row()
                    .gap(8.0)
                    .child(
                        ActionButton::new("Check request")
                            .key("reindex-check-request")
                            .icon(&icons::REFRESH)
                            .finish(MetalFinish::GoldCta)
                            .disabled(state.maintenance.busy())
                            .on_press(|state: &mut State| state.reconcile_actor_reindex())
                            .build(kit, preferences, pointer_ref),
                    )
                    .child(
                        ActionButton::new("Retry exact request")
                            .key("reindex-retry-exact")
                            .finish(MetalFinish::Gunmetal)
                            .disabled(!can_replay)
                            .on_press(|state: &mut State| state.retry_pending_actor_reindex())
                            .build(kit, preferences, pointer_ref),
                    )
                    .child(
                        ActionButton::new("Clear stale attempt")
                            .key("reindex-clear-stale")
                            .finish(MetalFinish::ObsidianPill)
                            .disabled(state.maintenance.busy())
                            .on_press(|state: &mut State| state.clear_pending_actor_reindex())
                            .build(kit, preferences, pointer_ref),
                    ),
            ),
    ))
}

/// The one footer under the cycle history: a count, then a single local control for continuous
/// loading, as under the Jobs list. Rows and selection stay put while it changes.
fn cycles_tail(
    state: &State,
    loaded: usize,
    kit: &Kit,
    preferences: ViewPreferences,
    pointer_ref: Option<&rune::Live<[f32; 2]>>,
) -> El {
    let more = |label: &'static str| {
        ActionButton::new(label)
            .key("cycles-load-more")
            .finish(MetalFinish::ObsidianPill)
            .on_press(|state: &mut State| state.more_cycles())
    };
    let mut footer = rune::row()
        .key("cycles-tail")
        .w_full()
        .gap(8.0)
        .center_y()
        .child(quiet(
            format!(
                "{loaded} {}",
                if loaded == 1 {
                    "cycle loaded"
                } else {
                    "cycles loaded"
                }
            ),
            kit,
        ));
    if state.cycles.loading_more() {
        footer = footer.child(
            more("Loading more")
                .busy(true)
                .build(kit, preferences, pointer_ref),
        );
    } else if state.cycles_can_load_more() {
        let label = if state.cycles.read_error.is_some() {
            "Retry"
        } else {
            "Load more"
        };
        footer = footer.child(more(label).build(kit, preferences, pointer_ref));
        if state.cycles.read_error.is_none() {
            // A one-pixel sentinel, new for every loaded count: coming on screen loads the next
            // segment, and a segment that leaves it on screen loads another.
            footer = footer.child(
                rune::el()
                    .key(format!("cycles-near-end-{loaded}"))
                    .size(1.0, 1.0)
                    .a11y_hidden()
                    .on_visible(|state: &mut State, on_screen, _| state.cycles_near_end(on_screen)),
            );
        }
    } else if matches!(&state.cycles.history, Load::Ready(history) if history.next_cursor.is_some())
    {
        footer = footer.child(quiet(
            "View limit reached. Newer cycles load as they arrive.",
            kit,
        ));
    }
    footer
}

/// The one footer under the Jobs list: a count, then a single local control for continuous
/// loading. Rows, selection and scroll position stay put while it changes.
fn jobs_tail(
    state: &State,
    loaded: usize,
    kit: &Kit,
    preferences: ViewPreferences,
    pointer_ref: Option<&rune::Live<[f32; 2]>>,
) -> El {
    let jobs = &state.scoped_jobs;
    let more = |label: &'static str| {
        ActionButton::new(label)
            .key("jobs-load-more")
            .finish(MetalFinish::ObsidianPill)
            .on_press(|state: &mut State| state.request_jobs(crate::jobs_query::JobsEvent::More))
    };
    let mut footer = rune::row()
        .key("jobs-tail")
        .w_full()
        .gap(8.0)
        .center_y()
        .child(quiet(
            format!(
                "{loaded} {}",
                if loaded == 1 {
                    "job loaded"
                } else {
                    "jobs loaded"
                }
            ),
            kit,
        ));
    if jobs.loading_more() {
        footer = footer.child(
            more("Loading more")
                .busy(true)
                .build(kit, preferences, pointer_ref),
        );
    } else if let Some(reason) = jobs.tail_error() {
        footer = footer
            .child(ui_components::status_pill(
                "Could not load more",
                None,
                StatusTone::Warn,
                kit,
            ))
            .child(more("Retry").build(kit, preferences, pointer_ref))
            .child(quiet(reason.to_owned(), kit));
    } else if jobs.can_load_more() {
        footer = footer
            .child(more("Load more").build(kit, preferences, pointer_ref))
            // A one-pixel sentinel, new for every loaded count: when it comes on screen the next
            // segment loads, and a segment that leaves it on screen loads another.
            .child(
                rune::el()
                    .key(format!("jobs-near-end-{loaded}"))
                    .size(1.0, 1.0)
                    .a11y_hidden()
                    .on_visible(|state: &mut State, on_screen, _| state.jobs_near_end(on_screen)),
            );
    } else if jobs.at_bound() {
        footer = footer.child(quiet(
            "Showing the newest jobs. Narrow the filters to see others.",
            kit,
        ));
    }
    footer
}

#[cfg(test)]
mod tools_tests {
    use rune::Env;
    use rune::Runtime;

    use crate::app;
    use crate::app::Load;
    use crate::app::Page;
    use crate::backend::ProfileSnapshot;
    use crate::backend::ToolCatalog;
    use crate::backend::ToolCredentialMetadata;
    use crate::backend::ToolDefinition;

    #[test]
    fn tool_box_keeps_provider_controls_in_the_responsive_overview() {
        for (size, wide) in [((640.0, 480.0), false), ((1440.0, 860.0), true)] {
            let mut state = crate::tests::registered_fixture();
            state.page = Page::Tools;
            state.backend.profile = Load::Ready(ProfileSnapshot {
                configuration_revision: 1,
                configuration_format: 4,
                accounts: Vec::new(),
                tool_credentials: vec![ToolCredentialMetadata {
                    provider: "newsdata".into(),
                    primary_present: true,
                    fallback_present: false,
                }],
                setup_required: false,
                portable_schema_version: 4,
            });
            state.backend.tools = Load::Ready(ToolCatalog {
                tools: vec![ToolDefinition {
                    id: "fixture".into(),
                    name: "Fixture tool".into(),
                    category: "News".into(),
                    description: "Synthetic tool".into(),
                    inputs: Vec::new(),
                    restrictions: String::new(),
                    timeout_seconds: 1,
                }],
            });
            let mut runtime = Runtime::new(
                state,
                app::view,
                size,
                Env {
                    system_dark: true,
                    scale: 1.0,
                },
            );
            runtime.frame(0.0);
            runtime.frame(1.0 / 60.0);

            let overview = runtime.find("tool-box-overview-title").unwrap();
            let catalog = runtime.find("tool-fixture").unwrap();
            assert_eq!(runtime.find("tool-box-overview-scroll").is_some(), wide);
            if wide {
                assert!(overview.0 > catalog.0);
            } else {
                assert!(overview.1 < catalog.1);
            }
            let (x, y) = runtime.find("tool-key-open-newsdata").unwrap();
            assert!((0.0..size.0).contains(&x) && (0.0..size.1).contains(&y));
            runtime.pointer_down(x, y);
            runtime.pointer_up(x, y);
            runtime.frame(2.0 / 60.0);
            assert!(runtime.state.tool_key_dialog);
            assert_eq!(runtime.state.configuration.tool_provider.text, "newsdata");
            assert!(runtime.find("tool-key-primary").is_some());
            assert!(runtime.find("tool-key-fallback").is_some());
        }
    }
}
