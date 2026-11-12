//! Simple native controls and lists. Topology presentation belongs to Intel Web.

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
use crate::workspaces::WorkspaceCommand;
use fleetworks_workspace::native::RemovalKind;
use fleetworks_workspace::native::WorkspaceState;

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

pub(crate) fn page(state: &State, cx: &mut Cx<State>, kit: &Kit) -> El {
    match state.page {
        Page::Workstation => workstation(state, kit),
        Page::Brain => brain(state, cx, kit),
        Page::Workspaces => workspaces(state, kit),
        Page::Jobs | Page::Atlas | Page::Tools | Page::Models | Page::Profile => {
            crate::backend_views::page(state, kit)
        }
    }
}

fn workspaces(state: &State, kit: &Kit) -> El {
    let busy = state.workspaces.busy;
    let mut body = rune::col()
        .child(heading("Managed workspace folders", kit))
        .child(quiet("Select a folder for future request origin. Backend keys, model roles, corpus and search remain shared across folders.", kit))
        .child(Button::new("Refresh folders").disabled(busy)
            .on_press(|state: &mut State| {
                if matches!(state.workspaces.snapshot, Load::Idle | Load::Failed(_)) { state.initialize_workspaces(None); }
                else { state.workspace_command(WorkspaceCommand::Refresh); }
            }).build(kit).key("workspace-refresh"));
    match &state.workspaces.snapshot {
        Load::Idle => {
            body = body.child(quiet(
                "The protected home container has not been opened.",
                kit,
            ))
        }
        Load::Pending | Load::Loading => {
            body = body.child(quiet("Opening protected workspace identities…", kit).role("status"))
        }
        Load::Failed(error) => body = body.child(line(error.clone(), kit).role("alert")),
        Load::Ready(snapshot) => {
            body = body.child(quiet(snapshot.root.display().to_string(), kit));
            for attempt in &snapshot.registration_attempts {
                let id = attempt.request.request_id.clone();
                body = body.child(rune::col().gap(6.0).key(format!("registration-attempt-{id}"))
                    .child(heading(format!("Registration outcome pending: {}", attempt.request.workspace_name), kit))
                    .child(quiet(format!("Deployment {} · IP {}. The folder cannot be removed until this exact request is reconciled.", attempt.deployment_id, attempt.request.workstation_ip), kit))
                    .child(Button::new("Reconcile registration").disabled(busy || !state.permission(fleetworks_contracts::context::Permission::RegisterWorkstation)
                        || state.connection.deployment_label() != attempt.deployment_id)
                        .on_press(move |state: &mut State| state.reconcile_registration_attempt(&id)).build(kit).key("registration-reconcile")));
            }
            for attempt in &snapshot.rebind_attempts {
                let id = attempt.request.request_id.clone();
                body = body.child(rune::col().gap(6.0).key(format!("rebind-attempt-{id}"))
                    .child(heading(format!("IP change outcome pending: {}", attempt.request.workspace_name), kit))
                    .child(quiet(format!("Backend {} · {} → {}. Recheck this exact request before changing registration or removing folders.", attempt.prior.deployment_id, attempt.prior.workstation_ip, attempt.request.workstation_ip), kit))
                    .child(Button::new("Recheck IP change").disabled(busy || !state.permission(fleetworks_contracts::context::Permission::RebindWorkstation)
                        || state.connection.deployment_label() != attempt.prior.deployment_id)
                        .on_press(move |state: &mut State| state.reconcile_rebind(&id)).build(kit).key("workspace-rebind-reconcile")));
            }
            for row in &snapshot.rows {
                let selected = state
                    .workspaces
                    .selected
                    .as_ref()
                    .is_some_and(|selected| selected.marker == row.marker);
                let name = row.name.clone();
                let reveal_name = name.clone();
                let remove_name = name.clone();
                let delete_name = name.clone();
                let copy_name = name.clone();
                let path = row.path.display().to_string();
                let selectable = matches!(
                    row.state,
                    WorkspaceState::Local | WorkspaceState::Registered
                );
                body = body.child(
                    rune::col()
                        .gap(6.0)
                        .py(8.0)
                        .key(format!("workspace-{}", row.marker))
                        .child(heading(
                            format!(
                                "{}{}{}",
                                row.name,
                                if row.protected_default {
                                    " · protected default"
                                } else {
                                    ""
                                },
                                if selected { " · selected" } else { "" }
                            ),
                            kit,
                        ))
                        .child(quiet(format!("{:?} · {}", row.state, path), kit))
                        .child(
                            rune::row()
                                .gap(8.0)
                                .child(
                                    Button::new("Select")
                                        .variant(if selected {
                                            ButtonVariant::Submit
                                        } else {
                                            ButtonVariant::Outline
                                        })
                                        .disabled(busy || !selectable || selected)
                                        .on_press(move |state: &mut State| {
                                            state.workspace_command(WorkspaceCommand::Select(
                                                name.clone(),
                                            ))
                                        })
                                        .build(kit)
                                        .key("workspace-select"),
                                )
                                .child(
                                    Button::new("Reveal in Finder")
                                        .variant(ButtonVariant::Ghost)
                                        .disabled(busy || !selectable)
                                        .on_press(move |state: &mut State| {
                                            state.workspace_command(WorkspaceCommand::Reveal(
                                                reveal_name.clone(),
                                            ))
                                        })
                                        .build(kit)
                                        .key("workspace-reveal"),
                                )
                                .child(
                                    Button::new("Copy path")
                                        .variant(ButtonVariant::Ghost)
                                        .on_press_tx(move |state: &mut State, tx| {
                                            state.copy_workspace_path(&copy_name, tx)
                                        })
                                        .build(kit)
                                        .key("workspace-copy"),
                                ),
                        )
                        .child(
                            rune::row()
                                .gap(8.0)
                                .child(
                                    Button::new("Remove from app")
                                        .variant(ButtonVariant::Ghost)
                                        .disabled(busy || row.protected_default || !selectable)
                                        .on_press(move |state: &mut State| {
                                            state.preview_removal(
                                                &remove_name,
                                                RemovalKind::RemoveFromApp,
                                            )
                                        })
                                        .build(kit)
                                        .key("workspace-remove"),
                                )
                                .child(
                                    Button::new("Delete…")
                                        .variant(ButtonVariant::Outline)
                                        .disabled(busy || row.protected_default || !selectable)
                                        .on_press(move |state: &mut State| {
                                            state.preview_removal(&delete_name, RemovalKind::Delete)
                                        })
                                        .build(kit)
                                        .key("workspace-delete"),
                                ),
                        ),
                );
                for binding in &row.bindings {
                    body = body.child(quiet(
                        format!(
                            "Backend {} · {}{}",
                            binding.deployment_id,
                            if binding.retired {
                                "retired"
                            } else if binding.stale {
                                "refresh registration after IP change"
                            } else {
                                "registered"
                            },
                            if binding.receipt.is_none() {
                                " · refresh complete receipt"
                            } else {
                                ""
                            }
                        ),
                        kit,
                    ));
                }
            }
            if !snapshot.operations.is_empty() {
                body = body.child(heading("Pending folder operations", kit));
                for operation in &snapshot.operations {
                    let id = operation.operation_id.clone();
                    body = body.child(rune::col().gap(5.0).key(format!("workspace-operation-{id}"))
                        .child(line(format!("{} · {:?} · {:?}", operation.workspace_name, operation.kind, operation.phase), kit))
                        .child(quiet(format!("{} known backend bindings · {} retirement receipts. Files stay intact while any receipt is pending.", operation.bindings.len(), operation.receipts.len()), kit))
                        .child(Button::new("Recheck pending operation").disabled(busy)
                            .on_press(move |state: &mut State| state.workspace_command(WorkspaceCommand::Resume(id.clone())))
                            .build(kit).key("workspace-resume")));
                    for binding in &operation.bindings {
                        body = body.child(quiet(
                            format!(
                                "{} · {}",
                                binding.deployment_id,
                                if operation.receipts.contains_key(&binding.deployment_id) {
                                    "retirement confirmed"
                                } else {
                                    "retirement pending"
                                }
                            ),
                            kit,
                        ));
                    }
                }
            }
        }
    }
    if let Some(confirmation) = &state.workspaces.confirmation {
        body = body.child(rune::col().gap(8.0).key("workspace-removal-confirmation")
            .child(heading(format!("Confirm {:?}: {}", confirmation.kind, confirmation.name), kit))
            .child(quiet(confirmation.path.display().to_string(), kit))
            .child(quiet(format!("{} known backend bindings. Existing work must finish before removal. Unreachable backends keep this pending. Remove from app keeps all files; Delete moves this entire folder and its contents to macOS Trash after every backend confirms retirement.", confirmation.bindings.iter().filter(|binding| !binding.retired).count()), kit))
            .child(rune::row().gap(8.0)
                .child(Button::new("Confirm removal").disabled(busy).on_press(|state: &mut State| state.confirm_removal()).build(kit).key("workspace-confirm-remove"))
                .child(Button::new("Keep workspace").variant(ButtonVariant::Ghost).on_press(|state: &mut State| state.workspaces.confirmation = None).build(kit))));
    }
    let available = matches!(state.workspaces.snapshot, Load::Ready(_)) && !busy;
    body = body
        .child(heading("Create a named child", kit))
        .child(TextInput::new(&state.workspaces.create_name).label("New workspace name").placeholder("research")
            .width(None).on_edit(|state: &mut State| &mut state.workspaces.create_name).build(kit).key("workspace-create-name"))
        .child(Button::new("Create workspace").disabled(!available || state.workspaces.create_name.text.is_empty())
            .on_press(|state: &mut State| state.workspace_command(WorkspaceCommand::Create(state.workspaces.create_name.text.clone())))
            .build(kit).key("workspace-create"))
        .child(heading("Attach an existing owned child", kit))
        .child(quiet("Enter one immediate child name and its existing identity marker. External paths, symbolic links and unowned folders are rejected.", kit))
        .child(TextInput::new(&state.workspaces.attach_name).label("Owned child name").width(None)
            .on_edit(|state: &mut State| &mut state.workspaces.attach_name).build(kit).key("workspace-attach-name"))
        .child(TextInput::new(&state.workspaces.attach_marker).label("Existing folder identity marker").width(None)
            .on_edit(|state: &mut State| &mut state.workspaces.attach_marker).build(kit).key("workspace-attach-marker"))
        .child(Button::new("Attach owned child").disabled(!available || state.workspaces.attach_name.text.is_empty() || state.workspaces.attach_marker.text.is_empty())
            .on_press(|state: &mut State| state.workspace_command(WorkspaceCommand::Attach {
                name: state.workspaces.attach_name.text.clone(), marker: state.workspaces.attach_marker.text.clone(),
            })).build(kit).key("workspace-attach"));
    if let Some(message) = &state.workspaces.message {
        body = body.child(line(message.clone(), kit).role("status"));
    }
    scroll(body, "workspaces-body")
}

fn registration(state: &State, kit: &Kit) -> El {
    let selected = state
        .workspaces
        .selected
        .as_ref()
        .map_or("No owned workspace selected".to_owned(), |row| {
            format!("Selected workspace: {}", row.name)
        });
    let mut body = rune::col().gap(8.0).child(line(selected, kit)).child(
        Button::new("Choose LAN/VPN interface")
            .disabled(matches!(state.registration.interfaces, Load::Loading))
            .on_press(|state: &mut State| state.refresh_interfaces())
            .build(kit)
            .key("interfaces-refresh"),
    );
    match &state.registration.interfaces {
        Load::Idle => body = body.child(quiet("Native IP is selected from this Mac. For a loopback gateway, choose the LAN or VPN address that browser users should identify.", kit)),
        Load::Pending | Load::Loading => body = body.child(quiet("Reading local interfaces and the gateway route…", kit)),
        Load::Failed(error) => body = body.child(line(error.clone(), kit).role("alert")),
        Load::Ready(snapshot) => {
            if snapshot.interfaces.is_empty() { body = body.child(quiet("No usable LAN/VPN address is available. Connect an interface and refresh.", kit)); }
            for (index, interface) in snapshot.interfaces.iter().enumerate() {
                let selected = state.registration.selected_interface.as_ref() == Some(interface);
                let suggestion = snapshot.route_suggestion == Some(interface.ip);
                body = body.child(Button::new(format!("{} · {}{}{}", interface.name, interface.ip,
                    if suggestion { " · gateway route" } else { "" }, if selected { " · selected" } else { "" }))
                    .variant(if selected { ButtonVariant::Submit } else { ButtonVariant::Outline })
                    .on_press(move |state: &mut State| state.choose_interface(index)).build(kit).key(format!("native-interface-{index}")));
            }
        }
    }
    body = body.child(
        Button::new("Register selected workspace")
            .disabled(!state.can_register())
            .on_press(|state: &mut State| state.ensure_registration())
            .build(kit)
            .key("registration-ensure"),
    );
    match &state.registration.receipt {
        Load::Loading | Load::Pending => {
            body = body.child(
                quiet(
                    "Ensuring registration and verifying current authority…",
                    kit,
                )
                .role("status"),
            )
        }
        Load::Failed(error) => body = body.child(line(error.clone(), kit).role("alert")),
        Load::Ready(receipt) => {
            body = body.child(
                line(
                    format!(
                        "Connect ID: {} · revision {}",
                        receipt.connect_id, receipt.revision
                    ),
                    kit,
                )
                .key("connect-id"),
            )
        }
        Load::Idle => {}
    }
    let active = state.registration.context.is_some();
    body = body.child(
        rune::row()
            .gap(8.0)
            .child(
                Button::new("Copy Connect ID")
                    .disabled(!active)
                    .on_press_tx(|state: &mut State, tx| state.copy_connect_id(tx))
                    .build(kit)
                    .key("connect-id-copy"),
            )
            .child(
                Button::new("Open Intel Web")
                    .disabled(!active)
                    .on_press_tx(|state: &mut State, tx| state.open_intel_web(tx))
                    .build(kit)
                    .key("intel-web-open"),
            ),
    );
    if let Some(message) = &state.registration.message {
        body = body.child(quiet(message.clone(), kit).role("status"));
    }
    body.child(crate::rebind_views::controls(state, kit))
}

fn workstation(state: &State, kit: &Kit) -> El {
    let connecting = state.connection.status == ConnectionStatus::Connecting;
    let status = match &state.connection.status {
        ConnectionStatus::Disconnected => {
            "Disconnected — connection settings remain available offline".to_owned()
        }
        ConnectionStatus::Connecting => {
            "Checking gateway authentication and capabilities…".to_owned()
        }
        ConnectionStatus::Connected => "Connected to a compatible gateway".to_owned(),
        ConnectionStatus::Failed(error) => error.clone(),
    };
    let origin = TextInput::new(&state.connection.origin)
        .label("Gateway API origin")
        .placeholder("https://gateway.example.test")
        .width(None)
        .on_edit(|state: &mut State| &mut state.connection.origin)
        .on_change(|state: &mut State, _| state.disconnect())
        .build(kit)
        .key("gateway-origin");
    let credential = TextInput::new(&state.connection.token_file)
        .label("Protected gateway credential file")
        .placeholder("Select a private token file through launch configuration")
        .width(None)
        .on_edit(|state: &mut State| &mut state.connection.token_file)
        .on_change(|state: &mut State, _| state.disconnect())
        .build(kit)
        .key("gateway-token-file");
    let actions = rune::row()
        .gap(8.0)
        .child(
            Button::new("Connect")
                .variant(ButtonVariant::Submit)
                .disabled(connecting)
                .on_press(|state: &mut State| state.connect())
                .build(kit)
                .key("connect"),
        )
        .child(
            Button::new("Disconnect")
                .variant(ButtonVariant::Ghost)
                .disabled(!state.connected() && !connecting)
                .on_press(|state: &mut State| state.disconnect())
                .build(kit)
                .key("disconnect"),
        );
    let mut body = rune::col()
        .child(heading("Connection and local workstation", kit))
        .child(quiet("Local and remote profiles connect to independently running gateways. Accepted jobs continue after this app closes.", kit))
        .child(quiet("Gateway API origin", kit))
        .child(origin)
        .child(quiet("Protected gateway credential file", kit))
        .child(credential)
        .child(actions)
        .child(line(status, kit).key("connection-status").role("status"))
        .child(heading("Registration", kit))
        .child(quiet("Select a verified managed child and native interface, then register through the gateway before interactive submission or browser launch.", kit))
        .child(registration(state, kit))
        .child(heading("Backend readiness & maintenance", kit))
        .child(quiet("These checks describe shared backend resources. Refresh is read-only; seed, training, activation and reindex are separate authorized jobs.", kit))
        .child(Button::new("Refresh Checks").disabled(!state.connected() || matches!(state.readiness, Load::Loading))
            .on_press(|state: &mut State| state.refresh_page()).build(kit).key("refresh-checks"));
    body = body.child(crate::readiness_cards::checks(state, kit));
    body = body.child(crate::backend_views::maintenance(state, kit));
    scroll(body, "workstation-body")
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
        .child(search)
        .child(
            Button::new("Search")
                .disabled(!state.search_available())
                .on_press(|state: &mut State| state.search_now())
                .build(kit)
                .key("search-submit"),
        );
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
