//! Cold-launch startup: the canonical logo splash, then an explicit connection choice.
//!
//! The splash runs once per cold launch, never on reconnect. Nothing here starts a companion,
//! opens a connection or falls back from one transport to another. Choosing a destination only
//! selects the existing setup flow; connecting stays an explicit action there.

use std::sync::Arc;

use fleetworks_workstation::native::BackendKind;
use rune::El;
use rune::Image;
use rune::ImageCache;
use rune::SurfaceFit;

use crate::accessibility::ViewPreferences;
use crate::theme;
use crate::State;

/// The packaged 1024x571 canonical logo.
const LOGO_JPG: &[u8] = include_bytes!("../../../assets/brand/fleetworks-logo.jpg");
/// Logo aspect ratio, so layout is stable before the decoder finishes.
const LOGO_WIDTH: f32 = 1024.0;
const LOGO_HEIGHT: f32 = 571.0;

/// Seconds, from the first visible frame.
pub(crate) const FADE_IN: f64 = 0.25;
pub(crate) const HOLD_UNTIL: f64 = 2.75;
pub(crate) const CHOOSE_AT: f64 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum StartupPhase {
    /// The logo is showing at this opacity.
    Splash(f32),
    /// Waiting for an explicit Local Embedded or Remote Gateway choice.
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
    logo: Option<Image>,
    /// Keeps the decoder worker alive while the logo is pending.
    cache: Option<Arc<ImageCache>>,
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
            // Before the first poll the mark is not yet visible.
            return StartupPhase::Splash(if reduced_motion { 1.0 } else { 0.0 });
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
        } else if (FADE_IN..HOLD_UNTIL).contains(&elapsed) {
            HOLD_UNTIL - elapsed
        } else {
            // Fading: ask for a frame at about 60 Hz.
            frame
        };
        Some(wait.max(frame))
    }

    pub(crate) fn logo(&self) -> Option<&Image> {
        self.logo.as_ref()
    }

    /// Decode the packaged logo once. The cache decodes off the UI thread and wakes the app.
    pub fn install_logo(&mut self, cache: Arc<ImageCache>) {
        if self.required && self.logo.is_none() {
            self.logo = Some(cache.load(LOGO_JPG));
            self.cache = Some(cache);
        }
    }
}

/// Splash opacity over elapsed seconds: 250 ms in, hold, 250 ms out, then choose at 3 s.
/// Reduced motion holds the mark static for the same 3 s.
pub(crate) fn phase_at(elapsed: f64, reduced_motion: bool) -> StartupPhase {
    if elapsed >= CHOOSE_AT {
        return StartupPhase::Choose;
    }
    if reduced_motion {
        return StartupPhase::Splash(1.0);
    }
    let opacity = if elapsed < FADE_IN {
        elapsed / FADE_IN
    } else if elapsed < HOLD_UNTIL {
        1.0
    } else {
        1.0 - (elapsed - HOLD_UNTIL) / (CHOOSE_AT - HOLD_UNTIL)
    };
    StartupPhase::Splash(opacity.clamp(0.0, 1.0) as f32)
}

/// The full-window startup view, shown instead of the shell while startup is active.
pub(crate) fn view(state: &State, preferences: ViewPreferences, kit: &rune_kit::Kit) -> El {
    let phase = state.startup.phase(state.clock, preferences.reduced_motion);
    let body = match phase {
        StartupPhase::Splash(opacity) => splash(state, opacity),
        StartupPhase::Choose | StartupPhase::Done => choose(kit),
    };
    rune::col()
        .key("startup")
        .w_full()
        .h_full()
        .center()
        .fill(theme::hex(theme::BACKGROUND))
        .child(body)
}

fn splash(state: &State, opacity: f32) -> El {
    // The logo carries the brand, so no wordmark is drawn beside it.
    let mut logo = rune::col()
        .key("startup-logo")
        .w(560.0)
        .aspect_ratio(LOGO_WIDTH / LOGO_HEIGHT)
        .opacity(opacity)
        .label("Fleetworks");
    if let Some(image) = state.startup.logo() {
        logo = logo.pixels_fit(image.pixels(), SurfaceFit::Fit);
    }
    logo
}

fn choose(kit: &rune_kit::Kit) -> El {
    rune::col()
        .key("startup-choose")
        .w_full()
        .max_w(520.0)
        .gap(16.0)
        .child(
            rune::text("Choose your connection")
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
            "Local Embedded",
            "Run the Fleetworks backend on this computer. You start it from setup.",
            BackendKind::Local,
        ))
        .child(destination(
            kit,
            "startup-remote",
            "Remote Gateway",
            "Connect to a gateway by URL with operator pairing.",
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
    rune_kit::Button::new(title)
        .variant(rune_kit::ButtonVariant::Outline)
        .on_press(move |state: &mut State| state.choose_startup_destination(kind))
        .build(kit)
        .key(key)
        .label(format!("{title}. {detail}"))
}

impl State {
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
    fn splash_fades_in_holds_fades_out_then_asks_for_a_choice() {
        assert_eq!(phase_at(0.0, false), StartupPhase::Splash(0.0));
        assert_eq!(phase_at(0.125, false), StartupPhase::Splash(0.5));
        assert_eq!(phase_at(0.25, false), StartupPhase::Splash(1.0));
        assert_eq!(phase_at(2.0, false), StartupPhase::Splash(1.0));
        assert_eq!(phase_at(2.75, false), StartupPhase::Splash(1.0));
        assert_eq!(phase_at(2.875, false), StartupPhase::Splash(0.5));
        assert_eq!(phase_at(3.0, false), StartupPhase::Choose);
        assert_eq!(phase_at(60.0, false), StartupPhase::Choose);
    }

    #[test]
    fn reduced_motion_shows_a_static_mark_for_the_same_three_seconds() {
        assert_eq!(phase_at(0.0, true), StartupPhase::Splash(1.0));
        assert_eq!(phase_at(1.5, true), StartupPhase::Splash(1.0));
        assert_eq!(phase_at(2.99, true), StartupPhase::Splash(1.0));
        assert_eq!(phase_at(3.0, true), StartupPhase::Choose);
    }

    #[test]
    fn clock_starts_at_the_first_frame_and_never_restarts() {
        let mut startup = Startup::cold_launch();
        assert_eq!(startup.phase(100.0, false), StartupPhase::Splash(0.0));
        startup.begin(10.0);
        startup.begin(50.0);
        assert_eq!(startup.phase(10.125, false), StartupPhase::Splash(0.5));
        assert_eq!(startup.phase(13.0, false), StartupPhase::Choose);
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
        assert!(startup.next_wake(1.0, false).is_some_and(|wait| wait > 1.0));
        assert_eq!(startup.next_wake(3.0, false), None);
        assert_eq!(startup.next_wake(3.0, true), None);
    }

    #[test]
    fn real_view_shows_splash_then_choice_then_the_shell_without_connecting() {
        let mut state = State::default();
        state.startup = Startup::cold_launch();
        let mut runtime = rune::Runtime::new(
            state,
            crate::app::view,
            (1180.0, 780.0),
            rune::Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        runtime.frame(0.0);
        assert!(runtime.find("startup-logo").is_some());
        assert!(runtime.find("startup-choose").is_none());
        assert!(runtime.find("nav-workstation").is_none());

        runtime.update(|state, _| {
            state.startup.begin(0.0);
            state.clock = 3.5;
        });
        runtime.frame(1.0 / 60.0);
        assert!(runtime.find("startup-logo").is_none());
        assert!(runtime.find("startup-local").is_some());
        assert!(runtime.find("startup-remote").is_some());
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
