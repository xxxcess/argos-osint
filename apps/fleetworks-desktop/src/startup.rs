//! The cold-launch backend chooser and its bounded Quiet Orbit animation.

use std::cell::Cell;

use fleetworks_workstation::native::BackendKind;
use rune::El;
use rune::Key;
use rune::KeyEvent;
use rune::Mods;
use rune::Transition;
use rune::Tx;
use rune_kit::Kit;

use crate::accessibility::ViewPreferences;
use crate::app;
use crate::connection::ConnectionStatus;
use crate::icons;
use crate::orbit;
use crate::theme;
use crate::ui_components::ActionButton;
use crate::views;
use crate::State;

/// The opening samples three seconds at the reference pace, then holds the final pose.
pub(crate) const INTRO_SECONDS: f64 = 3.0;
const LOCAL: &str = "startup-local";
const REMOTE: &str = "startup-remote";

/// Cold-launch presentation state. Scripted launches and ordinary fixtures skip it by default.
#[derive(Default)]
pub struct Startup {
    active: bool,
    started_at: Option<f64>,
    settled: bool,
    reduced_motion: Cell<bool>,
}

impl Startup {
    /// The state for one cold launch of the production executable.
    pub fn cold_launch() -> Self {
        Self {
            active: true,
            ..Self::default()
        }
    }

    /// Whether startup still owns the window.
    pub fn active(&self) -> bool {
        self.active
    }

    pub(crate) fn finish(&mut self) {
        self.active = false;
    }

    /// Start the orbit clock once, at the first visible frame.
    pub(crate) fn begin(&mut self, now: f64) {
        self.started_at.get_or_insert(now);
    }

    fn seconds(&self, now: f64) -> f64 {
        if self.settled {
            return INTRO_SECONDS;
        }
        self.started_at
            .map_or(0.0, |started| (now - started).clamp(0.0, INTRO_SECONDS))
    }

    /// Continue drawing through the final pose, then let the window idle.
    pub(crate) fn next_wake(&self, now: f64) -> Option<f64> {
        if self.reduced_motion.get() {
            return None;
        }
        let remaining = INTRO_SECONDS - self.seconds(now);
        (remaining > 0.0).then_some(remaining.min(1.0 / 60.0))
    }
}

/// The chooser and both setup paths share the same canvas.
pub(crate) fn view(
    state: &State,
    cx: &rune::Cx<State>,
    preferences: ViewPreferences,
    kit: &Kit,
) -> El {
    state.startup.reduced_motion.set(preferences.reduced_motion);
    let root = rune::col()
        .key("startup")
        .w_full()
        .h_full()
        .center()
        .fill(theme::hex(theme::BACKGROUND));
    if state.setup_gateway {
        let setup = rune::col()
            .key("startup-setup")
            .w_full()
            .max_w(820.0)
            .h_full()
            .pt(32.0)
            .child(
                rune::row().w_full().px(20.0).child(
                    ActionButton::new("Back")
                        .key("startup-back")
                        .disabled(state.connected())
                        .on_press_tx(State::return_to_startup_choice)
                        .build(kit, preferences, None)
                        .on_key(|state: &mut State, event, tx| {
                            activate_on_return(state, event, tx, State::return_to_startup_choice)
                        }),
                ),
            )
            .child(views::setup_gateway(state, cx, kit))
            .transition(app::surface_transition(
                preferences.reduced_motion,
                Transition::opacity().combined(Transition::offset(0.0, 6.0)),
            ));
        return root.child(setup);
    }
    root.child(choose(state, preferences, kit, cx.layout_size("startup")))
}

fn choose(state: &State, preferences: ViewPreferences, kit: &Kit, window: Option<[f32; 2]>) -> El {
    let [width, height] = window.unwrap_or([640.0, 480.0]);
    // Leave room for the controls before containing the complete orbit in the remaining space.
    let [orbit_width, orbit_height] = orbit::display_size(Some([width, height - 210.0]));
    let canvas = theme::hex(theme::BACKGROUND);
    rune::col()
        .key("startup-choose")
        .w_full()
        .items_center()
        .child(
            rune::col()
                .size(orbit_width, (orbit_height - 18.0).max(0.0))
                .child(
                    orbit::view(
                        state.startup.seconds(state.clock) as f32,
                        !preferences.reduced_motion,
                        orbit_width,
                        orbit_height,
                    )
                    .absolute(0.0, 0.0)
                    .inert(),
                ),
        )
        .child(
            rune::col()
                .key("startup-choice-controls")
                .w_full()
                .max_w(400.0)
                .items_center()
                .gradient(canvas.alpha(0.76), canvas)
                .gap(14.0)
                .child(
                    rune::text("Choose a backend")
                        .font(kit.ty.body_medium)
                        .color(theme::hex(theme::TEXT_PRIMARY)),
                )
                .child(
                    rune::row()
                        .gap(14.0)
                        .center()
                        .child(destination(
                            kit,
                            LOCAL,
                            &icons::WORKSTATION,
                            "Embedded",
                            "Run on this PC",
                            BackendKind::Local,
                        ))
                        .child(destination(
                            kit,
                            REMOTE,
                            &icons::SERVER,
                            "Distributed",
                            "Use a gateway",
                            BackendKind::Remote,
                        )),
                ),
        )
}

fn destination(
    kit: &Kit,
    key: &'static str,
    glyph: &'static rune::Icon,
    title: &'static str,
    detail: &'static str,
    kind: BackendKind,
) -> El {
    rune::col()
        .key(key)
        .size(150.0, 150.0)
        .center()
        .gap(7.0)
        .radius(12.0)
        .fill(theme::hex(theme::SURFACE_RAISED))
        .hover(theme::hex(theme::SURFACE_ELEVATED))
        .press(theme::hex(theme::SURFACE))
        .border(1.0, theme::hex(theme::BORDER_CONTROL))
        .focus_ring(2.0, 2.0, theme::hex(theme::ACCENT_HIGHLIGHT))
        .role("button")
        .focusable(true)
        .label(format!("{title}. {detail}"))
        .on_press_tx(move |state: &mut State, tx| state.open_startup_setup(kind, tx))
        .on_key(move |state: &mut State, event, tx| {
            activate_on_return(state, event, tx, |state, tx| {
                state.open_startup_setup(kind, tx)
            })
        })
        .child(
            rune::icon(glyph, 27.0)
                .color(theme::hex(theme::TEXT_SECONDARY))
                .a11y_hidden(),
        )
        .child(
            rune::text(title)
                .font(kit.ty.body_medium)
                .color(theme::hex(theme::TEXT_PRIMARY))
                .a11y_hidden(),
        )
        .child(
            rune::text(detail)
                .font(kit.ty.small)
                .color(theme::hex(theme::TEXT_SECONDARY))
                .a11y_hidden(),
        )
}

fn activate_on_return(
    state: &mut State,
    event: &KeyEvent,
    tx: &mut Tx,
    activate: impl FnOnce(&mut State, &mut Tx),
) -> bool {
    if event.key != Key::Enter || event.mods != Mods::default() {
        return false;
    }
    if !tx.key_repeat() {
        activate(state, tx);
    }
    true
}

impl State {
    fn open_startup_setup(&mut self, kind: BackendKind, tx: &mut Tx) {
        self.choose_startup_destination(kind);
        tx.focus("startup-back", tx.by_keys());
    }

    /// Render the settled backend choice through the production startup view in a fixture.
    pub fn fixture_show_backend_choice(&mut self) {
        if self.fixture {
            self.startup = Startup::cold_launch();
            self.startup.settled = true;
        }
    }

    /// Select a setup path while retaining the explicit Start or Connect action.
    pub(crate) fn choose_startup_destination(&mut self, kind: BackendKind) {
        if !self.startup.active() || self.setup_gateway {
            return;
        }
        // An outstanding profile read can populate the catalog but must respect the explicit choice.
        self.connection_profiles.initial_selection_applied = true;
        self.startup.settled = true;
        self.set_backend_kind(kind);
        self.setup_gateway = true;
    }

    pub(crate) fn return_to_startup_choice(&mut self, tx: &mut Tx) {
        if !self.startup.active() || !self.setup_gateway || self.connected() {
            return;
        }
        // Leaving a pending attempt uses the same cancellation and stale-response fence as setup.
        if self.connection.status == ConnectionStatus::Connecting
            || self.pairing.busy()
            || self.pairing.paused
        {
            self.disconnect();
        }
        self.setup_gateway = false;
        tx.focus(
            if self.is_local_backend() {
                LOCAL
            } else {
                REMOTE
            },
            tx.by_keys(),
        );
    }

    pub(crate) fn finish_gateway_setup(&mut self, tx: &mut Tx) {
        if self.startup.active() {
            self.startup.finish();
            tx.focus("nav-workstation", tx.by_keys());
        }
        self.setup_gateway = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accessibility::FixtureAccessibility;
    use crate::app::Intent;

    #[test]
    fn opening_clock_runs_once_and_holds_the_last_pose() {
        let mut startup = Startup::cold_launch();
        startup.begin(10.0);
        startup.begin(15.0);
        assert_eq!(startup.seconds(11.5), 1.5);
        assert!(startup.next_wake(11.5).is_some());
        assert_eq!(startup.seconds(15.0), INTRO_SECONDS);
        assert_eq!(startup.next_wake(15.0), None);
        startup.reduced_motion.set(true);
        assert_eq!(startup.next_wake(11.5), None);
    }

    fn runtime(size: (f32, f32), reduced_motion: bool) -> rune::Runtime<State> {
        let mut state = State::fixture();
        state.startup = Startup::cold_launch();
        state.accessibility.fixture_override = Some(FixtureAccessibility {
            reduced_motion,
            ..FixtureAccessibility::default()
        });
        rune::Runtime::new(
            state,
            crate::app::view,
            size,
            rune::Env {
                system_dark: true,
                scale: 2.0,
            },
        )
    }

    fn key(runtime: &mut rune::Runtime<State>, key: rune::Key) {
        let event = rune::KeyEvent {
            key,
            mods: rune::Mods::default(),
        };
        runtime.key(event.clone());
        runtime.key_up(event);
    }

    fn click(runtime: &mut rune::Runtime<State>, key: &str) {
        let (x, y) = runtime.find(key).expect("presented control");
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
    }

    #[test]
    fn both_paths_are_keyboard_usable_on_the_first_frame_and_back_does_not_replay() {
        for (size, reduced) in [
            ((640.0, 480.0), false),
            ((640.0, 480.0), true),
            ((1180.0, 780.0), false),
            ((1180.0, 780.0), true),
        ] {
            let mut runtime = runtime(size, reduced);
            runtime.frame(0.0);
            assert!(runtime.find("startup-orbit").is_some());
            for (choice, name) in [(LOCAL, "Embedded"), (REMOTE, "Distributed")] {
                let ([x, y, w, h], alpha) = runtime.presented(choice).expect("first-frame tile");
                assert_eq!(w, h, "backend choice must be square");
                assert_eq!(alpha, 1.0, "the tile must be visible immediately");
                assert!(x >= 0.0 && y >= 0.0 && x + w <= size.0 && y + h <= size.1);
                assert!(runtime.elements().iter().any(|element| {
                    element.key.as_deref() == Some(choice)
                        && element
                            .label
                            .as_deref()
                            .is_some_and(|v| v.starts_with(name))
                }));
            }
            key(&mut runtime, rune::Key::Tab);
            runtime.frame(0.02);
            key(&mut runtime, rune::Key::Enter);
            runtime.frame(0.30);
            assert_eq!(runtime.state.backend_kind, BackendKind::Local);
            assert!(runtime.find("connect-local").is_some());

            click(&mut runtime, "startup-back");
            runtime.frame(0.60);
            assert_eq!(runtime.state.startup.seconds(0.6), INTRO_SECONDS);
            assert!(runtime.find("startup-orbit").is_some());
            assert!(runtime.find(REMOTE).is_some());

            key(&mut runtime, rune::Key::Tab);
            runtime.frame(0.62);
            key(&mut runtime, rune::Key::Text(" ".into()));
            runtime.frame(0.90);
            assert_eq!(runtime.state.backend_kind, BackendKind::Remote);
            assert!(runtime.find("gateway-origin").is_some());
            assert!(runtime.find("connect").is_some());
            assert!(runtime.find("nav-workstation").is_none());
            assert!(!runtime.state.connected());
            assert!(runtime.state.client.is_none());
            assert!(!runtime
                .state
                .intents
                .iter()
                .any(|intent| { matches!(intent, Intent::Connect | Intent::PairingConnect) }));
        }
    }

    #[test]
    fn backend_choice_capture_has_the_fixture_marker() {
        let mut state = State::fixture();
        state.fixture_show_backend_choice();
        let mut runtime = rune::Runtime::new(
            state,
            crate::app::view,
            (640.0, 480.0),
            rune::Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        runtime.frame(0.0);
        assert!(runtime.find("startup-choose").is_some());
        assert!(runtime.find("fixture-marker").is_some());
    }
}
