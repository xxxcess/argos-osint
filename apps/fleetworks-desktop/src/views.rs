//! Selected native workstation destinations. Topology presentation belongs to Intel Web.

use fleetworks_contracts::capabilities::Capabilities;
use fleetworks_contracts::readiness::MaintenanceOperation;
use fleetworks_contracts::readiness::ReadinessCheckId;
use fleetworks_contracts::readiness::ReadinessState;
use fleetworks_contracts::search::IndexState;
use fleetworks_workstation::native::BackendKind;
use rune::Cx;
use rune::El;
use rune::ScrollSpec;
use rune::Transition;
use rune_kit::Collapsible;
use rune_kit::Dialog;
use rune_kit::Kit;
use rune_kit::Progress;
use rune_kit::SegmentedControl;
use rune_kit::TextInput;

use crate::accessibility::ViewPreferences;
use crate::app::DossierFacet;
use crate::app::Load;
use crate::app::Page;
use crate::app::SeedStage;
use crate::app::State;
use crate::app::WorkstationInspector;
use crate::brain::Association;
use crate::connection::ConnectionStatus;
use crate::icons;
use crate::local_backend::assess_local_capabilities;
use crate::local_backend::BinaryDiscoveryStatus;
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
    if state.fixture_catalog {
        let preferences = state
            .accessibility
            .for_view(state.fixture, cx.is_reduced_motion());
        return crate::catalog::view(kit, preferences);
    }
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
        ConnectionStatus::Disconnected => {
            if state.is_local_backend() {
                "Local companion backend is stopped".into()
            } else {
                "No gateway connection".into()
            }
        }
        ConnectionStatus::Connecting => {
            if state.is_local_backend() {
                "Starting local companion backend over stdio IPC…".into()
            } else {
                "Checking gateway authentication and compatibility…".into()
            }
        }
        ConnectionStatus::Connected => {
            if state.is_local_backend() {
                "Connected to local embedded companion backend (stdio IPC)".into()
            } else {
                "Connected to a compatible gateway".into()
            }
        }
        ConnectionStatus::Failed(error) => error.clone(),
    }
}

fn backend_mode_card(
    state: &State,
    narrow: bool,
    busy: bool,
    preferences: crate::accessibility::ViewPreferences,
    kit: &Kit,
    pointer_ref: Option<&rune::Live<[f32; 2]>>,
    shadow_ref: Option<&rune::Live<[f32; 2]>>,
) -> El {
    let remote_btn = ActionButton::new("Remote Gateway")
        .key("backend-mode-remote")
        .icon(&icons::SERVER)
        .finish(if !state.is_local_backend() {
            MetalFinish::GoldCta
        } else {
            MetalFinish::Gunmetal
        })
        .disabled(busy)
        .on_press(|state: &mut State| state.set_backend_kind(BackendKind::Remote))
        .build(kit, preferences, pointer_ref);
    let local_btn = ActionButton::new("Local Embedded")
        .key("backend-mode-local")
        .icon(&icons::WORKSTATION)
        .finish(if state.is_local_backend() {
            MetalFinish::GoldCta
        } else {
            MetalFinish::Gunmetal
        })
        .disabled(busy)
        .on_press(|state: &mut State| state.set_backend_kind(BackendKind::Local))
        .build(kit, preferences, pointer_ref);
    let buttons = if narrow {
        rune::col()
            .key("backend-mode")
            .w_full()
            .gap(8.0)
            .child(remote_btn)
            .child(local_btn)
    } else {
        rune::row()
            .key("backend-mode")
            .w_full()
            .gap(8.0)
            .child(remote_btn)
            .child(local_btn)
    };
    material::luminous_card(
        rune::col()
            .padding(16.0)
            .gap(10.0)
            .child(ui_components::section_header(
                &icons::SERVER,
                "Backend Mode",
                Some(if state.is_local_backend() {
                    "Supervise a private local companion backend on this workstation via framed stdio IPC"
                } else {
                    "Connect over HTTPS or loopback HTTP to a paired Fleetworks gateway"
                }),
                None,
                None,
                kit,
            ))
            .child(buttons),
        StatusTone::Neutral,
        false,
        preferences,
        pointer_ref,
        shadow_ref,
    )
}

fn local_capabilities_summary(
    capabilities: &Capabilities,
    preferences: crate::accessibility::ViewPreferences,
    kit: &Kit,
) -> El {
    let assessment = assess_local_capabilities(capabilities);
    let features_label = if capabilities.features.is_empty() {
        "none advertised".to_owned()
    } else {
        capabilities.features.join(", ")
    };
    let build_label = capabilities.build.as_ref().map_or_else(
        || "unknown build".to_owned(),
        |build| format!("{} ({})", build.version, build.revision),
    );
    let mut section = rune::col()
        .key("local-capabilities-card")
        .gap(8.0)
        .child(heading("Advertised Capabilities", kit))
        .child(quiet(
            format!(
                "API v{} · Build {} · Features: {}",
                capabilities.api_version, build_label, features_label
            ),
            kit,
        ))
        .child(
            rune::row()
                .w_full()
                .gap(6.0)
                .child(ui_components::status_pill(
                    if assessment.nlp_available {
                        "NLP ready"
                    } else {
                        "NLP absent"
                    },
                    None,
                    if assessment.nlp_available {
                        StatusTone::Ok
                    } else {
                        StatusTone::Warn
                    },
                    kit,
                ))
                .child(ui_components::status_pill(
                    if assessment.classifier_available {
                        "Classifier ready"
                    } else {
                        "Classifier absent"
                    },
                    None,
                    if assessment.classifier_available {
                        StatusTone::Ok
                    } else {
                        StatusTone::Warn
                    },
                    kit,
                ))
                .child(ui_components::status_pill(
                    if assessment.actor_search_available {
                        "Search ready"
                    } else {
                        "Search absent"
                    },
                    None,
                    if assessment.actor_search_available {
                        StatusTone::Ok
                    } else {
                        StatusTone::Warn
                    },
                    kit,
                )),
        );
    if !assessment.warnings.is_empty() {
        section = section.child(
            ui_components::async_status_strip(
                "local-capability-warnings",
                "Optional capabilities unavailable",
                assessment.warnings.join(" "),
                false,
                StatusTone::Warn,
                preferences,
                kit,
            )
            .role("status"),
        );
    }
    section
}

fn local_runtime_diagnostics(
    state: &State,
    preferences: crate::accessibility::ViewPreferences,
    kit: &Kit,
) -> Option<El> {
    let mut diagnostics = rune::col().gap(8.0);
    let mut has_any = false;
    if let Some(error) = &state.local_backend.pipe_closed_error {
        has_any = true;
        diagnostics = diagnostics.child(
            ui_components::async_status_strip(
                "local-pipe-closed-alert",
                "Local IPC disconnected",
                error.clone(),
                false,
                StatusTone::Err,
                preferences,
                kit,
            )
            .role("alert"),
        );
    }
    if let Some(reason_code) = &state.local_backend.pressure_paused {
        has_any = true;
        diagnostics = diagnostics.child(
            ui_components::async_status_strip(
                "local-pressure-paused-alert",
                "Local backend paused under resource pressure",
                format!(
                    "Reason code: {reason_code}. Active work paused safely until pressure clears."
                ),
                false,
                StatusTone::Warn,
                preferences,
                kit,
            )
            .role("alert"),
        );
    }
    if let Some(progress) = &state.local_backend.last_job_progress {
        has_any = true;
        diagnostics = diagnostics.child(
            quiet(
                format!(
                    "Local job {} [{}]: {}",
                    progress.job_id, progress.phase, progress.summary
                ),
                kit,
            )
            .key("local-job-progress"),
        );
    }
    if let Some(message) = &state.local_backend.shutdown_message {
        has_any = true;
        diagnostics = diagnostics.child(
            ui_components::async_status_strip(
                "local-shutdown-status",
                "Local companion lifecycle",
                message.clone(),
                state.local_backend.stopping,
                StatusTone::Neutral,
                preferences,
                kit,
            )
            .role("status"),
        );
    }
    if let Some(stderr) = &state.local_backend.stderr_excerpt {
        if !stderr.is_empty() {
            has_any = true;
            diagnostics = diagnostics.child(
                quiet(format!("Companion stderr (bounded): {stderr}"), kit)
                    .key("local-stderr-diagnostics"),
            );
        }
    }
    has_any.then_some(diagnostics)
}

fn local_backend_setup_card(
    state: &State,
    narrow: bool,
    preferences: crate::accessibility::ViewPreferences,
    kit: &Kit,
    pointer_ref: Option<&rune::Live<[f32; 2]>>,
    shadow_ref: Option<&rune::Live<[f32; 2]>>,
) -> El {
    let mut body = rune::col()
        .padding(16.0)
        .gap(12.0)
        .key("local-backend-card")
        .child(ui_components::section_header(
            &icons::WORKSTATION,
            "Local Embedded Backend",
            Some(&state.connection.origin.text),
            Some(ui_components::status_pill(
                if state.connected() {
                    "Running"
                } else if state.connection.status == ConnectionStatus::Connecting {
                    "Starting"
                } else if state.local_backend.stopping {
                    "Stopping"
                } else {
                    "Stopped"
                },
                None,
                if state.connected() {
                    StatusTone::Ok
                } else if state.connection.status == ConnectionStatus::Connecting {
                    StatusTone::Warn
                } else {
                    StatusTone::Neutral
                },
                kit,
            )),
            None,
            kit,
        ))
        .child(
            quiet(
                format!(
                    "Private 0700 state directory: {}",
                    state.local_state_dir_label()
                ),
                kit,
            )
            .key("local-state-dir"),
        );

    let binary_strip = match &state.local_backend.binary_status {
        BinaryDiscoveryStatus::Ready(path) => ui_components::async_status_strip(
            "local-binary-status",
            "Companion binary verified",
            path.display().to_string(),
            false,
            StatusTone::Ok,
            preferences,
            kit,
        ),
        BinaryDiscoveryStatus::Missing(reason) => ui_components::async_status_strip(
            "local-binary-status",
            "Companion binary missing or invalid",
            reason.clone(),
            false,
            StatusTone::Err,
            preferences,
            kit,
        )
        .role("alert"),
        BinaryDiscoveryStatus::Unknown => ui_components::async_status_strip(
            "local-binary-status",
            "Companion binary resolution pending",
            "Sibling `argos-api-host`, `FLEETWORKS_LOCAL_BACKEND_BIN`, or `--local-backend-bin <PATH>` will be checked on start (PATH is never searched).",
            false,
            StatusTone::Neutral,
            preferences,
            kit,
        ),
    };
    body = body.child(binary_strip);

    if let Some(diagnostics) = local_runtime_diagnostics(state, preferences, kit) {
        body = body.child(diagnostics);
    }

    if state.connected() {
        body = body
            .child(ui_components::async_status_strip(
                "connection-status",
                "Connected to local embedded backend",
                "Framed stdio IPC verified (no HTTP fallback)",
                false,
                StatusTone::Ok,
                preferences,
                kit,
            ))
            .child(registration_status(state, kit));
        if let Some(capabilities) = &state.connection.capabilities {
            body = body.child(local_capabilities_summary(capabilities, preferences, kit));
        }
        body = body.child(ui_components::inline_toggle_row(
            "setup-advanced-toggle",
            state.setup_advanced_open,
            "Inspect local readiness checks",
            Some("Reveal per-service probes and observation freshness"),
            false,
            kit,
            |state: &mut State| state.setup_advanced_open = !state.setup_advanced_open,
        ));
        if state.setup_advanced_open {
            body = body.child(crate::readiness_cards::checks(state, preferences, kit));
        }
        let stop_btn = ActionButton::new("Stop Local Backend")
            .key("stop-local-backend")
            .finish(MetalFinish::Gunmetal)
            .disabled(state.local_backend.stopping)
            .on_press(|state: &mut State| state.stop_local_backend())
            .build(kit, preferences, pointer_ref);
        let switch_btn = ActionButton::new("Open Gateway")
            .key("switch-remote-gateway")
            .finish(MetalFinish::Gunmetal)
            .on_press(|state: &mut State| state.set_backend_kind(BackendKind::Remote))
            .build(kit, preferences, pointer_ref);
        let mut actions = if narrow {
            rune::col().w_full().gap(8.0)
        } else {
            rune::row().w_full().gap(8.0)
        };
        if state.registration.context.is_some() {
            actions = actions.child(
                ActionButton::new("Done")
                    .key("gateway-setup-done")
                    .icon(&icons::SHIELD_CHECK)
                    .finish(MetalFinish::GoldCta)
                    .on_press(|state: &mut State| state.setup_gateway = false)
                    .build(kit, preferences, pointer_ref),
            );
        }
        actions = actions.child(stop_btn).child(switch_btn);
        body = body.child(actions);
    } else {
        if let ConnectionStatus::Failed(error) = &state.connection.status {
            body = body.child(
                ui_components::async_status_strip(
                    "local-connection-error",
                    "Local companion backend failed to start",
                    error.clone(),
                    false,
                    StatusTone::Err,
                    preferences,
                    kit,
                )
                .role("alert"),
            );
        }
        let connecting = state.connection.status == ConnectionStatus::Connecting;
        let start_btn = ActionButton::new("Start Local Backend")
            .key("connect-local")
            .icon(&icons::WORKSTATION)
            .finish(MetalFinish::GoldCta)
            .busy(connecting)
            .disabled(connecting)
            .on_press(|state: &mut State| state.connect_local())
            .build(kit, preferences, pointer_ref);
        let switch_btn = ActionButton::new("Open Gateway")
            .key("switch-remote-gateway")
            .finish(MetalFinish::Gunmetal)
            .disabled(connecting)
            .on_press(|state: &mut State| state.set_backend_kind(BackendKind::Remote))
            .build(kit, preferences, pointer_ref);
        let actions = if narrow {
            rune::col().w_full().gap(8.0)
        } else {
            rune::row().w_full().gap(8.0)
        }
        .child(start_btn)
        .child(switch_btn);
        body = body.child(actions);
    }

    let tone = if state.connected() {
        StatusTone::Ok
    } else if matches!(state.connection.status, ConnectionStatus::Failed(_)) {
        StatusTone::Err
    } else {
        StatusTone::Accent
    };
    material::luminous_card(body, tone, false, preferences, pointer_ref, shadow_ref)
}

fn setup_gateway(state: &State, cx: &Cx<State>, kit: &Kit) -> El {
    let preferences = state
        .accessibility
        .for_view(state.fixture, cx.is_reduced_motion());
    let pointer_ref = (!preferences.matte()).then_some(&state.pointer_offset);
    let shadow_ref = (!preferences.matte()).then_some(&state.shadow_offset);
    let narrow = cx
        .layout_size("page-content")
        .is_some_and(|size| size[0] < 520.0);
    let busy = state.pairing.busy()
        || state.connection.status == ConnectionStatus::Connecting
        || state.local_backend.stopping;
    let active_step = if state.connected() {
        2
    } else if (state.is_local_backend() && state.connection.status == ConnectionStatus::Connecting)
        || matches!(
            state.pairing.stage,
            PairingStage::Working(_) | PairingStage::Waiting(_) | PairingStage::Connected
        )
    {
        1
    } else {
        0
    };

    let header = ui_components::section_header(
        &icons::KEY,
        "Setup Gateway",
        Some(if state.is_local_backend() {
            "Start and supervise the local companion backend over framed stdio IPC."
        } else if cfg!(target_os = "macos") {
            "Pair once via macOS Keychain. Accepted jobs persist independently."
        } else {
            "Pair once via your system keyring or for this session. Accepted jobs persist independently."
        }),
        Some(ui_components::status_pill(
            match active_step {
                2 => {
                    if state.is_local_backend() {
                        "Local Ready"
                    } else {
                        "Paired"
                    }
                }
                1 => {
                    if state.is_local_backend() {
                        "Starting"
                    } else {
                        "Pairing"
                    }
                }
                _ => {
                    if state.is_local_backend() {
                        "Local Setup"
                    } else {
                        "Origin Setup"
                    }
                }
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

    let mut body = rune::col()
        .gap(14.0)
        .child(header)
        .child(ui_components::stage_stepper(
            "gateway-setup-stepper",
            if state.is_local_backend() {
                &["Local Backend", "IPC Handshake", "Backend Ready"]
            } else {
                &["Origin URL", "Workstation Pair", "Gateway Ready"]
            },
            active_step,
            preferences,
            kit,
        ))
        .child(backend_mode_card(
            state,
            narrow,
            busy,
            preferences,
            kit,
            pointer_ref,
            shadow_ref,
        ));

    if state.is_local_backend() {
        body = body.child(local_backend_setup_card(
            state,
            narrow,
            preferences,
            kit,
            pointer_ref,
            shadow_ref,
        ));
    } else {
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

        let url_card = material::luminous_card(
            rune::col()
                .padding(16.0)
                .gap(10.0)
                .child(ui_components::section_header(
                    &icons::SERVER,
                    "Gateway URL",
                    Some(if cfg!(target_os = "macos") {
                        "Target API origin for keychain-bound requests"
                    } else {
                        "Target API origin for keyring-bound requests"
                    }),
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
        body = body
            .child(url_card)
            .child(pairing_status(state, preferences, kit));
    }
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
            body = body.child(crate::readiness_cards::checks(state, preferences, kit));
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
                    if cfg!(target_os = "macos") {
                        "Pair this Mac once. Your gateway operator approves access, then Fleetworks saves it in macOS Keychain."
                    } else {
                        "Pair this workstation once. Your gateway operator approves access, then Fleetworks saves it in your system keyring."
                    },
                    kit,
                ))
                .child(
                    ActionButton::new("Connect")
                        .key("connect")
                        .icon(&icons::KEY)
                        .finish(MetalFinish::Fractured)
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
                    if crate::pairing::local_review_handoff_configured(
                        &state.connection.origin.text,
                    ) {
                        "Confirm the matching values in the local approval prompt."
                    } else {
                        "Share this approval code with your gateway operator."
                    },
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
                )
                .child(
                    rune::row().child(
                        ActionButton::new("Pair again")
                            .key("pairing-restart-waiting")
                            .icon(&icons::REFRESH)
                            .finish(MetalFinish::ObsidianPill)
                            .on_press(|state: &mut State| state.restart_pairing())
                            .build(kit, preferences, pointer_ref),
                    ),
                ),
            PairingStage::Failed(failure) => {
                let restart = matches!(
                    failure,
                    PairingFailure::Denied
                        | PairingFailure::Expired
                        | PairingFailure::KeychainMissing
                        | PairingFailure::SessionEnded
                        | PairingFailure::Authentication
                );
                let offer_session_only = matches!(
                    failure,
                    PairingFailure::KeychainDenied
                        | PairingFailure::KeychainUnavailable
                        | PairingFailure::SessionEnded
                );
                let primary_label = if restart {
                    "Pair again"
                } else if matches!(failure, PairingFailure::KeychainDenied)
                    && cfg!(target_os = "linux")
                {
                    "Unlock keyring"
                } else {
                    "Retry"
                };
                let mut actions = rune::row().gap(8.0).child(
                    ActionButton::new(primary_label)
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
                );
                if offer_session_only {
                    actions = actions.child(
                        ActionButton::new("Use for this session")
                            .key("pairing-session-only")
                            .icon(&icons::KEY)
                            .finish(MetalFinish::ObsidianPill)
                            .disabled(state.connection.origin.text.trim().is_empty())
                            .on_press(|state: &mut State| state.use_session_only_pairing())
                            .build(kit, preferences, pointer_ref),
                    );
                }
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
                .child(actions)
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
    if !state.connected() {
        return quiet("Connect to a gateway to register this workstation.", kit);
    }
    if state.registration.context.is_some() {
        return ui_components::status_pill(
            "Workstation registered",
            Some(&icons::SHIELD_CHECK),
            StatusTone::Ok,
            kit,
        )
        .key("workstation-registered");
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
                    "Fleetworks {} · {} · {}",
                    env!("CARGO_PKG_VERSION"),
                    if cfg!(target_os = "linux") {
                        "Linux"
                    } else {
                        "macOS"
                    },
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
    let mut gateway_body = rune::col()
        .padding(16.0)
        .gap(10.0)
        .key("gateway-backend-card")
        .child(ui_components::section_header(
            &icons::SERVER,
            "Gateway Backend",
            Some(if state.is_local_backend() {
                "Local Embedded (framed stdio IPC)"
            } else {
                &state.connection.origin.text
            }),
            Some(ui_components::status_pill(
                match &state.connection.status {
                    ConnectionStatus::Connected => "Connected".to_owned(),
                    ConnectionStatus::Connecting => "Connecting".to_owned(),
                    ConnectionStatus::Failed(_) => "Fault".to_owned(),
                    ConnectionStatus::Disconnected => "Disconnected".to_owned(),
                },
                None,
                gateway_tone,
                kit,
            )),
            Some(ui_components::status_pill(
                state.connection.deployment_label().to_owned(),
                None,
                StatusTone::Neutral,
                kit,
            )),
            kit,
        ))
        .child(
            line(connection_status(state), kit)
                .key("connection-status")
                .role("status"),
        );
    if state.is_local_backend() {
        gateway_body = gateway_body.child(
            quiet(
                format!("Private 0700 state dir: {}", state.local_state_dir_label()),
                kit,
            )
            .key("workstation-local-state-dir"),
        );
        if let Some(diagnostics) = local_runtime_diagnostics(state, preferences, kit) {
            gateway_body = gateway_body.child(diagnostics);
        }
        if state.connected() {
            if let Some(capabilities) = &state.connection.capabilities {
                gateway_body =
                    gateway_body.child(local_capabilities_summary(capabilities, preferences, kit));
            }
            gateway_body = gateway_body.child(
                ActionButton::new("Stop Local Backend")
                    .key("workstation-stop-local-backend")
                    .finish(MetalFinish::Gunmetal)
                    .disabled(state.local_backend.stopping)
                    .on_press(|state: &mut State| state.stop_local_backend())
                    .build(kit, preferences, pointer_ref),
            );
        }
    }
    if !state.connected() {
        gateway_body = gateway_body.child(
            ActionButton::new("Setup Gateway")
                .key("workstation-setup-gateway")
                .icon(&icons::KEY)
                .finish(MetalFinish::GoldCta)
                .on_press(|state: &mut State| state.setup_gateway = true)
                .build(kit, preferences, pointer_ref),
        );
    } else {
        gateway_body = gateway_body.child(crate::readiness_cards::checks(state, preferences, kit));
    }
    let gateway = material::luminous_card(
        gateway_body,
        gateway_tone,
        false,
        preferences,
        pointer_ref,
        shadow_ref,
    );

    let narrow_workstation = cx
        .layout_size("page-content")
        .is_some_and(|size| size[0] < 540.0);
    let mut body = rune::col()
        .gap(14.0)
        .child(heading("Workstation Status", kit));
    if state.connected() {
        body = body.child(actor_search_summary_card(state, preferences, kit));
        if state.confirm_maintenance_dialog {
            if let Some(confirmation) = &state.maintenance.confirmation {
                let width = cx
                    .layout_size("page-content")
                    .map(|size| (size[0] - 24.0).clamp(320.0, 520.0))
                    .unwrap_or(460.0);
                Dialog::new("maintenance-confirm-dialog", "Build actor search index")
                    .open(true)
                    .width(width)
                    .fit()
                    .click_outside(true)
                    .on_close(|state: &mut State| state.cancel_maintenance_confirmation())
                    .child(rune::col().w_full().px(16.0).pb(16.0).child(
                        crate::backend_views::maintenance_confirmation_content(
                            state,
                            confirmation,
                            preferences,
                            kit,
                        ),
                    ))
                    .build(kit, layers);
            }
        }
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
                resources = resources.child(ui_components::skeleton_metric_tiles(preferences, kit));
            }
            Load::Failed(error) => {
                resources = resources.child(line(error.clone(), kit).role("alert"));
            }
            Load::Ready(metrics) => {
                let stale_notice = state.host_metrics_stale_notice();
                if let Some(notice) = &stale_notice {
                    resources = resources.child(
                        ui_components::async_status_strip(
                            "host-metrics-stale",
                            "Telemetry unavailable",
                            notice.clone(),
                            false,
                            StatusTone::Warn,
                            preferences,
                            kit,
                        )
                        .role("alert"),
                    );
                }
                let mut tiles = if narrow_workstation {
                    rune::col().w_full().gap(10.0)
                } else {
                    rune::row().w_full().gap(10.0)
                };
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
                        if stale_notice.is_some() {
                            "Stale"
                        } else if valid.is_some() {
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
        body = body.child(gateway).child(resources);
    } else {
        body = body.child(gateway);
    }
    scroll(body.child(native), "workstation-body")
}

struct ActorSearchSummary {
    status_label: &'static str,
    tone: StatusTone,
    next_action: String,
    freshness: String,
    can_search: bool,
    building: bool,
    stage: Option<String>,
    scanned: Option<u64>,
    total: Option<u64>,
    needs_seed: bool,
    details: Vec<String>,
}

fn summarize_actor_search(state: &State) -> ActorSearchSummary {
    let reindex_job = active_or_selected_reindex_job(state);
    let receipt_job = match &state.maintenance.receipt {
        Load::Ready(receipt) if receipt.operation == MaintenanceOperation::ReindexActors => {
            Some(receipt.job_id.as_str())
        }
        _ => None,
    };
    let mut details = Vec::new();
    let (catalog_check, search_check, observed_at) = match &state.readiness {
        Load::Ready(readiness) => (
            readiness
                .checks
                .iter()
                .find(|check| check.id == ReadinessCheckId::ActorCatalog),
            readiness
                .checks
                .iter()
                .find(|check| check.id == ReadinessCheckId::ActorSearch),
            Some(readiness.observed_at.as_str()),
        ),
        _ => (None, None, None),
    };
    let search_fresh = search_check.is_some_and(|check| state.readiness_check_fresh(check));
    let catalog_ready = catalog_check.is_some_and(|check| {
        check.state == ReadinessState::Ready && state.readiness_check_fresh(check)
    });
    let capability_ready = state
        .connection
        .capabilities
        .as_ref()
        .is_some_and(|capabilities| capabilities.require_actor_search().is_ok());

    if let Some(check) = search_check {
        details.push(format!("Reason: {}", check.reason_code));
        if !check.job_ids.is_empty() {
            details.push(format!("Jobs: {}", check.job_ids.join(", ")));
        }
        if !check.artifact_ids.is_empty() {
            details.push(format!("Serving refs: {}", check.artifact_ids.join(", ")));
        }
        if !check.explanation.is_empty() {
            details.push(check.explanation.clone());
        }
    }
    if let Some(pending) = &state.maintenance.pending_reindex {
        details.push(format!("Pending request: {}", pending.request.request_id));
    }

    let freshness = match (observed_at, search_check) {
        (Some(ts), Some(_)) if search_fresh => format!("Observed {ts} · Fresh"),
        (Some(ts), _) => format!("Observed {ts} · Stale — refresh checks"),
        (None, _) => "Readiness has not been observed yet.".to_owned(),
    };

    let (status_label, tone, next_action, can_search, building, needs_seed) = if state
        .maintenance
        .pending_reindex
        .is_some()
    {
        (
            "Needs attention",
            StatusTone::Warn,
            "Check request status or clear the unresolved actor index attempt before rebuilding."
                .to_owned(),
            false,
            false,
            false,
        )
    } else if let Some(check) = search_check {
        let reason_lower = check.reason_code.to_ascii_lowercase();
        let explanation_lower = check.explanation.to_ascii_lowercase();
        let paused = reason_lower.contains("pause")
            || reason_lower.contains("paced")
            || explanation_lower.contains("paused");
        if check.state == ReadinessState::Ready && search_fresh {
            (
                "Ready",
                StatusTone::Ok,
                "Actor search is ready; open Search actors to query the catalog.".to_owned(),
                capability_ready,
                false,
                false,
            )
        } else if paused {
            (
                    "Paused",
                    StatusTone::Warn,
                    "Indexing is paused or write-paced on the backend; inspect job details or refresh checks."
                        .to_owned(),
                    false,
                    false,
                    false,
                )
        } else if check.state == ReadinessState::Running
            || reindex_job.is_some_and(|job| crate::backend::job_is_active(&job.state))
        {
            (
                    "Building",
                    StatusTone::Info,
                    "Actor search index build is active; readiness updates automatically when finished."
                        .to_owned(),
                    false,
                    true,
                    false,
                )
        } else if receipt_job.is_some() {
            (
                "Queued",
                StatusTone::Info,
                "Actor index request is queued on the backend; follow progress in Jobs.".to_owned(),
                false,
                false,
                false,
            )
        } else if !catalog_ready
            && catalog_check.is_some_and(|c| c.state == ReadinessState::NotReady)
        {
            (
                    "Seed actors first",
                    StatusTone::Warn,
                    "Seed a non-empty actor CSV catalog in DB Seed & Train before building the search index."
                        .to_owned(),
                    false,
                    false,
                    true,
                )
        } else if check.state == ReadinessState::NotReady {
            (
                "Not built",
                StatusTone::Warn,
                "Click Build actor index to generate the shared backend search projection."
                    .to_owned(),
                false,
                false,
                false,
            )
        } else if check.state == ReadinessState::Degraded || !search_fresh {
            (
                "Needs attention",
                StatusTone::Warn,
                "Refresh checks or inspect backend diagnostics to resolve actor search status."
                    .to_owned(),
                false,
                false,
                false,
            )
        } else {
            (
                    "Unavailable",
                    StatusTone::Err,
                    "Actor search backend status is currently unavailable; refresh checks or inspect gateway health."
                        .to_owned(),
                    false,
                    false,
                    false,
                )
        }
    } else if receipt_job.is_some() {
        (
            "Queued",
            StatusTone::Info,
            "Actor index request is queued on the backend; follow progress in Jobs.".to_owned(),
            false,
            false,
            false,
        )
    } else {
        (
            "Unavailable",
            StatusTone::Neutral,
            "Refresh checks to inspect actor catalog and search index readiness.".to_owned(),
            false,
            false,
            false,
        )
    };

    let (stage, scanned, total) = match reindex_job {
        Some(job) => (
            (!job.phase.is_empty()).then(|| job.phase.clone()),
            job.progress_done
                .and_then(|value| u64::try_from(value).ok()),
            job.progress_total
                .and_then(|value| u64::try_from(value).ok()),
        ),
        None => (None, None, None),
    };

    ActorSearchSummary {
        status_label,
        tone,
        next_action,
        freshness,
        can_search,
        building,
        stage,
        scanned,
        total,
        needs_seed,
        details,
    }
}

fn active_or_selected_reindex_job(state: &State) -> Option<&crate::backend::ActivityJob> {
    if let Load::Ready(detail) = &state.backend.detail {
        if detail.job.operation == "actor_reindex" {
            return Some(&detail.job);
        }
    }
    if let Load::Ready(jobs) = &state.backend.jobs {
        return jobs
            .rows
            .iter()
            .find(|job| job.operation == "actor_reindex");
    }
    None
}

fn actor_search_summary_card(state: &State, preferences: ViewPreferences, kit: &Kit) -> El {
    let pointer_ref = (!preferences.matte()).then_some(&state.pointer_offset);
    let shadow_ref = (!preferences.matte()).then_some(&state.shadow_offset);
    let summary = summarize_actor_search(state);
    let disabled_reason = state.maintenance_disabled_reason(MaintenanceOperation::ReindexActors);
    let refreshing = matches!(state.readiness, Load::Pending | Load::Loading);
    let busy_submit =
        state.maintenance.busy() || matches!(state.maintenance.receipt, Load::Loading);

    let mut card_body = rune::col()
        .padding(16.0)
        .gap(10.0)
        .key("actor-search-summary-card")
        .child(ui_components::section_header(
            &icons::BRAIN,
            "Actor search",
            Some(&summary.freshness),
            Some(ui_components::status_pill(
                summary.status_label,
                None,
                summary.tone,
                kit,
            )),
            Some(ui_components::status_pill(
                "Connected",
                None,
                StatusTone::Ok,
                kit,
            )),
            kit,
        ))
        .child(line(summary.next_action, kit));

    if summary.building || summary.scanned.is_some() {
        let stage_label = summary.stage.as_deref().unwrap_or("building");
        let progress_label = match (summary.scanned, summary.total) {
            (Some(done), Some(total)) if total > 0 => {
                format!("{stage_label} · Scanned {done} / {total}")
            }
            (Some(done), _) => format!("{stage_label} · Scanned {done} (total unknown)"),
            (None, _) => format!("Stage: {stage_label}"),
        };
        let frac = summary
            .scanned
            .zip(summary.total)
            .and_then(|(done, total)| {
                (total > 0).then(|| (done as f32 / total as f32).clamp(0.0, 1.0))
            });
        card_body = card_body.child(
            Progress::new(frac)
                .label(progress_label)
                .build(kit)
                .w_full(),
        );
    }

    let mut actions = rune::row().w_full().gap(8.0).center_y();
    actions = actions.child(
        ActionButton::new("Build actor index")
            .key("workstation-build-actor-index")
            .icon(&icons::BRAIN)
            .finish(if disabled_reason.is_none() && !summary.can_search {
                MetalFinish::GoldCta
            } else {
                MetalFinish::Gunmetal
            })
            .busy(busy_submit)
            .disabled(disabled_reason.is_some() || busy_submit)
            .on_press(|state: &mut State| {
                state.preview_maintenance(MaintenanceOperation::ReindexActors);
            })
            .build(kit, preferences, pointer_ref),
    );
    if summary.can_search {
        actions = actions.child(
            ActionButton::new("Search actors")
                .key("workstation-search-actors")
                .icon(&icons::BRAIN)
                .finish(MetalFinish::GoldCta)
                .on_press(|state: &mut State| state.navigate(Page::Brain))
                .build(kit, preferences, pointer_ref),
        );
    }
    if summary.needs_seed {
        actions = actions.child(
            ActionButton::new("Open DB Seed & Train")
                .key("workstation-open-seed")
                .icon(&icons::SEED)
                .finish(MetalFinish::GoldCta)
                .on_press(|state: &mut State| {
                    state.seed_stage = SeedStage::Seed;
                    state.navigate(Page::Seed);
                })
                .build(kit, preferences, pointer_ref),
        );
    }
    actions = actions.child(
        ActionButton::new("Refresh checks")
            .key("workstation-refresh-readiness")
            .icon(&icons::REFRESH)
            .finish(MetalFinish::ObsidianPill)
            .busy(refreshing)
            .disabled(refreshing)
            .on_press(|state: &mut State| state.refresh_readiness())
            .build(kit, preferences, pointer_ref),
    );
    card_body = card_body.child(actions);

    if let Some(banner) = crate::backend_views::actor_index_receipt_banner(state, preferences, kit)
    {
        card_body = card_body.child(banner);
    }
    if let Some(unresolved) =
        crate::backend_views::unresolved_actor_reindex_banner(state, preferences, kit)
    {
        card_body = card_body.child(unresolved);
    }
    if !summary.details.is_empty() {
        let mut details_col = rune::col().gap(4.0);
        for detail in summary.details.iter().take(6) {
            details_col = details_col.child(quiet(detail.clone(), kit));
        }
        card_body = card_body.child(
            ActionButton::new(if state.maintenance.reindex_details_open {
                "Hide details"
            } else {
                "Details"
            })
            .key("workstation-index-details-toggle")
            .finish(MetalFinish::ObsidianPill)
            .on_press(|state: &mut State| {
                state.maintenance.reindex_details_open = !state.maintenance.reindex_details_open;
            })
            .build(kit, preferences, pointer_ref),
        );
        if state.maintenance.reindex_details_open {
            card_body = card_body.child(
                Collapsible::new("Bounded index diagnostics", true)
                    .child(details_col)
                    .build(kit),
            );
        }
    }

    material::luminous_card(
        card_body,
        summary.tone,
        false,
        preferences,
        pointer_ref,
        shadow_ref,
    )
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
            list = list.child(ui_components::skeleton_actor_results(preferences, kit));
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
        Load::Pending | Load::Loading => ui_components::skeleton_dossier(preferences, kit),
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
    fn pending_approval_can_request_a_new_pairing_from_the_waiting_screen() {
        let challenge = PairingChallenge {
            pairing_id: "synthetic-pairing-request-0001".into(),
            approval_code: "SYNTHETICAPPROVALCODE0001".into(),
            expires_at_ms: 1_900_000_000_000,
            poll_after_ms: 1500,
        };
        let mut runtime = pairing_runtime(PairingStage::Waiting(challenge), (640.0, 480.0));
        let mut clock = 0.0;
        click(&mut runtime, "pairing-restart-waiting", &mut clock);
        assert!(runtime.state.intents.contains(&Intent::PairingRestart));
        assert!(matches!(
            runtime.state.pairing.stage,
            PairingStage::Working(_)
        ));
        assert!(runtime.find("pairing-approval-code").is_none());
    }

    #[test]
    fn actionable_pairing_failures_retry_or_request_new_approval_through_real_controls() {
        for failure in [
            PairingFailure::InvalidOrigin,
            PairingFailure::KeychainMissing,
            PairingFailure::KeychainDenied,
            PairingFailure::KeychainUnavailable,
            PairingFailure::SessionEnded,
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
                    | PairingFailure::SessionEnded
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
        assert!(runtime.find("workstation-registered").is_some());
        assert!(runtime.find("connect-id").is_none());
        let tree = runtime.accessibility_json().to_string();
        assert!(!tree.contains("Connect ID"));
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

    fn click_within(runtime: &mut Runtime<State>, key: &str, size: (f32, f32), clock: &mut f64) {
        runtime.update(|_, tx| tx.scroll_into_view(key));
        *clock += 1.0 / 60.0;
        runtime.frame(*clock);
        let (x, y) = runtime
            .find(key)
            .unwrap_or_else(|| panic!("visible control {key} at {size:?}"));
        assert!(
            (0.0..size.0).contains(&x) && (0.0..size.1).contains(&y),
            "control {key} ({x}, {y}) must fit inside viewport {size:?}"
        );
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        *clock += 1.0 / 60.0;
        runtime.frame(*clock);
    }

    #[test]
    fn workstation_hero_card_covers_actor_search_states_details_and_confirmation_escape() {
        use fleetworks_contracts::context::Permission;
        use fleetworks_contracts::readiness::{
            ReadinessAction, ReadinessActionId, ReadinessCheck, ReadinessCheckId, ReadinessState,
            WorkstationReadiness,
        };

        let mut initial = crate::tests::registered_fixture();
        if let Some(capabilities) = &mut initial.connection.capabilities {
            let mut search = fleetworks_contracts::capabilities::SearchCapabilities::default();
            search.actors = Some(Default::default());
            capabilities.search = Some(search);
        }
        initial.backend.profile = crate::app::Load::Ready(crate::backend::ProfileSnapshot {
            configuration_revision: 0,
            configuration_format: 1,
            accounts: vec![],
            tool_credentials: vec![],
            setup_required: false,
            portable_schema_version: 4,
        });
        let receipt = fleetworks_contracts::connect::RegistrationReceipt::new(
            "synthetic-deployment".into(),
            "synthetic-node".into(),
            "synthetic-registration".into(),
            Some("192.0.2.42".parse().expect("fixture IP")),
            1,
        );
        let mut cat = ReadinessCheck::new(
            ReadinessCheckId::ActorCatalog,
            ReadinessState::Ready,
            "ready".into(),
            "Catalog ready".into(),
            "2026-11-14T22:00:00Z".into(),
        );
        cat.fresh_for_ms = 30_000;
        let mut search = ReadinessCheck::new(
            ReadinessCheckId::ActorSearch,
            ReadinessState::NotReady,
            "not_built".into(),
            "Actor search index has not been built yet.".into(),
            "2026-11-14T22:00:00Z".into(),
        );
        search.fresh_for_ms = 30_000;
        search.actions.push(ReadinessAction::new(
            ReadinessActionId::RebuildActorIndex,
            Permission::MaintainBackend,
            true,
        ));
        let mut readiness = WorkstationReadiness::new(
            "synthetic-deployment".into(),
            "2026-11-14T22:00:00Z".into(),
            vec![cat, search],
        );
        readiness.registration = Some(receipt);
        initial.readiness = crate::app::Load::Ready(readiness);
        initial.readiness_received_at = Some(0.0);

        let mut rt = Runtime::new(
            initial,
            app::view,
            (1280.0, 800.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        let mut clock = 0.0;
        rt.frame(clock);
        // Frame 1 resolves layout sizes cleanly.
        clock += 1.0 / 60.0;
        rt.frame(clock);

        // Toggle bounded Details disclosure on the Workstation hero card.
        assert!(rt.find("actor-search-summary-card").is_some());
        click_within(
            &mut rt,
            "workstation-index-details-toggle",
            (1280.0, 800.0),
            &mut clock,
        );
        assert!(rt.state.maintenance.reindex_details_open);
        click_within(
            &mut rt,
            "workstation-index-details-toggle",
            (1280.0, 800.0),
            &mut clock,
        );
        assert!(!rt.state.maintenance.reindex_details_open);

        // Hero card renders distinct status badges & single-sentence guidance across backend states.
        let scenarios = [
            (
                ReadinessState::NotReady,
                ReadinessState::NotReady,
                "not_seeded",
                "Seed actors first",
            ),
            (
                ReadinessState::Ready,
                ReadinessState::NotReady,
                "not_built",
                "Not built",
            ),
            (
                ReadinessState::Ready,
                ReadinessState::NotReady,
                "paused",
                "Paused",
            ),
            (
                ReadinessState::Ready,
                ReadinessState::Degraded,
                "failed",
                "Needs attention",
            ),
            (
                ReadinessState::Ready,
                ReadinessState::Unavailable,
                "unavailable",
                "Unavailable",
            ),
        ];
        for (cat_state, search_state, reason, label) in scenarios {
            rt.update(move |state, _| {
                if let crate::app::Load::Ready(readiness) = &mut state.readiness {
                    readiness.checks.retain(|c| {
                        !matches!(
                            c.id,
                            ReadinessCheckId::ActorCatalog | ReadinessCheckId::ActorSearch
                        )
                    });
                    let mut cat = ReadinessCheck::new(
                        ReadinessCheckId::ActorCatalog,
                        cat_state,
                        "cat".into(),
                        "Catalog check".into(),
                        "2026-11-14T22:00:00Z".into(),
                    );
                    cat.fresh_for_ms = 30_000;
                    let mut search = ReadinessCheck::new(
                        ReadinessCheckId::ActorSearch,
                        search_state,
                        reason.into(),
                        format!("Explanation for {label}"),
                        "2026-11-14T22:00:00Z".into(),
                    );
                    search.fresh_for_ms = 30_000;
                    search.actions.push(ReadinessAction::new(
                        ReadinessActionId::RebuildActorIndex,
                        Permission::MaintainBackend,
                        true,
                    ));
                    readiness.checks.push(cat);
                    readiness.checks.push(search);
                }
            });
            clock += 1.0 / 60.0;
            rt.frame(clock);
            let tree = rt.accessibility_json().to_string();
            assert!(tree.contains("Connected"));
            assert!(tree.contains(label), "missing hero state {label}");
        }

        // Building state with unknown total keeps indeterminate progress & no invented percent.
        rt.update(|state, _| {
            if let crate::app::Load::Ready(readiness) = &mut state.readiness {
                if let Some(c) = readiness
                    .checks
                    .iter_mut()
                    .find(|c| c.id == ReadinessCheckId::ActorSearch)
                {
                    c.state = ReadinessState::NotReady;
                    c.reason_code = "building".into();
                }
            }
            let job: crate::backend::ActivityJob = serde_json::from_value(serde_json::json!({
                "id": "job-build-1",
                "app": "worker",
                "operation": "actor_reindex",
                "title": "Actor index",
                "state": "running",
                "phase": "scanning",
                "progress_done": 128,
                "progress_total": null,
                "error_summary": "",
                "private_details_available": false,
                "activity_actions_available": false,
                "status_only": false
            }))
            .unwrap();
            state.backend.jobs = crate::app::Load::Ready(crate::backend::ActivityJobs {
                rows: vec![job],
                activity_scope: fleetworks_contracts::context::ActivityScope::for_workstation(
                    state.registration.context.as_ref().unwrap(),
                ),
                relation_counts_available: false,
                activity_actions_available: false,
            });
        });
        clock += 1.0 / 60.0;
        rt.frame(clock);
        let building_tree = rt.accessibility_json().to_string();
        assert!(building_tree.contains("Building"));
        assert!(
            !building_tree.contains('%'),
            "unknown total cannot invent a percentage"
        );

        // Clicking Build actor index opens the modal confirmation, and Escape returns focus cleanly.
        click_within(
            &mut rt,
            "workstation-build-actor-index",
            (1280.0, 800.0),
            &mut clock,
        );
        assert!(rt.state.confirm_maintenance_dialog);
        let dialog_tree = rt.accessibility_json().to_string();
        assert!(dialog_tree.contains("Shared backend catalog"));
        assert!(rt.find("maintenance-confirm").is_some());
        assert!(rt.find("maintenance-cancel").is_some());
        rt.key(KeyEvent {
            key: Key::Escape,
            mods: Mods::default(),
        });
        clock += 1.0 / 60.0;
        rt.frame(clock);
        assert!(!rt.state.confirm_maintenance_dialog);
        assert!(rt.state.maintenance.confirmation.is_none());

        // Ready state offers Search actors which navigates directly to Page::Brain.
        rt.update(|state, _| {
            if let crate::app::Load::Ready(readiness) = &mut state.readiness {
                if let Some(c) = readiness
                    .checks
                    .iter_mut()
                    .find(|c| c.id == ReadinessCheckId::ActorSearch)
                {
                    c.state = ReadinessState::Ready;
                    c.reason_code = "ready".into();
                }
            }
        });
        clock += 1.0 / 60.0;
        rt.frame(clock);
        click_within(
            &mut rt,
            "workstation-search-actors",
            (1280.0, 800.0),
            &mut clock,
        );
        assert_eq!(rt.state.page, Page::Brain);
    }

    #[test]
    fn responsive_shell_and_jobs_layout_adapt_at_1364x1024_1280x800_and_640x480() {
        for size in [(1364.0, 1024.0), (1280.0, 800.0), (640.0, 480.0)] {
            let mut state = crate::tests::registered_fixture();
            state.page = Page::Jobs;
            let one_job: crate::backend::ActivityJob = serde_json::from_value(serde_json::json!({
                "id": "job-long-identifier-1234567890-abcdef-987654321",
                "app": "worker",
                "operation": "actor_reindex",
                "title": "Build actor index",
                "state": "completed",
                "phase": "completed",
                "progress_done": 100,
                "progress_total": 100,
                "error_summary": "",
                "private_details_available": true,
                "activity_actions_available": false,
                "status_only": false
            }))
            .unwrap();
            state.backend.jobs = crate::app::Load::Ready(crate::backend::ActivityJobs {
                rows: vec![one_job],
                activity_scope: fleetworks_contracts::context::ActivityScope::for_workstation(
                    state.registration.context.as_ref().unwrap(),
                ),
                relation_counts_available: false,
                activity_actions_available: false,
            });
            let mut rt = Runtime::new(
                state,
                app::view,
                size,
                Env {
                    system_dark: true,
                    scale: 1.0,
                },
            );
            let mut clock = 0.0;
            rt.frame(clock);
            clock += 1.0 / 60.0;
            rt.frame(clock);

            if size.0 < 820.0 {
                assert!(
                    rt.find("compact-icon-rail").is_some(),
                    "640x480 must collapse sidebar into compact-icon-rail"
                );
            } else {
                assert!(rt.find("compact-icon-rail").is_none());
            }
            assert!(rt.find("nav-workstation").is_some());
            assert!(rt.find("nav-jobs").is_some());
            assert!(rt.find("connection-editor").is_some());
            let jobs_tree = rt.accessibility_json().to_string();
            assert!(jobs_tree.contains("1 job") && !jobs_tree.contains("1 jobs"));

            // Selecting a job on compact width shows detail pane behind selection + Back button.
            if size.0 < 760.0 {
                rt.update(|s, _| {
                    s.backend.selected_job =
                        Some("job-long-identifier-1234567890-abcdef-987654321".into())
                });
                clock += 1.0 / 60.0;
                rt.frame(clock);
                assert!(rt.find("jobs-back-to-list").is_some());
                click_within(&mut rt, "jobs-back-to-list", size, &mut clock);
                assert!(rt.state.backend.selected_job.is_none());
            }
        }
    }

    #[test]
    fn setup_backend_mode_switches_between_remote_and_local_at_640x480_1280x800_and_1440x860() {
        use fleetworks_workstation::native::BackendKind;
        for size in [(640.0, 480.0), (1280.0, 800.0), (1440.0, 860.0)] {
            let mut rt = runtime(Page::Workstation, true, size);
            let mut clock = 0.0;
            rt.frame(clock);
            assert!(rt.find("backend-mode").is_some());
            assert!(rt.find("backend-mode-remote").is_some());
            assert!(rt.find("backend-mode-local").is_some());
            assert!(rt.find("gateway-origin").is_some());
            assert!(rt.find("connect").is_some());
            assert!(rt.find("local-backend-card").is_none());

            click_within(&mut rt, "backend-mode-local", size, &mut clock);
            assert_eq!(rt.state.backend_kind, BackendKind::Local);
            assert!(rt.find("gateway-origin").is_none());
            assert!(rt.find("local-backend-card").is_some());
            assert!(rt.find("local-state-dir").is_some());
            assert!(rt.find("local-binary-status").is_some());
            assert!(rt.find("connect-local").is_some());
            assert!(rt.find("switch-remote-gateway").is_some());

            // Verify keyboard focus and viewport fit for local mode controls.
            rt.update(|_, tx| tx.focus("connect-local", true));
            clock += 1.0 / 60.0;
            rt.frame(clock);
            let (cx, cy) = rt.find("connect-local").expect("connect-local coordinates");
            assert!(
                (0.0..size.0).contains(&cx) && (0.0..size.1).contains(&cy),
                "connect-local ({cx}, {cy}) outside viewport {size:?}"
            );

            click_within(&mut rt, "switch-remote-gateway", size, &mut clock);
            assert_eq!(rt.state.backend_kind, BackendKind::Remote);
            assert!(rt.find("gateway-origin").is_some());
            assert!(rt.find("local-backend-card").is_none());
        }
    }

    #[test]
    fn local_connected_setup_and_status_render_capability_warnings_and_pressure_alert() {
        use fleetworks_contracts::capabilities::Capabilities;
        use fleetworks_contracts::capabilities::CONNECT_REGISTRY_FEATURE;
        use fleetworks_workstation::native::BackendKind;

        let mut base = crate::tests::registered_fixture();
        base.backend_kind = BackendKind::Local;
        base.connection.origin = rune::TextEdit::new("local://embedded");
        base.local_backend.state_dir = Some(std::path::PathBuf::from("/private/tmp/local-state"));
        base.local_backend.pressure_paused = Some("memory_pressure".into());
        let mut caps = Capabilities::default();
        caps.deployment_id = Some("local-deployment".into());
        caps.features = vec![CONNECT_REGISTRY_FEATURE.into()];
        base.connection.capabilities = Some(caps);

        // Setup Gateway view in connected Local Embedded mode.
        base.setup_gateway = true;
        let mut setup_rt = Runtime::new(
            base,
            app::view,
            (1280.0, 800.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        setup_rt.frame(0.0);
        assert!(setup_rt.find("local-backend-card").is_some());
        assert!(setup_rt.find("local-capabilities-card").is_some());
        assert!(setup_rt.find("local-capability-warnings").is_some());
        assert!(setup_rt.find("local-pressure-paused-alert").is_some());
        assert!(setup_rt.find("stop-local-backend").is_some());
        let setup_tree = setup_rt.accessibility_json().to_string();
        assert!(setup_tree.contains("Optional capabilities unavailable"));
        assert!(setup_tree.contains("memory_pressure"));

        // Workstation Status view in connected Local Embedded mode.
        setup_rt.update(|state, _| state.setup_gateway = false);
        setup_rt.frame(1.0 / 60.0);
        assert!(setup_rt.find("gateway-backend-card").is_some());
        assert!(setup_rt.find("workstation-local-state-dir").is_some());
        assert!(setup_rt.find("workstation-stop-local-backend").is_some());
        assert!(setup_rt.find("local-pressure-paused-alert").is_some());
        assert!(setup_rt.find("local-capability-warnings").is_some());
    }
}
