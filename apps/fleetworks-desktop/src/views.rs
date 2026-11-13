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
    let connecting = state.connection.status == ConnectionStatus::Connecting;
    let origin = TextInput::new(&state.connection.origin)
        .label("Gateway URL")
        .placeholder("https://gateway.example.test")
        .width(None)
        .disabled(connecting)
        .on_edit(|state: &mut State| &mut state.connection.origin)
        .on_change(|state: &mut State, _| state.profile_editor_changed(true))
        .on_submit(|state: &mut State, _| state.connect())
        .build(kit)
        .key("gateway-origin");
    let mut body = rune::col()
        .child(heading("Setup Gateway", kit))
        .child(quiet("Connect to your independently running gateway. Accepted jobs continue when this app closes.", kit))
        .child(crate::material::matte_card(rune::col().padding(16.0).gap(10.0)
            .child(heading("1. Gateway URL", kit))
            .child(origin)
            .child(Button::new("Check gateway")
                .variant(ButtonVariant::Submit)
                .disabled(connecting || state.connection.origin.text.trim().is_empty())
                .on_press(|state: &mut State| state.connect())
                .build(kit).key("connect"))))
        .child(crate::material::matte_card(rune::col().padding(16.0).gap(8.0)
            .child(heading("2. Connectivity Check", kit))
            .child(line(connection_status(state), kit).key("connection-status").role("status"))
            .child(quiet("The gateway URL uses its already provisioned protected credential binding. Missing or replaced credentials require provisioning before this check can succeed.", kit))
            .child(registration_status(state, kit))
            .child(crate::readiness_cards::checks(state, kit))));
    if state.connected() && state.registration.context.is_some() {
        body = body.child(
            Button::new("Done")
                .on_press(|state: &mut State| state.setup_gateway = false)
                .build(kit)
                .key("gateway-setup-done"),
        );
    }
    scroll(body, "gateway-setup-body")
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
    use rune::Env;
    use rune::Runtime;

    use crate::app;
    use crate::app::Page;
    use crate::app::State;

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
    fn setup_has_url_and_connectivity_progress_without_credential_picker() {
        let runtime = runtime(Page::Workstation, true, (1180.0, 780.0));
        let tree = runtime.accessibility_json().to_string();
        assert!(runtime.find("gateway-origin").is_some());
        assert!(runtime.find("connect").is_some());
        assert!(tree.contains("Connectivity Check"));
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
