//! Cold-launch startup: the Quiet Orbit splash, then an explicit connection choice.
//!
//! The splash runs once per cold launch, never on reconnect. Nothing here starts a companion,
//! opens a connection or falls back from one transport to another. Choosing a destination only
//! selects the existing setup flow; connecting stays an explicit action there.
//!
//! Timeline, from the first visible frame: the complete scene plays at its reference pace until
//! [`CROSSFADE_AT`], crossfades into the connection choice until [`CHOOSE_AT`], then hands over.
//! The scene is vector geometry drawn over the shared `#060709` canvas, so no asset can fail to
//! load and no media rectangle can flash. Reduced motion shows one complete still for the same
//! three seconds, then the choice.

use fleetworks_workstation::native::BackendKind;
use rune::El;

use crate::accessibility::ViewPreferences;
use crate::orbit;
use crate::theme;
use crate::State;

/// Seconds, from the first visible frame.
pub(crate) const CROSSFADE_AT: f64 = 2.65;
pub(crate) const CHOOSE_AT: f64 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum StartupPhase {
    /// The scene is playing, this many seconds in.
    Splash(f32),
    /// The scene fades out as the choice fades in; `progress` runs 0 to 1.
    Crossfade { seconds: f32, progress: f32 },
    /// Waiting for an explicit Embedded or Distributed backend choice.
    Choose,
    /// The normal shell.
    Done,
}

/// Cold-launch startup state. `Default` is finished, so fixtures and tests are unaffected.
#[derive(Default)]
pub struct Startup {
    required: bool,
    started_at: Option<f64>,
    chosen: Option<BackendKind>,
}

impl Startup {
    /// The state for one cold launch of the production executable.
    pub fn cold_launch() -> Self {
        Self {
            required: true,
            ..Self::default()
        }
    }

    /// Whether startup still owns the window.
    pub fn active(&self) -> bool {
        self.required && self.chosen.is_none()
    }

    /// Start the splash clock at the first visible frame. Later calls do nothing.
    pub(crate) fn begin(&mut self, now: f64) {
        if self.required && self.started_at.is_none() {
            self.started_at = Some(now);
        }
    }

    /// Record the explicit choice and finish startup for this launch.
    pub(crate) fn choose(&mut self, kind: BackendKind) {
        if self.required {
            self.chosen = Some(kind);
        }
    }

    pub(crate) fn phase(&self, now: f64, reduced_motion: bool) -> StartupPhase {
        if !self.active() {
            return StartupPhase::Done;
        }
        let Some(started) = self.started_at else {
            // Before the first poll the clock has not started; show the first frame.
            return StartupPhase::Splash(0.0);
        };
        phase_at((now - started).max(0.0), reduced_motion)
    }

    /// Seconds until the phase next changes enough to need a new frame, if any.
    pub(crate) fn next_wake(&self, now: f64, reduced_motion: bool) -> Option<f64> {
        let started = self.started_at?;
        if !self.active() {
            return None;
        }
        let elapsed = (now - started).max(0.0);
        if elapsed >= CHOOSE_AT {
            return None;
        }
        let frame = 1.0 / 60.0;
        let wait = if reduced_motion {
            CHOOSE_AT - elapsed
        } else {
            frame
        };
        Some(wait.max(frame))
    }
}

/// The phase `elapsed` seconds after the first frame. Reduced motion holds one still for the
/// whole three seconds and then hands over without a crossfade.
pub(crate) fn phase_at(elapsed: f64, reduced_motion: bool) -> StartupPhase {
    if elapsed >= CHOOSE_AT {
        return StartupPhase::Choose;
    }
    if reduced_motion || elapsed < CROSSFADE_AT {
        return StartupPhase::Splash(elapsed as f32);
    }
    StartupPhase::Crossfade {
        seconds: elapsed as f32,
        progress: ((elapsed - CROSSFADE_AT) / (CHOOSE_AT - CROSSFADE_AT)).clamp(0.0, 1.0) as f32,
    }
}

/// The full-window startup view, shown instead of the shell while startup is active.
/// `window` is the previous frame's size of this view, `None` on the first frame.
pub(crate) fn view(
    state: &State,
    preferences: ViewPreferences,
    kit: &rune_kit::Kit,
    window: Option<[f32; 2]>,
) -> El {
    let animated = !preferences.reduced_motion;
    let root = rune::col()
        .key("startup")
        .w_full()
        .h_full()
        .center()
        .fill(theme::hex(theme::BACKGROUND));
    match state.startup.phase(state.clock, preferences.reduced_motion) {
        StartupPhase::Splash(seconds) => root.child(orbit::view(seconds, animated, window)),
        StartupPhase::Crossfade { seconds, progress } => root
            // The choice appears under the fading scene and takes no input until the hand-over.
            .child(choose(kit).opacity(progress).inert())
            .child(
                rune::col()
                    .key("startup-fade")
                    .absolute(0.0, 0.0)
                    .w_full()
                    .h_full()
                    .center()
                    .opacity(1.0 - progress)
                    .child(orbit::view(seconds, animated, window)),
            ),
        StartupPhase::Choose | StartupPhase::Done => root.child(choose(kit)),
    }
}

fn choose(kit: &rune_kit::Kit) -> El {
    rune::col()
        .key("startup-choose")
        .w_full()
        .max_w(520.0)
        .gap(16.0)
        .child(
            rune::text("Choose a backend")
                .font(kit.ty.body_medium)
                .color(theme::hex(theme::TEXT_PRIMARY)),
        )
        .child(
            rune::text("Nothing starts or connects until you choose and act.")
                .font(kit.ty.small)
                .color(theme::hex(theme::TEXT_MUTED)),
        )
        .child(destination(
            kit,
            "startup-local",
            "Embedded",
            "Run the backend on this workstation.",
            BackendKind::Local,
        ))
        .child(destination(
            kit,
            "startup-remote",
            "Distributed",
            "Pair with a deployment by gateway URL.",
            BackendKind::Remote,
        ))
}

fn destination(
    kit: &rune_kit::Kit,
    key: &'static str,
    title: &'static str,
    detail: &'static str,
    kind: BackendKind,
) -> El {
    rune::col()
        .gap(4.0)
        .child(
            rune_kit::Button::new(title)
                .variant(rune_kit::ButtonVariant::Outline)
                .on_press(move |state: &mut State| state.choose_startup_destination(kind))
                .build(kit)
                .key(key)
                .label(format!("{title}. {detail}")),
        )
        .child(
            rune::text(detail)
                .font(kit.ty.small)
                .color(theme::hex(theme::TEXT_MUTED))
                .a11y_hidden(),
        )
}

impl State {
    /// Render the settled backend choice through the production startup view in a fixture.
    pub fn fixture_show_backend_choice(&mut self) {
        if self.fixture {
            self.startup = Startup::cold_launch();
            self.startup.begin(self.clock - CHOOSE_AT);
        }
    }

    /// Select the existing setup flow for a destination. This never starts or connects.
    pub(crate) fn choose_startup_destination(&mut self, kind: BackendKind) {
        if !self.startup.active() {
            return;
        }
        self.startup.choose(kind);
        self.set_backend_kind(kind);
        self.setup_gateway = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_finished_so_fixtures_skip_startup() {
        let startup = Startup::default();
        assert!(!startup.active());
        assert_eq!(startup.phase(10.0, false), StartupPhase::Done);
    }

    #[test]
    fn the_scene_is_complete_from_the_first_frame_then_crossfades_then_asks_for_a_choice() {
        assert_eq!(phase_at(0.0, false), StartupPhase::Splash(0.0));
        assert_eq!(phase_at(1.5, false), StartupPhase::Splash(1.5));
        assert_eq!(phase_at(2.6, false), StartupPhase::Splash(2.6));
        assert_eq!(
            phase_at(CROSSFADE_AT, false),
            StartupPhase::Crossfade {
                seconds: CROSSFADE_AT as f32,
                progress: 0.0
            }
        );
        match phase_at(2.825, false) {
            StartupPhase::Crossfade { progress, .. } => assert!((progress - 0.5).abs() < 1e-3),
            other => panic!("expected a crossfade, got {other:?}"),
        }
        assert_eq!(phase_at(3.0, false), StartupPhase::Choose);
        assert_eq!(phase_at(60.0, false), StartupPhase::Choose);
    }

    #[test]
    fn reduced_motion_holds_a_still_for_the_same_three_seconds_without_a_crossfade() {
        assert_eq!(phase_at(0.0, true), StartupPhase::Splash(0.0));
        assert_eq!(phase_at(2.8, true), StartupPhase::Splash(2.8));
        assert_eq!(phase_at(2.99, true), StartupPhase::Splash(2.99));
        assert_eq!(phase_at(3.0, true), StartupPhase::Choose);
    }

    #[test]
    fn clock_starts_at_the_first_frame_and_never_restarts() {
        let mut startup = Startup::cold_launch();
        assert_eq!(startup.phase(100.0, false), StartupPhase::Splash(0.0));
        startup.begin(10.0);
        startup.begin(50.0);
        assert_eq!(startup.phase(10.5, false), StartupPhase::Splash(0.5));
        assert_eq!(startup.phase(13.0, false), StartupPhase::Choose);
    }

    #[test]
    fn a_late_wake_goes_straight_to_the_choice_and_never_replays() {
        let mut startup = Startup::cold_launch();
        startup.begin(0.0);
        assert_eq!(startup.phase(30.0, false), StartupPhase::Choose);
        assert_eq!(startup.phase(30.0, true), StartupPhase::Choose);
    }

    #[test]
    fn choosing_finishes_startup_and_cannot_be_repeated() {
        let mut startup = Startup::cold_launch();
        startup.begin(0.0);
        assert!(startup.active());
        startup.choose(BackendKind::Local);
        assert!(!startup.active());
        assert_eq!(startup.phase(1.0, false), StartupPhase::Done);
        assert_eq!(startup.next_wake(1.0, false), None);
    }

    #[test]
    fn wakes_stop_once_the_choice_is_showing() {
        let mut startup = Startup::cold_launch();
        assert_eq!(startup.next_wake(0.0, false), None);
        startup.begin(0.0);
        assert!(startup.next_wake(0.1, false).is_some());
        assert!(startup.next_wake(1.0, true).is_some_and(|wait| wait > 1.0));
        assert_eq!(startup.next_wake(3.0, false), None);
        assert_eq!(startup.next_wake(3.0, true), None);
    }

    fn runtime(size: (f32, f32), scale: f32) -> rune::Runtime<State> {
        let mut state = State::default();
        state.startup = Startup::cold_launch();
        rune::Runtime::new(
            state,
            crate::app::view,
            size,
            rune::Env {
                system_dark: true,
                scale,
            },
        )
    }

    #[test]
    fn real_view_shows_the_orbit_then_the_choice_then_the_shell_without_connecting() {
        let mut runtime = runtime((1180.0, 780.0), 1.0);
        runtime.frame(0.0);
        assert!(runtime.find("startup-orbit").is_some());
        assert!(runtime.find("startup-choose").is_none());
        assert!(runtime.find("nav-workstation").is_none());

        runtime.update(|state, _| {
            state.startup.begin(0.0);
            state.clock = 3.5;
        });
        runtime.frame(1.0 / 60.0);
        assert!(runtime.find("startup-orbit").is_none());
        assert!(runtime.find("startup-local").is_some());
        assert!(runtime.find("startup-remote").is_some());
        let elements = runtime.elements();
        for (key, name) in [
            ("startup-local", "Embedded"),
            ("startup-remote", "Distributed"),
        ] {
            let choice = elements
                .iter()
                .find(|element| element.key.as_deref() == Some(key))
                .unwrap();
            assert!(choice
                .label
                .as_deref()
                .is_some_and(|label| label.starts_with(name)));
        }
        assert!(runtime.find("nav-workstation").is_none());

        let (x, y) = runtime.find("startup-remote").expect("remote choice");
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(2.0 / 60.0);
        assert!(!runtime.state.startup.active());
        assert_eq!(runtime.state.backend_kind, BackendKind::Remote);
        assert!(runtime.state.setup_gateway);
        assert!(!runtime.state.connected());
        assert!(runtime.find("nav-setup").is_some());
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

    #[test]
    fn the_crossfade_shows_both_layers_and_the_orbit_fits_a_compact_window() {
        let mut runtime = runtime((640.0, 480.0), 2.0);
        runtime.update(|state, _| {
            state.startup.begin(0.0);
            state.clock = 2.8;
        });
        runtime.frame(0.0);
        assert!(runtime.find("startup-orbit").is_some());
        assert!(runtime.find("startup-local").is_some());
        let (rect, _) = runtime.presented("startup-orbit").expect("orbit drawn");
        assert!(rect[2] <= 640.0 && rect[3] <= 480.0, "{rect:?}");
        assert!(rect[2] > 0.0 && rect[3] > 0.0);
    }

    #[test]
    fn choosing_a_destination_selects_setup_without_connecting() {
        let mut state = State::default();
        state.startup = Startup::cold_launch();
        state.choose_startup_destination(BackendKind::Local);
        assert!(!state.startup.active());
        assert_eq!(state.backend_kind, BackendKind::Local);
        assert!(state.setup_gateway);
        assert!(!state.connected());
        assert!(state.client.is_none());
        // A second choice after startup is ignored.
        state.choose_startup_destination(BackendKind::Remote);
        assert_eq!(state.backend_kind, BackendKind::Local);
    }
}
