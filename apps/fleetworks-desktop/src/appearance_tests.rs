//! Same-shell geometry and state assertions for explicit fixture preferences.

use fleetworks_contracts::readiness::ReadinessCheck;
use fleetworks_contracts::readiness::ReadinessCheckId;
use fleetworks_contracts::readiness::ReadinessState;
use fleetworks_contracts::readiness::WorkstationReadiness;
use rune::Env;
use rune::Runtime;

use crate::accessibility::FixtureAccessibility;
use crate::app;
use crate::app::Load;
use crate::app::Page;
use crate::app::State;

fn runtime(page: Page, preferences: FixtureAccessibility) -> Runtime<State> {
    let mut state = State::fixture();
    state.page = page;
    state.accessibility.fixture_override = Some(preferences);
    Runtime::new(
        state,
        app::view,
        (1280.0, 800.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    )
}

#[test]
fn material_and_matte_keep_actual_toolbar_and_navigation_hit_bounds_on_all_pages() {
    for page in Page::ALL {
        let mut satin = runtime(page, FixtureAccessibility::default());
        let mut matte = runtime(
            page,
            FixtureAccessibility {
                reduced_transparency: true,
                matte_fallback: true,
                ..FixtureAccessibility::default()
            },
        );
        satin.frame(0.0);
        matte.frame(0.0);
        for key in [
            "connection-editor".to_owned(),
            format!("nav-{}", page.key()),
        ] {
            let satin_box = satin
                .presented(&key)
                .expect("production control in satin mode");
            let matte_box = matte
                .presented(&key)
                .expect("same production control in matte mode");
            assert_eq!(
                satin_box, matte_box,
                "{key} paint mode cannot alter geometry or opacity on {page:?}"
            );
        }
        assert_eq!(
            satin.accessibility_json(),
            matte.accessibility_json(),
            "material changes no accessible names, roles or selected state"
        );
    }
}

#[test]
fn real_rune_reduced_motion_survives_fixture_false_and_fixture_true_reaches_runtime() {
    let mut inherited = runtime(Page::Workstation, FixtureAccessibility::default());
    inherited.set_system_reduced_motion(true);
    inherited.frame(0.0);
    assert!(
        inherited.reduced_motion,
        "an actual host preference cannot be disabled by a fixture false"
    );
    let mut forced = runtime(
        Page::Workstation,
        FixtureAccessibility {
            reduced_motion: true,
            ..FixtureAccessibility::default()
        },
    );
    forced.frame(0.0);
    assert!(
        forced.reduced_motion,
        "explicit fixture preference must reach Rune's actual motion gate"
    );
}

#[test]
fn actual_readiness_view_distinguishes_stale_ready_and_unverified_inference() {
    let mut runtime = runtime(Page::Workstation, FixtureAccessibility::default());
    let ready = ReadinessCheck::new(
        ReadinessCheckId::Gateway,
        ReadinessState::Ready,
        "previous_observation".into(),
        "Synthetic previous gateway check".into(),
        "2026-01-01T00:00:00Z".into(),
    );
    let mut inference = ReadinessCheck::new(
        ReadinessCheckId::ClassifierInference,
        ReadinessState::Unknown,
        "inference_not_observed".into(),
        "No real inference receipt observed".into(),
        "2026-01-01T00:00:00Z".into(),
    );
    inference.fresh_for_ms = 30_000;
    runtime.state.readiness = Load::Ready(WorkstationReadiness::new(
        "synthetic-deployment".into(),
        "2026-01-01T00:00:00Z".into(),
        vec![ready, inference],
    ));
    runtime.state.readiness_received_at = Some(0.0);
    runtime.frame(0.0);
    let tree = runtime.accessibility_json().to_string();
    assert!(tree.contains("Gateway: Ready · stale observation"));
    assert!(tree.contains("Classifier inference: Unknown"));
    assert!(!tree.contains("Classifier inference: Ready"));
    assert!(
        runtime.state.intents.is_empty(),
        "rendering status never starts maintenance or inference"
    );
}
