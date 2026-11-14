use fleetworks_contracts::capabilities::Capabilities;
use fleetworks_workstation::native::WorkspaceState;
use rune::Env;
use rune::Runtime;

use super::*;
use crate::app;
use crate::app::Page;
use crate::connection::ConnectionStatus;
use crate::connection::ConnectionTestPhase;
use crate::workspaces::WorkspaceRow;

fn reference(origin: &str) -> CredentialReference {
    serde_json::from_value(serde_json::json!({"path":"/fixture/private/gateway-token","origin":origin,"object":{"device":1,"inode":42,"owner":501}})).expect("synthetic file reference; no file is opened")
}
fn catalog() -> ConnectionProfilesSnapshot {
    serde_json::from_value(serde_json::json!({"revision":1,"selected_profile_id":"a".repeat(64),"profiles":[
        {"id":"a".repeat(64),"revision":1,"name":"Local review","origin":"http://127.0.0.1:8080/","credential":reference("http://127.0.0.1:8080/")},
        {"id":"b".repeat(64),"revision":1,"name":"Remote review","origin":"https://gateway.example.test/","credential":reference("https://gateway.example.test/")}
    ]})).expect("synthetic current profile catalog")
}
fn fixture() -> State {
    let mut state = State::fixture();
    state.page = Page::Workstation;
    state.connection_profiles.catalog = Load::Ready(catalog());
    state.connection_profiles.initial_selection_applied = true;
    state.install_profile(&catalog().profiles[0]);
    state.fixture_effect = Some(reply);
    state
}
fn reply(state: &mut State, intent: &Intent) {
    match intent {
        Intent::ConnectionProfile {
            action: ProfileAction::Test,
        } => state.complete_profile_test(
            state.connection_profiles.test_revision(),
            Ok(ConnectionTestSummary {
                origin: state.connection.origin.text.clone(),
                deployment_id: "synthetic-deployment".into(),
                api_version: 1,
                ready: false,
                storage: true,
                credential: state.connection_profiles.credential.clone(),
            }),
        ),
        Intent::ConnectionProfile {
            action: ProfileAction::Select(id),
        } => {
            let Load::Ready(mut catalog) = state.connection_profiles.catalog.clone() else {
                return;
            };
            catalog.revision += 1;
            catalog.selected_profile_id = Some(id.clone());
            state.complete_profile_catalog(state.connection_profiles.revision(), Ok(catalog));
        }
        Intent::ConnectionProfile {
            action: ProfileAction::Save,
        } => {
            let Load::Ready(mut catalog) = state.connection_profiles.catalog.clone() else {
                return;
            };
            let id = state
                .connection_profiles
                .selected_id
                .clone()
                .unwrap_or_else(|| "c".repeat(64));
            let profile:GatewayProfile = serde_json::from_value(serde_json::json!({"id":id,"revision":catalog.revision+1,"name":state.connection_profiles.name.text,
                "origin":state.connection.origin.text,"credential":reference(&state.connection.origin.text)})).expect("synthetic explicit metadata save");
            catalog.profiles.retain(|saved| saved.id != profile.id);
            catalog.profiles.push(profile.clone());
            catalog.revision += 1;
            catalog.selected_profile_id = Some(profile.id);
            state.complete_profile_catalog(state.connection_profiles.revision(), Ok(catalog));
        }
        _ => {}
    }
}

#[test]
fn test_connection_never_installs_client_context_or_connected_state() {
    let mut state = fixture();
    state.profile_action(ProfileAction::Test);
    assert!(
        matches!(state.connection_profiles.test,Load::Ready(ref summary) if !summary.ready && summary.storage)
    );
    assert_eq!(state.connection.status, ConnectionStatus::Disconnected);
    assert!(state.client.is_none() && state.registration.context.is_none());
    assert_eq!(
        state.intents,
        vec![Intent::ConnectionProfile {
            action: ProfileAction::Test
        }]
    );
    state.connection.status = ConnectionStatus::Connected;
    state.connection.capabilities = Some(Capabilities::default());
    state.brain.query = TextEdit::new("preserved current backend query");
    state.profile_action(ProfileAction::Test);
    assert_eq!(state.connection.status, ConnectionStatus::Connected);
    assert_eq!(
        state.brain.query.text, "preserved current backend query",
        "testing the current reference does not switch active data"
    );
}

#[test]
fn edited_destination_clears_credential_binding_and_rejects_stale_test_and_connect_callbacks() {
    let mut state = fixture();
    state.fixture_effect = None;
    state.profile_action(ProfileAction::Test);
    let old_test = state.connection_profiles.test_revision();
    let old_epoch = state.connection.epoch;
    state.connection.origin = TextEdit::new("https://different.example.test/");
    state.profile_editor_changed(true);
    assert!(
        state.connection_profiles.credential.is_none()
            && state.connection.token_file.text.is_empty()
    );
    state.complete_profile_test(
        old_test,
        Ok(ConnectionTestSummary {
            origin: "http://127.0.0.1:8080/".into(),
            deployment_id: "old".into(),
            api_version: 1,
            ready: true,
            storage: true,
            credential: None,
        }),
    );
    state.complete_connection(old_epoch, Err("stale old connection".into()));
    assert!(matches!(state.connection_profiles.test, Load::Idle));
    assert_eq!(state.connection.status, ConnectionStatus::Disconnected);
    assert!(state.client.is_none());
}

#[test]
fn saved_profile_switch_isolates_backend_data_and_keeps_explicit_workspace() {
    let mut state = fixture();
    state.connection.status = ConnectionStatus::Connected;
    state.brain.query = TextEdit::new("old backend query");
    state.workspaces.selected = Some(WorkspaceRow {
        name: "default".into(),
        path: "/fixture/home/.fleetworks/default".into(),
        marker: "synthetic-default".into(),
        protected_default: true,
        state: WorkspaceState::Local,
        bindings: Vec::new(),
    });
    let old_epoch = state.connection.epoch;
    state.profile_action(ProfileAction::Select("b".repeat(64)));
    assert!(state.connection.epoch > old_epoch && state.brain.query.text.is_empty());
    assert_eq!(
        state.connection.origin.text,
        "https://gateway.example.test/"
    );
    assert_eq!(
        state
            .workspaces
            .selected
            .as_ref()
            .expect("explicit workspace is independent")
            .marker,
        "synthetic-default"
    );
    assert!(state.client.is_none() && state.registration.context.is_none());
}

#[test]
fn retained_profile_switch_confirms_dirty_drafts_and_native_setup_has_no_profile_crud() {
    let mut state = fixture();
    state.configuration.draft_revision = 1;
    state
        .configuration
        .account_api_key
        .replace_text("synthetic-unsaved-key");
    state.profile_action(ProfileAction::Select("b".repeat(64)));
    assert!(state.connection_profiles.confirmation.is_some());
    assert_eq!(state.connection.origin.text, "http://127.0.0.1:8080/");
    assert_eq!(
        state.configuration.account_api_key.text, "synthetic-unsaved-key",
        "review must preserve drafts until explicit confirmation"
    );
    let mut runtime = Runtime::new(
        state,
        app::view,
        (1180.0, 780.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    runtime.frame(0.0);
    assert!(runtime
        .find(&format!("connection-profile-{}", "b".repeat(64)))
        .is_none());
    assert!(runtime.find("connection-profile-switch-confirm").is_none());
    assert!(!runtime
        .accessibility_json()
        .to_string()
        .contains("synthetic-unsaved-key"));
    let (x, y) = runtime
        .find("connection-editor")
        .expect("allowlisted Setup Gateway action");
    runtime.pointer_down(x, y);
    runtime.pointer_up(x, y);
    runtime.frame(1.0 / 60.0);
    assert!(runtime.state.setup_gateway);
    assert!(
        !runtime.state.configuration.account_api_key.text.is_empty(),
        "opening URL setup alone cannot discard an unrelated draft"
    );
    runtime.update(|state, _| state.confirm_profile_switch());
    runtime.frame(2.0 / 60.0);
    assert_eq!(
        runtime.state.connection.origin.text,
        "https://gateway.example.test/"
    );
    assert!(runtime.state.configuration.account_api_key.text.is_empty());
}

#[test]
fn offline_add_edit_save_uses_same_handlers_and_never_serializes_token_bytes() {
    let mut state = fixture();
    state.profile_action(ProfileAction::New);
    state.connection_profiles.name = TextEdit::new("Another review");
    state.connection.origin = TextEdit::new("https://another.example.test/");
    state.connection.token_file = TextEdit::new("/fixture/private/another-token");
    state.profile_action(ProfileAction::Save);
    assert!(!state.connection_profiles.editing && !state.connection_profiles.busy);
    assert_eq!(
        state.connection_profiles.selected_id.as_deref(),
        Some("c".repeat(64).as_str())
    );
    assert_eq!(state.connection.status, ConnectionStatus::Disconnected);
    state.profile_action(ProfileAction::Edit);
    state.connection_profiles.name = TextEdit::new("Renamed review");
    state.profile_action(ProfileAction::Save);
    assert_eq!(state.connection_profiles.name.text, "Renamed review");
    assert!(state
        .intents
        .iter()
        .all(|intent| !format!("{intent:?}").contains("synthetic-unsaved-key")));
    assert!(!state.intents.iter().any(|intent| matches!(
        intent,
        Intent::Connect | Intent::Register | Intent::OpenIntel
    )));
}

#[test]
fn launch_override_and_user_edits_are_not_overwritten_by_late_saved_selection() {
    let mut explicit = State::fixture();
    explicit.connection_profiles.launch_override = true;
    explicit.connection.origin = TextEdit::new("https://explicit.example.test/");
    explicit.complete_profile_catalog(explicit.connection_profiles.revision(), Ok(catalog()));
    assert_eq!(
        explicit.connection.origin.text,
        "https://explicit.example.test/"
    );
    assert_eq!(explicit.connection.status, ConnectionStatus::Disconnected);
    let mut restored = State::fixture();
    restored.complete_profile_catalog(restored.connection_profiles.revision(), Ok(catalog()));
    assert_eq!(restored.connection_profiles.name.text, "Local review");
    assert!(!restored.connected() && restored.client.is_none());
    let mut edited = State::fixture();
    edited.connection.origin = TextEdit::new("https://typed.example.test/");
    edited.profile_editor_changed(true);
    edited.complete_profile_catalog(edited.connection_profiles.revision(), Ok(catalog()));
    assert_eq!(edited.connection.origin.text, "https://typed.example.test/");
}

#[test]
fn failed_auth_test_is_separate_from_reachability_and_does_not_change_active_state() {
    let mut state = fixture();
    state.fixture_effect = None;
    state.profile_action(ProfileAction::Test);
    state.complete_profile_test(
        state.connection_profiles.test_revision(),
        Err(ConnectionTestFailure {
            phase: ConnectionTestPhase::Authentication,
            message: "API returned HTTP 401".into(),
        }),
    );
    assert!(matches!(state.connection_profiles.test, Load::Failed(_)));
    assert_eq!(
        state
            .connection_profiles
            .test_failure
            .as_ref()
            .expect("typed failure")
            .phase,
        ConnectionTestPhase::Authentication
    );
    assert_eq!(state.connection.status, ConnectionStatus::Disconnected);
}

#[test]
fn cleared_or_changed_profile_catalog_drops_cached_credential_reference_and_authority() {
    let mut state = fixture();
    state.connection.status = ConnectionStatus::Connected;
    let mut cleared = ConnectionProfilesSnapshot::empty();
    cleared.revision = 2;
    state.complete_profile_catalog(state.connection_profiles.revision(), Ok(cleared));
    assert!(
        state.connection_profiles.credential.is_none()
            && state.connection_profiles.selected_id.is_none()
    );
    assert!(state.connection.token_file.text.is_empty() && state.client.is_none());
    assert_eq!(state.connection.status, ConnectionStatus::Disconnected);
    assert!(
        state.connection_profiles.editing,
        "a cleared profile requires an explicit fresh selection"
    );
}
