//! Selected native workstation destinations. Topology presentation belongs to Intel Web.

use fleetworks_contracts::capabilities::Capabilities;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::readiness::MaintenanceOperation;
use fleetworks_contracts::readiness::ReadinessCheck;
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
use rune_kit::SegmentedControl;
use rune_kit::TextInput;

use crate::accessibility::ViewPreferences;
use crate::app::DossierFacet;
use crate::app::Load;
use crate::app::Page;
use crate::app::SeedStage;
use crate::app::State;
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
        // Keep the row at the top of the view where it is when rows are inserted above it, so a
        // continuation, refresh or eviction never moves a reader who has scrolled down.
        .scroll_anchor()
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
                "Embedded backend is stopped".into()
            } else {
                "Distributed gateway is disconnected".into()
            }
        }
        ConnectionStatus::Connecting => {
            if state.is_local_backend() {
                "Starting the embedded backend…".into()
            } else {
                "Checking distributed gateway authentication and compatibility…".into()
            }
        }
        ConnectionStatus::Connected => {
            if state.is_local_backend() {
                "Connected to the embedded backend".into()
            } else {
                "Connected to a compatible distributed gateway".into()
            }
        }
        ConnectionStatus::Failed(error) => error.clone(),
    }
}

fn backend_mode_card(
    state: &State,
    compact: bool,
    busy: bool,
    preferences: crate::accessibility::ViewPreferences,
    kit: &Kit,
) -> El {
    let mut options = rune::row().key("backend-mode").w_full().gap(8.0);
    for (kind, key, title, description, glyph) in [
        (
            BackendKind::Local,
            "backend-mode-local",
            "Embedded",
            "Run the bundled companion on this workstation.",
            &icons::WORKSTATION,
        ),
        (
            BackendKind::Remote,
            "backend-mode-remote",
            "Distributed",
            "Connect to a gateway over HTTPS or loopback HTTP.",
            &icons::SERVER,
        ),
    ] {
        let selected = (kind == BackendKind::Local) == state.is_local_backend();
        let mut option = rune::col()
            .key(key)
            .role("button")
            .label(title)
            .description(if selected {
                format!("Selected. {description}")
            } else {
                description.to_owned()
            })
            .focusable(!busy)
            .disabled(busy)
            .focus_ring(2.0, 2.0, kit.c.border_focus)
            .grow(1.0)
            .min_w(0.0)
            .padding(if compact { 9.0 } else { 12.0 })
            .gap(4.0)
            .child(
                rune::icon(glyph, 16.0)
                    .color(if selected {
                        kit.c.bg_accent
                    } else {
                        kit.c.icon_muted
                    })
                    .a11y_hidden(),
            )
            .child(heading(title, kit).a11y_hidden())
            .child(quiet(description, kit).selectable(false).a11y_hidden());
        if !busy {
            option = option
                .hover(kit.c.layer_03)
                .on_press(move |state: &mut State| state.set_backend_kind(kind));
        }
        options = options.child(material::choice_tile(option, selected, preferences));
    }
    material::responsive_card(
        rune::col()
            .padding(if compact { 8.0 } else { 16.0 })
            .gap(10.0)
            .child(ui_components::section_header(
                &icons::SERVER,
                "Backend type",
                None,
                None,
                None,
                kit,
            ))
            .child(options),
        compact,
        preferences,
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
                "Embedded IPC disconnected",
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
                "Embedded backend paused under resource pressure",
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
                "Embedded companion lifecycle",
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

fn local_setup_diagnostic(
    key: &str,
    title: &str,
    detail: impl Into<String>,
    tone: StatusTone,
    preferences: ViewPreferences,
    kit: &Kit,
) -> El {
    ui_components::async_status_strip(key, title, "", false, tone, preferences, kit).child(
        quiet(detail, kit)
            .key(format!("{key}-detail"))
            .w_full()
            .min_w(0.0)
            .wrap_anywhere(),
    )
}

fn local_backend_setup_card(
    state: &State,
    narrow: bool,
    compact: bool,
    preferences: crate::accessibility::ViewPreferences,
    kit: &Kit,
    pointer_ref: Option<&rune::Live<[f32; 2]>>,
    _shadow_ref: Option<&rune::Live<[f32; 2]>>,
) -> El {
    let mut body = rune::col()
        .padding(16.0)
        .gap(12.0)
        .key("local-backend-card")
        .child(ui_components::section_header(
            &icons::WORKSTATION,
            "Embedded backend",
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

    let (title, detail, tone) = match &state.local_backend.binary_status {
        BinaryDiscoveryStatus::Ready(path) => (
            "Companion binary verified",
            path.display().to_string(),
            StatusTone::Ok,
        ),
        BinaryDiscoveryStatus::Missing(reason) => (
            "Companion binary missing or invalid",
            reason.clone(),
            StatusTone::Err,
        ),
        BinaryDiscoveryStatus::Unknown => (
            "Companion binary resolution pending",
            "Sibling `argos-api-host`, `FLEETWORKS_LOCAL_BACKEND_BIN`, or `--local-backend-bin <PATH>` will be checked on start (PATH is never searched).".to_owned(),
            StatusTone::Neutral,
        ),
    };
    let binary_strip =
        local_setup_diagnostic("local-binary-status", title, detail, tone, preferences, kit);
    let binary_strip = if matches!(
        state.local_backend.binary_status,
        BinaryDiscoveryStatus::Missing(_)
    ) {
        binary_strip.role("alert")
    } else {
        binary_strip
    };
    let diagnostics = local_runtime_diagnostics(state, preferences, kit);

    if state.connected() {
        body = body
            .child(binary_strip)
            .children(diagnostics)
            .child(ui_components::async_status_strip(
                "connection-status",
                "Connected to the embedded backend",
                "Private Unix socket verified",
                false,
                StatusTone::Ok,
                preferences,
                kit,
            ))
            .child(registration_status(state, kit));
        if let Some(capabilities) = &state.connection.capabilities {
            body = body.child(local_capabilities_summary(capabilities, preferences, kit));
        }
        body = body.child(crate::readiness_cards::setup_section(
            state,
            preferences,
            kit,
        ));
        let stop_btn = ActionButton::new("Stop Embedded Backend")
            .key("stop-local-backend")
            .finish(MetalFinish::Gunmetal)
            .disabled(state.local_backend.stopping)
            .on_press(|state: &mut State| state.stop_local_backend())
            .build(kit, preferences, pointer_ref);
        let mut actions = if narrow {
            rune::col().w_full().gap(8.0)
        } else {
            rune::row().w_full().gap(8.0)
        };
        if !state.startup.active() && state.registration.context.is_some() {
            actions = actions.child(
                ActionButton::new("Done")
                    .key("gateway-setup-done")
                    .icon(&icons::SHIELD_CHECK)
                    .finish(MetalFinish::GoldCta)
                    .on_press_tx(State::finish_gateway_setup)
                    .build(kit, preferences, pointer_ref),
            );
        }
        actions = actions
            .child(stop_btn)
            .child(crate::backend_views::switch_to_distributed(
                state,
                preferences,
                kit,
            ));
        body = body.child(actions);
    } else {
        let connecting = state.connection.status == ConnectionStatus::Connecting;
        let start_btn = ActionButton::new("Start Embedded Backend")
            .key("connect-local")
            .icon(&icons::WORKSTATION)
            .finish(MetalFinish::GoldCta)
            .busy(connecting)
            .disabled(connecting)
            .on_press(|state: &mut State| state.connect_local())
            .build(kit, preferences, pointer_ref);
        let actions = if narrow {
            rune::col().w_full().gap(8.0)
        } else {
            rune::row().w_full().gap(8.0)
        }
        .child(start_btn)
        .child(crate::backend_views::switch_to_distributed(
            state,
            preferences,
            kit,
        ));
        body = body
            .child(actions)
            .child(binary_strip)
            .children(diagnostics);
        if let ConnectionStatus::Failed(error) = &state.connection.status {
            body = body.child(
                local_setup_diagnostic(
                    "local-connection-error",
                    "Embedded backend failed to start",
                    error.clone(),
                    StatusTone::Err,
                    preferences,
                    kit,
                )
                .role("alert"),
            );
        }
    }

    material::responsive_card(body, compact, preferences)
}

pub(crate) fn setup_gateway(state: &State, cx: &Cx<State>, kit: &Kit) -> El {
    let preferences = state
        .accessibility
        .for_view(state.fixture, cx.is_reduced_motion());
    let size = if state.startup.active() {
        cx.layout_size("startup").or(Some([640.0, 480.0]))
    } else {
        cx.layout_size("page-content")
    };
    scroll(
        setup_gateway_content(state, kit, preferences, size),
        "gateway-setup-body",
    )
}

pub(crate) fn setup_gateway_content(
    state: &State,
    kit: &Kit,
    preferences: ViewPreferences,
    size: Option<[f32; 2]>,
) -> El {
    let pointer_ref = (!preferences.matte()).then_some(&state.pointer_offset);
    let shadow_ref = (!preferences.matte()).then_some(&state.shadow_offset);
    let narrow = size.is_some_and(|size| size[0] < 520.0);
    // A short window also takes the compact group edge so every setup control stays reachable.
    let compact = narrow || size.is_some_and(|size| size[1] < 560.0);
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
            crate::submission::ATLAS_DISTRIBUTED_REQUIREMENT
        } else if cfg!(target_os = "macos") {
            "Pair once via macOS Keychain. Accepted jobs persist independently."
        } else {
            "Pair once via your system keyring or for this session. Accepted jobs persist independently."
        }),
        Some(ui_components::status_pill(
            match active_step {
                2 => {
                    if state.is_local_backend() {
                        "Ready"
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
                        "Embedded setup"
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
                &["Embedded backend", "IPC handshake", "Backend ready"]
            } else {
                &["Origin URL", "Workstation Pair", "Gateway Ready"]
            },
            active_step,
            preferences,
            kit,
        ));
    if state.connected() {
        body = body.children(crate::readiness_cards::primary_issue(
            state,
            crate::readiness_cards::PrimaryIssueScope::GatewaySetup,
            preferences,
            kit,
        ));
    }
    if !state.startup.active() {
        body = body.child(backend_mode_card(state, compact, busy, preferences, kit));
    }

    if state.is_local_backend() {
        body = body.child(local_backend_setup_card(
            state,
            narrow,
            compact,
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
    body
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
            .child(crate::readiness_cards::setup_section(
                state,
                preferences,
                kit,
            ));
        if !state.startup.active() && state.registration.context.is_some() {
            body = body.child(
                ActionButton::new("Done")
                    .key("gateway-setup-done")
                    .icon(&icons::SHIELD_CHECK)
                    .finish(MetalFinish::GoldCta)
                    .on_press_tx(State::finish_gateway_setup)
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
                .child(material::glass_panel(
                    rune::col()
                        .padding(12.0)
                        .gap(6.0)
                        .radius(8.0)
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
                    preferences,
                ))
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

fn workstation_registration_tag(state: &State, kit: &Kit) -> El {
    let (backend, icon) = if state.is_local_backend() {
        ("Embedded", &icons::WORKSTATION)
    } else {
        ("Distributed", &icons::SERVER)
    };
    let registration = if state.connected() {
        state.registration.context.as_ref()
    } else {
        None
    };
    let (label, tone) = if let Some(context) = registration {
        (context.registration_id.as_str(), StatusTone::Ok)
    } else if state.connected() {
        match state.registration.receipt {
            Load::Pending | Load::Loading => ("Registering…", StatusTone::Info),
            Load::Failed(_) => ("Registration failed", StatusTone::Err),
            _ => ("Unregistered", StatusTone::Neutral),
        }
    } else {
        ("Unregistered", StatusTone::Neutral)
    };
    rune::row()
        .key("workstation-registration-tag")
        .role("status")
        .label(if registration.is_some() {
            format!("{backend} registration ID {label}")
        } else {
            format!("{backend} · {label}")
        })
        .min_w(0.0)
        .max_w(520.0)
        .shrink()
        .min_h(22.0)
        .center_y()
        .gap(6.0)
        .px(8.0)
        .py(3.0)
        .radius(6.0)
        .fill(tone.bg().alpha(kit.c.bg_button_neutral.a))
        .border(1.0, tone.border())
        .child(rune::icon(icon, 14.0).color(tone.fg()).a11y_hidden())
        .child(
            rune::text(format!("{backend} · {label}"))
                .min_w(0.0)
                .shrink()
                .font(kit.ty.small_medium)
                .color(tone.fg())
                .wrap_anywhere()
                .selectable(registration.is_some())
                .a11y_hidden(),
        )
}

fn workstation(state: &State, cx: &Cx<State>, kit: &Kit, layers: &mut Vec<El>) -> El {
    let preferences = state
        .accessibility
        .for_view(state.fixture, cx.is_reduced_motion());
    let pointer_ref = (!preferences.matte()).then_some(&state.pointer_offset);

    let transport = match &state.connection.status {
        ConnectionStatus::Connected => ("Connected", StatusTone::Ok),
        ConnectionStatus::Connecting => ("Connecting", StatusTone::Warn),
        ConnectionStatus::Failed(_) => ("Offline", StatusTone::Err),
        ConnectionStatus::Disconnected => ("Disconnected", StatusTone::Neutral),
    };
    let deployment = state
        .connection
        .capabilities
        .as_ref()
        .and_then(|capabilities| capabilities.deployment_id.as_deref())
        .filter(|id| state.connected() && !id.is_empty());
    let transport_kind = if state.is_local_backend() {
        "IPC"
    } else if state.connection.origin.text.starts_with("https://") {
        "HTTPS"
    } else {
        "HTTP"
    };
    let mut body = rune::col().gap(10.0).child(
        rune::col()
            .key("workstation-status-header")
            .role("region")
            .label("Workstation Status")
            .w_full()
            .child(
                rune::row()
                    .w_full()
                    .min_w(0.0)
                    .flex_wrap()
                    .center_y()
                    .gap(6.0)
                    .child(workstation_registration_tag(state, kit))
                    .child(
                        ui_components::status_pill(
                            format!("{transport_kind} · {}", transport.0),
                            None,
                            transport.1,
                            kit,
                        )
                        .key("connection-status")
                        .label(format!("Transport: {}", transport.0))
                        .description(transport_kind),
                    ),
            ),
    );
    let mut services = rune::col()
        .key("workstation-services")
        .w_full()
        .gap(8.0)
        .child(heading("Services", kit).role("heading"));
    if state.connected() {
        if state.registration.context.is_none() {
            if state.can_register() {
                services = services.child(
                    ActionButton::new("Retry registration")
                        .key("workstation-register")
                        .finish(MetalFinish::Gunmetal)
                        .on_press(|state: &mut State| state.ensure_registration())
                        .build(kit, preferences, pointer_ref),
                );
            }
        } else {
            let actor = summarize_actor_search(state);
            let prefer_actor = actor.needs_seed
                || actor.building
                || actor.background_indexing
                || actor.status_label == "Queued"
                || state.maintenance.pending_reindex.is_some()
                || state
                    .maintenance_disabled_reason(MaintenanceOperation::ReindexActors)
                    .is_none()
                    && !actor.can_search;
            let active_operation = workstation_active_operation(state, preferences, kit);
            let issue = crate::readiness_cards::primary_issue(
                state,
                crate::readiness_cards::PrimaryIssueScope::Workstation {
                    include_other_blockers: !prefer_actor && active_operation.is_none(),
                },
                preferences,
                kit,
            );
            let show_actor_action = issue.is_none() && active_operation.is_none();
            services = services
                .child(actor_search_summary_card(
                    state,
                    preferences,
                    kit,
                    actor,
                    show_actor_action,
                ))
                .children(issue.or(active_operation));
            if state.permission(Permission::ReadWorkstationActivity) {
                services = services.child(workstation_job_counts(state, preferences, kit));
            }
        }
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
                    .on_close(State::close_workstation_index_confirmation)
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
    } else {
        services = services.child(
            ActionButton::new("Setup Gateway")
                .key("workstation-setup-gateway")
                .icon(&icons::KEY)
                .finish(MetalFinish::Gunmetal)
                .on_press(State::open_connection_settings)
                .build(kit, preferences, pointer_ref),
        );
    }
    body = body.child(services);
    body = body.child(crate::readiness_cards::workstation_section(
        state,
        preferences,
        kit,
    ));

    let mut diagnostics = rune::col()
        .gap(8.0)
        .key("gateway-backend-card")
        .child(line(connection_status(state), kit))
        .children(deployment.map(|id| quiet(format!("Deployment {id}"), kit)));
    if let Load::Failed(error) = &state.registration.receipt {
        diagnostics = diagnostics.child(quiet(error.clone(), kit).role("alert"));
    }
    if state.is_local_backend() {
        diagnostics = diagnostics
            .child(
                quiet(
                    format!("Private 0700 state dir: {}", state.local_state_dir_label()),
                    kit,
                )
                .key("workstation-local-state-dir"),
            )
            .children(local_runtime_diagnostics(state, preferences, kit));
        if state.connected() {
            if let Some(capabilities) = &state.connection.capabilities {
                diagnostics =
                    diagnostics.child(local_capabilities_summary(capabilities, preferences, kit));
            }
            diagnostics = diagnostics.child(
                ActionButton::new("Stop Embedded Backend")
                    .key("workstation-stop-local-backend")
                    .finish(MetalFinish::Gunmetal)
                    .disabled(state.local_backend.stopping)
                    .on_press(|state: &mut State| state.stop_local_backend())
                    .build(kit, preferences, pointer_ref),
            );
        }
    } else {
        diagnostics = diagnostics.child(quiet(state.connection.origin.text.clone(), kit));
    }
    scroll(
        body.child(
            Collapsible::new("Diagnostics", state.workstation_diagnostics_open)
                .on_toggle(|state: &mut State| {
                    state.workstation_diagnostics_open = !state.workstation_diagnostics_open;
                })
                .child(diagnostics)
                .build(kit)
                .key("workstation-diagnostics"),
        ),
        "workstation-body",
    )
}

fn workstation_job_counts(
    state: &State,
    preferences: crate::accessibility::ViewPreferences,
    kit: &Kit,
) -> El {
    let counts = state.registration.context.as_ref().and_then(|context| {
        state.scoped_jobs.page.as_ref().and_then(|page| {
            let scope = &page.activity_scope;
            (state.permission(Permission::ReadWorkstationActivity)
                && page.dashboard_counts_scope == "unfiltered_top_level_workstation_activity"
                && scope.deployment_id == context.deployment_id
                && scope.workstation_id == context.workstation_id
                && scope.scope_revision == context.scope_revision)
                .then_some(page.counts.as_ref())
                .flatten()
        })
    });
    let last_known = counts.is_some() && state.scoped_jobs.message.is_some();
    let mut tiles = rune::row().w_full().gap(6.0);
    for (key, name, count) in [
        (
            "workstation-jobs-queued",
            "Queued",
            counts.map(|counts| counts.queued),
        ),
        (
            "workstation-jobs-running",
            "Running",
            counts.map(|counts| counts.active),
        ),
        (
            "workstation-jobs-retrying",
            "Retrying",
            counts.map(|counts| counts.retrying),
        ),
    ] {
        let value = count.map(|count| count.to_string());
        let label = format!("{name}: {}", value.as_deref().unwrap_or("Unavailable"));
        let description = if last_known {
            "Last known workstation job count. Open Jobs & Logs."
        } else {
            "Workstation job count. Open Jobs & Logs."
        };
        tiles = tiles.child(material::canvas_tile(
            rune::col()
                .key(key)
                .role("button")
                .label(label)
                .description(description)
                .focusable(true)
                .focus_ring(2.0, 0.0, kit.c.border_focus)
                .min_w(0.0)
                .grow(1.0)
                .padding(7.0)
                .gap(1.0)
                .child(
                    quiet(format!("{name} jobs"), kit)
                        .selectable(false)
                        .a11y_hidden(),
                )
                .child(
                    rune::text(value.unwrap_or_else(|| "—".to_owned()))
                        .font(kit.ty.body_medium)
                        .color(kit.c.text_base)
                        .a11y_hidden(),
                )
                .on_press(|state: &mut State| {
                    state.select_activity_tab(crate::logs::ActivityTab::Jobs);
                    state.navigate(Page::Jobs);
                }),
            false,
            preferences,
        ));
    }
    rune::col()
        .key("workstation-jobs-counts")
        .role("group")
        .label("Workstation jobs")
        .min_w(0.0)
        .gap(4.0)
        .children(last_known.then(|| {
            unsynced_glyph(
                "workstation-jobs-unsynced",
                "Last known workstation job counts. Open Jobs & Logs for details.",
                kit,
            )
        }))
        .child(tiles)
}

fn workstation_active_operation(
    state: &State,
    preferences: ViewPreferences,
    kit: &Kit,
) -> Option<El> {
    let record = state
        .maintenance
        .operation_job(MaintenanceOperation::SeedActors)?;
    if record
        .detail
        .as_ref()
        .is_some_and(|detail| !crate::backend::job_is_active(&detail.job.state))
    {
        return None;
    }
    let (label, phase) = record.detail.as_ref().map_or(("Queued", ""), |detail| {
        let label = if detail.job.state == "running" {
            "Running"
        } else {
            "Queued"
        };
        (label, detail.job.phase.as_str())
    });
    let job_id = record.job_id.clone();
    Some(
        rune::row()
            .key("workstation-active-operation")
            .flex_wrap()
            .center_y()
            .gap(6.0)
            .child(heading("Seed actors", kit))
            .child(ui_components::status_pill(
                label,
                None,
                StatusTone::Info,
                kit,
            ))
            .children((!phase.is_empty()).then(|| quiet(phase.to_owned(), kit)))
            .child(
                ActionButton::new("View job")
                    .key("workstation-active-job")
                    .icon(&icons::JOBS)
                    .on_press(move |state: &mut State| state.open_job(&job_id))
                    .build(kit, preferences, None),
            ),
    )
}

/// Small, muted unsynced indicator for last-good data whose refresh is out of sync.
/// Distinct from the offline gateway indicator; exposes age/reason on demand via accessibility.
pub(crate) fn unsynced_glyph(key: &str, reason: impl Into<String>, kit: &Kit) -> El {
    let reason = reason.into();
    rune::row()
        .key(key.to_owned())
        .center()
        .w(20.0)
        .h(20.0)
        .radius(999.0)
        .fill(StatusTone::Warn.bg())
        .role("status")
        .label(reason)
        .child(
            rune::icon(&icons::REFRESH, 12.0)
                .color(kit.c.text_muted)
                .a11y_hidden(),
        )
}

struct ActorSearchSummary {
    status_label: &'static str,
    tone: StatusTone,
    next_action: String,
    unsynced_reason: Option<&'static str>,
    can_search: bool,
    building: bool,
    background_indexing: bool,
    stage: Option<String>,
    scanned: Option<u64>,
    total: Option<u64>,
    needs_seed: bool,
}

fn summarize_actor_search(state: &State) -> ActorSearchSummary {
    let reindex_job = active_actor_reindex(state);
    let receipt_job = queued_actor_reindex_id(state);
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
    let capability_ready = state
        .connection
        .capabilities
        .as_ref()
        .is_some_and(|capabilities| capabilities.require_actor_search().is_ok());

    let unsynced_reason = (observed_at.is_some()
        && (!search_fresh || state.readiness_error.is_some()))
    .then_some("Actor search observation unsynced. Open Status checks for details.");

    let paused = search_check.is_some_and(actor_search_is_paused);
    let catalog_needs_data =
        catalog_check.is_some_and(|check| actor_catalog_needs_data(state, check));
    let background_indexing =
        search_check.is_some_and(|check| actor_search_is_background_indexing(state, check));
    let can_query_current_index = capability_ready
        && search_fresh
        && search_check.is_some_and(|check| check.state == ReadinessState::Ready);

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
    } else if let Some(job) = reindex_job {
        let (label, guidance) = match job.state.as_str() {
            "queued" | "pending" => ("Queued", "The actor index request is queued."),
            "retry_scheduled" => ("Retrying", "The actor index job is scheduled to retry."),
            _ => ("Building", "The actor index job is running."),
        };
        (
            label,
            StatusTone::Info,
            guidance.to_owned(),
            can_query_current_index,
            true,
            false,
        )
    } else if receipt_job.is_some() {
        (
            "Queued",
            StatusTone::Info,
            "Actor index request is queued on the backend; follow progress in Jobs.".to_owned(),
            can_query_current_index,
            false,
            false,
        )
    } else if background_indexing {
        (
            "Background indexing",
            StatusTone::Info,
            crate::readiness_cards::ACTOR_INDEX_CATCH_UP.to_owned(),
            false,
            false,
            false,
        )
    } else if catalog_needs_data {
        (
            "Needs data",
            StatusTone::Warn,
            "Seed an actor CSV catalog before building the search index.".to_owned(),
            false,
            false,
            true,
        )
    } else if let Some(check) = search_check {
        if check.state == ReadinessState::Ready && search_fresh {
            (
                "Ready",
                StatusTone::Ok,
                "Actor search is ready; open Search actors to query the catalog.".to_owned(),
                can_query_current_index,
                false,
                false,
            )
        } else if paused {
            (
                "Paused",
                StatusTone::Warn,
                "Indexing is paused or write-paced on the backend. Open Status checks for details."
                    .to_owned(),
                false,
                false,
                false,
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
                "Refresh checks or open Status checks for details on actor search.".to_owned(),
                false,
                false,
                false,
            )
        } else {
            (
                    "Unavailable",
                    StatusTone::Err,
                    "Actor search status is unavailable. Refresh checks or open Status checks for details."
                        .to_owned(),
                    false,
                    false,
                    false,
                )
        }
    } else {
        (
            if matches!(state.readiness, Load::Pending | Load::Loading) {
                "Checking"
            } else {
                "Unavailable"
            },
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
        unsynced_reason,
        can_search,
        building,
        background_indexing,
        stage,
        scanned,
        total,
        needs_seed,
    }
}

pub(crate) fn actor_search_is_background_indexing(state: &State, check: &ReadinessCheck) -> bool {
    check.id == ReadinessCheckId::ActorSearch
        && check.state == ReadinessState::Running
        && state.readiness_check_fresh(check)
        && state.maintenance.pending_reindex.is_none()
        && active_actor_reindex(state).is_none()
        && queued_actor_reindex_id(state).is_none()
        && !matches!(&state.readiness, Load::Ready(readiness) if readiness
            .checks
            .iter()
            .any(|check| actor_catalog_needs_data(state, check)))
        && !actor_search_is_paused(check)
}

fn actor_search_is_paused(check: &ReadinessCheck) -> bool {
    let reason = check.reason_code.to_ascii_lowercase();
    reason.contains("pause")
        || reason.contains("paced")
        || check.explanation.to_ascii_lowercase().contains("paused")
}

fn actor_catalog_needs_data(state: &State, check: &ReadinessCheck) -> bool {
    check.id == ReadinessCheckId::ActorCatalog
        && check.state == ReadinessState::NotReady
        && state.readiness_check_fresh(check)
}

pub(crate) fn active_actor_reindex(state: &State) -> Option<&crate::backend::ActivityJob> {
    let rows = match &state.backend.jobs {
        Load::Ready(jobs) => jobs.rows.as_slice(),
        _ => &[],
    };
    let selected = match &state.backend.detail {
        Load::Ready(detail) if detail.job.operation == "actor_reindex" => Some(&detail.job),
        _ => None,
    };
    let receipt_detail = state
        .maintenance
        .operation_job(MaintenanceOperation::ReindexActors)
        .and_then(|record| record.detail.as_ref())
        .map(|detail| &detail.job);
    let details = [selected, receipt_detail];
    rows.iter()
        .filter(|job| job.operation == "actor_reindex")
        .filter(|job| {
            !details
                .into_iter()
                .flatten()
                .any(|detail| detail.id == job.id && !crate::backend::job_is_active(&detail.state))
        })
        .chain(details.into_iter().flatten().filter(|detail| {
            !rows
                .iter()
                .any(|row| row.id == detail.id && !crate::backend::job_is_active(&row.state))
        }))
        .find(|job| crate::backend::job_is_active(&job.state))
}

pub(crate) fn queued_actor_reindex_id(state: &State) -> Option<&str> {
    let id = if let Some(record) = state
        .maintenance
        .operation_job(MaintenanceOperation::ReindexActors)
    {
        if record.detail.is_some() {
            return None;
        }
        record.job_id.as_str()
    } else if let Load::Ready(receipt) = &state.maintenance.receipt {
        if receipt.operation != MaintenanceOperation::ReindexActors || receipt.already_completed {
            return None;
        }
        receipt.job_id.as_str()
    } else {
        return None;
    };
    let observed = matches!(&state.backend.jobs, Load::Ready(jobs) if jobs.rows.iter().any(|job| job.id == id))
        || matches!(&state.backend.detail, Load::Ready(detail) if detail.job.id == id);
    (!observed && !id.trim().is_empty()).then_some(id)
}

fn open_workstation_seed(state: &mut State) {
    if summarize_actor_search(state).needs_seed {
        state.seed_stage = SeedStage::Seed;
        state.navigate(Page::Seed);
    }
}

fn workstation_search_action() -> ActionButton {
    ActionButton::new("Search actors")
        .key("workstation-search-actors")
        .icon(&icons::BRAIN)
        .on_press(|state: &mut State| {
            if summarize_actor_search(state).can_search {
                state.navigate(Page::Brain);
            }
        })
}

fn actor_search_summary_card(
    state: &State,
    preferences: ViewPreferences,
    kit: &Kit,
    summary: ActorSearchSummary,
    show_actor_action: bool,
) -> El {
    let pointer_ref = (!preferences.matte()).then_some(&state.pointer_offset);
    let disabled_reason = state.maintenance_disabled_reason(MaintenanceOperation::ReindexActors);
    let refreshing =
        state.readiness_pending() || matches!(state.readiness, Load::Pending | Load::Loading);
    let busy_submit =
        state.maintenance.busy() || matches!(state.maintenance.receipt, Load::Loading);

    let (status_label, tone) =
        if summary.status_label == "Ready" && summary.unsynced_reason.is_some() {
            ("Unsynced", StatusTone::Warn)
        } else {
            (summary.status_label, summary.tone)
        };
    let status_slot = rune::row().w(132.0).center_y().child(
        ui_components::status_pill(status_label, None, tone, kit).key("actor-search-status-tag"),
    );

    let relevant_job = active_actor_reindex(state)
        .map(|job| job.id.as_str())
        .or_else(|| queued_actor_reindex_id(state));
    let primary = if state.maintenance.pending_reindex.is_some() {
        ActionButton::new("Check request")
            .key("workstation-check-index-request")
            .icon(&icons::REFRESH)
            .disabled(state.maintenance.busy())
            .on_press(|state: &mut State| state.reconcile_actor_reindex())
    } else if summary.needs_seed {
        ActionButton::new("Seed actors…")
            .key("workstation-open-seed")
            .icon(&icons::SEED)
            .on_press(open_workstation_seed)
    } else if let Some(job_id) = relevant_job {
        let id = job_id.to_owned();
        ActionButton::new("View job")
            .key("workstation-view-index-job")
            .icon(&icons::JOBS)
            .on_press(move |state: &mut State| state.open_job(&id))
    } else if summary.can_search {
        workstation_search_action()
    } else if disabled_reason.is_none() && !busy_submit && !summary.background_indexing {
        ActionButton::new("Build actor index")
            .key(crate::maintenance::WORKSTATION_REINDEX_ACTION_KEY)
            .icon(&icons::BRAIN)
            .on_press(|state: &mut State| {
                state.preview_maintenance(MaintenanceOperation::ReindexActors)
            })
    } else {
        let failed = matches!(state.readiness, Load::Failed(_));
        ActionButton::new(if failed { "Retry" } else { "Refresh checks" })
            .key(if failed {
                "workstation-readiness-retry"
            } else {
                "workstation-refresh-readiness"
            })
            .icon(&icons::REFRESH)
            .busy(refreshing)
            .disabled(refreshing)
            .on_press(|state: &mut State| state.refresh_readiness())
    };
    let primary = primary
        .finish(MetalFinish::Gunmetal)
        .build(kit, preferences, pointer_ref)
        .w(146.0);
    let primary = if summary.needs_seed {
        primary.on_key(|state: &mut State, event, tx| {
            if event.key != rune::Key::Enter || event.mods != rune::Mods::default() {
                return false;
            }
            if !tx.key_repeat() {
                open_workstation_seed(state);
            }
            true
        })
    } else {
        primary
    };
    let details_toggle = ActionButton::new(if state.maintenance.reindex_details_open {
        "Hide details"
    } else {
        "Details"
    })
    .key("workstation-index-details-toggle")
    .finish(MetalFinish::ObsidianPill)
    .on_press(|state: &mut State| {
        state.maintenance.reindex_details_open = !state.maintenance.reindex_details_open;
    })
    .build(kit, preferences, pointer_ref)
    .w(96.0);
    let mut card_body = rune::col()
        .padding(10.0)
        .gap(6.0)
        .key("actor-search-summary-card")
        .child(
            rune::row()
                .w_full()
                .gap(6.0)
                .center_y()
                .flex_wrap()
                .child(heading("Actor search", kit).min_w(104.0))
                .child(status_slot)
                .child(
                    rune::row()
                        .w(146.0)
                        .h(32.0)
                        .children(show_actor_action.then_some(primary)),
                )
                .child(details_toggle),
        );
    if state.maintenance.reindex_details_open {
        let mut details_col = rune::col().gap(4.0).child(quiet(summary.next_action, kit));
        if summary.building {
            let stage = summary.stage.as_deref().unwrap_or(summary.status_label);
            let progress = match (summary.scanned, summary.total) {
                (Some(done), Some(total)) if total > 0 => {
                    format!("{stage} · {done} / {total} scanned")
                }
                (Some(done), _) => format!("{stage} · {done} scanned"),
                (None, _) => stage.to_owned(),
            };
            details_col = details_col.child(quiet(progress, kit));
        }
        if let Some(pending) = &state.maintenance.pending_reindex {
            details_col = details_col.child(quiet(
                format!("Pending request: {}", pending.request.request_id),
                kit,
            ));
        }
        if let Some(reason) = disabled_reason {
            details_col = details_col.child(quiet(reason, kit));
        }
        if let Some(reason) = summary.unsynced_reason {
            details_col = details_col.child(quiet(reason.to_owned(), kit));
        }
        card_body = card_body.child(
            details_col
                .children(
                    (summary.can_search && relevant_job.is_some())
                        .then(|| workstation_search_action().build(kit, preferences, pointer_ref)),
                )
                .children(crate::backend_views::actor_index_receipt_banner(
                    state,
                    preferences,
                    kit,
                ))
                .children(crate::backend_views::unresolved_actor_reindex_banner(
                    state,
                    preferences,
                    kit,
                )),
        );
    }

    material::glass(
        card_body.padding(24.0).radius(20.0),
        material::GlassSurface::Panel,
        preferences,
    )
}

fn index_state_label(state: IndexState) -> (&'static str, StatusTone) {
    match state {
        IndexState::Ready => ("Search ready", StatusTone::Ok),
        IndexState::Indexing
        | IndexState::Paused
        | IndexState::Throttled
        | IndexState::Stale
        | IndexState::Degraded => ("Index catching up", StatusTone::Warn),
        IndexState::NotBuilt | IndexState::Unavailable | IndexState::Unknown | _ => {
            ("Not built", StatusTone::Warn)
        }
    }
}

fn brain(state: &State, cx: &mut Cx<State>, kit: &Kit) -> El {
    let preferences = state
        .accessibility
        .for_view(state.fixture, cx.is_reduced_motion());
    let pointer_ref = (!preferences.matte()).then_some(&state.pointer_offset);
    let shadow_ref = (!preferences.matte()).then_some(&state.shadow_offset);
    let search = TextInput::new(&state.brain.query)
        .label("Search actors")
        .placeholder("Search actors")
        .width(None)
        .on_edit(|state: &mut State| &mut state.brain.query)
        .on_change(|state: &mut State, tx| state.query_edited(tx))
        .on_key(|state: &mut State, event, tx| state.search_key(event, tx.key_repeat()))
        .on_submit(|state: &mut State, _| state.search_now())
        .build(kit)
        .key("actor-search");
    let mut list = rune::col()
        .gap(8.0)
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
        Load::Pending | Load::Loading => {
            // Debounce and the first fetch share one results-shaped skeleton: no narration, no
            // layout change when the request starts.
            list = list.child(ui_components::skeleton_actor_results(preferences, kit));
        }
        Load::Failed(error) => {
            list = list.child(
                rune::col()
                    .gap(8.0)
                    .child(line(error.clone(), kit).key("search-error").role("alert"))
                    .child(
                        ActionButton::new("Retry search")
                            .key("search-retry")
                            .icon(&icons::REFRESH)
                            .finish(MetalFinish::Gunmetal)
                            .disabled(
                                !state.search_available()
                                    || state.brain.query.text.trim().is_empty(),
                            )
                            .on_press(|state: &mut State| state.search_now())
                            .build(kit, preferences, pointer_ref),
                    ),
            );
        }
        Load::Ready(response) => {
            let current = response.index.state == IndexState::Ready;
            if !current {
                let (index_label, index_tone) = index_state_label(response.index.state);
                list = list.child(
                    ui_components::status_pill(index_label, Some(&icons::ALERT), index_tone, kit)
                        .label(format!("{index_label} ({})", response.index.state))
                        .key("index-status"),
                );
            }
            if response.rows.is_empty() && current {
                list = list.child(quiet("No matching actors.", kit).key("search-empty"));
            }
            for (index, hit) in response.rows.iter().enumerate() {
                let selected = state.brain.highlighted == Some(index);
                let expanded = state.brain.showing_detail
                    && state.brain.selected_actor.as_deref() == Some(hit.actor_id.as_str());
                let tile = material::interactive_tile(
                    rune::row()
                        .w_full()
                        .padding(8.0)
                        .gap(8.0)
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
                            rune::icon(
                                if expanded {
                                    &icons::CHEVRON_DOWN
                                } else {
                                    &icons::CHEVRON_RIGHT
                                },
                                14.0,
                            )
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
                .expanded(expanded)
                .focusable(true)
                .focus_ring(2.0, 2.0, theme::hex(theme::ACCENT_HIGHLIGHT))
                .press(theme::hex(theme::SELECTION_SURFACE))
                .on_press(move |state: &mut State| state.toggle_actor(index))
                .on_key(move |state: &mut State, event, tx| {
                    if event.key == rune::Key::Enter && event.mods == rune::Mods::default() {
                        if !tx.key_repeat() {
                            state.toggle_actor(index);
                        }
                        true
                    } else {
                        false
                    }
                });
                let mut record = rune::col()
                    .key(format!("actor-record-{}", hit.actor_id))
                    .w_full()
                    .gap(8.0)
                    .child(tile);
                if expanded {
                    record = record.child(dossier(state, preferences, kit));
                }
                list = list.child(record);
            }
            if response.truncated {
                list = list.child(quiet(
                    "Results are bounded. Refine the search to narrow matches.",
                    kit,
                ));
            }
        }
    }
    scroll(list, "actor-results")
}

fn dossier(state: &State, preferences: crate::accessibility::ViewPreferences, kit: &Kit) -> El {
    let pointer_ref = (!preferences.matte()).then_some(&state.pointer_offset);
    let shadow_ref = (!preferences.matte()).then_some(&state.shadow_offset);
    let body = match &state.brain.dossier {
        Load::Idle => material::luminous_card(
            rune::col()
                .padding(12.0)
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
        Load::Failed(error) => rune::col()
            .gap(8.0)
            .child(line(error.clone(), kit).key("dossier-error").role("alert"))
            .child(
                ActionButton::new("Retry dossier")
                    .key("dossier-retry")
                    .finish(MetalFinish::Gunmetal)
                    .on_press(|state: &mut State| {
                        state.retry_dossier();
                    })
                    .build(kit, preferences, pointer_ref),
            ),
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
                .padding(12.0)
                .gap(12.0)
                .child(
                    rune::col()
                        .w_full()
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
    rune::col()
        .key("actor-dossier")
        .w_full()
        .transition(crate::app::surface_transition(
            preferences.reduced_motion,
            Transition::offset(0.0, -4.0),
        ))
        // The disclosure owns the entrance. Loading and resolved content share its position.
        .instant_changes()
        .child(body.w_full())
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
    use fleetworks_contracts::readiness::ReadinessCheckId;
    use fleetworks_contracts::readiness::ReadinessState;
    use rune::Env;
    use rune::Key;
    use rune::KeyEvent;
    use rune::Mods;
    use rune::Runtime;

    use crate::app;
    use crate::app::Intent;
    use crate::app::Load;
    use crate::app::Page;
    use crate::app::SeedStage;
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
    fn status_exposes_identity_and_scoped_disclosures_at_both_sizes() {
        for size in [(1180.0, 780.0), (640.0, 480.0)] {
            let runtime = runtime(Page::Workstation, false, size);
            let tree = runtime.accessibility_json().to_string();
            for label in [
                "Workstation Status",
                "Transport: Disconnected",
                "Unregistered",
                "Status checks · offline",
            ] {
                assert!(tree.contains(label), "missing {label} at {size:?}");
            }
            assert!(runtime.find("workstation-diagnostics/trigger").is_some());
            assert!(runtime.find("status-checks-toggle/trigger").is_some());
            assert!(runtime.find("backend-readiness-checks").is_none());
            assert!(runtime.find("gateway-backend-card").is_none());
            assert!(runtime.find("workstation-services").is_some());
            assert!(runtime.find("workstation-jobs-counts").is_none());
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
            for label in [
                "Workstation Status",
                "Services",
                "Distributed registration ID synthetic-registration",
                "Transport: Connected",
            ] {
                assert!(
                    connected_tree.contains(label),
                    "missing connected {label} at {size:?}"
                );
            }
            assert!(connected.find("workstation-status-header").is_some());
            assert!(connected.find("workstation-services").is_some());
            assert!(connected.find("workstation-jobs-counts").is_some());
            assert!(connected.find("host-resources-card").is_none());
            assert!(connected.find("toolbar-telemetry").is_some());

            connected.update(|state, _| state.navigate(Page::Brain));
            connected.frame(1.0 / 60.0);
            let elements = connected.elements();
            let home = elements
                .iter()
                .find(|element| element.key.as_deref() == Some("nav-workstation"))
                .expect("brand Home button");
            assert_eq!(home.role, "button");
            assert_eq!(home.label.as_deref(), Some("Home"));
            let logo_path = format!("{}/startup-rail-mark-slot", home.path);
            assert!(elements.iter().any(|element| element.path == logo_path));
            assert!(connected.find("sidebar-workstation-heading").is_none());
            connected.update(|_, tx| tx.focus("nav-workstation", true));
            connected.frame(2.0 / 60.0);
            connected.key(KeyEvent {
                key: Key::Enter,
                mods: Mods::default(),
            });
            connected.frame(3.0 / 60.0);
            assert_eq!(connected.state.page, Page::Workstation);
            let mut clock = 3.0 / 60.0;
            assert!(connected.find("gateway-backend-card").is_none());
            click_within(
                &mut connected,
                "workstation-diagnostics/trigger",
                size,
                &mut clock,
            );
            assert!(connected.find("gateway-backend-card").is_some());
            assert!(connected
                .accessibility_json()
                .to_string()
                .contains("Deployment synthetic-deployment"));
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
    fn compact_workstation_keeps_first_action_visible_and_focused_on_refresh() {
        use crate::accessibility::FixtureAccessibility;
        use fleetworks_contracts::readiness::ReadinessCheck;
        use fleetworks_contracts::readiness::WorkstationReadiness;

        for reduced_motion in [false, true] {
            let mut initial = crate::tests::registered_fixture();
            initial.accessibility.fixture_override = Some(FixtureAccessibility {
                reduced_motion,
                ..FixtureAccessibility::default()
            });
            let mut catalog = ReadinessCheck::new(
                ReadinessCheckId::ActorCatalog,
                ReadinessState::NotReady,
                "not_seeded".into(),
                "Import an actor catalog".into(),
                "2026-11-14T22:00:00Z".into(),
            );
            catalog.fresh_for_ms = 30_000;
            initial.readiness = Load::Ready(WorkstationReadiness::new(
                "synthetic-deployment".into(),
                "2026-11-14T22:00:00Z".into(),
                vec![catalog],
            ));
            initial.readiness_received_at = Some(0.0);
            let mut runtime = Runtime::new(
                initial,
                app::view,
                (640.0, 480.0),
                Env {
                    system_dark: true,
                    scale: 1.0,
                },
            );
            runtime.frame(0.0);
            runtime.frame(1.0 / 60.0);
            let key = "workstation-open-seed";
            let elements = runtime.elements();
            let action = elements
                .iter()
                .find(|el| el.key.as_deref() == Some(key))
                .expect("compact primary actor catalog action");
            assert!(action.rect[2] > 0.0 && action.rect[3] > 0.0);
            assert_eq!(
                action.visible, action.rect,
                "the primary action must fit before scrolling"
            );
            assert!(!runtime
                .accessibility_json()
                .to_string()
                .contains("Build actor index"));

            runtime.update(|_, tx| tx.focus(key, true));
            runtime.frame(2.0 / 60.0);
            runtime.update(|state, _| {
                let Load::Ready(readiness) = &state.readiness else {
                    panic!("expected fixture readiness");
                };
                state.complete_readiness(Ok(readiness.clone()));
            });
            runtime.frame(3.0 / 60.0);
            let after = runtime
                .elements()
                .into_iter()
                .find(|el| el.key.as_deref() == Some(key))
                .expect("anchor retained after refresh");
            assert!(after.focused);
            assert_eq!(after.scrolled, action.scrolled);
            assert_eq!(after.visible, after.rect);
            runtime.key(KeyEvent {
                key: Key::Enter,
                mods: Mods::default(),
            });
            runtime.frame(4.0 / 60.0);
            assert_eq!(runtime.state.page, Page::Seed);
            assert_eq!(runtime.state.seed_stage, SeedStage::Seed);
        }
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

        let details_position = rt.find("workstation-index-details-toggle").unwrap();
        let scenarios = [
            (
                ReadinessState::NotReady,
                ReadinessState::NotReady,
                "not_seeded",
                "Needs data",
            ),
            (
                ReadinessState::NotReady,
                ReadinessState::Running,
                "index_catching_up",
                "Needs data",
            ),
            (
                ReadinessState::Ready,
                ReadinessState::NotReady,
                "not_built",
                "Not built",
            ),
            (
                ReadinessState::Ready,
                ReadinessState::Running,
                "index_catching_up",
                "Background indexing",
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
            let status = rt
                .elements()
                .into_iter()
                .find(|element| element.path.ends_with("actor-search-status-tag"))
                .expect("actor search status tag");
            assert_eq!(status.label.as_deref(), Some(label));
            let position = rt.find("workstation-index-details-toggle").unwrap();
            assert!((position.0 - details_position.0).abs() < 0.5);
            assert!((position.1 - details_position.1).abs() < 0.5);
            if cat_state == ReadinessState::NotReady {
                assert!(rt.find("workstation-open-seed").is_some());
                assert!(rt.find("workstation-build-actor-index").is_none());
            }
            if search_state == ReadinessState::Running {
                assert!(rt.find("workstation-view-index-job").is_none());
                assert!(rt.find("workstation-build-actor-index").is_none());
                assert!(
                    app::sidebar_row_badge(&rt.state, Page::Jobs).is_none(),
                    "readiness alone must not contribute to the jobs badge",
                );
                if cat_state == ReadinessState::Ready {
                    click_within(
                        &mut rt,
                        "workstation-index-details-toggle",
                        (1280.0, 800.0),
                        &mut clock,
                    );
                    assert!(rt
                        .accessibility_json()
                        .to_string()
                        .contains(crate::readiness_cards::ACTOR_INDEX_CATCH_UP));
                    click_within(
                        &mut rt,
                        "workstation-index-details-toggle",
                        (1280.0, 800.0),
                        &mut clock,
                    );
                }
            }
        }

        // Building state with an unknown total exposes a phase and the observed count.
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
        assert!(!building_tree.contains("scanning · 128 scanned"));
        assert!(rt.find("workstation-view-index-job").is_some());
        assert!(rt.find("workstation-build-actor-index").is_none());
        click_within(
            &mut rt,
            "workstation-index-details-toggle",
            (1280.0, 800.0),
            &mut clock,
        );
        let details = rt.accessibility_json().to_string();
        assert!(details.contains("scanning · 128 scanned"));
        assert!(!details.contains('%'));

        rt.update(|state, _| {
            if let Load::Ready(readiness) = &mut state.readiness {
                let check = readiness
                    .checks
                    .iter_mut()
                    .find(|check| check.id == ReadinessCheckId::ActorSearch)
                    .expect("actor search check");
                check.state = ReadinessState::Ready;
                check.reason_code = "index_reader_visible".into();
            }
        });
        clock += 1.0 / 60.0;
        rt.frame(clock);
        assert!(rt.find("workstation-view-index-job").is_some());
        assert!(rt.find("workstation-search-actors").is_some());

        rt.update(|state, _| {
            state.backend.jobs = Load::Idle;
            if let Load::Ready(readiness) = &mut state.readiness {
                let check = readiness
                    .checks
                    .iter_mut()
                    .find(|check| check.id == ReadinessCheckId::ActorSearch)
                    .expect("actor search check");
                check.state = ReadinessState::NotReady;
                check.reason_code = "not_built".into();
            }
        });
        clock += 1.0 / 60.0;
        rt.frame(clock);
        // The single actionable build control still opens its confirmation.
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
        assert!(rt.find("workstation-index-details-toggle").is_some());
        click_within(
            &mut rt,
            "workstation-index-details-toggle",
            (1280.0, 800.0),
            &mut clock,
        );
        assert!(rt.find("workstation-index-details-toggle").is_some());
        assert!(!rt.state.maintenance.reindex_details_open);
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
            assert!(rt.find("inbox-toggle").is_some());
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
            let elements = rt.elements();
            for (key, label) in [
                ("backend-mode-local", "Embedded"),
                ("backend-mode-remote", "Distributed"),
            ] {
                assert!(
                    elements.iter().any(|element| {
                        element.key.as_deref() == Some(key)
                            && element.label.as_deref() == Some(label)
                    }),
                    "missing {label} backend option at {size:?}"
                );
            }
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
    fn embedded_startup_companion_diagnostic_wraps_inside_its_strip() {
        use crate::local_backend::BinaryDiscoveryStatus;
        use fleetworks_workstation::native::BackendKind;

        let reason = format!(
            "Companion executable was not found at /private/{}/argos-api-host. Select a verified companion before starting the embedded backend.",
            "synthetic-path".repeat(12),
        );
        for size in [(640.0, 480.0), (1280.0, 800.0)] {
            let mut state = State::fixture();
            state.fixture_show_startup_setup(BackendKind::Local);
            state.local_backend.binary_status = BinaryDiscoveryStatus::Missing(reason.clone());
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
            let elements = runtime.elements();
            let strip = elements
                .iter()
                .find(|el| el.key.as_deref() == Some("local-binary-status"))
                .expect("companion diagnostic");
            let detail = elements
                .iter()
                .find(|el| el.key.as_deref() == Some("local-binary-status-detail"))
                .expect("wrapped diagnostic detail");
            assert_eq!(strip.role, "alert");
            assert_eq!(detail.text.as_deref(), Some(reason.as_str()));
            assert!(detail.rect[3] > 20.0, "diagnostic must wrap at {size:?}");
            assert!(
                detail.rect[0] >= strip.rect[0]
                    && detail.rect[0] + detail.rect[2] <= strip.rect[0] + strip.rect[2],
                "diagnostic must fit its strip at {size:?}: {:?} inside {:?}",
                detail.rect,
                strip.rect,
            );
            let start = elements
                .iter()
                .find(|el| el.key.as_deref() == Some("connect-local"))
                .expect("Start Embedded Backend");
            assert_eq!(start.visible, start.rect, "Start must fit at {size:?}");
            assert!(
                start.rect[1] + start.rect[3] <= strip.rect[1],
                "the diagnostic must follow Start at {size:?}"
            );
        }
    }

    #[test]
    fn connected_embedded_capability_detail_fits_its_status_strip() {
        use fleetworks_workstation::native::BackendKind;

        for size in [(640.0, 480.0), (1280.0, 800.0)] {
            let mut state = crate::tests::registered_fixture();
            state.backend_kind = BackendKind::Local;
            state.connection.origin = rune::TextEdit::new("local://embedded");
            state.setup_gateway = true;
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
            let elements = runtime.elements();
            let strip = elements
                .iter()
                .find(|el| el.key.as_deref() == Some("local-capability-warnings"))
                .expect("optional capability warnings");
            let detail = elements
                .iter()
                .find(|el| {
                    el.key.as_deref() == Some("async-status-detail")
                        && el.text.as_deref().is_some_and(|value| {
                            value
                                .starts_with("The embedded backend is not advertising NLP support.")
                        })
                })
                .expect("capability warning detail");
            assert!(
                detail.rect[0] >= strip.rect[0]
                    && detail.rect[0] + detail.rect[2] <= strip.rect[0] + strip.rect[2]
                    && detail.rect[0] + detail.rect[2] <= size.0,
                "detail must fit the strip at {size:?}: {:?} inside {:?}",
                detail.rect,
                strip.rect,
            );
            if size.0 == 640.0 {
                assert!(detail.rect[3] > 20.0, "compact detail must wrap");
            }
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

        // Setup Gateway view with the embedded backend connected.
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

        // Workstation Status view with the embedded backend connected.
        setup_rt.update(|state, _| state.setup_gateway = false);
        setup_rt.frame(1.0 / 60.0);
        assert!(setup_rt.find("workstation-local-state-dir").is_none());
        let mut clock = 1.0 / 60.0;
        click_within(
            &mut setup_rt,
            "workstation-diagnostics/trigger",
            (1280.0, 800.0),
            &mut clock,
        );
        assert!(setup_rt.find("gateway-backend-card").is_some());
        assert!(setup_rt.find("workstation-local-state-dir").is_some());
        assert!(setup_rt.find("workstation-stop-local-backend").is_some());
        assert!(setup_rt.find("local-pressure-paused-alert").is_some());
        assert!(setup_rt.find("local-capability-warnings").is_some());
    }
}
