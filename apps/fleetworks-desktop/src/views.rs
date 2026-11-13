//! Selected native workstation destinations. Topology presentation belongs to Intel Web.

use fleetworks_contracts::search::IndexState;
use rune::Cx;
use rune::El;
use rune::ScrollSpec;
use rune::Transition;
use rune_kit::Collapsible;
use rune_kit::Kit;
use rune_kit::SegmentedControl;
use rune_kit::TextInput;

use crate::app::DossierFacet;
use crate::app::Load;
use crate::app::Page;
use crate::app::State;
use crate::app::WorkstationInspector;
use crate::brain::Association;
use crate::connection::ConnectionStatus;
use crate::icons;
use crate::material;
use crate::material::MetalFinish;
use crate::pairing::PairingFailure;
use crate::pairing::PairingStage;
use crate::theme;
use crate::theme::StatusTone;
use crate::ui_components;
use crate::ui_components::ActionButton;

pub(crate) fn line(value: impl Into<String>, kit: &Kit) -> El {
    rune::text(value.into())
        .font(kit.ty.body)
        .color(kit.c.text_base)
        .wrap()
        .selectable(true)
}

pub(crate) fn quiet(value: impl Into<String>, kit: &Kit) -> El {
    rune::text(value.into())
        .font(kit.ty.small)
        .color(kit.c.text_muted)
        .wrap()
        .selectable(true)
}

pub(crate) fn heading(value: impl Into<String>, kit: &Kit) -> El {
    rune::text(value.into())
        .font(kit.ty.body_medium)
        .color(kit.c.text_base)
        .wrap()
}

pub(crate) fn scroll(body: El, key: &str) -> El {
    rune::col()
        .key(key.to_owned())
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(body.w_full().padding(20.0).gap(14.0))
}

pub(crate) fn page(state: &State, cx: &mut Cx<State>, kit: &Kit, layers: &mut Vec<El>) -> El {
    if state.setup_gateway {
        return setup_gateway(state, cx, kit);
    }
    if !state.connected() {
        return workstation(state, cx, kit, layers);
    }
    match state.page {
        Page::Workstation | Page::Workspaces | Page::Profile => workstation(state, cx, kit, layers),
        Page::Brain => brain(state, cx, kit),
        Page::Seed | Page::Jobs | Page::Atlas | Page::Tools | Page::Models => {
            let log_width = cx
                .layout_size("scoped-logs-table")
                .map(|size| size[0])
                .or_else(|| {
                    cx.layout_size("page-content")
                        .map(|size| (size[0] - 80.0).max(0.0))
                })
                .unwrap_or(340.0);
            crate::backend_views::page(state, cx, kit, layers, log_width)
        }
    }
}

fn connection_status(state: &State) -> String {
    match &state.connection.status {
        ConnectionStatus::Disconnected => "No gateway connection".into(),
        ConnectionStatus::Connecting => "Checking gateway authentication and compatibility…".into(),
        ConnectionStatus::Connected => "Connected to a compatible gateway".into(),
        ConnectionStatus::Failed(error) => error.clone(),
    }
}

fn setup_gateway(state: &State, cx: &Cx<State>, kit: &Kit) -> El {
    let preferences = state
        .accessibility
        .for_view(state.fixture, cx.is_reduced_motion());
    let pointer_ref = (!preferences.matte()).then_some(&state.pointer_offset);
    let shadow_ref = (!preferences.matte()).then_some(&state.shadow_offset);
    let busy = state.pairing.busy() || state.connection.status == ConnectionStatus::Connecting;
    let active_step = if state.connected() {
        2
    } else if matches!(
        state.pairing.stage,
        PairingStage::Working(_) | PairingStage::Waiting(_) | PairingStage::Connected
    ) {
        1
    } else {
        0
    };

    let origin = TextInput::new(&state.connection.origin)
        .label("Gateway URL")
        .placeholder("https://gateway.example.test")
        .width(None)
        .disabled(busy || state.connected())
        .on_edit(|state: &mut State| &mut state.connection.origin)
        .on_change(|state: &mut State, _| state.profile_editor_changed(true))
        .on_submit(|state: &mut State, _| state.connect())
        .build(kit)
        .key("gateway-origin");

    let header = ui_components::section_header(
        &icons::KEY,
        "Setup Gateway",
        Some("Pair once via macOS Keychain. Accepted jobs persist independently."),
        Some(ui_components::status_pill(
            match active_step {
                2 => "Paired",
                1 => "Pairing",
                _ => "Origin Setup",
            },
            None,
            match active_step {
                2 => StatusTone::Ok,
                1 => StatusTone::Accent,
                _ => StatusTone::Neutral,
            },
            kit,
        )),
        None,
        kit,
    );

    let url_card = material::luminous_card(
        rune::col()
            .padding(16.0)
            .gap(10.0)
            .child(ui_components::section_header(
                &icons::SERVER,
                "Gateway URL",
                Some("Target API origin for keychain-bound requests"),
                None,
                None,
                kit,
            ))
            .child(origin),
        if state.connected() {
            StatusTone::Ok
        } else {
            StatusTone::Accent
        },
        false,
        preferences,
        pointer_ref,
        shadow_ref,
    );

    let body = rune::col()
        .gap(14.0)
        .child(header)
        .child(ui_components::stage_stepper(
            "gateway-setup-stepper",
            &["Origin URL", "Workstation Pair", "Gateway Ready"],
            active_step,
            preferences,
            kit,
        ))
        .child(url_card)
        .child(pairing_status(state, preferences, kit));
    scroll(body, "gateway-setup-body")
}

fn pairing_status(
    state: &State,
    preferences: crate::accessibility::ViewPreferences,
    kit: &Kit,
) -> El {
    let pointer_ref = (!preferences.matte()).then_some(&state.pointer_offset);
    let shadow_ref = (!preferences.matte()).then_some(&state.shadow_offset);
    let mut body = rune::col()
        .padding(16.0)
        .gap(12.0)
        .key("pairing-status-card")
        .child(ui_components::section_header(
            &icons::SHIELD_CHECK,
            "Gateway access",
            None,
            None,
            None,
            kit,
        ));
    if state.connected() {
        body = body
            .child(ui_components::async_status_strip(
                "connection-status",
                "Connected to your gateway",
                "Workstation trust verified",
                false,
                StatusTone::Ok,
                preferences,
                kit,
            ))
            .child(registration_status(state, kit))
            .child(ui_components::inline_toggle_row(
                "setup-advanced-toggle",
                state.setup_advanced_open,
                "Inspect gateway readiness checks",
                Some("Reveal per-service probes and observation freshness"),
                false,
                kit,
                |state: &mut State| state.setup_advanced_open = !state.setup_advanced_open,
            ));
        if state.setup_advanced_open {
            body = body.child(crate::readiness_cards::checks(state, kit));
        }
        if state.registration.context.is_some() {
            body = body.child(
                ActionButton::new("Done")
                    .key("gateway-setup-done")
                    .icon(&icons::SHIELD_CHECK)
                    .finish(MetalFinish::GoldCta)
                    .on_press(|state: &mut State| state.setup_gateway = false)
                    .build(kit, preferences, pointer_ref),
            );
        }
    } else {
        body = match &state.pairing.stage {
            PairingStage::Idle => body
                .child(quiet(
                    "Pair this Mac once. Your gateway operator approves access, then Fleetworks saves it in macOS Keychain.",
                    kit,
                ))
                .child(
                    ActionButton::new("Connect")
                        .key("connect")
                        .icon(&icons::KEY)
                        .finish(MetalFinish::GoldCta)
                        .busy(state.connection.status == ConnectionStatus::Connecting)
                        .disabled(
                            state.connection.status == ConnectionStatus::Connecting
                                || state.connection.origin.text.trim().is_empty(),
                        )
                        .on_press(|state: &mut State| state.connect())
                        .build(kit, preferences, pointer_ref),
                ),
            PairingStage::Working(message) => body.child(ui_components::async_status_strip(
                "pairing-progress",
                "Pairing in progress",
                message.clone(),
                true,
                StatusTone::Accent,
                preferences,
                kit,
            )),
            PairingStage::Waiting(challenge) => body
                .child(ui_components::async_status_strip(
                    "pairing-waiting",
                    "Waiting for gateway approval",
                    "Share this approval code with your gateway operator.",
                    true,
                    StatusTone::Warn,
                    preferences,
                    kit,
                ))
                .child(
                    rune::col()
                        .padding(12.0)
                        .gap(6.0)
                        .radius(8.0)
                        .fill(theme::hex(theme::SURFACE_RAISED))
                        .border(1.0, theme::hex(theme::ACCENT_DEEP))
                        .child(quiet("Approval code", kit))
                        .child(
                            rune::text(challenge.approval_code.clone())
                                .key("pairing-approval-code")
                                .font(kit.ty.body_medium)
                                .color(theme::hex(theme::ACCENT_HIGHLIGHT))
                                .selectable(true),
                        )
                        .child(quiet("Pairing request", kit))
                        .child(quiet(challenge.pairing_id.clone(), kit).key("pairing-request-id")),
                ),
            PairingStage::Failed(failure) => {
                let restart = matches!(
                    failure,
                    PairingFailure::Denied
                        | PairingFailure::Expired
                        | PairingFailure::KeychainMissing
                        | PairingFailure::Authentication
                );
                body.child(
                    ui_components::async_status_strip(
                        "pairing-failure",
                        if restart {
                            "Pairing requires restart"
                        } else {
                            "Pairing interrupted"
                        },
                        failure.message(),
                        false,
                        StatusTone::Err,
                        preferences,
                        kit,
                    )
                    .role("alert"),
                )
                .child(
                    ActionButton::new(if restart { "Pair again" } else { "Retry" })
                        .key(if restart { "pairing-restart" } else { "connect" })
                        .icon(&icons::REFRESH)
                        .finish(MetalFinish::GoldCta)
                        .disabled(state.connection.origin.text.trim().is_empty())
                        .on_press(move |state: &mut State| {
                            if restart {
                                state.restart_pairing();
                            } else {
                                state.connect();
                            }
                        })
                        .build(kit, preferences, pointer_ref),
                )
            }
            PairingStage::Connected => body.child(ui_components::async_status_strip(
                "pairing-progress",
                "Verifying gateway access…",
                "Confirming workstation credentials",
                true,
                StatusTone::Ok,
                preferences,
                kit,
            )),
        };
    }
    if !matches!(state.pairing.stage, PairingStage::Idle) || state.connected() {
        body = body.child(
            ActionButton::new("Change gateway URL")
                .key("pairing-change-origin")
                .finish(MetalFinish::Gunmetal)
                .on_press(|state: &mut State| state.disconnect())
                .build(kit, preferences, pointer_ref),
        );
    }
    let card_tone = if state.connected() {
        StatusTone::Ok
    } else if matches!(state.pairing.stage, PairingStage::Failed(_)) {
        StatusTone::Err
    } else {
        StatusTone::Accent
    };
    material::luminous_card(body, card_tone, false, preferences, pointer_ref, shadow_ref)
}

fn registration_status(state: &State, kit: &Kit) -> El {
    if let Some(context) = &state.registration.context {
        return ui_components::status_pill(
            format!("Connect ID: {}", context.connect_id),
            Some(&icons::KEY),
            StatusTone::Ok,
            kit,
        )
        .key("connect-id");
    }
    if !state.connected() {
        return quiet("Connect to a gateway to register this workstation.", kit);
    }
    match &state.registration.receipt {
        Load::Pending | Load::Loading => quiet("Registering this workstation…", kit).role("status"),
        Load::Failed(error) => line(error.clone(), kit).role("alert"),
        _ => quiet(
            "Workstation registration unavailable. Permitted shared backend reads remain usable.",
            kit,
        ),
    }
}

fn metric_tone(value: Option<f32>) -> StatusTone {
    match value {
        Some(v) if v >= 85.0 => StatusTone::Err,
        Some(v) if v >= 65.0 => StatusTone::Warn,
        Some(_) => StatusTone::Ok,
        None => StatusTone::Neutral,
    }
}

fn workstation(state: &State, cx: &Cx<State>, kit: &Kit, layers: &mut Vec<El>) -> El {
    let preferences = state
        .accessibility
        .for_view(state.fixture, cx.is_reduced_motion());
    let pointer_ref = (!preferences.matte()).then_some(&state.pointer_offset);
    let shadow_ref = (!preferences.matte()).then_some(&state.shadow_offset);

    let native = material::luminous_card(
        rune::col()
            .padding(16.0)
            .gap(10.0)
            .key("native-client-card")
            .child(ui_components::section_header(
                &icons::WORKSTATION,
                "Native Client",
                Some("Signed workstation identity & platform runtime"),
                Some(ui_components::status_pill(
                    format!("v{}", env!("CARGO_PKG_VERSION")),
                    None,
                    StatusTone::Accent,
                    kit,
                )),
                None,
                kit,
            ))
            .child(line(
                format!(
                    "Fleetworks {} · macOS · {}",
                    env!("CARGO_PKG_VERSION"),
                    std::env::consts::ARCH
                ),
                kit,
            ))
            .child(registration_status(state, kit)),
        StatusTone::Accent,
        false,
        preferences,
        pointer_ref,
        shadow_ref,
    );

    let gateway_tone = match &state.connection.status {
        ConnectionStatus::Connected => StatusTone::Ok,
        ConnectionStatus::Connecting => StatusTone::Warn,
        ConnectionStatus::Failed(_) => StatusTone::Err,
        ConnectionStatus::Disconnected => StatusTone::Neutral,
    };
    let gateway = material::luminous_card(
        rune::col()
            .padding(16.0)
            .gap(10.0)
            .key("gateway-backend-card")
            .child(ui_components::section_header(
                &icons::SERVER,
                "Gateway Backend",
                Some(&state.connection.origin.text),
                Some(ui_components::status_pill(
                    state.connection.deployment_label().to_owned(),
                    None,
                    gateway_tone,
                    kit,
                )),
                None,
                kit,
            ))
            .child(
                line(connection_status(state), kit)
                    .key("connection-status")
                    .role("status"),
            )
            .children((!state.connected()).then(|| {
                ActionButton::new("Setup Gateway")
                    .key("workstation-setup-gateway")
                    .icon(&icons::KEY)
                    .finish(MetalFinish::GoldCta)
                    .on_press(|state: &mut State| state.setup_gateway = true)
                    .build(kit, preferences, pointer_ref)
            }))
            .children(
                state
                    .connected()
                    .then(|| crate::readiness_cards::checks(state, kit)),
            ),
        gateway_tone,
        false,
        preferences,
        pointer_ref,
        shadow_ref,
    );

    let mut body = rune::col()
        .gap(14.0)
        .child(heading("Workstation Status", kit));
    if state.connected() {
        let mut resources = rune::col()
            .padding(16.0)
            .gap(12.0)
            .key("host-resources-card")
            .child(ui_components::section_header(
                &icons::CPU,
                "Host Resources",
                Some("Click a telemetry tile to inspect host specifications"),
                None,
                None,
                kit,
            ));
        match &state.host_metrics {
            Load::Idle => {
                resources = resources.child(ui_components::async_status_strip(
                    "host-metrics-idle",
                    "Telemetry idle",
                    "Host resource telemetry has not been observed.",
                    false,
                    StatusTone::Neutral,
                    preferences,
                    kit,
                ));
            }
            Load::Pending | Load::Loading => {
                resources = resources.child(ui_components::async_status_strip(
                    "host-metrics-loading",
                    "Reading API host resources…",
                    "Polling CPU, memory and storage utilization",
                    true,
                    StatusTone::Info,
                    preferences,
                    kit,
                ));
            }
            Load::Failed(error) => {
                resources = resources.child(line(error.clone(), kit).role("alert"));
            }
            Load::Ready(metrics) => {
                let mut tiles = rune::row().w_full().gap(10.0);
                for (inspector, key, label_name, value, glyph) in [
                    (
                        WorkstationInspector::Cpu,
                        "workstation-tile-cpu",
                        "CPU",
                        metrics.cpu_percent,
                        &icons::CPU,
                    ),
                    (
                        WorkstationInspector::Memory,
                        "workstation-tile-memory",
                        "Memory",
                        metrics.memory_percent,
                        &icons::MEMORY,
                    ),
                    (
                        WorkstationInspector::Disk,
                        "workstation-tile-disk",
                        "Disk",
                        metrics.disk_percent,
                        &icons::DISK,
                    ),
                ] {
                    let valid = value.filter(|v| v.is_finite() && (0.0..=100.0).contains(v));
                    let formatted = valid.map_or_else(|| "N/A".to_owned(), |v| format!("{v:.1}%"));
                    let tone = metric_tone(valid.map(|v| v as f32));
                    let tile = ui_components::metric_ring_tile(
                        key,
                        label_name,
                        formatted,
                        valid,
                        valid.map(|v| (v / 100.0) as f32),
                        if valid.is_some() {
                            "API host load"
                        } else {
                            "Unavailable"
                        },
                        glyph,
                        tone,
                        state.workstation_inspector == Some(inspector),
                        preferences,
                        kit,
                        pointer_ref,
                        shadow_ref,
                    )
                    .grow(1.0)
                    .on_press(move |state: &mut State| {
                        state.workstation_inspector = Some(inspector);
                    });
                    tiles = tiles.child(tile);
                }
                resources = resources.child(tiles);

                let inspector_detail = state.workstation_inspector.map(|kind| match kind {
                    WorkstationInspector::Cpu => ("CPU Telemetry", metrics.cpu_percent),
                    WorkstationInspector::Memory => ("Memory Telemetry", metrics.memory_percent),
                    WorkstationInspector::Disk => ("Storage Telemetry", metrics.disk_percent),
                });
                if let Some((title, reading)) = inspector_detail {
                    let detail_body = rune::col()
                        .gap(8.0)
                        .child(line(
                            format!(
                                "Observed reading: {}",
                                reading
                                    .filter(|v| v.is_finite() && (0.0..=100.0).contains(v))
                                    .map_or_else(
                                        || "unavailable".to_owned(),
                                        |v| format!("{v:.1}%")
                                    )
                            ),
                            kit,
                        ))
                        .children(
                            metrics
                                .platform
                                .as_ref()
                                .map(|platform| quiet(format!("Platform: {platform}"), kit)),
                        )
                        .children(
                            metrics
                                .chipset
                                .as_ref()
                                .map(|chipset| quiet(format!("Architecture: {chipset}"), kit)),
                        );
                    ui_components::modal_prompt(
                    "workstation-telemetry",
                    title,
                    "Measurements describe the API host; unreported metrics remain unavailable.",
                    true,
                    440.0,
                    |state: &mut State| state.workstation_inspector = None,
                    detail_body,
                    Some(
                        ActionButton::new("Close")
                            .finish(MetalFinish::GoldCta)
                            .on_press(|state: &mut State| state.workstation_inspector = None)
                            .build(kit, preferences, pointer_ref),
                    ),
                    kit,
                    layers,
                );
                }
            }
        }
        let resources = material::luminous_card(
            resources,
            StatusTone::Info,
            false,
            preferences,
            pointer_ref,
            shadow_ref,
        );
        body = body.child(resources);
    }
    scroll(body.child(native).child(gateway), "workstation-body")
}

fn brain(state: &State, cx: &mut Cx<State>, kit: &Kit) -> El {
    let preferences = state
        .accessibility
        .for_view(state.fixture, cx.is_reduced_motion());
    let pointer_ref = (!preferences.matte()).then_some(&state.pointer_offset);
    let shadow_ref = (!preferences.matte()).then_some(&state.shadow_offset);
    let narrow = cx
        .layout_size("page-content")
        .is_some_and(|size| size[0] < 720.0);
    if narrow && state.brain.showing_detail {
        return rune::col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(
                ActionButton::new("Back to results")
                    .key("brain-back")
                    .finish(MetalFinish::Gunmetal)
                    .on_press(|state: &mut State| state.brain.showing_detail = false)
                    .build(kit, preferences, pointer_ref),
            )
            .child(dossier(state, preferences, kit));
    }
    let search = TextInput::new(&state.brain.query)
        .label("Search actors")
        .placeholder("Search actors")
        .width(None)
        .on_edit(|state: &mut State| &mut state.brain.query)
        .on_change(|state: &mut State, tx| state.query_edited(tx))
        .on_key(|state: &mut State, event, _| state.search_key(event))
        .on_submit(|state: &mut State, _| state.search_now())
        .build(kit)
        .key("actor-search");
    let mut list = rune::col()
        .gap(10.0)
        .child(ui_components::section_header(
            &icons::BRAIN,
            "Search actors",
            Some("Type a name or alias to inspect intelligence dossiers"),
            None,
            None,
            kit,
        ))
        .child(search);
    match &state.brain.results {
        Load::Idle => {
            list = list.child(
                quiet(
                    "Enter a name or alias to search the shared actor catalog.",
                    kit,
                )
                .key("search-prompt"),
            );
        }
        Load::Pending => {
            list = list.child(ui_components::async_status_strip(
                "search-pending",
                "Preparing query",
                "Debouncing catalog lookup…",
                true,
                StatusTone::Neutral,
                preferences,
                kit,
            ));
        }
        Load::Loading => {
            list = list.child(ui_components::async_status_strip(
                "search-loading",
                "Searching catalog",
                "Querying configured gateway…",
                true,
                StatusTone::Accent,
                preferences,
                kit,
            ));
        }
        Load::Failed(error) => {
            list = list.child(line(error.clone(), kit).key("search-error").role("alert"));
        }
        Load::Ready(response) => {
            let current = response.index.state == IndexState::Ready;
            if !current {
                list = list.child(
                    ui_components::status_pill(
                        format!("Index: {:?}", response.index.state),
                        Some(&icons::ALERT),
                        StatusTone::Warn,
                        kit,
                    )
                    .key("index-status"),
                );
            }
            if response.rows.is_empty() && current {
                list = list.child(quiet("No matching actors.", kit).key("search-empty"));
            }
            for (index, hit) in response.rows.iter().enumerate() {
                let selected = state.brain.highlighted == Some(index);
                let tile = material::interactive_tile(
                    rune::row()
                        .w_full()
                        .padding(10.0)
                        .gap(10.0)
                        .center_y()
                        .child(ui_components::status_dot(
                            if selected {
                                StatusTone::Accent
                            } else {
                                StatusTone::Violet
                            },
                            preferences,
                        ))
                        .child(
                            rune::col()
                                .grow(1.0)
                                .gap(2.0)
                                .child(
                                    rune::text(hit.label.clone())
                                        .font(kit.ty.ui_medium)
                                        .color(theme::hex(theme::TEXT_PRIMARY)),
                                )
                                .child(quiet(
                                    hit.summary.clone().unwrap_or_else(|| hit.actor_id.clone()),
                                    kit,
                                )),
                        )
                        .child(
                            rune::icon(&icons::CHEVRON_RIGHT, 14.0)
                                .color(kit.c.icon_muted)
                                .a11y_hidden(),
                        ),
                    selected,
                    if selected {
                        StatusTone::Accent
                    } else {
                        StatusTone::Violet
                    },
                    preferences,
                    pointer_ref,
                    shadow_ref,
                )
                .key(format!("actor-hit-{}", hit.actor_id))
                .role("button")
                .label(hit.label.clone())
                .focusable(true)
                .on_press(move |state: &mut State| state.select_actor(index));
                list = list.child(tile);
            }
            if response.truncated {
                list = list.child(quiet(
                    "Results are bounded. Refine the search to narrow matches.",
                    kit,
                ));
            }
        }
    }
    let results = scroll(list, "actor-results");
    if narrow {
        results
    } else {
        let detail_panel = if preferences.reduced_motion {
            dossier(state, preferences, kit).grow(1.0).min_w(0.0)
        } else {
            rune::col()
                .key(format!(
                    "dossier-pane-{}",
                    state.brain.highlighted.unwrap_or(9999)
                ))
                .grow(1.0)
                .min_w(0.0)
                .transition(Transition::offset(8.0, 0.0).combined(Transition::opacity()))
                .child(dossier(state, preferences, kit))
        };
        rune::row()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .gap(16.0)
            .child(results.w(340.0).min_w(300.0).grow(0.0))
            .child(detail_panel)
    }
}

fn dossier(state: &State, preferences: crate::accessibility::ViewPreferences, kit: &Kit) -> El {
    let pointer_ref = (!preferences.matte()).then_some(&state.pointer_offset);
    let shadow_ref = (!preferences.matte()).then_some(&state.shadow_offset);
    let body = match &state.brain.dossier {
        Load::Idle => material::luminous_card(
            rune::col()
                .padding(16.0)
                .gap(8.0)
                .child(ui_components::section_header(
                    &icons::BRAIN,
                    "Actor dossier",
                    Some("Select an actor result to reveal intelligence attributes & links."),
                    None,
                    None,
                    kit,
                )),
            StatusTone::Neutral,
            false,
            preferences,
            pointer_ref,
            shadow_ref,
        ),
        Load::Pending | Load::Loading => ui_components::async_status_strip(
            "dossier-loading",
            "Loading actor dossier",
            "Fetching intelligence record & associations…",
            true,
            StatusTone::Accent,
            preferences,
            kit,
        ),
        Load::Failed(error) => {
            rune::col().child(line(error.clone(), kit).key("dossier-error").role("alert"))
        }
        Load::Ready(actor) => {
            let facet_idx = match state.brain_facet {
                DossierFacet::Overview => 0,
                DossierFacet::Associations => 1,
            };
            let facet_picker =
                SegmentedControl::new(&["Overview", "Associations"], Some(facet_idx))
                    .label("Dossier facet")
                    .fit()
                    .on_pick(|state: &mut State, index, _| {
                        state.brain_facet = if index == 1 {
                            DossierFacet::Associations
                        } else {
                            DossierFacet::Overview
                        };
                    })
                    .build(kit);

            let mut card_body = rune::col()
                .padding(16.0)
                .gap(12.0)
                .child(
                    rune::row()
                        .w_full()
                        .center_y()
                        .gap(10.0)
                        .child(
                            rune::col()
                                .grow(1.0)
                                .gap(4.0)
                                .child(heading(actor.display_name.clone(), kit).key("dossier-name"))
                                .child(
                                    rune::row()
                                        .gap(6.0)
                                        .child(ui_components::status_pill(
                                            actor.subtype.clone(),
                                            None,
                                            StatusTone::Violet,
                                            kit,
                                        ))
                                        .child(ui_components::status_pill(
                                            actor.identity_state.clone(),
                                            None,
                                            StatusTone::Cyan,
                                            kit,
                                        )),
                                ),
                        )
                        .child(facet_picker),
                )
                .child(quiet(actor.id.clone(), kit));

            match state.brain_facet {
                DossierFacet::Overview => {
                    card_body = card_body.child(line(
                        actor
                            .description
                            .clone()
                            .unwrap_or_else(|| "Description unavailable".into()),
                        kit,
                    ));
                    if !actor.aliases.is_empty() {
                        let mut alias_box = rune::col().gap(4.0);
                        for alias in actor.aliases.iter().take(50) {
                            alias_box = alias_box.child(line(alias.clone(), kit));
                        }
                        card_body = card_body.child(
                            Collapsible::new(format!("Aliases ({})", actor.aliases.len()), true)
                                .ghost()
                                .child(alias_box)
                                .build(kit),
                        );
                    }
                    if !actor.metadata.is_empty() {
                        let mut meta_box = rune::col().gap(4.0);
                        for field in actor.metadata.iter().take(50) {
                            meta_box = meta_box.child(quiet(
                                format!(
                                    "{}: {}",
                                    field.key,
                                    field.value.as_deref().unwrap_or("Unavailable")
                                ),
                                kit,
                            ));
                        }
                        card_body = card_body.child(
                            Collapsible::new(
                                format!("Attributes ({})", actor.metadata.len()),
                                true,
                            )
                            .ghost()
                            .child(meta_box)
                            .build(kit),
                        );
                    }
                }
                DossierFacet::Associations => {
                    for (name, rows) in [
                        ("Claims", &actor.claims),
                        ("Briefs", &actor.briefs),
                        ("Inferences", &actor.inferences),
                    ] {
                        card_body = card_body.child(associations(name, rows, kit));
                    }
                }
            }
            for gap in &actor.gaps {
                card_body = card_body.child(quiet(gap.clone(), kit));
            }
            if actor.truncated {
                card_body = card_body.child(quiet(
                    "The server bounded this dossier; some associated records are not loaded.",
                    kit,
                ));
            }
            material::luminous_card(
                card_body,
                StatusTone::Violet,
                false,
                preferences,
                pointer_ref,
                shadow_ref,
            )
        }
    };
    scroll(body, "actor-dossier")
}

fn associations(label: &str, rows: &[Association], kit: &Kit) -> El {
    let mut body = rune::col()
        .gap(6.0)
        .child(heading(format!("{label} ({})", rows.len()), kit));
    for row in rows.iter().take(50) {
        body = body.child(line(
            format!("{} · {}", row.node.label, row.relationship.label),
            kit,
        ));
    }
    if rows.len() > 50 {
        body = body.child(quiet(
            "Additional loaded records are outside this bounded preview.",
            kit,
        ));
    }
    body
}

#[cfg(test)]
mod tests {
    use fleetworks_contracts::pairing::PairingChallenge;
    use rune::Env;
    use rune::Key;
    use rune::KeyEvent;
    use rune::Mods;
    use rune::Runtime;

    use crate::app;
    use crate::app::Intent;
    use crate::app::Page;
    use crate::app::State;
    use crate::pairing::PairingFailure;
    use crate::pairing::PairingStage;

    fn runtime(page: Page, setup: bool, size: (f32, f32)) -> Runtime<State> {
        let mut state = State::fixture();
        state.page = page;
        state.setup_gateway = setup;
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
        runtime
    }

    #[test]
    fn status_uses_only_identity_gateway_and_resource_cards_at_both_sizes() {
        for size in [(1180.0, 780.0), (640.0, 480.0)] {
            let runtime = runtime(Page::Workstation, false, size);
            let tree = runtime.accessibility_json().to_string();
            for title in ["Native Client", "Gateway Backend"] {
                assert!(tree.contains(title), "missing {title} at {size:?}");
            }
            assert!(
                !tree.contains("Host Resources"),
                "disconnected workstation must hide Host Resources card at {size:?}"
            );
            assert!(runtime.find("host-resources-card").is_none());
            assert!(!tree.contains("6 checks"));
            assert!(!tree.contains("Workstation activity: registration required"));
            assert!(!tree.contains("Permitted shared backend reads"));
            for key in [
                "gateway-origin",
                "gateway-token-file",
                "disconnect",
                "registration-ensure",
                "workspace-create",
                "refresh-checks",
                "dataset-upload",
                "sidebar-context-footer",
                "nav-seed",
            ] {
                assert!(runtime.find(key).is_none(), "retired status control {key}");
            }
            assert!(runtime.find("workstation-setup-gateway").is_some());
            assert!(runtime.find("nav-workstation").is_some());
            assert!(runtime.find("nav-setup").is_some());
            assert!(
                runtime.state.intents.is_empty(),
                "rendering status cannot submit work"
            );

            let mut stale_page = crate::State::fixture();
            stale_page.page = Page::Brain;
            let mut guarded = Runtime::new(
                stale_page,
                crate::app::view,
                size,
                Env {
                    system_dark: true,
                    scale: 1.0,
                },
            );
            guarded.frame(0.0);
            let guarded_tree = guarded.accessibility_json().to_string();
            assert!(guarded_tree.contains("Workstation Status"));
            assert!(!guarded_tree.contains("Brain Search"));

            let mut connected = Runtime::new(
                crate::tests::registered_fixture(),
                crate::app::view,
                size,
                Env {
                    system_dark: true,
                    scale: 1.0,
                },
            );
            connected.frame(0.0);
            let connected_tree = connected.accessibility_json().to_string();
            for title in ["Native Client", "Gateway Backend", "Host Resources"] {
                assert!(
                    connected_tree.contains(title),
                    "missing connected {title} at {size:?}"
                );
            }
            assert!(connected.find("host-resources-card").is_some());
        }
    }

    #[test]
    fn setup_has_url_and_pairing_progress_without_credential_picker_or_pre_auth_noise() {
        let runtime = runtime(Page::Workstation, true, (1180.0, 780.0));
        let tree = runtime.accessibility_json().to_string();
        assert!(runtime.find("gateway-origin").is_some());
        assert!(runtime.find("connect").is_some());
        assert!(tree.contains("Gateway access"));
        assert!(!tree.contains("Workstation registration unavailable"));
        for key in [
            "gateway-token-file",
            "disconnect",
            "workspace-create-name",
            "profile-save",
            "registration-ensure",
        ] {
            assert!(runtime.find(key).is_none(), "unlisted setup control {key}");
        }
        assert!(runtime.state.intents.is_empty());
    }

    fn pairing_runtime(stage: PairingStage, size: (f32, f32)) -> Runtime<State> {
        let mut state = State::fixture();
        state.setup_gateway = true;
        state.pairing.fixture_enabled = true;
        state.pairing.stage = stage;
        state.connection.origin = rune::TextEdit::new("https://gateway.example.test");
        state.connection.token_file = rune::TextEdit::new("/fixture/private-token-reference");
        state.configuration.account_api_key =
            rune::TextEdit::new("synthetic-hidden-provider-secret");
        state.registration.receipt =
            crate::app::Load::Failed("synthetic pre-auth registration details".into());
        state.readiness = crate::app::Load::Failed("synthetic pre-auth readiness details".into());
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
        runtime
    }

    fn click(runtime: &mut Runtime<State>, key: &str, clock: &mut f64) {
        runtime.update(|_, tx| tx.scroll_into_view(key));
        *clock += 1.0 / 60.0;
        runtime.frame(*clock);
        let (x, y) = runtime
            .find(key)
            .unwrap_or_else(|| panic!("production pairing control {key}"));
        let ([left, top, width, height], _) = runtime
            .presented("gateway-setup-body")
            .expect("actual setup viewport");
        assert!(
            x >= left && x <= left + width && y >= top && y <= top + height,
            "{key} must be inside its actual scroll viewport before pointer input"
        );
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        *clock += 1.0 / 60.0;
        runtime.frame(*clock);
    }

    fn assert_pre_auth_is_private(runtime: &Runtime<State>) {
        let tree = runtime.accessibility_json().to_string();
        for private in [
            "/fixture/private-token-reference",
            "synthetic-hidden-provider-secret",
            "synthetic pre-auth registration details",
            "synthetic pre-auth readiness details",
        ] {
            assert!(!tree.contains(private), "setup must not announce {private}");
        }
        for key in [
            "gateway-token-file",
            "registration-ensure",
            "connect-id",
            "gateway-setup-done",
        ] {
            assert!(
                runtime.find(key).is_none(),
                "pre-auth control/details {key} cannot appear"
            );
        }
    }

    #[test]
    fn pairing_connect_uses_real_control_without_network_keychain_or_duplicate_start() {
        for size in [(1180.0, 780.0), (640.0, 480.0)] {
            let mut runtime = pairing_runtime(PairingStage::Idle, size);
            assert_pre_auth_is_private(&runtime);
            assert!(runtime.state.intents.is_empty());
            let mut clock = 0.0;
            click(&mut runtime, "connect", &mut clock);
            assert!(runtime.state.intents.contains(&Intent::PairingConnect));
            assert!(
                !runtime.state.intents.contains(&Intent::Connect),
                "URL-only setup uses the pairing path"
            );
            assert!(matches!(
                runtime.state.pairing.stage,
                PairingStage::Working(_)
            ));
            assert!(runtime.find("pairing-progress").is_some());
            assert!(
                runtime.find("connect").is_none(),
                "working state cannot submit another start"
            );
            assert_pre_auth_is_private(&runtime);
        }
    }

    #[test]
    fn pending_approval_is_selectable_and_change_url_fences_old_progress_before_edit() {
        let challenge = PairingChallenge {
            pairing_id: "synthetic-pairing-request-0001".into(),
            approval_code: "SYNTHETICAPPROVALCODE0001".into(),
            expires_at_ms: 1_900_000_000_000,
            poll_after_ms: 1500,
        };
        let mut runtime = pairing_runtime(PairingStage::Waiting(challenge.clone()), (640.0, 480.0));
        assert_pre_auth_is_private(&runtime);
        let tree = runtime.accessibility_json().to_string();
        assert!(tree.contains(&challenge.approval_code) && tree.contains(&challenge.pairing_id));
        let code = runtime
            .elements()
            .into_iter()
            .find(|element| element.key.as_deref() == Some("pairing-approval-code"))
            .expect("actual approval code");
        assert_eq!(code.text.as_deref(), Some(challenge.approval_code.as_str()));
        let mut clock = 0.0;
        click(&mut runtime, "pairing-approval-code", &mut clock);
        runtime.key(KeyEvent {
            key: Key::Command('a'),
            mods: Mods {
                cmd: true,
                ..Mods::default()
            },
        });
        clock += 1.0 / 60.0;
        runtime.frame(clock);
        assert!(
            runtime
                .selected_text()
                .is_some_and(|text| text.contains(&challenge.approval_code)),
            "the actual approval code must be selectable for sharing"
        );
        let epoch = runtime.state.connection.epoch;
        click(&mut runtime, "pairing-change-origin", &mut clock);
        assert_eq!(runtime.state.pairing.stage, PairingStage::Idle);
        assert!(
            runtime.state.connection.epoch > epoch,
            "changing destination fences old callbacks"
        );
        assert!(runtime.find("pairing-approval-code").is_none());
        click(&mut runtime, "gateway-origin", &mut clock);
        runtime.key(KeyEvent {
            key: Key::Command('a'),
            mods: Mods {
                cmd: true,
                ..Mods::default()
            },
        });
        runtime.paste("https://another-gateway.example.test");
        clock += 1.0 / 60.0;
        runtime.frame(clock);
        assert_eq!(
            runtime.state.connection.origin.text,
            "https://another-gateway.example.test"
        );
        assert_eq!(runtime.state.pairing.stage, PairingStage::Idle);
        assert!(
            !runtime
                .state
                .intents
                .iter()
                .any(|intent| matches!(intent, Intent::PairingConnect | Intent::PairingRestart)),
            "editing URL alone cannot send a pairing request"
        );
    }

    #[test]
    fn actionable_pairing_failures_retry_or_request_new_approval_through_real_controls() {
        for failure in [
            PairingFailure::InvalidOrigin,
            PairingFailure::KeychainMissing,
            PairingFailure::KeychainDenied,
            PairingFailure::KeychainUnavailable,
            PairingFailure::CatalogUnavailable,
            PairingFailure::Network,
            PairingFailure::GatewayUnavailable,
            PairingFailure::Authentication,
            PairingFailure::Unsupported,
            PairingFailure::Denied,
            PairingFailure::Expired,
        ] {
            let mut runtime = pairing_runtime(PairingStage::Failed(failure), (1180.0, 780.0));
            assert_pre_auth_is_private(&runtime);
            assert!(runtime
                .accessibility_json()
                .to_string()
                .contains(failure.message()));
            let restart = matches!(
                failure,
                PairingFailure::KeychainMissing
                    | PairingFailure::Authentication
                    | PairingFailure::Denied
                    | PairingFailure::Expired
            );
            let mut clock = 0.0;
            click(
                &mut runtime,
                if restart {
                    "pairing-restart"
                } else {
                    "connect"
                },
                &mut clock,
            );
            assert!(runtime.state.intents.contains(if restart {
                &Intent::PairingRestart
            } else {
                &Intent::PairingConnect
            }));
            assert!(matches!(
                runtime.state.pairing.stage,
                PairingStage::Working(_)
            ));
            assert_pre_auth_is_private(&runtime);
        }
    }

    #[test]
    fn authenticated_pair_shows_registration_and_done_without_restarting_pairing() {
        let mut runtime = pairing_runtime(PairingStage::Connected, (1180.0, 780.0));
        runtime.update(|state, _| {
            state.connection.status = crate::connection::ConnectionStatus::Connected;
            let receipt = fleetworks_contracts::connect::RegistrationReceipt::new(
                "synthetic-deployment".into(),
                "synthetic-workstation".into(),
                "synthetic-registration".into(),
                None,
                1,
            );
            state.registration.context = Some(
                fleetworks_contracts::context::RegisteredContext::new(
                    receipt.clone(),
                    1,
                    1,
                    "synthetic-config".into(),
                    "synthetic-access".into(),
                )
                .expect("authenticated synthetic context"),
            );
            state.registration.receipt = crate::app::Load::Ready(receipt);
            state.readiness = crate::app::Load::Idle;
        });
        runtime.frame(1.0 / 60.0);
        assert!(runtime.find("connect-id").is_some());
        assert!(runtime.find("connect").is_none());
        let mut clock = 1.0 / 60.0;
        click(&mut runtime, "gateway-setup-done", &mut clock);
        assert!(!runtime.state.setup_gateway);
        assert!(
            runtime.state.intents.is_empty(),
            "Done only changes the displayed destination"
        );
    }

    #[test]
    fn maintained_destinations_do_not_render_retired_administration_controls() {
        for page in Page::ALL {
            let runtime = runtime(page, false, (1180.0, 780.0));
            for key in [
                "article-day",
                "article-category",
                "article-search",
                "tool-inputs",
                "dataset-choose-actors",
                "dataset-upload",
                "jobs-workspace-scope",
                "activity-tabs",
                "account-base-url",
                "workspace-create-name",
            ] {
                assert!(
                    runtime.find(key).is_none(),
                    "unlisted control {key} on {page:?}"
                );
            }
            assert!(
                runtime.state.intents.is_empty(),
                "rendering a destination cannot submit work"
            );
        }
    }
}
