//! Selected native workstation destinations. Topology presentation belongs to Intel Web.

use fleetworks_contracts::search::IndexState;
use rune::Cx;
use rune::El;
use rune::ScrollSpec;
use rune_kit::Button;
use rune_kit::ButtonVariant;
use rune_kit::Kit;
use rune_kit::TextInput;

use crate::app::Load;
use crate::app::Page;
use crate::app::State;
use crate::brain::Association;
use crate::connection::ConnectionStatus;
use crate::pairing::PairingFailure;
use crate::pairing::PairingStage;

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
        return setup_gateway(state, kit);
    }
    match state.page {
        Page::Workstation | Page::Workspaces | Page::Profile => workstation(state, kit),
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
            crate::backend_views::page(state, kit, layers, log_width)
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

fn setup_gateway(state: &State, kit: &Kit) -> El {
    let busy = state.pairing.busy() || state.connection.status == ConnectionStatus::Connecting;
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
    let body = rune::col()
        .child(heading("Setup Gateway", kit))
        .child(quiet("Connect to your independently running gateway. Accepted jobs continue when this app closes.", kit))
        .child(crate::material::matte_card(rune::col().padding(16.0).gap(10.0)
            .child(heading("Gateway URL", kit))
            .child(origin)))
        .child(pairing_status(state, kit));
    scroll(body, "gateway-setup-body")
}

fn pairing_status(state: &State, kit: &Kit) -> El {
    let mut body = rune::col()
        .padding(16.0)
        .gap(10.0)
        .key("pairing-status-card")
        .child(heading("Gateway access", kit));
    if state.connected() {
        body = body
            .child(
                line("Connected to your gateway", kit)
                    .key("connection-status")
                    .role("status"),
            )
            .child(registration_status(state, kit))
            .child(crate::readiness_cards::checks(state, kit));
        if state.registration.context.is_some() {
            body = body.child(
                Button::new("Done")
                    .on_press(|state: &mut State| state.setup_gateway = false)
                    .build(kit)
                    .key("gateway-setup-done"),
            );
        }
    } else {
        body = match &state.pairing.stage {
            PairingStage::Idle => body
                .child(quiet("Pair this Mac once. Your gateway operator approves access, then Fleetworks saves it in macOS Keychain.", kit))
                .child(Button::new("Connect")
                    .disabled(state.connection.status == ConnectionStatus::Connecting || state.connection.origin.text.trim().is_empty())
                    .on_press(|state: &mut State| state.connect())
                    .build(kit).key("connect")),
            PairingStage::Working(message) => body
                .child(line(message.clone(), kit).key("pairing-progress").role("status")),
            PairingStage::Waiting(challenge) => body
                .child(line("Waiting for gateway approval", kit).key("pairing-waiting").role("status"))
                .child(quiet("Share this approval code with your gateway operator. Keep this window open while they approve this workstation.", kit))
                .child(quiet("Approval code", kit))
                .child(line(challenge.approval_code.clone(), kit).key("pairing-approval-code"))
                .child(quiet("Pairing request", kit))
                .child(quiet(challenge.pairing_id.clone(), kit).key("pairing-request-id")),
            PairingStage::Failed(failure) => {
                let restart = matches!(failure, PairingFailure::Denied | PairingFailure::Expired | PairingFailure::KeychainMissing | PairingFailure::Authentication);
                body
                    .child(line(failure.message(), kit).key("pairing-failure").role("alert"))
                    .child(Button::new(if restart { "Pair again" } else { "Retry" })
                        .disabled(state.connection.origin.text.trim().is_empty())
                        .on_press(move |state: &mut State| {
                            if restart { state.restart_pairing(); } else { state.connect(); }
                        })
                        .build(kit).key(if restart { "pairing-restart" } else { "connect" }))
            }
            PairingStage::Connected => body
                .child(line("Verifying gateway access…", kit).key("pairing-progress").role("status")),
        };
    }
    if !matches!(state.pairing.stage, PairingStage::Idle) || state.connected() {
        body = body.child(
            Button::new("Change gateway URL")
                .variant(ButtonVariant::Ghost)
                .on_press(|state: &mut State| state.disconnect())
                .build(kit)
                .key("pairing-change-origin"),
        );
    }
    crate::material::matte_card(body)
}

fn registration_status(state: &State, kit: &Kit) -> El {
    if let Some(context) = &state.registration.context {
        return line(format!("Connect ID: {}", context.connect_id), kit).key("connect-id");
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

fn workstation(state: &State, kit: &Kit) -> El {
    let native = crate::material::matte_card(
        rune::col()
            .padding(16.0)
            .gap(8.0)
            .key("native-client-card")
            .child(heading("Native Client", kit))
            .child(line(
                format!(
                    "Fleetworks {} · macOS · {}",
                    env!("CARGO_PKG_VERSION"),
                    std::env::consts::ARCH
                ),
                kit,
            ))
            .child(registration_status(state, kit)),
    );
    let gateway = crate::material::matte_card(
        rune::col()
            .padding(16.0)
            .gap(8.0)
            .key("gateway-backend-card")
            .child(heading("Gateway Backend", kit))
            .child(
                line(connection_status(state), kit)
                    .key("connection-status")
                    .role("status"),
            )
            .child(quiet(state.connection.origin.text.clone(), kit))
            .child(quiet(
                format!("Deployment {}", state.connection.deployment_label()),
                kit,
            ))
            .child(crate::readiness_cards::checks(state, kit)),
    );
    let mut resources = rune::col()
        .padding(16.0)
        .gap(8.0)
        .key("host-resources-card")
        .child(heading("Host Resources", kit));
    match &state.host_metrics {
        Load::Idle => {
            resources =
                resources.child(quiet("Host resource telemetry has not been observed.", kit))
        }
        Load::Pending | Load::Loading => {
            resources = resources.child(quiet("Reading API host resources…", kit).role("status"))
        }
        Load::Failed(error) => resources = resources.child(line(error.clone(), kit).role("alert")),
        Load::Ready(metrics) => {
            for (label, value) in [
                ("CPU", metrics.cpu_percent),
                ("Memory", metrics.memory_percent),
                ("Disk", metrics.disk_percent),
            ] {
                resources = resources.child(line(
                    value
                        .filter(|value| value.is_finite() && (0.0..=100.0).contains(value))
                        .map_or_else(
                            || format!("{label}: unavailable"),
                            |value| format!("{label}: {value:.1}%"),
                        ),
                    kit,
                ));
            }
            if let Some(platform) = &metrics.platform {
                resources = resources.child(quiet(platform.clone(), kit));
            }
            if let Some(chipset) = &metrics.chipset {
                resources = resources.child(quiet(chipset.clone(), kit));
            }
            resources = resources.child(quiet(
                "Measurements describe the API host. Unreported measurements remain unavailable.",
                kit,
            ));
        }
    }
    let resources = crate::material::matte_card(resources);
    scroll(
        rune::col()
            .child(heading("Workstation Status", kit))
            .child(native)
            .child(gateway)
            .child(resources),
        "workstation-body",
    )
}

fn brain(state: &State, cx: &mut Cx<State>, kit: &Kit) -> El {
    let narrow = cx
        .layout_size("page-content")
        .is_some_and(|size| size[0] < 720.0);
    if narrow && state.brain.showing_detail {
        return rune::col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(
                Button::new("Back to results")
                    .variant(ButtonVariant::Ghost)
                    .on_press(|state: &mut State| state.brain.showing_detail = false)
                    .build(kit)
                    .key("brain-back"),
            )
            .child(dossier(state, kit));
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
        .child(heading("Search actors", kit))
        .child(search);
    match &state.brain.results {
        Load::Idle => {
            list = list.child(
                quiet(
                    "Enter a name or alias to search the shared actor catalog.",
                    kit,
                )
                .key("search-prompt"),
            )
        }
        Load::Pending => {
            list = list.child(quiet("Waiting for the latest query…", kit).role("status"))
        }
        Load::Loading => {
            list = list.child(quiet("Searching the configured gateway…", kit).role("status"))
        }
        Load::Failed(error) => {
            list = list.child(line(error.clone(), kit).key("search-error").role("alert"))
        }
        Load::Ready(response) => {
            let current = response.index.state == IndexState::Ready;
            if !current {
                list = list.child(
                    quiet(
                        format!(
                            "Actor search: {:?}. Results may be partial or stale.",
                            response.index.state
                        ),
                        kit,
                    )
                    .key("index-status")
                    .role("status"),
                );
            }
            if response.rows.is_empty() && current {
                list = list.child(quiet("No matching actors.", kit).key("search-empty"));
            }
            for (index, hit) in response.rows.iter().enumerate() {
                let variant = if state.brain.highlighted == Some(index) {
                    ButtonVariant::Outline
                } else {
                    ButtonVariant::Ghost
                };
                list = list.child(
                    rune::col()
                        .gap(4.0)
                        .child(
                            Button::new(hit.label.clone())
                                .variant(variant)
                                .on_press(move |state: &mut State| state.select_actor(index))
                                .build(kit)
                                .key(format!("actor-hit-{}", hit.actor_id)),
                        )
                        .child(quiet(
                            hit.summary.clone().unwrap_or_else(|| hit.actor_id.clone()),
                            kit,
                        )),
                );
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
        rune::row()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(results.w(340.0).min_w(220.0).grow(0.0))
            .child(dossier(state, kit).grow(1.0).min_w(0.0))
    }
}

fn dossier(state: &State, kit: &Kit) -> El {
    let body = match &state.brain.dossier {
        Load::Idle => rune::col()
            .child(heading("Actor dossier", kit))
            .child(quiet("Select an actor to view its dossier.", kit)),
        Load::Pending | Load::Loading => {
            rune::col().child(quiet("Reading the selected actor…", kit).role("status"))
        }
        Load::Failed(error) => {
            rune::col().child(line(error.clone(), kit).key("dossier-error").role("alert"))
        }
        Load::Ready(actor) => {
            let mut body = rune::col()
                .child(heading(actor.display_name.clone(), kit).key("dossier-name"))
                .child(quiet(
                    format!(
                        "{} · {} · {}",
                        actor.id, actor.subtype, actor.identity_state
                    ),
                    kit,
                ))
                .child(line(
                    actor
                        .description
                        .clone()
                        .unwrap_or_else(|| "Description unavailable".into()),
                    kit,
                ));
            if !actor.aliases.is_empty() {
                body = body.child(heading("Aliases", kit));
                for alias in actor.aliases.iter().take(50) {
                    body = body.child(line(alias.clone(), kit));
                }
            }
            for field in actor.metadata.iter().take(50) {
                body = body.child(quiet(
                    format!(
                        "{}: {}",
                        field.key,
                        field.value.as_deref().unwrap_or("Unavailable")
                    ),
                    kit,
                ));
            }
            for (name, rows) in [
                ("Claims", &actor.claims),
                ("Briefs", &actor.briefs),
                ("Inferences", &actor.inferences),
            ] {
                body = body.child(associations(name, rows, kit));
            }
            for gap in &actor.gaps {
                body = body.child(quiet(gap.clone(), kit));
            }
            if actor.truncated {
                body = body.child(quiet(
                    "The server bounded this dossier; some associated records are not loaded.",
                    kit,
                ));
            }
            body
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
            for title in ["Native Client", "Gateway Backend", "Host Resources"] {
                assert!(tree.contains(title), "missing {title} at {size:?}");
            }
            for key in [
                "gateway-origin",
                "gateway-token-file",
                "disconnect",
                "registration-ensure",
                "workspace-create",
                "refresh-checks",
                "dataset-upload",
            ] {
                assert!(runtime.find(key).is_none(), "retired status control {key}");
            }
            assert!(
                runtime.state.intents.is_empty(),
                "rendering status cannot submit work"
            );
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
