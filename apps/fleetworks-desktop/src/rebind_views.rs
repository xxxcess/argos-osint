//! Same production and fixture controls for explicit workstation IP changes.

use rune::El;
use rune_kit::Button;
use rune_kit::ButtonVariant;
use rune_kit::Kit;

use crate::app::State;
use crate::views::heading;
use crate::views::quiet;

pub(crate) fn controls(state: &State, kit: &Kit) -> El {
    let available = state.rebind_confirmation();
    let mut body = rune::col().gap(7.0).key("workspace-rebind")
        .child(heading("Change this workstation's IP", kit))
        .child(quiet("Choose the new LAN or VPN interface above. This changes Connect IDs for every workspace on this backend and signs out their browser sessions. Files and job history stay in place.", kit))
        .child(Button::new("Review IP change…").variant(ButtonVariant::Outline).disabled(available.is_err())
            .on_press(|state: &mut State| state.preview_rebind()).build(kit).key("workspace-rebind-preview"));
    if let Err(reason) = available {
        body = body.child(quiet(reason, kit));
    }
    if let Some(confirmation) = &state.rebind.confirmation {
        body = body.child(rune::col().gap(7.0).key("workspace-rebind-confirmation")
            .child(heading(format!("Change {} → {}", confirmation.prior.workstation_ip, confirmation.interface.ip), kit))
            .child(quiet(format!("Backend {} · selected workspace {}. Other workspaces will need their registrations refreshed before further requests.", confirmation.prior.deployment_id, confirmation.workspace.name), kit))
            .child(rune::row().gap(8.0)
                .child(Button::new("Confirm IP change").disabled(state.workspaces.busy)
                    .on_press(|state: &mut State| state.confirm_rebind()).build(kit).key("workspace-rebind-confirm"))
                .child(Button::new("Keep current IP").variant(ButtonVariant::Ghost)
                    .on_press(|state: &mut State| state.rebind.confirmation = None).build(kit).key("workspace-rebind-keep"))));
    }
    body
}
