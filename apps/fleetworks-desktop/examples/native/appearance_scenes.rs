//! Same production shell with explicit display preferences; fixtures never query
//! or change the host accessibility settings.

use fleetworks_desktop::accessibility::FixtureAccessibility;
use fleetworks_desktop::State;

pub fn satin(state: &mut State) {
    super::atlas_ready(state);
    state.accessibility.fixture_override = Some(FixtureAccessibility::default());
}
pub fn matte(state: &mut State) {
    satin(state);
    state.accessibility.fixture_override = Some(FixtureAccessibility {
        reduced_transparency: true,
        ..FixtureAccessibility::default()
    });
}
pub fn reduced_motion(state: &mut State) {
    satin(state);
    state.accessibility.fixture_override = Some(FixtureAccessibility {
        reduced_motion: true,
        ..FixtureAccessibility::default()
    });
}
pub fn fallback(state: &mut State) {
    satin(state);
    state.accessibility.fixture_override = Some(FixtureAccessibility {
        matte_fallback: true,
        ..FixtureAccessibility::default()
    });
}
