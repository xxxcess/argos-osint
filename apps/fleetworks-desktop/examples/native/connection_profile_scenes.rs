//! Same production profile controls with synthetic local-store/test effects.

use fleetworks_desktop::app::Intent;
use fleetworks_desktop::app::Load;
use fleetworks_desktop::connection::ConnectionTestFailure;
use fleetworks_desktop::connection::ConnectionTestPhase;
use fleetworks_desktop::connection::ConnectionTestSummary;
use fleetworks_desktop::connection_profiles::ProfileAction;
use fleetworks_desktop::Page;
use fleetworks_desktop::State;
use fleetworks_workstation::native::ConnectionProfilesSnapshot;
use fleetworks_workstation::native::CredentialReference;
use fleetworks_workstation::native::GatewayProfile;
use rune::TextEdit;

fn reference(origin: &str) -> CredentialReference {
    serde_json::from_value(serde_json::json!({"path":"/fixture/private/gateway-token","origin":origin,"object":{"device":1,"inode":42,"owner":501}})).expect("synthetic reference only")
}
fn snapshot() -> ConnectionProfilesSnapshot {
    serde_json::from_value(serde_json::json!({"revision":1,"selected_profile_id":"a".repeat(64),"profiles":[
        {"id":"a".repeat(64),"revision":1,"name":"Local review gateway","origin":"http://127.0.0.1:8080/","credential":reference("http://127.0.0.1:8080/")},
        {"id":"b".repeat(64),"revision":1,"name":"Remote review gateway","origin":"https://gateway.example.test/","credential":reference("https://gateway.example.test/")}
    ]})).expect("synthetic owned profile catalog")
}
pub fn saved(state: &mut State) {
    super::workspaces_ready(state);
    state.disconnect();
    state.page = Page::Workstation;
    state.complete_profile_catalog(state.connection_profiles.revision(), Ok(snapshot()));
}
pub fn empty(state: &mut State) {
    super::workspaces_ready(state);
    state.disconnect();
    state.page = Page::Workstation;
    state.complete_profile_catalog(
        state.connection_profiles.revision(),
        Ok(ConnectionProfilesSnapshot::empty()),
    );
}
pub fn add(state: &mut State) {
    saved(state);
    state.profile_action(ProfileAction::New);
    state.connection_profiles.name = TextEdit::new("Research gateway");
    state.connection.origin = TextEdit::new("https://research.example.test/");
    state.profile_editor_changed(true);
    state.select_profile_credential("/fixture/private/research-token".into());
}
pub fn edit(state: &mut State) {
    saved(state);
    state.profile_action(ProfileAction::Edit);
}
pub fn tested(state: &mut State) {
    saved(state);
    state.profile_action(ProfileAction::Test);
}
pub fn auth_error(state: &mut State) {
    fn effect(state: &mut State, intent: &Intent) {
        if matches!(
            intent,
            Intent::ConnectionProfile {
                action: ProfileAction::Test
            }
        ) {
            state.complete_profile_test(
                state.connection_profiles.test_revision(),
                Err(ConnectionTestFailure {
                    phase: ConnectionTestPhase::Authentication,
                    message: "API returned HTTP 401".into(),
                }),
            );
        } else {
            super::reply(state, intent);
        }
    }
    saved(state);
    state.fixture_effect = Some(effect);
    state.profile_action(ProfileAction::Test);
}
pub fn conflict(state: &mut State) {
    fn effect(state: &mut State, intent: &Intent) {
        if matches!(
            intent,
            Intent::ConnectionProfile {
                action: ProfileAction::Save
            }
        ) {
            state.complete_profile_catalog(state.connection_profiles.revision(),Err("Saved connections changed in another window. Refresh before saving this draft.".into()));
        } else {
            super::reply(state, intent);
        }
    }
    edit(state);
    state.fixture_effect = Some(effect);
    state.connection_profiles.name = TextEdit::new("Edited review gateway");
    state.profile_action(ProfileAction::Save);
}
pub fn dirty_switch(state: &mut State) {
    saved(state);
    state.configuration_edited();
    state
        .configuration
        .account_api_key
        .replace_text("synthetic-unsaved-key-must-not-appear");
    state.profile_action(ProfileAction::Select("b".repeat(64)));
}

pub fn reply(state: &mut State, intent: &Intent) -> bool {
    match intent {
        Intent::ChooseConnectionCredential => {
            state.select_profile_credential("/fixture/private/gateway-token".into())
        }
        Intent::ConnectionCredentialSelected => {
            state.connection_profiles.credential = Some(reference(&state.connection.origin.text))
        }
        Intent::ConnectionProfile {
            action: ProfileAction::Test,
        } => state.complete_profile_test(
            state.connection_profiles.test_revision(),
            Ok(ConnectionTestSummary {
                origin: state.connection.origin.text.clone(),
                deployment_id: "synthetic-deployment".into(),
                api_version: 1,
                ready: true,
                storage: true,
                credential: state.connection_profiles.credential.clone(),
            }),
        ),
        Intent::ConnectionProfile {
            action: ProfileAction::Refresh,
        } => state.complete_profile_catalog(state.connection_profiles.revision(), Ok(snapshot())),
        Intent::ConnectionProfile {
            action: ProfileAction::Select(id),
        } => {
            let Load::Ready(mut snapshot) = state.connection_profiles.catalog.clone() else {
                return true;
            };
            snapshot.revision += 1;
            snapshot.selected_profile_id = Some(id.clone());
            state.complete_profile_catalog(state.connection_profiles.revision(), Ok(snapshot));
        }
        Intent::ConnectionProfile {
            action: ProfileAction::Save,
        } => {
            let Load::Ready(mut snapshot) = state.connection_profiles.catalog.clone() else {
                return true;
            };
            let id = state
                .connection_profiles
                .selected_id
                .clone()
                .unwrap_or_else(|| "c".repeat(64));
            let profile:GatewayProfile = serde_json::from_value(serde_json::json!({"id":id,"revision":snapshot.revision+1,"name":state.connection_profiles.name.text,
                "origin":state.connection.origin.text,"credential":reference(&state.connection.origin.text)})).expect("synthetic metadata save");
            snapshot.profiles.retain(|saved| saved.id != profile.id);
            snapshot.profiles.push(profile.clone());
            snapshot.revision += 1;
            snapshot.selected_profile_id = Some(profile.id);
            state.complete_profile_catalog(state.connection_profiles.revision(), Ok(snapshot));
        }
        _ => return false,
    }
    true
}
