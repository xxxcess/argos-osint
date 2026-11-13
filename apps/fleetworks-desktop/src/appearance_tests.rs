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
    let disconnected_satin = runtime(Page::Workstation, FixtureAccessibility::default());
    let disconnected_matte = runtime(
        Page::Workstation,
        FixtureAccessibility {
            reduced_transparency: true,
            matte_fallback: true,
            ..FixtureAccessibility::default()
        },
    );
    for mut rt in [disconnected_satin, disconnected_matte] {
        rt.frame(0.0);
        assert!(rt.find("nav-workstation").is_some());
        assert!(rt.find("nav-setup").is_some());
        assert!(rt.find("workstation-setup-gateway").is_some());
    }

    let connected_runtime = |page: Page, preferences: FixtureAccessibility| {
        let mut state = crate::tests::registered_fixture();
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
    };
    for page in Page::ALL {
        let mut satin = connected_runtime(page, FixtureAccessibility::default());
        let mut matte = connected_runtime(
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
            "nav-workstation".to_owned(),
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
    let mut state = crate::tests::registered_fixture();
    state.page = Page::Workstation;
    state.accessibility.fixture_override = Some(FixtureAccessibility::default());
    let mut runtime = Runtime::new(
        state,
        app::view,
        (1280.0, 800.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
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

#[test]
fn material_and_matte_preserve_interactive_hit_bounds_and_a11y_on_status_and_seed() {
    use crate::workstation_dashboard::HostMetrics;

    let metrics = HostMetrics {
        cpu_percent: Some(38.4),
        memory_percent: Some(54.2),
        disk_percent: Some(67.0),
        network_kbps: None,
        platform: Some("linux/x86_64".into()),
        chipset: None,
    };

    for (page, keys) in [
        (
            Page::Workstation,
            vec![
                "workstation-tile-cpu",
                "workstation-tile-memory",
                "workstation-tile-disk",
            ],
        ),
        (
            Page::Seed,
            vec![
                "maintenance-refresh",
                "preflight-gateway",
                "preflight-datastore",
                "preflight-dataset",
                "maintenance-seed",
            ],
        ),
    ] {
        let build_runtime = |preferences: FixtureAccessibility| {
            let mut state = crate::tests::registered_fixture();
            state.page = page;
            state.accessibility.fixture_override = Some(preferences);
            state.host_metrics = Load::Ready(metrics.clone());
            state.readiness = Load::Ready(WorkstationReadiness::new(
                "synthetic-deployment".into(),
                "2026-01-01T00:00:00Z".into(),
                vec![ReadinessCheck::new(
                    ReadinessCheckId::Gateway,
                    ReadinessState::Ready,
                    "ok".into(),
                    "Gateway probe ok".into(),
                    "2026-01-01T00:00:00Z".into(),
                )],
            ));
            state.readiness_received_at = Some(0.0);
            Runtime::new(
                state,
                app::view,
                (1280.0, 800.0),
                Env {
                    system_dark: true,
                    scale: 1.0,
                },
            )
        };

        let mut satin = build_runtime(FixtureAccessibility::default());
        let mut matte = build_runtime(FixtureAccessibility {
            reduced_transparency: true,
            matte_fallback: true,
            ..FixtureAccessibility::default()
        });
        satin.frame(0.0);
        matte.frame(0.0);

        for key in keys {
            let satin_box = satin
                .presented(key)
                .unwrap_or_else(|| panic!("{key} presented in satin mode on {page:?}"));
            let matte_box = matte
                .presented(key)
                .unwrap_or_else(|| panic!("{key} presented in matte mode on {page:?}"));
            assert_eq!(
                satin_box, matte_box,
                "{key} paint mode cannot alter geometry on {page:?}"
            );
        }
        assert_eq!(
            satin.accessibility_json(),
            matte.accessibility_json(),
            "matte parity must preserve accessible names, roles, and states on {page:?}"
        );
    }
}
