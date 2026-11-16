//! Assistant dock and command overlay on the production shell. Each pair shows the glass
//! form and the opaque form with the same bounds. Synthetic conversation only; nothing is sent.

use fleetworks_desktop::accessibility::FixtureAccessibility;
use fleetworks_desktop::State;

fn opaque(state: &mut State) {
    state.accessibility.fixture_override = Some(FixtureAccessibility {
        reduced_transparency: true,
        ..FixtureAccessibility::default()
    });
}

pub fn assistant_open(state: &mut State) {
    super::registered(state);
    state.fixture_show_assistant();
}

pub fn assistant_opaque(state: &mut State) {
    assistant_open(state);
    opaque(state);
}

pub fn command_panel(state: &mut State) {
    super::registered(state);
    state.fixture_open_command_panel();
}

pub fn command_panel_opaque(state: &mut State) {
    command_panel(state);
    opaque(state);
}
