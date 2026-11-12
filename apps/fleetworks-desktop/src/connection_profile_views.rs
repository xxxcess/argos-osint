//! Local connection controls, separate from backend-wide Profile settings.

use rune::El;
use rune_kit::Button;
use rune_kit::ButtonVariant;
use rune_kit::Kit;
use rune_kit::TextInput;

use crate::app::Load;
use crate::app::State;
use crate::connection::ConnectionTestPhase;
use crate::connection_profiles::ProfileAction;
use crate::views::heading;
use crate::views::line;
use crate::views::quiet;

pub(crate) fn profiles(state: &State, kit: &Kit) -> El {
    let profiles = &state.connection_profiles;
    let mut body = rune::col().gap(7.0).key("local-connection-profiles")
        .child(heading("Gateway connections on this Mac",kit))
        .child(quiet("Saved names and protected file references stay local. They do not contain backend provider keys or choose a workspace.",kit));
    if let Load::Ready(catalog) = &profiles.catalog {
        if catalog.profiles.is_empty() {
            body = body.child(quiet(
                "No connections have been saved. Add one or connect from the current editor.",
                kit,
            ));
        }
        for profile in &catalog.profiles {
            let id = profile.id.clone();
            let selected = profiles.selected_id.as_ref() == Some(&id);
            let label = format!(
                "{}{}",
                profile.name,
                if selected { " · selected" } else { "" }
            );
            let visible = label.clone();
            body = body
                .child(
                    Button::new(label)
                        .label_with(move |font, ink| {
                            rune::text(visible.clone())
                                .font(font)
                                .color(ink)
                                .wrap()
                                .min_w(0.0)
                        })
                        .variant(if selected {
                            ButtonVariant::Submit
                        } else {
                            ButtonVariant::Outline
                        })
                        .disabled(profiles.busy || selected && !profiles.editing)
                        .on_press(move |state: &mut State| {
                            state.profile_action(ProfileAction::Select(id.clone()))
                        })
                        .build(kit)
                        .w_full()
                        .key(format!("connection-profile-{}", profile.id)),
                )
                .child(quiet(profile.origin.clone(), kit));
        }
        body = body.child(quiet(
            format!("Saved connection revision {}", catalog.revision),
            kit,
        ));
    } else if let Load::Failed(error) = &profiles.catalog {
        body = body.child(line(error.clone(), kit).role("alert"));
    } else if profiles.busy {
        body = body.child(quiet("Opening protected saved connections…", kit).role("status"));
    }
    body = body
        .child(
            rune::col()
                .gap(8.0)
                .child(
                    Button::new("Add connection")
                        .disabled(profiles.busy)
                        .on_press(|state: &mut State| state.profile_action(ProfileAction::New))
                        .build(kit)
                        .key("connection-profile-add"),
                )
                .child(
                    Button::new("Edit connection")
                        .disabled(profiles.busy || profiles.editing)
                        .on_press(|state: &mut State| state.profile_action(ProfileAction::Edit))
                        .build(kit)
                        .key("connection-profile-edit"),
                )
                .child(
                    Button::new("Refresh saved connections")
                        .variant(ButtonVariant::Ghost)
                        .disabled(profiles.busy)
                        .on_press(|state: &mut State| state.profile_action(ProfileAction::Refresh))
                        .build(kit)
                        .key("connection-profiles-refresh"),
                ),
        )
        .child(line("Connection name", kit))
        .child(
            TextInput::new(&profiles.name)
                .label("Connection name")
                .placeholder("Local review gateway")
                .width(None)
                .disabled(!profiles.editing || profiles.busy || state.connected())
                .on_edit(|state: &mut State| &mut state.connection_profiles.name)
                .on_change(|state: &mut State, _| state.profile_editor_changed(false))
                .build(kit)
                .key("connection-profile-name"),
        );
    body
}

pub(crate) fn actions(state: &State, kit: &Kit) -> El {
    let profiles = &state.connection_profiles;
    let mut body = rune::col()
        .gap(7.0)
        .key("connection-profile-actions")
        .child(
            rune::col()
                .gap(8.0)
                .child(
                    Button::new("Choose credential file")
                        .disabled(!profiles.editing || profiles.busy || state.connected())
                        .on_press_tx(|state: &mut State, tx| state.choose_profile_credential(tx))
                        .build(kit)
                        .key("connection-profile-credential"),
                )
                .child(
                    Button::new("Save connection")
                        .disabled(
                            !profiles.editing
                                || profiles.busy
                                || profiles.name.text.is_empty()
                                || state.connection.token_file.text.is_empty()
                                || !matches!(profiles.catalog, Load::Ready(_)),
                        )
                        .on_press(|state: &mut State| state.profile_action(ProfileAction::Save))
                        .build(kit)
                        .key("connection-profile-save"),
                )
                .child(
                    Button::new(if matches!(profiles.test, Load::Loading) {
                        "Testing connection…"
                    } else {
                        "Test connection"
                    })
                    .disabled(profiles.busy || matches!(profiles.test, Load::Loading))
                    .on_press(|state: &mut State| state.profile_action(ProfileAction::Test))
                    .build(kit)
                    .key("connection-profile-test"),
                ),
        );
    if profiles.editing {
        if let Load::Ready(catalog) = &profiles.catalog {
            if let Some(id) = &catalog.selected_profile_id {
                let id = id.clone();
                body = body.child(
                    Button::new("Cancel edit")
                        .variant(ButtonVariant::Ghost)
                        .disabled(profiles.busy)
                        .on_press(move |state: &mut State| {
                            state.profile_action(ProfileAction::Select(id.clone()))
                        })
                        .build(kit)
                        .key("connection-profile-cancel"),
                );
            }
        }
    }
    match &profiles.test {
        Load::Ready(summary) => body = body.child(line(format!("Last test: {} · API {} · backend {} · gateway {} · storage {}. Test does not connect or register this app.",summary.origin,summary.api_version,summary.deployment_id,
            if summary.ready { "ready" } else { "not ready" },if summary.storage { "available" } else { "unavailable" }),kit).key("connection-test-result").role("status")),
        Load::Failed(error) => {
            let phase = profiles.test_failure.as_ref().map_or("Connection",|failure| match failure.phase {
                ConnectionTestPhase::Origin => "Gateway address",ConnectionTestPhase::CredentialReference => "Credential reference",ConnectionTestPhase::Reachability => "Reachability",
                ConnectionTestPhase::Authentication => "Authentication",ConnectionTestPhase::Compatibility => "API compatibility",ConnectionTestPhase::Readiness => "Gateway readiness",
            });
            body = body.child(line(format!("Test failed · {phase}: {error}"),kit).key("connection-test-result").role("alert"));
        }
        _ => {}
    }
    if let Some(action) = &profiles.confirmation {
        let action = match action {
            ProfileAction::New => "Add a connection",
            ProfileAction::Edit => "Edit the connection",
            ProfileAction::Select(_) => "Select another connection",
            ProfileAction::Connect => "Connect or reconnect",
            _ => "Change connection",
        };
        body = body.child(rune::col().gap(7.0).key("connection-profile-switch-confirmation")
            .child(heading("Change connection and discard local drafts?",kit))
            .child(quiet(format!("{action}. Unsaved backend form drafts and private results will be cleared. Accepted backend jobs continue, and on-disk workspace recovery journals remain intact."),kit))
            .child(Button::new("Discard drafts and continue").on_press(|state:&mut State| state.confirm_profile_switch()).build(kit).key("connection-profile-switch-confirm"))
            .child(Button::new("Keep current connection").variant(ButtonVariant::Ghost).on_press(|state:&mut State| state.connection_profiles.confirmation = None).build(kit).key("connection-profile-switch-keep")));
    }
    if let Some(message) = &profiles.message {
        body = body.child(quiet(message.clone(), kit).role("status"));
    }
    body
}
