//! Small native editors; secret inputs never appear in confirmation summaries.

use fleetworks_contracts::context::Permission;
use rune::El;
use rune::TextEdit;
use rune_kit::Button;
use rune_kit::ButtonVariant;
use rune_kit::Kit;
use rune_kit::TextArea;
use rune_kit::TextInput;

use crate::app::Load;
use crate::app::State;
use crate::views::heading;
use crate::views::line;
use crate::views::quiet;

fn field(
    label: &str,
    edit: &TextEdit,
    getter: for<'a> fn(&'a mut State) -> &'a mut TextEdit,
    secure: bool,
    key: &str,
    kit: &Kit,
) -> El {
    let input = TextInput::new(edit)
        .label(label.to_owned())
        .width(None)
        .on_edit(getter)
        .on_change(|state: &mut State, _| state.configuration_edited());
    let input = if secure { input.secure() } else { input };
    rune::col()
        .gap(4.0)
        .child(quiet(label.to_owned(), kit))
        .child(input.build(kit).key(key.to_owned()))
}

pub(crate) fn model_editor(state: &State, kit: &Kit) -> El {
    let enabled = state.permission(Permission::ManageConfiguration)
        && matches!(state.backend.models, Load::Ready(_))
        && !matches!(state.configuration.result, Load::Loading);
    rune::col().gap(10.0).child(heading("Edit a canonical role", kit))
        .child(quiet("Choose Edit on a role above. Enter its primary provider/model/account route and an ordered fallback array. Saving one role never adds inheritance from another role.", kit))
        .child(field("Role", &state.configuration.role, |state| &mut state.configuration.role, false, "model-role-name", kit))
        .child(field("Primary provider", &state.configuration.model_provider, |state| &mut state.configuration.model_provider, false, "model-primary-provider", kit))
        .child(field("Primary model", &state.configuration.model, |state| &mut state.configuration.model, false, "model-primary-model", kit))
        .child(field("Saved account ID", &state.configuration.model_account, |state| &mut state.configuration.model_account, false, "model-primary-account", kit))
        .child(TextArea::new(&state.configuration.fallbacks).label("Ordered fallback routes (JSON)").width(None).rows(5)
            .placeholder("[{\"provider\":\"local\",\"model\":\"model-name\",\"account\":\"account-id\"}]")
            .on_edit(|state: &mut State| &mut state.configuration.fallbacks)
            .on_change(|state: &mut State, _| state.configuration_edited()).build(kit).key("model-fallbacks"))
        .child(rune::row().gap(8.0)
            .child(Button::new("Review role assignment").disabled(!enabled).on_press_tx(|state: &mut State, tx| { state.preview_model_role(false); tx.scroll_into_view("configuration-confirmation"); }).build(kit).key("model-preview"))
            .child(Button::new("Clear role assignment…").variant(ButtonVariant::Outline).disabled(!enabled)
                .on_press_tx(|state: &mut State, tx| { state.preview_model_role(true); tx.scroll_into_view("configuration-confirmation"); }).build(kit).key("model-preview-clear")))
        .child(confirmation(state, kit))
}

pub(crate) fn profile_editor(state: &State, kit: &Kit) -> El {
    let enabled = state.permission(Permission::ManageConfiguration)
        && matches!(state.backend.profile, Load::Ready(_))
        && !matches!(state.configuration.result, Load::Loading);
    rune::col().gap(10.0).child(heading("Add or update provider account", kit))
        .child(quiet("Enter current backend credentials explicitly. An omitted key preserves the saved key only when provider and base URL match; turning preservation off with an empty key explicitly clears it.", kit))
        .child(field("Account ID", &state.configuration.account_id, |state| &mut state.configuration.account_id, false, "account-id", kit))
        .child(field("Provider", &state.configuration.account_provider, |state| &mut state.configuration.account_provider, false, "account-provider", kit))
        .child(field("Base URL", &state.configuration.account_base_url, |state| &mut state.configuration.account_base_url, false, "account-base-url", kit))
        .child(field("Default model (optional)", &state.configuration.account_model, |state| &mut state.configuration.account_model, false, "account-model", kit))
        .child(field("New API key", &state.configuration.account_api_key, |state| &mut state.configuration.account_api_key, true, "account-api-key", kit))
        .child(Button::new(if state.configuration.preserve_account_key { "Saved key preservation: on" } else { "Saved key preservation: off — empty clears" })
            .variant(ButtonVariant::Outline).on_press(|state: &mut State| {
                state.configuration.preserve_account_key = !state.configuration.preserve_account_key; state.configuration_edited();
            }).build(kit).key("account-preserve-key"))
        .child(Button::new("Review account change").disabled(!enabled)
            .on_press_tx(|state: &mut State, tx| { state.preview_provider_account(); tx.scroll_into_view("configuration-confirmation"); }).build(kit).key("account-preview"))
        .child(heading("Replace tool credential slots", kit))
        .child(quiet("Choose a provider above. This change replaces both slots; an empty slot is an explicit clear. Keys remain masked and are sent only when you confirm.", kit))
        .child(field("Tool provider", &state.configuration.tool_provider, |state| &mut state.configuration.tool_provider, false, "tool-credential-provider", kit))
        .child(field("Primary tool key", &state.configuration.tool_primary, |state| &mut state.configuration.tool_primary, true, "tool-key-primary", kit))
        .child(field("Fallback tool key", &state.configuration.tool_fallback, |state| &mut state.configuration.tool_fallback, true, "tool-key-fallback", kit))
        .child(Button::new("Review tool credential replacement").disabled(!enabled)
            .on_press_tx(|state: &mut State, tx| { state.preview_tool_credentials(); tx.scroll_into_view("configuration-confirmation"); }).build(kit).key("tool-credentials-preview"))
        .child(confirmation(state, kit))
}

fn confirmation(state: &State, kit: &Kit) -> El {
    let mut body = rune::col().gap(8.0);
    if let Some(confirmation) = &state.configuration.confirmation {
        body = body.key("configuration-confirmation").child(heading("Confirm backend configuration change", kit))
            .child(line(confirmation.change.summary(), kit))
            .child(quiet(format!("Backend {} · expected configuration revision {}. This changes the shared backend for every workspace.",
                state.connection.deployment_label(), confirmation.change.expected_revision()), kit))
            .child(Button::new("Confirm configuration change").variant(ButtonVariant::Submit)
                .on_press(|state: &mut State| state.confirm_configuration()).build(kit).key("configuration-confirm"))
            .child(Button::new("Keep current backend configuration").variant(ButtonVariant::Ghost)
                .on_press(|state: &mut State| state.configuration.confirmation = None).build(kit).key("configuration-cancel"));
    }
    match &state.configuration.result {
        Load::Loading | Load::Pending => {
            body = body.child(quiet("Saving the confirmed backend revision…", kit).role("status"))
        }
        Load::Failed(error) => body = body.child(line(error.clone(), kit).role("alert")),
        Load::Ready(saved) => {
            body = body.child(
                line(
                    format!(
                        "Backend configuration saved at revision {}",
                        saved.configuration_revision
                    ),
                    kit,
                )
                .role("status"),
            )
        }
        Load::Idle => {}
    }
    if let Some(message) = &state.configuration.message {
        body = body.child(line(message.clone(), kit).role("status"));
    }
    body
}

pub(crate) fn profile_transfer(state: &State, kit: &Kit) -> El {
    let enabled = state.permission(Permission::ManageConfiguration)
        && matches!(&state.backend.profile, Load::Ready(profile) if profile.configuration_format == 1 && profile.portable_schema_version == 4)
        && !matches!(state.transfer.result, Load::Loading);
    let mut body = rune::col().gap(10.0).child(heading("Explicit current-format profile transfer", kit))
        .child(quiet("Profile exports contain credentials. Export creates a new private file; import accepts schema 4 only and replaces the backend profile after a revision-pinned confirmation. Nothing is imported from old local settings automatically.", kit))
        .child(rune::row().gap(8.0)
            .child(Button::new("Review export with credentials…").disabled(!enabled)
                .on_press_tx(|state: &mut State, tx| { state.preview_profile_export(); tx.scroll_into_view("profile-transfer-confirmation"); }).build(kit).key("profile-export-preview"))
            .child(Button::new("Choose current-format import…").disabled(!enabled)
                .on_press_tx(|state: &mut State, tx| state.choose_profile_import(tx)).build(kit).key("profile-import-choose")));
    if let Some(confirmation) = &state.transfer.confirmation {
        body = body.child(
            rune::col()
                .gap(8.0)
                .key("profile-transfer-confirmation")
                .child(line(confirmation.summary(), kit))
                .child(quiet(
                    format!(
                        "Backend {} · expected revision {}",
                        state.connection.deployment_label(),
                        confirmation.expected_revision()
                    ),
                    kit,
                ))
                .child(
                    Button::new("Confirm profile transfer")
                        .on_press_tx(|state: &mut State, tx| state.confirm_profile_transfer(tx))
                        .build(kit)
                        .key("profile-transfer-confirm"),
                )
                .child(
                    Button::new("Cancel profile transfer")
                        .variant(ButtonVariant::Ghost)
                        .on_press(|state: &mut State| state.transfer.confirmation = None)
                        .build(kit)
                        .key("profile-transfer-cancel"),
                ),
        );
    }
    match &state.transfer.result {
        Load::Idle => {}
        Load::Pending | Load::Loading => {
            body = body.child(
                quiet(
                    "Transferring the explicitly selected current-format profile…",
                    kit,
                )
                .role("status"),
            )
        }
        Load::Failed(error) => body = body.child(line(error.clone(), kit).role("alert")),
        Load::Ready(message) => body = body.child(line(message.clone(), kit).role("status")),
    }
    body
}
