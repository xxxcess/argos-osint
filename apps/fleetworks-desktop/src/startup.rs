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
use crate::material;
use crate::orbit;
use crate::startup_data;
use crate::theme;
use crate::ui_components::ActionButton;
use crate::views;
use crate::State;

/// The opening samples three seconds at the reference pace, then holds the final pose.
pub(crate) const INTRO_SECONDS: f64 = 3.0;
const LOCAL: &str = "startup-local";
const REMOTE: &str = "startup-remote";
const SETUP_MARK: &str = "startup-setup-mark";
const RAIL_SLOT: &str = "startup-rail-mark-slot";
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
            Self::Rail => RAIL_SLOT,
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
    pub(crate) data_step: Option<startup_data::Step>,
    pub(crate) data_reset: [bool; 2],
    pub(crate) data_checks: bool,
    pub(crate) csv_inputs: startup_data::CsvInputs,
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

    pub(crate) fn data_setup_active(&self) -> bool {
        self.data_step.is_some() || self.data_checks
    }

    pub(crate) fn finish(&mut self) {
        self.active = false;
        self.data_step = None;
        self.data_reset = [false; 2];
        self.data_checks = false;
        self.csv_inputs = startup_data::CsvInputs::default();
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
            if flight.dock == Dock::Rail {
                // The first layout may change from the wide pane to the compact rail. Land
                // any presence tracks from that change as the floating mark is replaced.
                tx.settle();
            }
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
    let settle = if dock == Dock::Rail {
        ((progress - 0.55) / 0.45).clamp(0.0, 1.0)
    } else {
        0.0
    };
    Some(
        orbit::handoff(state.startup.docked_pose, settle, width, height)
            .key("startup-orbit-flight")
            .absolute(x - root_x, y - root_y)
            .transition(Transition::none())
            .instant_changes()
            .inert()
            .a11y_hidden(),
    )
}

/// Keep the rail mark's layout slot while the pointer-transparent orbit moves toward it.
pub(crate) fn rail_mark(state: &State, height: f32, preferences: ViewPreferences) -> El {
    let slot = rune::el()
        .key(RAIL_SLOT)
        .size(height * orbit::ENVELOPE[0] / orbit::ENVELOPE[1], height)
        .transition(Transition::none());
    if state.startup.flying_to(Dock::Rail, preferences) {
        slot
    } else {
        slot.child(orbit::mark(height).transition(Transition::none()))
    }
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
        .fill(theme::hex(theme::BACKGROUND))
        .transition(Transition::none());
    if preferences.reduced_motion {
        root = root.instant_changes();
    }
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
        .transition(Transition::none())
        .instant_changes()
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
        let data_step = state
            .startup
            .data_step
            .filter(|_| state.connected() && state.registration.context.is_some());
        let data_checks =
            state.startup.data_checks && state.connected() && state.registration.context.is_some();
        let showing_data = data_step.is_some() || data_checks;
        let mut header = rune::row()
            .key("startup-setup-header")
            .transition(Transition::none())
            .instant_changes()
            .w_full()
            .px(20.0)
            .gap(12.0)
            .center_y();
        if !showing_data {
            header = header.child(back);
        }
        header = header.child(rune::spacer());
        if !showing_data && state.connected() && state.registration.context.is_some() {
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
        header = header.child(setup_mark);
        if showing_data {
            header = header.child(rune::spacer());
        }
        let body = if data_checks {
            startup_data::checks_view(state, preferences, kit)
        } else if let Some(step) = data_step {
            startup_data::view(state, step, preferences, kit)
        } else {
            views::setup_gateway(state, cx, kit).transition(app::surface_transition(
                preferences.reduced_motion,
                Transition::opacity().combined(Transition::offset(0.0, 6.0)),
            ))
        };
        let setup = rune::col()
            .key("startup-setup")
            .transition(Transition::none())
            .w_full()
            .max_w(820.0)
            .h_full()
            .pt(20.0)
            .child(header)
            .child(body);
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
        .transition(Transition::none())
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
            .transition(Transition::none())
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
                            preferences,
                            LOCAL,
                            &icons::WORKSTATION,
                            "Embedded",
                            "Run on this PC",
                            BackendKind::Local,
                        ))
                        .child(destination(
                            kit,
                            preferences,
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
    preferences: ViewPreferences,
    key: &'static str,
    glyph: &'static rune::Icon,
    title: &'static str,
    detail: &'static str,
    kind: BackendKind,
) -> El {
    let tile = rune::col()
        .key(key)
        .size(150.0, 150.0)
        .center()
        .gap(7.0)
        .radius(12.0)
        .hover(theme::hex(theme::SURFACE_ELEVATED))
        .press(theme::hex(theme::SURFACE))
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
        );
    material::glass_control(tile, preferences)
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
        // The input-triggered commit would otherwise fade newly mounted descendants, even
        // when the setup container itself has no transition.
        if self.startup.reduced_motion.get() {
            tx.settle();
        }
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
        self.prefill_startup_distributed_url();
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
        if self.startup.reduced_motion.get() {
            tx.settle();
        }
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
        if !self.startup.active() {
            self.setup_gateway = false;
            return;
        }
        if !self.connected() || self.registration.context.is_none() {
            return;
        }
        self.startup.start_flight(Dock::Setup, self.clock);
        if self.startup.flight.is_some() {
            tx.wake_after(std::time::Duration::from_secs_f64(1.0 / 60.0));
        }
        self.startup.data_step = startup_data::first_unfinished(self);
        self.startup.data_checks = self.startup.data_step.is_none();
        self.backend_read(crate::backend::ReadRequest::Profile);
        if !self.readiness_pending() {
            self.refresh_readiness();
        }
        self.drive_startup_dataset();
        tx.instant();
        tx.focus("startup-data-stage", tx.by_keys());
    }

    pub(crate) fn finish_startup(&mut self, tx: &mut Tx) {
        self.startup.start_flight(Dock::Rail, self.clock);
        if self.startup.flight.is_some() {
            tx.wake_after(std::time::Duration::from_secs_f64(1.0 / 60.0));
        }
        self.startup.finish();
        self.setup_gateway = false;
        self.page = app::Page::Workstation;
        // Mount the shell on this commit, so it does not animate over the mark's own flight.
        tx.instant();
        tx.focus("nav-workstation", tx.by_keys());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accessibility::FixtureAccessibility;
    use crate::app::Intent;
    use fleetworks_contracts::connect::RegistrationReceipt;
    use fleetworks_contracts::context::RegisteredContext;
    use fleetworks_contracts::readiness::{
        DatasetKind, DatasetRef, MaintenanceOperation, ReadinessAction, ReadinessActionId,
        ReadinessCheck, ReadinessCheckId, ReadinessState, WorkstationReadiness,
    };
    use fleetworks_contracts::DatasetUpload;

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
        let (x, y) = runtime
            .find(key)
            .unwrap_or_else(|| panic!("expected presented control {key}"));
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
    }

    fn assert_opaque(runtime: &rune::Runtime<State>, key: &str) {
        let (_, opacity) = runtime
            .presented(key)
            .unwrap_or_else(|| panic!("expected presented {key}"));
        assert_eq!(opacity, 1.0, "{key} must be fully visible");
    }

    #[test]
    fn embedded_fixture_preserves_synthetic_companion_discovery() {
        use crate::local_backend::BinaryDiscoveryStatus;

        let mut runtime = runtime((640.0, 480.0), false);
        let synthetic = BinaryDiscoveryStatus::Ready("/fixture/bundle/argos-api-host".into());
        runtime.state.local_backend.binary_status = synthetic.clone();
        runtime.frame(0.0);
        click(&mut runtime, "startup-local");
        runtime.frame(0.2);
        assert_eq!(runtime.state.local_backend.binary_status, synthetic);

        runtime.state.connect_local();
        assert_eq!(runtime.state.local_backend.binary_status, synthetic);
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
            if reduced {
                runtime.frame(0.0);
                assert_opaque(&runtime, LOCAL);
                assert_opaque(&runtime, REMOTE);
                assert_opaque(&runtime, "startup-orbit");
            }
            runtime.frame(0.60);
            assert_eq!(runtime.state.startup.seconds(0.6), INTRO_SECONDS);
            assert!(runtime.find("startup-orbit").is_some());
            assert!(runtime.find(REMOTE).is_some());

            key(&mut runtime, rune::Key::Tab);
            runtime.frame(0.62);
            key(&mut runtime, rune::Key::Text(" ".into()));
            if reduced {
                runtime.frame(0.0);
                assert_opaque(&runtime, "gateway-origin");
                assert_opaque(&runtime, "connect");
            }
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
            if reduced {
                assert_opaque(&runtime, "connect-local");
            }
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
                        receipt.clone(),
                        1,
                        1,
                        "synthetic-fresh-config-v1".into(),
                        "synthetic-access-1".into(),
                    )
                    .unwrap(),
                );
                complete_data_checks(
                    state,
                    receipt,
                    [
                        ReadinessState::Ready,
                        ReadinessState::Ready,
                        ReadinessState::NotApplicable,
                    ],
                );
            });
            runtime.frame(0.02);
            click(&mut runtime, "gateway-setup-done");
            runtime.frame(0.02);
            assert!(runtime.find("startup-data-setup").is_some());
            click(&mut runtime, "startup-data-continue");
            runtime.frame(0.02);
            assert_eq!(runtime.state.page, app::Page::Workstation);
            assert!(runtime.find("screen-workstation").is_some());
            assert_eq!(runtime.find("startup-orbit-flight").is_some(), !reduced);
            tick(&mut runtime, 1.48);
            assert!(runtime.find("startup-orbit-flight").is_none());
            let (rail, opacity) = runtime.presented("brand-mark").expect("landed rail mark");
            assert_eq!(opacity, 1.0);
            assert!(rail[0] >= 0.0 && rail[1] >= 0.0);
        }
    }

    fn complete_data_checks(
        state: &mut State,
        receipt: RegistrationReceipt,
        states: [ReadinessState; 3],
    ) {
        let observed = "2026-11-19T00:00:00Z";
        let mut checks: Vec<_> = [
            (
                ReadinessCheckId::ActorCatalog,
                ReadinessActionId::SeedActors,
            ),
            (
                ReadinessCheckId::ActorSearch,
                ReadinessActionId::RebuildActorIndex,
            ),
            (
                ReadinessCheckId::ClassifierTraining,
                ReadinessActionId::TrainClassifier,
            ),
        ]
        .into_iter()
        .zip(states)
        .map(|((id, action), observation)| {
            let mut check = ReadinessCheck::new(
                id,
                observation,
                "synthetic_state".into(),
                String::new(),
                observed.into(),
            );
            check.fresh_for_ms = 60_000;
            check.actions.push(ReadinessAction::new(
                action,
                fleetworks_contracts::context::Permission::MaintainBackend,
                true,
            ));
            check
        })
        .collect();
        for (id, status, reason, freshness) in [
            (
                ReadinessCheckId::Gateway,
                ReadinessState::Ready,
                "gateway_reachable",
                60_000,
            ),
            (
                ReadinessCheckId::Jobs,
                ReadinessState::NotReady,
                "process_not_ready",
                60_000,
            ),
            (
                ReadinessCheckId::ClassifierInference,
                ReadinessState::Ready,
                "model_verified",
                60_000,
            ),
            (
                ReadinessCheckId::Datastore,
                ReadinessState::Ready,
                "datastore_reachable",
                0,
            ),
        ] {
            let mut check =
                ReadinessCheck::new(id, status, reason.into(), String::new(), observed.into());
            check.fresh_for_ms = freshness;
            checks.push(check);
        }
        let mut readiness =
            WorkstationReadiness::new(receipt.deployment_id.clone(), observed.into(), checks);
        readiness.registration = Some(receipt);
        // The motion fixture sets its context without making a real capabilities request.
        state.readiness = app::Load::Ready(readiness);
        state.readiness_received_at = Some(state.clock);
    }

    fn observe_job(
        state: &mut State,
        operation: MaintenanceOperation,
        wire_operation: &str,
        job_state: &str,
        error_summary: &str,
        result: serde_json::Value,
    ) {
        let id = format!("fixture-{wire_operation}");
        if state.maintenance.operation_job(operation).is_none() {
            state
                .maintenance
                .set_operation_job(operation, id.clone(), false);
        }
        let detail = serde_json::from_value(serde_json::json!({
            "job": {
                "id": id,
                "app": "fixture",
                "title": "Fixture maintenance",
                "operation": wire_operation,
                "state": job_state,
                "phase": "reading data",
                "progress_done": 8,
                "progress_total": 20,
                "error_summary": error_summary,
                "private_details_available": true,
                "activity_actions_available": false,
                "status_only": false
            },
            "stages": [],
            "private_details_available": true,
            "activity_actions_available": false,
            "maintenance_result": result
        }))
        .expect("valid scoped fixture job detail");
        state
            .maintenance
            .update_operation_job_detail(operation, &id, detail);
    }

    #[test]
    fn actor_seed_pressure_uses_only_matching_committed_progress() {
        let kind = DatasetKind::ActorCatalog;
        let (mut runtime, _) = data_setup_runtime([ReadinessState::NotReady; 3]);
        runtime.frame(0.0);
        click(&mut runtime, "gateway-setup-done");
        runtime.frame(0.02);
        let dataset = DatasetRef::new("fixture-actors".into(), kind, "a".repeat(64), 120).unwrap();
        let mut upload = DatasetUpload::new(dataset.clone(), 60);
        upload.row_count = Some(94_987);
        runtime.update(|state, _| {
            state.complete_dataset_upload(
                state.connection.epoch,
                state.maintenance.upload_revision,
                state.origin_revision(),
                kind,
                Ok(upload),
            );
        });
        let checkpoint = serde_json::json!({
            "operation": "actor_seed",
            "dataset_sha256": dataset.sha256,
            "processed": 822,
            "total": 94_987,
            "database_seeded": false
        });
        for (job_state, error, label, message) in [
            (
                "queued",
                "setup_worker_memory_pressure",
                "Retrying",
                "The backend reached its memory limit.",
            ),
            (
                "failed",
                "setup_worker_disk_pressure",
                "Needs attention",
                "Free space in the backend workspace fell below its disk limit.",
            ),
            (
                "failed",
                "setup_worker_pressure_unavailable",
                "Needs attention",
                "The backend could not measure memory use or free workspace space.",
            ),
        ] {
            runtime.update(|state, _| {
                observe_job(
                    state,
                    MaintenanceOperation::SeedActors,
                    "actor_seed",
                    job_state,
                    error,
                    checkpoint.clone(),
                );
            });
            runtime.frame(0.02);
            let tree = runtime.accessibility_json().to_string();
            assert!(tree.contains(label), "{job_state}: {tree}");
            assert!(tree.contains(message), "{error}: {tree}");
            assert!(tree.contains("822 / 94,987 actor rows committed"));
            assert!(!tree.contains("8 / 20 processed"));
            assert!(!tree.contains("Ready to seed"));
            assert!(runtime.find("startup-data-continue").is_none());
            assert!(runtime.find("maintenance-confirm").is_none());
        }

        let mut other_dataset = checkpoint.clone();
        other_dataset["dataset_sha256"] = "b".repeat(64).into();
        let mut seeded = checkpoint;
        seeded["database_seeded"] = true.into();
        for result in [other_dataset, seeded, serde_json::Value::Null] {
            runtime.update(|state, _| {
                observe_job(
                    state,
                    MaintenanceOperation::SeedActors,
                    "actor_seed",
                    "failed",
                    "setup_worker_disk_pressure",
                    result,
                );
            });
            runtime.frame(0.02);
            assert!(runtime.find("startup-seed-committed").is_none());
            assert!(runtime.find("startup-data-continue").is_none());
        }
    }

    fn data_setup_runtime(
        checks: [ReadinessState; 3],
    ) -> (rune::Runtime<State>, RegistrationReceipt) {
        let mut runtime = runtime((640.0, 480.0), true);
        let mut state = crate::tests::registered_fixture();
        state.startup = Startup::cold_launch();
        state.setup_gateway = true;
        state.accessibility.fixture_override = Some(FixtureAccessibility {
            reduced_motion: true,
            ..FixtureAccessibility::default()
        });
        state.backend.profile = app::Load::Ready(crate::backend::ProfileSnapshot {
            configuration_revision: 0,
            configuration_format: 1,
            accounts: vec![],
            tool_credentials: vec![],
            setup_required: false,
            portable_schema_version: 4,
        });
        let receipt = RegistrationReceipt::new(
            "synthetic-deployment".into(),
            "synthetic-node".into(),
            "synthetic-registration".into(),
            Some("192.0.2.42".parse().unwrap()),
            1,
        );
        complete_data_checks(&mut state, receipt.clone(), checks);
        runtime.state = state;
        (runtime, receipt)
    }

    #[test]
    fn startup_github_csv_is_opt_in_and_only_downloads_after_press() {
        let kind = DatasetKind::ActorCatalog;
        let (mut runtime, _) = data_setup_runtime([ReadinessState::NotReady; 3]);
        runtime.frame(0.0);
        click(&mut runtime, "gateway-setup-done");
        runtime.frame(0.02);
        assert!(runtime.find("startup-csv-github-url").is_none());
        click(&mut runtime, "startup-csv-source/segment1");
        runtime.frame(0.01);
        let default = crate::github_dataset::default_url(kind).expect("default CSV URL");
        let tree = runtime.accessibility_json().to_string();
        assert!(tree.contains(default));
        assert!(tree.contains("Local CSV"));
        assert!(tree.contains("Download & validate"));
        assert!(runtime.find("startup-csv-ready").is_none());
        assert!(!runtime
            .state
            .intents
            .contains(&Intent::DownloadGithubDataset { kind }));
        click(&mut runtime, startup_data::PRIMARY);
        runtime.frame(0.04);
        assert!(runtime
            .state
            .intents
            .contains(&Intent::DownloadGithubDataset { kind }));
        assert!(runtime.state.maintenance.confirmation.is_none());
    }

    #[test]
    fn startup_validates_bundled_actor_csv_once_then_waits_for_confirmation() {
        let kind = DatasetKind::ActorCatalog;
        let operation = MaintenanceOperation::SeedActors;
        let ready_label = "Ready to seed";
        let action_label = "Seed actors";
        let (mut runtime, _) = data_setup_runtime([ReadinessState::NotReady; 3]);
        runtime.state.fixture_bundled_csv = true;
        runtime.state.intents.clear();
        let profile = std::mem::replace(&mut runtime.state.backend.profile, app::Load::Idle);
        runtime.frame(0.0);
        click(&mut runtime, "gateway-setup-done");
        runtime.frame(0.02);
        assert!(!runtime
            .state
            .intents
            .contains(&Intent::UploadDataset { kind }));
        runtime.update(|state, _| {
            state.backend.profile = profile;
            assert!(state.drive_startup_dataset());
        });
        runtime.frame(0.02);
        assert!(runtime
            .state
            .intents
            .contains(&Intent::UploadDataset { kind }));
        let tree = runtime.accessibility_json().to_string();
        assert!(tree.contains("Bundled CSV"));
        assert!(tree.contains("Validating"));
        assert!(!tree.contains(action_label));
        let complete = move |state: &mut State, result| {
            state.complete_dataset_upload(
                state.connection.epoch,
                state.maintenance.upload_revision,
                state.origin_revision(),
                kind,
                result,
            );
        };
        runtime.update(|state, _| {
            assert!(!state.drive_startup_dataset());
            complete(state, Err("Could not validate the bundled CSV".into()));
            assert!(!state.drive_startup_dataset());
        });
        runtime.frame(0.02);
        assert!(!runtime
            .accessibility_json()
            .to_string()
            .contains(action_label));
        assert_eq!(
            runtime
                .state
                .intents
                .iter()
                .filter(|intent| matches!(intent, Intent::UploadDataset { .. }))
                .count(),
            1
        );
        runtime.update(|_, tx| tx.scroll_into_view(startup_data::PRIMARY));
        runtime.frame(0.02);
        click(&mut runtime, startup_data::PRIMARY);
        runtime.frame(0.02);
        let dataset = DatasetRef::new("fixture-bundle".into(), kind, "b".repeat(64), 120)
            .expect("validated dataset");
        let mut upload = DatasetUpload::new(dataset.clone(), 60);
        upload.row_count = Some(94_987);
        runtime.update(|state, _| complete(state, Ok(upload)));
        runtime.frame(0.02);
        let tree = runtime.accessibility_json().to_string();
        assert!(tree.contains(ready_label));
        assert!(tree.contains("Bundled CSV · 94,987 rows validated"));
        assert!(runtime.state.maintenance.confirmation.is_none());
        runtime.update(|_, tx| tx.scroll_into_view(startup_data::PRIMARY));
        runtime.frame(0.02);
        click(&mut runtime, startup_data::PRIMARY);
        runtime.frame(0.02);
        assert_eq!(
            runtime
                .state
                .maintenance
                .confirmation
                .as_ref()
                .and_then(|confirmation| confirmation.dataset.as_ref()),
            Some(&dataset)
        );
        assert!(!runtime
            .state
            .intents
            .contains(&Intent::Maintenance { operation }));
    }

    #[test]
    fn startup_github_csv_requires_validation_and_discards_stale_edits_and_uploads() {
        const URL_EDITOR: &str = "startup-csv-github-url/value/input";
        const LOCAL_SOURCE: &str = "startup-csv-source/segment0";

        let kind = DatasetKind::ActorCatalog;
        let (mut runtime, _) = data_setup_runtime([ReadinessState::NotReady; 3]);
        runtime.frame(0.0);
        click(&mut runtime, "gateway-setup-done");
        runtime.frame(0.02);
        click(&mut runtime, "startup-csv-source/segment1");
        runtime.frame(0.01);
        click(&mut runtime, startup_data::PRIMARY);
        runtime.update(|state, _| {
            state.maintenance.github_downloading = true;
            state.maintenance.upload = app::Load::Loading;
        });
        runtime.frame(0.04);
        assert!(runtime
            .accessibility_json()
            .to_string()
            .contains("Downloading"));
        runtime.update(|state, _| state.maintenance.github_downloading = false);
        runtime.frame(0.06);
        assert!(runtime
            .accessibility_json()
            .to_string()
            .contains("Validating"));
        let complete = move |state: &mut State, result| {
            state.complete_dataset_upload(
                state.connection.epoch,
                state.maintenance.upload_revision,
                state.origin_revision(),
                kind,
                result,
            );
        };
        runtime.update(|state, _| {
            complete(state, Err("CSV row 2 is missing a required field".into()));
        });
        runtime.frame(0.08);
        assert!(runtime
            .accessibility_json()
            .to_string()
            .contains("CSV row 2"));
        assert!(runtime.find("startup-csv-ready").is_none());
        assert!(runtime.find("maintenance-confirm").is_none());

        runtime.state.intents.clear();
        runtime.update(|_, tx| tx.scroll_into_view(startup_data::PRIMARY));
        runtime.frame(0.09);
        click(&mut runtime, startup_data::PRIMARY);
        runtime.frame(0.01);
        assert!(runtime
            .state
            .intents
            .contains(&Intent::DownloadGithubDataset { kind }));
        let dataset = DatasetRef::new("fixture-actors".into(), kind, "a".repeat(64), 120)
            .expect("validated dataset");
        let mut receipt = DatasetUpload::new(dataset.clone(), 60);
        receipt.row_count = Some(12);
        runtime.update(|state, _| complete(state, Ok(receipt.clone())));
        runtime.frame(0.10);
        assert!(runtime.find("maintenance-confirm").is_none());
        assert!(runtime.find("startup-csv-ready").is_some());
        runtime.update(|_, tx| tx.scroll_into_view("startup-csv-ready"));
        runtime.frame(0.01);
        let tree = runtime.accessibility_json().to_string();
        assert!(tree.contains("Ready to seed"));
        assert!(tree.contains("GitHub CSV URL · 12 rows validated"));

        runtime.update(|_, tx| tx.scroll_into_view(startup_data::PRIMARY));
        runtime.frame(0.12);
        click(&mut runtime, startup_data::PRIMARY);
        runtime.frame(0.14);
        assert_eq!(
            runtime
                .state
                .maintenance
                .confirmation
                .as_ref()
                .and_then(|confirmation| confirmation.dataset.as_ref()),
            Some(&dataset)
        );
        assert!(!runtime.state.intents.contains(&Intent::Maintenance {
            operation: MaintenanceOperation::SeedActors,
        }));

        runtime.update(|state, tx| state.dismiss_maintenance_confirmation(tx));
        runtime.frame(0.16);
        runtime.update(|_, tx| tx.scroll_into_view(URL_EDITOR));
        runtime.frame(0.17);
        let editor = runtime
            .elements()
            .into_iter()
            .find(|element| element.path.ends_with(URL_EDITOR) && !element.exiting)
            .expect("compact GitHub URL editor");
        let [x, y, width, height] = editor.rect;
        assert_eq!(editor.visible, editor.rect, "the editor must not clip");
        assert!(
            x >= 0.0 && y >= 0.0 && x + width <= 640.5 && y + height <= 480.5,
            "the editor must fit the compact window"
        );
        click(&mut runtime, URL_EDITOR);
        runtime.frame(0.175);
        assert!(runtime.elements().iter().any(|element| {
            element.path.ends_with(URL_EDITOR) && !element.exiting && element.focused
        }));
        let revision = runtime.state.maintenance.upload_revision;
        let select_all = rune::KeyEvent {
            key: rune::Key::Command('a'),
            mods: rune::Mods {
                cmd: true,
                ..rune::Mods::default()
            },
        };
        runtime.key(select_all.clone());
        runtime.key_up(select_all);
        let custom = "https://raw.githubusercontent.com/example/fixture/main/small.csv";
        key(&mut runtime, rune::Key::Text(custom.into()));
        runtime.frame(0.18);
        assert!(runtime.accessibility_json().to_string().contains(custom));
        assert!(runtime.state.maintenance.selected_dataset.is_none());
        assert!(runtime.find("startup-csv-ready").is_none());
        runtime.update(|state, _| {
            state.complete_dataset_upload(
                state.connection.epoch,
                revision,
                state.origin_revision(),
                kind,
                Ok(receipt),
            );
        });
        runtime.frame(0.20);
        assert!(runtime.state.maintenance.selected_dataset.is_none());
        runtime.update(|_, tx| tx.scroll_into_view(LOCAL_SOURCE));
        runtime.frame(0.21);
        click(&mut runtime, LOCAL_SOURCE);
        runtime.frame(0.22);
        assert!(runtime.find("startup-csv-github-url").is_none());
        assert!(runtime.state.maintenance.confirmation.is_none());
    }

    #[test]
    fn data_setup_waits_for_scoped_work_and_only_advances_from_verified_results() {
        use ReadinessState::{NotReady, Ready};
        let (mut runtime, receipt) = data_setup_runtime([NotReady, NotReady, Ready]);
        runtime.frame(0.0);
        click(&mut runtime, "gateway-setup-done");
        runtime.frame(0.02);
        assert_eq!(
            runtime.state.startup.data_step,
            Some(startup_data::Step::Seed)
        );
        assert!(runtime.find("screen-workstation").is_none());
        let (mark, opacity) = runtime.presented(SETUP_MARK).unwrap();
        assert_eq!(opacity, 1.0);
        let (panel, _) = runtime.presented("startup-data-panel").unwrap();
        let (dots, _) = runtime.presented("startup-data-dots").unwrap();
        assert!(panel[1] + panel[3] <= dots[1]);
        let center = |rect: [f32; 4]| rect[0] + rect[2] / 2.0;
        assert!((center(mark) - center(panel)).abs() < 0.5);
        assert!((center(dots) - center(panel)).abs() < 0.5);
        assert!(dots[0] >= 0.0 && dots[0] + dots[2] <= 640.0);

        click(&mut runtime, "startup-csv-source/segment0");
        runtime.frame(0.03);
        click(&mut runtime, startup_data::PRIMARY);
        runtime.frame(0.04);
        if cfg!(target_os = "linux") {
            runtime.update(|state, _| {
                let prompt = state
                    .maintenance
                    .csv_path_prompt
                    .as_mut()
                    .expect("CSV prompt");
                assert_eq!(prompt.kind, DatasetKind::ActorCatalog);
                prompt.input.text = "/fixture/actors.csv".into();
            });
            runtime.frame(0.045);
            click(&mut runtime, "maintenance-csv-path-submit");
            runtime.frame(0.05);
            assert!(runtime.state.intents.contains(&Intent::UploadDataset {
                kind: DatasetKind::ActorCatalog,
            }));
        } else {
            assert!(runtime.state.intents.contains(&Intent::ChooseDataset {
                kind: DatasetKind::ActorCatalog,
            }));
        }
        runtime.update(|state, _| {
            state.maintenance.set_operation_job(
                MaintenanceOperation::SeedActors,
                "fixture-actor_seed".into(),
                false,
            );
        });
        runtime.frame(0.06);
        assert!(runtime.find("startup-data-continue").is_none());
        assert!(runtime.find(startup_data::PRIMARY).is_none());

        runtime.update(|state, _| {
            observe_job(
                state,
                MaintenanceOperation::SeedActors,
                "actor_seed",
                "retry_scheduled",
                "",
                serde_json::Value::Null,
            );
        });
        runtime.frame(0.08);
        let tree = runtime.accessibility_json().to_string();
        assert!(tree.contains("Retrying"));
        assert!(tree.contains("8 / 20 processed"));
        assert!(runtime.find("startup-data-continue").is_none());
        runtime.update(|state, _| {
            observe_job(
                state,
                MaintenanceOperation::SeedActors,
                "actor_seed",
                "completed",
                "",
                serde_json::Value::Null,
            );
        });
        runtime.frame(0.1);
        assert!(runtime
            .accessibility_json()
            .to_string()
            .contains("Verifying"));
        assert!(runtime.find("startup-data-continue").is_none());

        runtime.update(|state, _| {
            complete_data_checks(state, receipt.clone(), [Ready, NotReady, Ready]);
        });
        runtime.frame(0.12);
        click(&mut runtime, "startup-data-continue");
        runtime.frame(0.14);
        assert_eq!(
            runtime.state.startup.data_step,
            Some(startup_data::Step::Index)
        );
        click(&mut runtime, startup_data::PRIMARY);
        runtime.frame(0.16);
        assert!(runtime.find("maintenance-confirm").is_some());
        assert!(!runtime.state.confirm_maintenance_dialog);
        click(&mut runtime, "maintenance-confirm");
        runtime.frame(0.18);
        assert!(runtime.state.intents.contains(&Intent::Maintenance {
            operation: MaintenanceOperation::ReindexActors,
        }));
        assert!(runtime.find("startup-data-continue").is_none());

        runtime.update(|state, _| complete_data_checks(state, receipt, [Ready, Ready, Ready]));
        runtime.frame(0.2);
        click(&mut runtime, "startup-data-continue");
        runtime.frame(0.22);
        assert!(runtime.state.startup.data_step.is_none());
        assert!(runtime.find("startup-data-step-classifier").is_none());
        assert!(runtime
            .find("startup-data-check-classifier_training")
            .is_none());
        assert!(!runtime.state.intents.contains(&Intent::UploadDataset {
            kind: DatasetKind::ClassifierTraining,
        }));
        assert!(!runtime.state.intents.contains(&Intent::Maintenance {
            operation: MaintenanceOperation::TrainClassifier,
        }));
        assert!(runtime.state.startup.data_checks);
        assert!(runtime.find("screen-workstation").is_none());
        assert!(runtime.find("startup-data-checklist").is_some());
        click(&mut runtime, "startup-data-continue");
        runtime.frame(0.28);
        assert!(runtime.find("screen-workstation").is_some());
        assert!(runtime.find("startup-data-setup").is_none());
        assert!(runtime.find("nav-seed").is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn canceling_typed_startup_csv_restores_focus_without_an_error_or_a_job() {
        use ReadinessState::NotReady;

        for (dismiss_with_escape, dismissal) in [(false, "Cancel"), (true, "Escape")] {
            let (mut runtime, _) = data_setup_runtime([NotReady; 3]);
            runtime.on_key = app::spec().on_key;
            runtime.frame(0.0);
            click(&mut runtime, "gateway-setup-done");
            runtime.frame(0.02);
            runtime.state.intents.clear();

            click(&mut runtime, "startup-csv-source/segment0");
            runtime.frame(0.03);
            click(&mut runtime, startup_data::PRIMARY);
            runtime.frame(0.04);
            let prompt = runtime
                .state
                .maintenance
                .csv_path_prompt
                .as_ref()
                .expect("opening typed entry shows the CSV prompt");
            assert_eq!(prompt.kind, DatasetKind::ActorCatalog);
            assert!(prompt.input.focused);
            assert!(prompt.error.is_none());
            assert!(
                runtime.state.intents.is_empty(),
                "opening typed entry must not browse"
            );

            if dismiss_with_escape {
                key(&mut runtime, rune::Key::Escape);
            } else {
                click(&mut runtime, "maintenance-csv-path-cancel");
            }
            runtime.frame(0.06);

            let maintenance = &runtime.state.maintenance;
            assert!(
                maintenance.csv_path_prompt.is_none(),
                "{dismissal} must dismiss the CSV prompt"
            );
            assert!(maintenance.message.is_none());
            assert!(maintenance.local_dataset.is_none());
            assert!(maintenance.confirmation.is_none());
            assert!(matches!(maintenance.receipt, app::Load::Idle));
            assert!(
                runtime.state.intents.is_empty(),
                "{dismissal} must not submit work"
            );
            assert!(
                runtime.elements().iter().any(|element| {
                    element.key.as_deref() == Some(startup_data::PRIMARY) && element.focused
                }),
                "{dismissal} must return focus to the CSV entry"
            );
        }
    }

    #[test]
    fn ready_steps_can_reset_their_own_drafts_without_changing_backend_data() {
        use ReadinessState::{NotApplicable, Ready};
        let (mut runtime, _) = data_setup_runtime([Ready, Ready, NotApplicable]);
        runtime.frame(0.0);
        click(&mut runtime, "gateway-setup-done");
        runtime.frame(0.02);
        assert!(runtime.state.startup.data_checks);
        assert!(runtime.find("startup-data-step-checks").is_some());
        for (key, label) in [
            ("startup-data-check-gateway", "Gateway: Passed"),
            ("startup-data-check-jobs", "Jobs: Failed"),
            (
                "startup-data-check-classifier_inference",
                "Atlas BERT inference: Passed",
            ),
            ("startup-data-check-datastore", "Datastore: Pending"),
        ] {
            let row = runtime
                .elements()
                .into_iter()
                .find(|element| element.key.as_deref() == Some(key))
                .unwrap_or_else(|| panic!("expected checklist row {key}"));
            assert_eq!(row.label.as_deref(), Some(label));
        }
        for key in [
            "startup-data-review-seed",
            "startup-data-review-index",
            "startup-data-reset-seed",
            "startup-data-reset-index",
            "startup-data-checklist",
            "startup-data-continue",
            "startup-data-step-seed",
            "startup-data-step-index",
            "startup-data-step-checks",
        ] {
            let element = runtime
                .elements()
                .into_iter()
                .find(|element| element.key.as_deref() == Some(key) && !element.exiting)
                .unwrap_or_else(|| panic!("missing registered setup control {key}"));
            let [x, y, width, height] = element.rect;
            assert_eq!(element.scrolled, [0.0, 0.0], "{key} required scrolling");
            assert_eq!(element.visible, element.rect, "{key} is clipped");
            assert!(
                x >= 0.0 && y >= 0.0 && x + width <= 640.5 && y + height <= 472.5,
                "{key} must stay inside the compact window"
            );
        }

        runtime.state.maintenance.selected_dataset = Some(
            fleetworks_contracts::readiness::DatasetRef::new(
                "saved-training".into(),
                DatasetKind::ClassifierTraining,
                "a".repeat(64),
                100,
            )
            .unwrap(),
        );
        runtime.state.intents.clear();
        runtime.update(|_, tx| tx.scroll_into_view("startup-data-reset-seed"));
        runtime.frame(0.06);
        click(&mut runtime, "startup-data-reset-seed");
        runtime.frame(0.08);
        assert_eq!(
            runtime.state.startup.data_step,
            Some(startup_data::Step::Seed)
        );
        assert!(!runtime.state.startup.data_checks);
        assert!(runtime.find(startup_data::PRIMARY).is_some());
        assert!(runtime.state.intents.is_empty());
        assert!(runtime
            .state
            .maintenance
            .selected_dataset
            .as_ref()
            .is_some_and(|dataset| dataset.kind == DatasetKind::ClassifierTraining));
        let Some(readiness) = startup_data::scoped_readiness(&runtime.state) else {
            panic!("reset must preserve the authoritative readiness observation");
        };
        assert!(readiness
            .checks
            .iter()
            .any(|check| { check.id == ReadinessCheckId::ActorCatalog && check.state == Ready }));

        runtime.update(|_, tx| tx.scroll_into_view("startup-data-skip"));
        runtime.frame(0.09);
        click(&mut runtime, "startup-data-skip");
        runtime.frame(0.1);
        assert!(runtime.state.startup.data_checks);
        assert!(runtime.find("startup-data-checklist").is_some());
        assert!(runtime.find("screen-workstation").is_none());
        click(&mut runtime, "startup-data-continue");
        runtime.frame(0.12);
        assert!(runtime.find("screen-workstation").is_some());
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
