//! Cold-launch backend choice and the Quiet Orbit handoff into the workstation.

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
const SETUP_MARK: &str = "startup-setup-mark";
const RAIL_MARK: &str = "brand-mark";
const SETUP_SECONDS: f64 = 0.22;
const RAIL_SECONDS: f64 = 0.21;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dock {
    Setup,
    Rail,
}

impl Dock {
    fn key(self) -> &'static str {
        match self {
            Self::Setup => SETUP_MARK,
            Self::Rail => RAIL_MARK,
        }
    }

    fn duration(self) -> f64 {
        match self {
            Self::Setup => SETUP_SECONDS,
            Self::Rail => RAIL_SECONDS,
        }
    }
}

#[derive(Clone, Copy)]
struct Flight {
    from: [f32; 4],
    started_at: f64,
    dock: Dock,
}

impl Flight {
    fn progress(self, now: f64) -> f32 {
        ((now - self.started_at) / self.dock.duration()).clamp(0.0, 1.0) as f32
    }

    fn rect(self, to: [f32; 4], progress: f32) -> [f32; 4] {
        let eased = progress * progress * (3.0 - 2.0 * progress);
        std::array::from_fn(|axis| self.from[axis] + (to[axis] - self.from[axis]) * eased)
    }
}

/// Cold-launch presentation state. Scripted launches and ordinary fixtures skip it by default.
#[derive(Default)]
pub struct Startup {
    active: bool,
    started_at: Option<f64>,
    settled: bool,
    reduced_motion: Cell<bool>,
    mark_rect: Cell<Option<[f32; 4]>>,
    docked_pose: f32,
    flight: Option<Flight>,
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

    fn start_flight(&mut self, dock: Dock, now: f64) {
        self.flight = self.mark_rect.get().and_then(|from| {
            (!self.reduced_motion.get()).then_some(Flight {
                from,
                started_at: now,
                dock,
            })
        });
    }

    pub(crate) fn poll_flight(&mut self, now: f64, tx: &mut Tx) -> bool {
        let Some(flight) = self.flight else {
            return false;
        };
        let remaining = flight.started_at + flight.dock.duration() - now;
        if self.reduced_motion.get() || remaining <= 0.0 {
            self.flight = None;
        } else {
            tx.wake_after(std::time::Duration::from_secs_f64(
                remaining.min(1.0 / 60.0),
            ));
        }
        true
    }

    fn flying_to(&self, dock: Dock, preferences: ViewPreferences) -> bool {
        !preferences.reduced_motion && self.flight.is_some_and(|flight| flight.dock == dock)
    }
}

fn remember_mark(state: &State, cx: &rune::Cx<State>, key: &str) {
    if let Some(rect) = cx.layout(key) {
        state.startup.mark_rect.set(Some(rect));
    }
}

fn moving_mark(
    state: &State,
    cx: &rune::Cx<State>,
    dock: Dock,
    preferences: ViewPreferences,
    root: Option<[f32; 4]>,
) -> Option<El> {
    let flight = state.startup.flight?;
    if !state.startup.flying_to(dock, preferences) {
        return None;
    }
    let to = cx.layout(dock.key()).unwrap_or(flight.from);
    let progress = flight.progress(state.clock);
    let rect = flight.rect(to, progress);
    state.startup.mark_rect.set(Some(rect));
    let [x, y, width, height] = rect;
    let [root_x, root_y, _, _] = root.unwrap_or_default();
    let blend = if dock == Dock::Rail {
        ((progress - 0.55) / 0.45).clamp(0.0, 1.0)
    } else {
        0.0
    };
    Some(
        orbit::handoff(state.startup.docked_pose, blend, width, height)
            .key("startup-orbit-flight")
            .absolute(x - root_x, y - root_y)
            .instant_changes()
            .inert()
            .a11y_hidden(),
    )
}

/// Keep the rail mark's layout slot while the pointer-transparent orbit moves toward it.
pub(crate) fn rail_mark(state: &State, height: f32, preferences: ViewPreferences) -> El {
    orbit::mark(height).opacity(if state.startup.flying_to(Dock::Rail, preferences) {
        0.0
    } else {
        1.0
    })
}

pub(crate) fn rail_handoff(
    state: &State,
    cx: &rune::Cx<State>,
    preferences: ViewPreferences,
) -> Option<El> {
    state.startup.reduced_motion.set(preferences.reduced_motion);
    moving_mark(state, cx, Dock::Rail, preferences, None)
}

/// The chooser and both setup paths share the same canvas.
pub(crate) fn view(
    state: &State,
    cx: &rune::Cx<State>,
    preferences: ViewPreferences,
    kit: &Kit,
) -> El {
    state.startup.reduced_motion.set(preferences.reduced_motion);
    let mut root = rune::col()
        .key("startup")
        .w_full()
        .h_full()
        .center()
        .fill(theme::hex(theme::BACKGROUND));
    if state.setup_gateway {
        remember_mark(state, cx, SETUP_MARK);
        let setup_mark_height = 47.0;
        let setup_mark = orbit::view(
            state.startup.docked_pose,
            !preferences.reduced_motion,
            setup_mark_height * orbit::ENVELOPE[0] / orbit::ENVELOPE[1],
            setup_mark_height,
        )
        .key(SETUP_MARK)
        .opacity(if state.startup.flying_to(Dock::Setup, preferences) {
            0.0
        } else {
            1.0
        })
        .inert();
        let back = ActionButton::new("Back")
            .key("startup-back")
            .disabled(state.connected())
            .on_press_tx(State::return_to_startup_choice)
            .build(kit, preferences, None)
            .on_key(|state: &mut State, event, tx| {
                activate_on_return(state, event, tx, State::return_to_startup_choice)
            });
        let mut header = rune::row()
            .key("startup-setup-header")
            .w_full()
            .px(20.0)
            .gap(12.0)
            .center_y()
            .child(back)
            .child(rune::spacer());
        if state.connected() && state.registration.context.is_some() {
            header = header.child(
                ActionButton::new("Done")
                    .key("gateway-setup-done")
                    .icon(&icons::SHIELD_CHECK)
                    .finish(crate::material::MetalFinish::GoldCta)
                    .on_press_tx(State::finish_gateway_setup)
                    .build(kit, preferences, None)
                    .on_key(|state: &mut State, event, tx| {
                        activate_on_return(state, event, tx, State::finish_gateway_setup)
                    }),
            );
        }
        let setup = rune::col()
            .key("startup-setup")
            .w_full()
            .max_w(820.0)
            .h_full()
            .pt(20.0)
            .child(header.child(setup_mark))
            .child(
                views::setup_gateway(state, cx, kit).transition(app::surface_transition(
                    preferences.reduced_motion,
                    Transition::opacity().combined(Transition::offset(0.0, 6.0)),
                )),
            );
        root = root.child(setup);
        if let Some(mark) = moving_mark(state, cx, Dock::Setup, preferences, cx.layout("startup")) {
            root = root.child(mark);
        }
        return root;
    }
    remember_mark(state, cx, "startup-orbit");
    root.child(choose(state, preferences, kit, cx.layout_size("startup")))
}

fn choose(state: &State, preferences: ViewPreferences, kit: &Kit, window: Option<[f32; 2]>) -> El {
    let [width, height] = window.unwrap_or([640.0, 480.0]);
    // Leave room for the controls before containing the complete orbit in the remaining space.
    let [orbit_width, orbit_height] = orbit::display_size(Some([width, height - 210.0]));
    let scrim = theme::hex(theme::CAST_SHADOW);
    rune::col()
        .key("startup-choose")
        .w_full()
        .items_center()
        .gap(8.0)
        .child(
            orbit::view(
                state.startup.seconds(state.clock) as f32,
                !preferences.reduced_motion,
                orbit_width,
                orbit_height,
            )
            .inert(),
        )
        .child(
            rune::col()
                .key("startup-choice-controls")
                .w_full()
                .max_w(400.0)
                .items_center()
                .radial_gradient(
                    [0.5, 0.45],
                    [200.0, 82.0],
                    [(0.0, scrim.alpha(0.72)), (1.0, scrim.alpha(0.0))],
                )
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
        if self.startup.flight.is_some() {
            tx.wake_after(std::time::Duration::from_secs_f64(1.0 / 60.0));
        }
        tx.focus("startup-back", tx.by_keys());
    }

    /// Render the settled backend choice through the production startup view in a fixture.
    pub fn fixture_show_backend_choice(&mut self) {
        if self.fixture {
            self.startup = Startup::cold_launch();
            self.startup.settled = true;
        }
    }

    /// Select an initial backend in an effects-disabled scripted scene.
    pub fn fixture_show_startup_setup(&mut self, kind: BackendKind) {
        if self.fixture {
            self.fixture_show_backend_choice();
            self.choose_startup_destination(kind);
        }
    }

    /// Select a setup path while retaining the explicit Start or Connect action.
    pub(crate) fn choose_startup_destination(&mut self, kind: BackendKind) {
        if !self.startup.active() || self.setup_gateway {
            return;
        }
        // An outstanding profile read can populate the catalog but must respect the explicit choice.
        self.connection_profiles.initial_selection_applied = true;
        self.startup.docked_pose = self.startup.seconds(self.clock) as f32;
        self.startup.start_flight(Dock::Setup, self.clock);
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
        self.startup.flight = None;
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
            self.startup.start_flight(Dock::Rail, self.clock);
            if self.startup.flight.is_some() {
                tx.wake_after(std::time::Duration::from_secs_f64(1.0 / 60.0));
            }
            self.startup.finish();
            self.page = app::Page::Workstation;
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
    use fleetworks_contracts::connect::RegistrationReceipt;
    use fleetworks_contracts::context::RegisteredContext;

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
            let ([orbit_x, orbit_y, orbit_w, orbit_h], _) = runtime
                .presented("startup-orbit")
                .expect("first-frame orbit");
            let ([_, controls_y, _, _], _) = runtime
                .presented("startup-choice-controls")
                .expect("first-frame controls");
            assert!(orbit_x >= 0.0 && orbit_y >= 0.0 && orbit_x + orbit_w <= size.0);
            assert!(
                orbit_y + orbit_h <= controls_y,
                "controls backing must leave the full orbit visible"
            );
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

    fn tick(runtime: &mut rune::Runtime<State>, clock: f64) {
        runtime.update(|state, tx| {
            state.clock = clock;
            state.startup.begin(0.0);
            state.startup.poll_flight(clock, tx);
        });
        runtime.frame(0.02);
    }

    #[test]
    fn selected_pose_docks_and_done_mounts_status_without_waiting_for_the_mark() {
        for (size, reduced) in [
            ((640.0, 480.0), false),
            ((640.0, 480.0), true),
            ((1280.0, 800.0), false),
            ((1280.0, 800.0), true),
        ] {
            let mut runtime = runtime(size, reduced);
            tick(&mut runtime, 1.0);
            let (chooser, _) = runtime.presented("startup-orbit").expect("chooser mark");
            click(&mut runtime, LOCAL);
            runtime.frame(0.02);
            assert_eq!(runtime.state.startup.docked_pose, 1.0);
            assert!(runtime.find("connect-local").is_some());
            tick(&mut runtime, 1.11);
            let (dock, _) = runtime.presented(SETUP_MARK).expect("setup mark slot");
            if reduced {
                assert!(runtime.find("startup-orbit-flight").is_none());
            } else {
                let (in_flight, _) = runtime
                    .presented("startup-orbit-flight")
                    .expect("handoff frame");
                assert!(in_flight[2] > dock[2] && in_flight[2] < chooser[2]);
                assert!((in_flight[2] / in_flight[3] - chooser[2] / chooser[3]).abs() < 0.01);
            }
            tick(&mut runtime, 1.24);
            assert_eq!(runtime.presented(SETUP_MARK).unwrap().1, 1.0);

            runtime.update(|state, _| {
                state.page = app::Page::Brain;
                state.connection.status = ConnectionStatus::Connected;
                let receipt = RegistrationReceipt::new(
                    "synthetic-deployment".into(),
                    "synthetic-node".into(),
                    "synthetic-registration".into(),
                    Some("192.0.2.42".parse().unwrap()),
                    1,
                );
                state.registration.context = Some(
                    RegisteredContext::new(
                        receipt,
                        1,
                        1,
                        "synthetic-fresh-config-v1".into(),
                        "synthetic-access-1".into(),
                    )
                    .unwrap(),
                );
            });
            runtime.frame(0.02);
            click(&mut runtime, "gateway-setup-done");
            runtime.frame(0.02);
            assert_eq!(runtime.state.page, app::Page::Workstation);
            assert!(runtime.find("screen-workstation").is_some());
            assert_eq!(runtime.find("startup-orbit-flight").is_some(), !reduced);
            tick(&mut runtime, 1.48);
            assert!(runtime.find("startup-orbit-flight").is_none());
            assert_eq!(runtime.presented(RAIL_MARK).unwrap().1, 1.0);
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
