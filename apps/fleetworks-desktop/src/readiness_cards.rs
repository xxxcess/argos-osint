//! Shared status check summaries, disclosures, and admitted repairs.

use std::fmt::Write as _;

use fleetworks_contracts::context::Permission;
use fleetworks_contracts::readiness::DatasetKind;
use fleetworks_contracts::readiness::MaintenanceOperation;
use fleetworks_contracts::readiness::ReadinessActionId;
use fleetworks_contracts::readiness::ReadinessCheck;
use fleetworks_contracts::readiness::ReadinessCheckId;
use fleetworks_contracts::readiness::ReadinessState;
use fleetworks_contracts::readiness::WorkstationReadiness;
use rune::El;
use rune::Tx;
use rune_kit::glyphs;
use rune_kit::ControlSize;
use rune_kit::IconButton;
use rune_kit::IconButtonVariant;
use rune_kit::Kit;

use crate::accessibility::ViewPreferences;
use crate::app::Load;
use crate::app::Page;
use crate::app::SeedStage;
use crate::app::State;
use crate::icons;
use crate::material::MetalFinish;
use crate::theme::StatusTone;
use crate::ui_components;
use crate::ui_components::ActionButton;
use crate::views::line;
use crate::views::quiet;
use crate::views::unsynced_glyph;

pub(crate) const ACTOR_INDEX_CATCH_UP: &str = "Actor index catching up in the background";

impl State {
    pub(crate) fn readiness_check_applies(&self, check: &ReadinessCheck) -> bool {
        // Older Embedded companions used Unknown for absent standalone probes.
        check.state != ReadinessState::NotApplicable
            && !(self.is_local_backend()
                && check.id.requires_distributed_backend()
                && check.state == ReadinessState::Unknown
                && check.reason_code == "owner_not_configured")
    }

    pub(crate) fn readiness_check_fresh(&self, check: &ReadinessCheck) -> bool {
        self.readiness_check_fresh_at(check, self.clock)
    }

    pub(crate) fn readiness_check_fresh_at(&self, check: &ReadinessCheck, now: f64) -> bool {
        check.fresh_for_ms > 0
            && self.readiness_received_at.is_some_and(|received| {
                now >= received && (now - received) * 1000.0 <= check.fresh_for_ms as f64
            })
    }
}

fn status_checks<'a>(
    state: &'a State,
    readiness: &'a WorkstationReadiness,
) -> impl Iterator<Item = &'a ReadinessCheck> {
    readiness.checks.iter().filter(move |check| {
        check.id != ReadinessCheckId::ClassifierTraining && state.readiness_check_applies(check)
    })
}

pub(crate) fn workstation_health(state: &State) -> (&'static str, StatusTone) {
    if state.local_backend.pressure_paused.is_some() && state.is_local_backend() {
        return ("Resource pressure", StatusTone::Warn);
    }
    if state.registration.context.is_none() {
        return ("Registration required", StatusTone::Warn);
    }
    match &state.readiness {
        Load::Idle => ("Health unchecked", StatusTone::Neutral),
        Load::Pending | Load::Loading => ("Checking health", StatusTone::Info),
        Load::Failed(_) => ("Health checks unavailable", StatusTone::Warn),
        Load::Ready(readiness) => {
            let mut observed = false;
            let mut unresolved = false;
            let mut attention = false;
            for check in status_checks(state, readiness) {
                observed = true;
                if !state.readiness_check_fresh(check) {
                    unresolved = true;
                    continue;
                }
                let status = CheckStatus::of(check, true);
                if status == CheckStatus::Failed {
                    return ("Service failure", StatusTone::Err);
                }
                attention |= status.is_blocker();
                unresolved |= status != CheckStatus::Ready;
            }
            if attention {
                ("Needs attention", StatusTone::Warn)
            } else if state.readiness_error.is_some() {
                ("Health checks unsynced", StatusTone::Warn)
            } else if !observed {
                ("Health unchecked", StatusTone::Neutral)
            } else if unresolved {
                ("Checks unresolved", StatusTone::Neutral)
            } else {
                ("Workstation ready", StatusTone::Ok)
            }
        }
    }
}

pub(crate) fn workstation_section(state: &State, preferences: ViewPreferences, kit: &Kit) -> El {
    let open = state.workstation_status_checks_open;
    let connected = state.connected();
    let summary = if connected {
        summary_label(state)
    } else {
        "Status checks · offline".to_owned()
    };
    rune::col()
        .key("status-checks-toggle")
        .w_full()
        .gap(6.0)
        .child(
            section_disclosure_trigger(summary, open, kit).on_press(|state: &mut State| {
                state.workstation_status_checks_open = !state.workstation_status_checks_open;
            }),
        )
        .children(open.then(|| {
            if connected {
                checks(state, preferences, kit)
            } else {
                quiet("Connect a backend to see status checks.", kit)
            }
        }))
}

pub(crate) fn setup_section(state: &State, preferences: ViewPreferences, kit: &Kit) -> El {
    rune::col()
        .w_full()
        .gap(6.0)
        .child(ui_components::inline_toggle_row(
            "setup-advanced-toggle",
            state.setup_advanced_open,
            summary_label(state),
            Some("Show individual checks and their details"),
            false,
            kit,
            |state: &mut State| state.setup_advanced_open = !state.setup_advanced_open,
        ))
        .children(
            state
                .setup_advanced_open
                .then(|| checks(state, preferences, kit)),
        )
}

impl State {
    pub(crate) fn reveal_readiness_check(&mut self, id: ReadinessCheckId, tx: &mut Tx) {
        if self.setup_gateway {
            self.setup_advanced_open = true;
        } else {
            self.workstation_status_checks_open = true;
        }
        self.expanded_readiness.insert(id);
        self.selected_readiness = Some(id.as_str().to_owned());
        let trigger = check_trigger_key(id);
        tx.scroll_into_view(trigger.clone());
        tx.focus(trigger, tx.focus_visible());
    }
}

fn check_trigger_key(id: ReadinessCheckId) -> String {
    format!("readiness-trigger-{}", id.as_str())
}

fn summary_label(state: &State) -> String {
    let Load::Ready(readiness) = &state.readiness else {
        let progress = match state.readiness {
            Load::Idle => "not observed",
            Load::Pending | Load::Loading => "checking",
            Load::Failed(_) => "refresh failed",
            Load::Ready(_) => unreachable!(),
        };
        return format!("Status checks · {progress}");
    };
    if readiness.checks.is_empty() {
        return "Status checks · no observations".to_owned();
    }
    let (mut passed, mut failed, mut unresolved) = (0, 0, 0);
    for check in status_checks(state, readiness) {
        let fresh = state.readiness_check_fresh(check);
        match CheckStatus::of(check, fresh) {
            CheckStatus::Ready => passed += 1,
            CheckStatus::Failed if fresh => failed += 1,
            _ => unresolved += 1,
        }
    }
    if passed + failed + unresolved == 0 {
        return "Status checks · none applicable".to_owned();
    }
    let mut label = "Status checks".to_owned();
    for (count, caption) in [
        (passed, "passed"),
        (failed, "failed"),
        (unresolved, "unresolved"),
    ] {
        if count > 0 {
            let _ = write!(label, " · {count} {caption}");
        }
    }
    label
}

const STATUS_COLUMN_WIDTH: f32 = 148.0;
const ACTION_COLUMN_WIDTH: f32 = 52.0;

struct StatusCheckRow<'a> {
    check: &'a ReadinessCheck,
    fresh: bool,
    status: CheckStatus,
    primary: Option<ReadinessActionId>,
}

fn checks(state: &State, preferences: ViewPreferences, kit: &Kit) -> El {
    let mut body = rune::col()
        .key("backend-readiness-checks")
        .w_full()
        .min_w(0.0)
        .gap(4.0);
    let readiness = match &state.readiness {
        Load::Idle => {
            return body
                .child(quiet(
                    "Refresh checks to inspect current backend health.",
                    kit,
                ))
                .child(refresh_button("readiness-retry", state, preferences, kit));
        }
        Load::Pending | Load::Loading => {
            return body.child(ui_components::skeleton_readiness_list(preferences, kit));
        }
        Load::Failed(error) => {
            return body
                .child(
                    rune::text("Status checks unavailable")
                        .font(kit.ty.small)
                        .color(kit.c.std_text)
                        .role("alert")
                        .description(error.clone()),
                )
                .child(refresh_button(
                    "workstation-readiness-retry",
                    state,
                    preferences,
                    kit,
                ));
        }
        Load::Ready(readiness) => readiness,
    };
    if let Some(error) = &state.readiness_error {
        body = body.child(
            rune::row()
                .key("readiness-update-failed")
                .center_y()
                .gap(6.0)
                .child(unsynced_glyph(
                    "readiness-refresh-unsynced",
                    format!("Status check refresh failed: {error}"),
                    kit,
                ))
                .child(refresh_button("readiness-retry", state, preferences, kit)),
        );
    }
    if readiness.checks.is_empty() {
        return body.child(quiet("No check observations", kit));
    }
    let mut rows: Vec<_> = status_checks(state, readiness)
        .map(|check| {
            let fresh = state.readiness_check_fresh(check);
            let status = if crate::views::actor_search_is_background_indexing(state, check) {
                CheckStatus::BackgroundIndexing
            } else {
                CheckStatus::of(check, fresh)
            };
            StatusCheckRow {
                check,
                fresh,
                status,
                primary: if fresh && status.is_blocker() {
                    admitted_action(state, check)
                } else {
                    None
                },
            }
        })
        .collect();
    if rows.is_empty() {
        return body.child(quiet("No applicable status checks", kit));
    }
    rows.sort_by_key(|row| {
        (
            row.status != CheckStatus::Failed,
            row.primary.is_none(),
            row.status.rank(),
            row.check.id,
        )
    });
    let heading = |title: &'static str| {
        rune::text(title)
            .role("columnheader")
            .font(kit.ty.text_xsmall)
            .color(kit.c.text_muted)
            .wrap()
    };
    let header = table_cells(
        heading("Service").pl(8.0),
        heading("Status"),
        heading("Action"),
    )
    .key("readiness-status-table-header")
    .role("row")
    .min_h(24.0);
    body.child(
        rune::col()
            .key("readiness-status-table")
            .role("table")
            .label("Service status checks")
            .w_full()
            .min_w(0.0)
            .gap(3.0)
            .child(header)
            .children(
                rows.into_iter()
                    .map(|row| check_row(state, row, preferences, kit)),
            ),
    )
}

fn table_cells(service: El, status: El, action: El) -> El {
    rune::row()
        .w_full()
        .min_w(0.0)
        .gap(8.0)
        .center_y()
        .child(service.grow(1.0).min_w(0.0))
        .child(status.w(STATUS_COLUMN_WIDTH))
        .child(rune::row().w(ACTION_COLUMN_WIDTH).child(action))
}

fn row_icon(icon: &'static rune::Icon, label: &str) -> IconButton {
    IconButton::new(icon)
        .variant(IconButtonVariant::Ghost)
        .control_size(ControlSize::Large)
        .label(label)
        .help(label)
}

fn toggle_details(state: &mut State, id: ReadinessCheckId) {
    let check_id = id.as_str();
    if state.expanded_readiness.remove(&id) {
        if state.selected_readiness.as_deref() == Some(check_id) {
            state.selected_readiness = None;
        }
    } else {
        state.expanded_readiness.insert(id);
        state.selected_readiness = Some(check_id.to_owned());
    }
}

fn open_actor_maintenance(state: &mut State, tx: &mut Tx) {
    if state
        .maintenance_disabled_reason(MaintenanceOperation::SeedActors)
        .is_none()
    {
        state.seed_stage = SeedStage::Seed;
        state.navigate(Page::Seed);
        tx.focus(crate::maintenance::SEED_ACTION_KEY, tx.focus_visible());
    }
}

fn workstation_row_action(state: &State, row: &StatusCheckRow<'_>, kit: &Kit) -> Option<El> {
    let id = row.check.id;
    if state.page != Page::Workstation || state.setup_gateway {
        return None;
    }
    let registered = state.registration.context.is_some();
    let can_read_jobs = registered && state.permission(Permission::ReadWorkstationActivity);
    if id == ReadinessCheckId::ActorSearch && state.maintenance.pending_reindex.is_some() {
        return Some(
            row_icon(&icons::REFRESH, "Check actor index request")
                .disabled(state.maintenance.busy())
                .on_press(State::reconcile_actor_reindex)
                .build(kit)
                .key("workstation-check-index-request"),
        );
    }
    let active_job = match id {
        ReadinessCheckId::ActorCatalog => state
            .maintenance
            .operation_job(MaintenanceOperation::SeedActors)
            .filter(|record| {
                record
                    .detail
                    .as_ref()
                    .is_none_or(|detail| crate::backend::job_is_active(&detail.job.state))
            })
            .map(|record| record.job_id.as_str()),
        ReadinessCheckId::ActorSearch => crate::views::active_actor_reindex(state)
            .map(|job| job.id.as_str())
            .or_else(|| crate::views::queued_actor_reindex_id(state)),
        _ => None,
    };
    if let Some(job_id) = active_job.filter(|_| can_read_jobs) {
        let job_id = job_id.to_owned();
        let label = if id == ReadinessCheckId::ActorSearch {
            "View actor index job"
        } else {
            "View actor seed job"
        };
        return Some(
            row_icon(&icons::JOBS, label)
                .on_press(move |state: &mut State| {
                    if state.permission(Permission::ReadWorkstationActivity) {
                        state.open_job(&job_id);
                    }
                })
                .build(kit)
                .key(format!("readiness-job-{}", id.as_str())),
        );
    }
    match id {
        ReadinessCheckId::ActorSearch
            if row.status == CheckStatus::Ready
                && state.permission(Permission::ReadBackend)
                && state
                    .connection
                    .capabilities
                    .as_ref()
                    .is_some_and(|capabilities| capabilities.require_actor_search().is_ok()) =>
        {
            Some(
                row_icon(kit.glyph.search, "Search actors")
                    .on_press(|state: &mut State| {
                        if state.permission(Permission::ReadBackend) {
                            state.navigate(Page::Brain);
                        }
                    })
                    .build(kit)
                    .key("workstation-search-actors"),
            )
        }
        ReadinessCheckId::Jobs if row.status == CheckStatus::Ready && can_read_jobs => Some(
            row_icon(&icons::JOBS, "Open Jobs & Logs")
                .on_press(|state: &mut State| {
                    if state.registration.context.is_some()
                        && state.permission(Permission::ReadWorkstationActivity)
                    {
                        state.select_activity_tab(crate::logs::ActivityTab::Jobs);
                        state.navigate(Page::Jobs);
                    }
                })
                .build(kit)
                .key("readiness-open-jobs"),
        ),
        _ => None,
    }
}

fn check_action(state: &State, row: &StatusCheckRow<'_>, expanded: bool, kit: &Kit) -> El {
    if let Some(action) = workstation_row_action(state, row, kit) {
        return action;
    }
    let id = row.check.id;
    match row.primary {
        Some(ReadinessActionId::RebuildActorIndex) => row_icon(&icons::BRAIN, "Build actor index")
            .on_press(|state: &mut State| {
                state.preview_maintenance(MaintenanceOperation::ReindexActors);
            })
            .build(kit)
            .key(crate::maintenance::WORKSTATION_REINDEX_ACTION_KEY),
        Some(ReadinessActionId::SeedActors)
            if state.page == Page::Workstation && !state.setup_gateway =>
        {
            row_icon(&icons::SEED, "Open actor seed setup")
                .on_press_tx(open_actor_maintenance)
                .build(kit)
                .key("readiness-action-actor_catalog-SeedActors")
        }
        Some(ReadinessActionId::SeedActors) => row_icon(&icons::SEED, "Choose actor CSV")
            .on_press_tx(|state: &mut State, tx| {
                state.choose_dataset(DatasetKind::ActorCatalog, tx)
            })
            .build(kit)
            .key("readiness-action-actor_catalog-SeedActors"),
        _ => {
            let label = if expanded {
                format!("Hide {} details", title(id))
            } else {
                format!("View {} details", title(id))
            };
            row_icon(kit.glyph.eye, &label)
                .expanded(expanded)
                .on_press(move |state: &mut State| toggle_details(state, id))
                .build(kit)
                .key(format!("readiness-action-{}", id.as_str()))
        }
    }
}

fn check_row(
    state: &State,
    row: StatusCheckRow<'_>,
    preferences: ViewPreferences,
    kit: &Kit,
) -> El {
    let check = row.check;
    let status = row.status;
    let primary = row.primary;
    let id = check.id;
    let check_id = id.as_str();
    let expanded = state.expanded_readiness.contains(&id);
    let name = title(id);
    let observed = !check.observed_at.is_empty() || !check.reason_code.is_empty();
    let stale_reason = (!row.fresh)
        .then(|| format!("{name} observation unsynced. Open details for the last observation."));
    let label = format!("{name}: {}", status.label());
    let mut service = disclosure_trigger(check_trigger_key(id), label, expanded, kit)
        .child(disclosure_text(name, kit))
        .child(chevron(expanded, kit))
        .on_press(move |state: &mut State| toggle_details(state, id));
    if let Some(description) = &stale_reason {
        service = service.description(description.clone());
    }

    let tone = if stale_reason.is_some() {
        StatusTone::Neutral
    } else {
        status.tone()
    };
    let status_cell = rune::row()
        .key(format!("readiness-status-cell-{check_id}"))
        .center_y()
        .gap(5.0)
        .child(status_tag(status, check_id, tone, kit))
        .children(stale_reason.map(|description| {
            unsynced_glyph(&format!("readiness-unsynced-{check_id}"), description, kit)
                .a11y_hidden()
        }));
    let action = check_action(state, &row, expanded, kit);
    let mut row = rune::col()
        .key(format!("readiness-{check_id}"))
        .role("row")
        .w_full()
        .min_w(0.0)
        .child(table_cells(service, status_cell, action));
    if expanded {
        let mut detail = rune::col()
            .key(format!("readiness-details-{check_id}"))
            .role("region")
            .label(format!("{name} details"))
            .w_full()
            .min_w(0.0)
            .px(8.0)
            .pb(6.0)
            .gap(4.0)
            .children((!check.explanation.is_empty()).then(|| line(check.explanation.clone(), kit)))
            .children(observed.then(|| {
                quiet(
                    format!("Observed {} · {}", check.observed_at, check.reason_code),
                    kit,
                )
            }));
        if !observed
            && check.explanation.is_empty()
            && check.job_ids.is_empty()
            && check.artifact_ids.is_empty()
            && check.actions.is_empty()
        {
            detail = detail.child(quiet("No additional details were reported.", kit));
        }
        if status == CheckStatus::BackgroundIndexing {
            detail = detail.child(quiet(ACTOR_INDEX_CATCH_UP, kit));
        }
        if id == ReadinessCheckId::ActorSearch && !state.setup_gateway {
            detail = detail
                .children(crate::backend_views::actor_index_receipt_banner(
                    state,
                    preferences,
                    kit,
                ))
                .children(crate::backend_views::unresolved_actor_reindex_banner(
                    state,
                    preferences,
                    kit,
                ));
        }
        if !check.job_ids.is_empty() {
            detail = detail.child(quiet(format!("Jobs: {}", check.job_ids.join(", ")), kit));
        }
        if !check.artifact_ids.is_empty() {
            detail = detail.child(quiet(
                format!("Serving refs: {}", check.artifact_ids.join(", ")),
                kit,
            ));
        }
        for action in check
            .actions
            .iter()
            .filter(|action| Some(action.id) != primary)
        {
            let disabled_operation = maintenance_action(action.id)
                .filter(|_| !action.allowed || !state.permission(action.permission));
            if let Some((operation, _)) = disabled_operation {
                detail = detail.children(
                    state
                        .maintenance_disabled_reason(operation)
                        .or_else(|| action.disabled_reason.clone())
                        .map(|reason| quiet(reason, kit)),
                );
            } else if let Some((button, reason)) =
                maintenance_button(state, preferences, kit, check.id, action.id)
            {
                detail = detail
                    .child(button)
                    .children(reason.map(|reason| quiet(reason, kit)));
            }
        }
        row = row.child(detail);
    }
    row
}

fn status_tag(status: CheckStatus, check_id: &str, tone: StatusTone, kit: &Kit) -> El {
    rune::row()
        .key(format!("readiness-status-{check_id}"))
        .role("status")
        .label(status.label())
        .center_y()
        .gap(5.0)
        .h(22.0)
        .px(7.0)
        .radius(11.0)
        .fill(tone.bg())
        .border(1.0, tone.border())
        .child(
            rune::icon(status.glyph(), 12.0)
                .key(format!("readiness-icon-{check_id}"))
                .color(tone.fg())
                .a11y_hidden(),
        )
        .child(
            rune::text(status.label())
                .font(kit.ty.text_xsmall)
                .color(tone.fg())
                .a11y_hidden(),
        )
}

pub(crate) fn section_disclosure_trigger(title: impl Into<String>, open: bool, kit: &Kit) -> El {
    let title = title.into();
    disclosure_trigger("trigger", title.clone(), open, kit)
        .fill(kit.c.layer_01)
        .child(disclosure_text(title, kit))
        .child(chevron(open, kit))
}

fn disclosure_trigger(key: impl Into<String>, label: String, expanded: bool, kit: &Kit) -> El {
    rune::row()
        .key(key)
        .role("button")
        .label(label)
        .expanded(expanded)
        .focusable(true)
        .focus_ring(2.0, 0.0, kit.c.border_focus)
        .radius(6.0)
        .hover(kit.c.layer_02)
        .w_full()
        .min_w(0.0)
        .min_h(32.0)
        .px(8.0)
        .py(4.0)
        .gap(6.0)
        .center_y()
}

fn disclosure_text(text: impl Into<String>, kit: &Kit) -> El {
    rune::text(text.into())
        .font(kit.ty.text_small_medium)
        .color(kit.c.std_text)
        .grow(1.0)
        .min_w(0.0)
        .wrap()
        .a11y_hidden()
}

fn chevron(expanded: bool, kit: &Kit) -> El {
    rune::icon(
        if expanded {
            &icons::CHEVRON_DOWN
        } else {
            &icons::CHEVRON_RIGHT
        },
        14.0,
    )
    .color(kit.c.icon_weaker)
    .a11y_hidden()
}

fn refresh_button(key: &str, state: &State, preferences: ViewPreferences, kit: &Kit) -> El {
    ActionButton::new("Refresh checks")
        .key(key)
        .icon(&icons::REFRESH)
        .finish(MetalFinish::Gunmetal)
        .busy(state.readiness_pending())
        .disabled(state.readiness_pending())
        .on_press(|state: &mut State| state.refresh_readiness())
        .build(kit, preferences, None)
}

/// Keep the highest-priority observed blocker and a safe next action above the disclosure.
pub(crate) enum PrimaryIssueScope {
    GatewaySetup,
    Workstation { include_other_blockers: bool },
}

pub(crate) fn primary_issue(
    state: &State,
    scope: PrimaryIssueScope,
    preferences: ViewPreferences,
    kit: &Kit,
) -> Option<El> {
    let setup = matches!(scope, PrimaryIssueScope::GatewaySetup);
    let include_other_blockers = match scope {
        PrimaryIssueScope::GatewaySetup => true,
        PrimaryIssueScope::Workstation {
            include_other_blockers,
        } => include_other_blockers,
    };
    if setup && matches!(state.readiness, Load::Failed(_)) {
        return Some(
            rune::row()
                .key("readiness-critical")
                .w_full()
                .flex_wrap()
                .center_y()
                .gap(6.0)
                .child(line("Status checks unavailable", kit))
                .child(refresh_button(
                    "readiness-critical-action",
                    state,
                    preferences,
                    kit,
                )),
        );
    }
    let Load::Ready(readiness) = &state.readiness else {
        return None;
    };
    let (check, status) = status_checks(state, readiness)
        .filter(|check| {
            check.id != ReadinessCheckId::Registration
                && (setup
                    || !matches!(
                        check.id,
                        ReadinessCheckId::ActorCatalog | ReadinessCheckId::ActorSearch
                    ))
        })
        .filter_map(|check| {
            if !state.readiness_check_fresh(check) {
                return None;
            }
            let status = CheckStatus::of(check, true);
            (status == CheckStatus::Failed || include_other_blockers && status.is_blocker())
                .then_some((check, status))
        })
        .min_by_key(|(check, status)| (status.rank(), check.id))?;
    let id = check.id;
    let button = admitted_action(state, check)
        .and_then(|action| maintenance_button(state, preferences, kit, id, action))
        .map(|(button, _)| button.key("readiness-critical-action"))
        .unwrap_or_else(|| {
            if status == CheckStatus::Failed || status == CheckStatus::Unavailable {
                return refresh_button("readiness-critical-action", state, preferences, kit);
            }
            ActionButton::new("Inspect check")
                .key("readiness-critical-action")
                .finish(MetalFinish::Gunmetal)
                .on_press_tx(move |state: &mut State, tx| state.reveal_readiness_check(id, tx))
                .build(kit, preferences, None)
        });
    Some(
        rune::row()
            .key("readiness-critical")
            .w_full()
            .flex_wrap()
            .center_y()
            .gap(6.0)
            .child(status_icon(status, "status".to_owned()))
            .child(line(format!("{} · {}", title(id), status.label()), kit))
            .child(button),
    )
}

fn maintenance_action(action: ReadinessActionId) -> Option<(MaintenanceOperation, &'static str)> {
    Some(match action {
        ReadinessActionId::SeedActors => (MaintenanceOperation::SeedActors, "Seed actors…"),
        ReadinessActionId::RebuildActorIndex => {
            (MaintenanceOperation::ReindexActors, "Build actor index")
        }
        _ => return None,
    })
}

fn admitted_action(state: &State, check: &ReadinessCheck) -> Option<ReadinessActionId> {
    check
        .actions
        .iter()
        .filter(|action| action.allowed && state.permission(action.permission))
        .find_map(|action| {
            let (operation, _) = maintenance_action(action.id)?;
            state
                .maintenance_disabled_reason(operation)
                .is_none()
                .then_some(action.id)
        })
}

fn maintenance_button(
    state: &State,
    preferences: ViewPreferences,
    kit: &Kit,
    check: ReadinessCheckId,
    action: ReadinessActionId,
) -> Option<(El, Option<String>)> {
    let (operation, caption) = maintenance_action(action)?;
    let reason = state.maintenance_disabled_reason(operation);
    let button = ActionButton::new(caption)
        .key(format!("readiness-action-{}-{action:?}", check.as_str()))
        .finish(MetalFinish::Gunmetal)
        .disabled(reason.is_some())
        .on_press_tx(move |state: &mut State, tx| match operation {
            MaintenanceOperation::SeedActors
                if state.page == Page::Workstation && !state.setup_gateway =>
            {
                open_actor_maintenance(state, tx);
            }
            MaintenanceOperation::SeedActors => state.choose_dataset(DatasetKind::ActorCatalog, tx),
            MaintenanceOperation::ReindexActors => state.preview_maintenance(operation),
            _ => {}
        })
        .build(
            kit,
            preferences,
            (!preferences.matte()).then_some(&state.pointer_offset),
        );
    Some((button, reason))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CheckStatus {
    Failed,
    NeedsData,
    NotReady,
    Degraded,
    Unavailable,
    Checking,
    BackgroundIndexing,
    Unknown,
    RememberedReady,
    Ready,
}

impl CheckStatus {
    fn of(check: &ReadinessCheck, fresh: bool) -> Self {
        match check.state {
            ReadinessState::Ready if fresh => Self::Ready,
            ReadinessState::Ready => Self::RememberedReady,
            ReadinessState::NotReady if check.reason_code == "process_not_ready" => Self::Failed,
            ReadinessState::NotReady if check.id == ReadinessCheckId::ActorCatalog => {
                Self::NeedsData
            }
            ReadinessState::NotReady => Self::NotReady,
            ReadinessState::Degraded => Self::Degraded,
            ReadinessState::Running => Self::Checking,
            ReadinessState::Unavailable => Self::Unavailable,
            _ => Self::Unknown,
        }
    }

    fn is_blocker(self) -> bool {
        matches!(
            self,
            Self::Failed | Self::NeedsData | Self::NotReady | Self::Degraded | Self::Unavailable
        )
    }

    fn label(self) -> &'static str {
        match self {
            Self::Failed => "Failed",
            Self::NeedsData => "Needs data",
            Self::NotReady => "Not ready",
            Self::Degraded => "Degraded",
            Self::Unavailable => "Unavailable",
            Self::Checking => "Checking",
            Self::BackgroundIndexing => "Background indexing",
            Self::Unknown => "Unknown",
            Self::RememberedReady => "Last ready",
            Self::Ready => "Ready",
        }
    }

    fn rank(self) -> u8 {
        match self {
            Self::Failed => 0,
            Self::NeedsData | Self::NotReady | Self::Degraded => 1,
            Self::Unavailable => 2,
            Self::Checking | Self::BackgroundIndexing => 3,
            Self::Unknown | Self::RememberedReady => 4,
            Self::Ready => 5,
        }
    }

    fn tone(self) -> StatusTone {
        match self {
            Self::Failed => StatusTone::Err,
            Self::NeedsData | Self::NotReady | Self::Degraded => StatusTone::Warn,
            Self::Checking | Self::BackgroundIndexing => StatusTone::Neutral,
            Self::Ready => StatusTone::Ok,
            Self::Unavailable | Self::Unknown | Self::RememberedReady => StatusTone::Neutral,
        }
    }
    fn glyph(self) -> &'static rune::Icon {
        match self {
            Self::Failed => &glyphs::CLOSE,
            Self::NeedsData | Self::NotReady | Self::Degraded => &icons::ALERT,
            Self::Checking | Self::BackgroundIndexing => &icons::JOBS,
            Self::Ready => &glyphs::CHECK,
            Self::Unavailable | Self::Unknown | Self::RememberedReady => &glyphs::DASH,
        }
    }
}

fn status_icon(status: CheckStatus, key: String) -> El {
    rune::icon(status.glyph(), 14.0)
        .key(key)
        .color(status.tone().fg())
        .a11y_hidden()
}

fn title(check: ReadinessCheckId) -> &'static str {
    match check {
        ReadinessCheckId::Gateway => "Gateway",
        ReadinessCheckId::Datastore => "Datastore",
        ReadinessCheckId::Jobs => "Jobs",
        ReadinessCheckId::Brain => "Brain",
        ReadinessCheckId::Control => "Backend control",
        ReadinessCheckId::Nlp => "Language service",
        ReadinessCheckId::Classifier => "Classifier",
        ReadinessCheckId::Registration => "Registration",
        ReadinessCheckId::ActorCatalog => "Actor catalog",
        ReadinessCheckId::ActorSearch => "Actor search",
        ReadinessCheckId::ClassifierInference => "Atlas BERT inference",
        ReadinessCheckId::ClassifierTraining => "Classifier training",
        _ => "Backend check",
    }
}
