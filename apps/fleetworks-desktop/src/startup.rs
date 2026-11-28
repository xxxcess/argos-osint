//! Cold-launch Embedded folder selection and the Quiet Orbit handoff into the workstation.

use std::cell::Cell;
use std::time::Duration;

use fleetworks_workstation::native::BackendKind;
use rune::El;
use rune::Key;
use rune::KeyEvent;
use rune::Mods;
use rune::Transition;
use rune::Tx;
use rune_kit::glyphs;
use rune_kit::Kit;
use rune_kit::TextInput;

use crate::accessibility::ViewPreferences;
use crate::app;
use crate::app::Load;
use crate::connection::ConnectionStatus;
use crate::icons;
use crate::local_backend::BinaryDiscoveryStatus;
use crate::material;
use crate::material::MetalFinish;
use crate::orbit;
use crate::startup_checks;
use crate::startup_checks::CheckStatus;
use crate::theme;
use crate::theme::StatusTone;
use crate::ui_components;
use crate::ui_components::ActionButton;
use crate::views;
use crate::State;

/// The opening samples three seconds at the reference pace, then holds the final pose.
pub(crate) const INTRO_SECONDS: f64 = 3.0;
pub(crate) const FOLDER_FADE_SECONDS: f64 = 0.44;
const LOCAL: &str = "startup-local";
const ADD_FOLDER: &str = "startup-add-folder";
const SETUP_MARK: &str = "startup-setup-mark";
const RAIL_SLOT: &str = "startup-rail-mark-slot";
const SETUP_SECONDS: f64 = 0.22;
const RAIL_SECONDS: f64 = 0.21;
const WIRING_HOLD_SECONDS: f64 = 2.0;
const HEALTH_CHECK_WAIT_SECONDS: f64 = 10.0;
const EMBEDDED_STAGE: &str = "startup-embedded-stage";
const ENTER_WORKSTATION: &str = "startup-enter-workstation";
const CONNECTION_DOT: &str = "startup-wiring-step-connection";
const REGISTRATION_DOT: &str = "startup-wiring-step-registration";
const HEALTH_CHECKS_DOT: &str = "startup-wiring-step-health-checks";

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

#[derive(Clone, Copy, Default, PartialEq)]
enum WiringStage {
    #[default]
    Connection,
    Registration,
    HealthChecks {
        wait_until: f64,
    },
    ReadyToEnter,
}

#[derive(Default)]
struct Wiring {
    stage: WiringStage,
    hold_until: Option<f64>,
}

impl Wiring {
    fn hold(&mut self, now: f64, tx: &mut Tx) -> bool {
        let remaining = *self.hold_until.get_or_insert(now + WIRING_HOLD_SECONDS) - now;
        if remaining > 0.0 {
            tx.wake_after(Duration::from_secs_f64(remaining));
            return true;
        }
        false
    }
}

/// Cold-launch presentation state. Scripted launches and ordinary fixtures skip it by default.
#[derive(Default)]
pub struct Startup {
    active: bool,
    wiring: Option<Wiring>,
    started_at: Option<f64>,
    settled: bool,
    awaiting_intro: bool,
    reduced_motion: Cell<bool>,
    mark_rect: Cell<Option<[f32; 4]>>,
    target_id: Option<String>,
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
        self.reset_setup();
    }

    fn reset_setup(&mut self) {
        self.awaiting_intro = false;
        self.wiring = None;
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

    fn dock_setup(&mut self, now: f64) {
        self.awaiting_intro = false;
        self.docked_pose = self.seconds(now) as f32;
        self.start_flight(Dock::Setup, now);
        self.settled = true;
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
            } else {
                tx.focus(EMBEDDED_STAGE, tx.focus_visible());
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

/// Folder selection and setup stages share the same canvas and mark.
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
    if state.setup_gateway && !state.startup.awaiting_intro {
        remember_mark(state, cx, SETUP_MARK);
        let docking = state.startup.flying_to(Dock::Setup, preferences);
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
        .opacity(if docking { 0.0 } else { 1.0 })
        .inert();
        let (stage, body) = if state.is_local_backend() {
            embedded_stage(state, preferences, kit)
        } else {
            (
                "startup-stage-distributed",
                rune::col()
                    .w_full()
                    .grow(1.0)
                    .min_h(0.0)
                    .child(
                        rune::row()
                            .px(20.0)
                            .gap(8.0)
                            .child(back_button(state.connected(), preferences, kit))
                            .child(rune::spacer())
                            .children(
                                (state.connected() && state.registration.context.is_some()).then(
                                    || {
                                        ActionButton::new("Open Workstation")
                                            .key("startup-open-workstation")
                                            .on_press_tx(State::finish_startup)
                                            .build(kit, preferences, None)
                                    },
                                ),
                            ),
                    )
                    .child(views::setup_gateway(state, cx, kit)),
            )
        };
        let setup = rune::col()
            .key("startup-setup")
            .transition(Transition::none())
            .w_full()
            .max_w(820.0)
            .h_full()
            .pt(20.0)
            .child(
                rune::row()
                    .key("startup-setup-header")
                    .transition(Transition::none())
                    .instant_changes()
                    .w_full()
                    .center()
                    .child(setup_mark),
            )
            .children((!docking).then(|| {
                rune::col()
                    .key(stage)
                    .transition(app::surface_transition(
                        preferences.reduced_motion,
                        Transition::opacity().combined(Transition::offset(18.0, 0.0)),
                    ))
                    .w_full()
                    .grow(1.0)
                    .min_h(0.0)
                    .child(body)
            }));
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
    let [orbit_width, orbit_height] = orbit::display_size(Some([width, height - 210.0]));
    let creating = state.connection_profiles.embedded_folder.open;
    let controls = if creating {
        folder_form(state, preferences, kit)
    } else {
        folder_grid(state, preferences, kit, width)
    };
    rune::col()
        .key("startup-choose")
        .transition(Transition::none())
        .w_full()
        .h_full()
        .items_center()
        .gap(8.0)
        .pt((height * 0.08).clamp(22.0, 64.0))
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
                .grow(1.0)
                .min_h(0.0)
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                })
                .child(
                    rune::col()
                        .key(if creating {
                            "startup-folder-form"
                        } else {
                            "startup-folder-list"
                        })
                        .transition(app::surface_transition(
                            preferences.reduced_motion,
                            Transition::opacity().combined(Transition::offset(12.0, 0.0)),
                        ))
                        .w_full()
                        .items_center()
                        .px(16.0)
                        .pb(22.0)
                        .child(controls),
                ),
        )
}

fn reveal_opacity(state: &State, preferences: ViewPreferences, delay: f64) -> f32 {
    if preferences.reduced_motion {
        return 1.0;
    }
    let phase =
        ((state.startup.seconds(state.clock) - delay) / FOLDER_FADE_SECONDS).clamp(0.0, 1.0) as f32;
    0.02 + 0.98 * (phase * phase * (3.0 - 2.0 * phase))
}

fn folder_grid(state: &State, preferences: ViewPreferences, kit: &Kit, width: f32) -> El {
    const TILE_WIDTH: f32 = 172.0;
    const GAP: f32 = 10.0;
    let available = (width - 32.0).max(20.0);
    let columns = (((available + GAP) / (TILE_WIDTH + GAP)).floor() as usize).clamp(1, 4);
    let tile_width = TILE_WIDTH.min(available);
    let can_select = !state.startup.awaiting_intro
        && (state.fixture || state.workspaces.container.is_some())
        && !state.local_backend.stopping;
    let targets = state.embedded_targets();
    let mut grid = rune::col()
        .key("startup-folder-grid")
        .w_full()
        .gap(GAP)
        .child(
            rune::row().center().child(
                views::heading("Choose a workstation folder", kit)
                    .role("heading")
                    .instant_changes()
                    .opacity(reveal_opacity(state, preferences, 0.01)),
            ),
        );
    let mut row = rune::row().center().gap(GAP);
    for (index, target) in targets
        .iter()
        .map(Some)
        .chain(std::iter::once(None))
        .enumerate()
    {
        let opacity = reveal_opacity(state, preferences, 0.04 + 0.035 * index.min(4) as f64);
        row = row.child(
            folder_tile(target, can_select, tile_width, preferences, kit)
                .instant_changes()
                .opacity(opacity),
        );
        if (index + 1) % columns == 0 || index == targets.len() {
            grid = grid.child(row.key(format!("startup-folder-row-{}", index / columns)));
            row = rune::row().center().gap(GAP);
        }
    }
    let status = match &state.workspaces.snapshot {
        _ if state.startup.awaiting_intro => Some(ui_components::skeleton_text_value(
            "Opening workstation",
            150.0,
            StatusTone::Cyan,
            preferences,
        )),
        Load::Failed(error) => Some(
            rune::col().gap(7.0).child(startup_error(error, kit)).child(
                ActionButton::new("Retry folder access")
                    .key("startup-workspaces-retry")
                    .on_press(State::retry_workspace_access)
                    .build(kit, preferences, None),
            ),
        ),
        Load::Pending | Load::Loading => Some(ui_components::skeleton_text_value(
            "Opening workstation folders",
            150.0,
            StatusTone::Info,
            preferences,
        )),
        _ if state.local_backend.stopping => Some(ui_components::skeleton_text_value(
            "Stopping the previous Embedded backend",
            150.0,
            StatusTone::Info,
            preferences,
        )),
        _ => match &state.connection_profiles.catalog {
            Load::Failed(error) => Some(
                rune::col().gap(7.0).child(startup_error(error, kit)).child(
                    ActionButton::new("Reload saved folders")
                        .key("startup-folders-retry")
                        .on_press(State::reload_embedded_targets)
                        .build(kit, preferences, None),
                ),
            ),
            _ if state.connection_profiles.busy => Some(ui_components::skeleton_text_value(
                "Loading saved folders",
                150.0,
                StatusTone::Info,
                preferences,
            )),
            _ => None,
        },
    };
    grid.children(status.map(|body| {
        rune::row()
            .w_full()
            .center()
            .child(rune::col().w_full().max_w(360.0).child(body))
    }))
}

fn folder_tile(
    target: Option<&crate::connection_profiles::EmbeddedTarget>,
    enabled: bool,
    width: f32,
    preferences: ViewPreferences,
    kit: &Kit,
) -> El {
    let (key, glyph, title, path) = match target {
        Some(target) => (
            folder_key(target.id.as_deref()),
            &glyphs::FOLDER,
            target.name.as_str(),
            Some(target.path.as_str()),
        ),
        None => (ADD_FOLDER.to_owned(), &icons::PLUS, "Add folder", None),
    };
    let label = path.map_or_else(|| title.to_owned(), |path| format!("Open {title}. {path}"));
    let tile = rune::col()
        .key(key)
        .size(width, 108.0)
        .px(9.0)
        .center()
        .gap(5.0)
        .radius(12.0)
        .hover(theme::hex(theme::SURFACE_ELEVATED))
        .press(theme::hex(theme::SURFACE))
        .focus_ring(2.0, 2.0, theme::hex(theme::ACCENT_HIGHLIGHT))
        .role("button")
        .disabled(!enabled)
        .focusable(enabled)
        .label(label)
        .help(path.unwrap_or("Choose a folder or enter its path"))
        .child(
            rune::row()
                .size(44.0, 44.0)
                .radius(22.0)
                .center()
                .fill(StatusTone::Accent.bg())
                .border(1.0, StatusTone::Accent.border())
                .child(rune::icon(glyph, 21.0).color(StatusTone::Accent.fg()))
                .a11y_hidden(),
        )
        .child(
            rune::text(title)
                .font(kit.ty.body_medium)
                .color(theme::hex(theme::TEXT_PRIMARY))
                .max_w(width - 18.0)
                .truncate()
                .a11y_hidden(),
        )
        .children(path.map(|path| {
            rune::text(path)
                .font(kit.ty.small)
                .color(theme::hex(theme::TEXT_SECONDARY))
                .max_w(width - 18.0)
                .truncate_middle()
                .a11y_hidden()
        }));
    let tile = if let Some(target) = target {
        let pointer_id = target.id.clone();
        let keyboard_id = target.id.clone();
        tile.on_press_tx(move |state: &mut State, tx| {
            state.open_startup_target(pointer_id.as_deref(), tx);
        })
        .on_key(move |state: &mut State, event, tx| {
            activate_on_return(state, event, tx, |state, tx| {
                state.open_startup_target(keyboard_id.as_deref(), tx);
            })
        })
    } else {
        tile.on_press_tx(State::open_embedded_folder)
            .on_key(|state: &mut State, event, tx| {
                activate_on_return(state, event, tx, State::open_embedded_folder)
            })
    };
    material::glass_control(tile, preferences)
}

fn folder_key(id: Option<&str>) -> String {
    id.map_or_else(|| LOCAL.to_owned(), |id| format!("startup-folder-{id}"))
}

fn folder_form(state: &State, preferences: ViewPreferences, kit: &Kit) -> El {
    let form = &state.connection_profiles.embedded_folder;
    let ready = matches!(state.connection_profiles.catalog, Load::Ready(_));
    let mut actions = rune::row()
        .w_full()
        .center_y()
        .gap(8.0)
        .child(
            ActionButton::new("Choose folder")
                .key("startup-folder-browse")
                .icon(&glyphs::FOLDER_OPEN)
                .disabled(form.saving)
                .on_press_tx(State::choose_embedded_folder)
                .build(kit, preferences, None),
        )
        .child(rune::spacer())
        .child(
            ActionButton::new("Cancel")
                .key("startup-folder-cancel")
                .disabled(form.saving)
                .on_press_tx(State::close_embedded_folder)
                .build(kit, preferences, None),
        );
    actions = actions.child(
        ActionButton::new("Save folder")
            .key("startup-folder-save")
            .busy(form.saving)
            .disabled(state.connection_profiles.busy || !ready || form.path.text.trim().is_empty())
            .finish(MetalFinish::GoldCta)
            .on_press_tx(State::save_embedded_folder)
            .build(kit, preferences, None),
    );
    material::glass_panel(
        rune::col()
            .w_full()
            .max_w(484.0)
            .padding(15.0)
            .gap(9.0)
            .radius(12.0)
            .child(views::heading("Add a workstation folder", kit).role("heading"))
            .child(
                TextInput::new(&form.path)
                    .label("Folder path")
                    .placeholder("/absolute/path/to/workstation")
                    .width(None)
                    .disabled(form.saving)
                    .on_edit(|state: &mut State| {
                        &mut state.connection_profiles.embedded_folder.path
                    })
                    .on_change(|state: &mut State, _| state.embedded_folder_path_changed())
                    .on_submit(State::save_embedded_folder)
                    .build(kit)
                    .key("startup-folder-path"),
            )
            .children(form.error.as_deref().map(|error| startup_error(error, kit)))
            .children(match &state.connection_profiles.catalog {
                Load::Failed(error) if !form.saving => Some(
                    rune::col().gap(7.0).child(startup_error(error, kit)).child(
                        ActionButton::new("Reload saved folders")
                            .key("startup-folders-retry")
                            .on_press(State::reload_embedded_targets)
                            .build(kit, preferences, None),
                    ),
                ),
                _ if !ready && state.connection_profiles.busy => {
                    Some(ui_components::skeleton_text_value(
                        "Loading saved folders",
                        150.0,
                        StatusTone::Info,
                        preferences,
                    ))
                }
                _ => None,
            })
            .child(actions),
        preferences,
    )
}

fn startup_error(detail: &str, kit: &Kit) -> El {
    rune::text(detail)
        .font(kit.ty.small)
        .color(StatusTone::Err.fg())
        .w_full()
        .min_w(0.0)
        .wrap_anywhere()
        .role("alert")
}

fn back_button(disabled: bool, preferences: ViewPreferences, kit: &Kit) -> El {
    ActionButton::new("Back")
        .key("startup-back")
        .disabled(disabled)
        .on_press_tx(State::return_to_startup_choice)
        .build(kit, preferences, None)
        .on_key(|state: &mut State, event, tx| {
            activate_on_return(state, event, tx, State::return_to_startup_choice)
        })
}

fn embedded_stage(state: &State, preferences: ViewPreferences, kit: &Kit) -> (&'static str, El) {
    let stage = if state.connected() {
        state
            .startup
            .wiring
            .as_ref()
            .map_or(WiringStage::Registration, |wiring| wiring.stage)
    } else {
        WiringStage::Connection
    };
    let (key, title, dot, checks, detail, next) = match stage {
        WiringStage::Connection => {
            let (detail, next) = connection_actions(state, preferences, kit);
            (
                "startup-stage-starting",
                "Embedded connection",
                CONNECTION_DOT,
                connection_checks(state, preferences, kit),
                detail,
                next,
            )
        }
        WiringStage::Registration => {
            let (detail, next) = registration_actions(state, preferences, kit);
            (
                "startup-stage-registering",
                "Workstation registration",
                REGISTRATION_DOT,
                registration_checks(state, preferences, kit),
                detail,
                next,
            )
        }
        WiringStage::HealthChecks { .. } | WiringStage::ReadyToEnter => {
            let pending = matches!(stage, WiringStage::HealthChecks { .. })
                && state
                    .startup
                    .wiring
                    .as_ref()
                    .is_some_and(|wiring| wiring.hold_until.is_none())
                && state.startup_health_checks_pending();
            (
                "startup-stage-embedded-health",
                "Embedded health checks",
                HEALTH_CHECKS_DOT,
                startup_checks::embedded_content(state, pending, preferences, kit),
                None,
                None,
            )
        }
    };
    let heading = views::heading(title, kit)
        .key(EMBEDDED_STAGE)
        .role("heading")
        .focusable(true)
        .tab_index(-1);
    let header = if stage == WiringStage::ReadyToEnter {
        rune::row()
            .w_full()
            .center_y()
            .gap(8.0)
            .child(heading.grow(1.0).min_w(0.0))
            .child(
                ActionButton::new("Enter Workstation")
                    .key(ENTER_WORKSTATION)
                    .finish(MetalFinish::GoldCta)
                    .on_press_tx(State::finish_startup)
                    .build(kit, preferences, None)
                    .on_key(|state: &mut State, event, tx| {
                        activate_on_return(state, event, tx, State::finish_startup)
                    }),
            )
    } else {
        heading
    };
    let has_controls = matches!(stage, WiringStage::Connection | WiringStage::Registration);
    let footer = has_controls.then(|| {
        rune::row()
            .key("startup-embedded-actions")
            .w_full()
            .center_y()
            .gap(8.0)
            .child(back_button(false, preferences, kit))
            .child(rune::spacer())
            .children(next)
    });
    let body = material::glass_panel(
        rune::col()
            .w_full()
            .max_w(510.0)
            .padding(16.0)
            .gap(10.0)
            .radius(12.0)
            .child(header)
            .child(checks)
            .children(
                has_controls
                    .then(|| views::quiet(state.local_state_dir_label(), kit).wrap_anywhere()),
            )
            .children(footer)
            .children(detail),
        preferences,
    );
    let slide = rune::col()
        .items_center()
        .child(body)
        .child(step_dots(dot, kit));
    (key, views::scroll(slide, "startup-stage-scroll"))
}

fn step_dots(active: &str, kit: &Kit) -> El {
    let steps = [CONNECTION_DOT, REGISTRATION_DOT, HEALTH_CHECKS_DOT];
    let mut current = 1;
    let mut dots = rune::row().key("startup-wiring-dots").center_y().gap(7.0);
    for (index, step) in steps.into_iter().enumerate() {
        if step == active {
            current = index + 1;
        }
        dots = dots.child(
            rune::el()
                .key(step)
                .size(7.0, 7.0)
                .radius(8.0)
                .fill(if step == active {
                    kit.c.icon_accent
                } else {
                    kit.c.border_strong
                })
                .a11y_hidden(),
        );
    }
    dots.role("status")
        .label(format!("Step {current} of {}", steps.len()))
}

fn connection_checks(state: &State, preferences: ViewPreferences, kit: &Kit) -> El {
    let companion = match &state.local_backend.binary_status {
        _ if state.connected() => CheckStatus::Passed,
        BinaryDiscoveryStatus::Ready(_) => CheckStatus::Passed,
        BinaryDiscoveryStatus::Missing(_) => CheckStatus::Failed,
        BinaryDiscoveryStatus::Unknown
            if state.connection.status == ConnectionStatus::Connecting =>
        {
            CheckStatus::Checking
        }
        BinaryDiscoveryStatus::Unknown => CheckStatus::Pending,
    };
    let connection = match &state.connection.status {
        ConnectionStatus::Connected => CheckStatus::Passed,
        _ if companion == CheckStatus::Failed => CheckStatus::Pending,
        ConnectionStatus::Connecting => CheckStatus::Checking,
        ConnectionStatus::Failed(_) => CheckStatus::Failed,
        ConnectionStatus::Disconnected => CheckStatus::Pending,
    };
    startup_checks::list(
        "startup-wiring-connection-checks",
        "Embedded connection checks",
    )
    .child(startup_checks::row(
        "startup-wiring-check-companion",
        "Companion executable",
        companion,
        preferences,
        kit,
    ))
    .child(startup_checks::row(
        "startup-wiring-check-connection",
        "Private backend connection",
        connection,
        preferences,
        kit,
    ))
}

fn registration_checks(state: &State, preferences: ViewPreferences, kit: &Kit) -> El {
    let registration = match &state.registration.receipt {
        _ if state.registration.context.is_some() => CheckStatus::Passed,
        Load::Pending | Load::Loading => CheckStatus::Checking,
        Load::Failed(_) => CheckStatus::Failed,
        _ => CheckStatus::NeedsAttention,
    };
    startup_checks::list(
        "startup-wiring-registration-checks",
        "Workstation registration checks",
    )
    .child(startup_checks::row(
        "startup-wiring-check-connected",
        "Embedded backend connection",
        CheckStatus::Passed,
        preferences,
        kit,
    ))
    .child(startup_checks::row(
        "startup-wiring-check-registration",
        "Registered workstation access",
        registration,
        preferences,
        kit,
    ))
}

fn connection_actions(
    state: &State,
    preferences: ViewPreferences,
    kit: &Kit,
) -> (Option<El>, Option<El>) {
    let start = |title| {
        ActionButton::new(title)
            .key("connect-local")
            .disabled(state.local_backend.stopping || state.resource_save_pending())
            .finish(MetalFinish::GoldCta)
            .on_press(State::connect_local)
            .build(kit, preferences, None)
    };
    match &state.connection.status {
        ConnectionStatus::Disconnected => (
            match &state.local_backend.binary_status {
                BinaryDiscoveryStatus::Missing(error) => Some(
                    rune::col()
                        .key("local-binary-status")
                        .role("alert")
                        .child(startup_error(error, kit).key("local-binary-status-detail")),
                ),
                _ => None,
            },
            Some(start("Start Embedded")),
        ),
        ConnectionStatus::Connecting | ConnectionStatus::Connected => (None, None),
        ConnectionStatus::Failed(error) => {
            (Some(startup_error(error, kit)), Some(start("Retry start")))
        }
    }
}

fn registration_actions(
    state: &State,
    preferences: ViewPreferences,
    kit: &Kit,
) -> (Option<El>, Option<El>) {
    match &state.registration.receipt {
        _ if state.registration.context.is_some() => (None, None),
        Load::Pending | Load::Loading => (None, None),
        receipt => (
            match receipt {
                Load::Failed(error) => Some(startup_error(error, kit)),
                _ => None,
            },
            state.can_register().then(|| {
                ActionButton::new("Retry registration")
                    .key("workstation-register")
                    .finish(MetalFinish::GoldCta)
                    .on_press(State::ensure_registration)
                    .build(kit, preferences, None)
            }),
        ),
    }
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
    fn open_startup_target(&mut self, id: Option<&str>, tx: &mut Tx) {
        if !self.select_embedded_target(id) {
            return;
        }
        self.startup.target_id = id.map(str::to_owned);
        self.choose_startup_destination(BackendKind::Local);
        self.startup.wiring = Some(Wiring::default());
        self.connect_local();
        if !self.startup.awaiting_intro {
            self.wake_for_setup_dock(tx);
        }
    }

    fn wake_for_setup_dock(&self, tx: &mut Tx) {
        if self.startup.reduced_motion.get() {
            tx.settle();
        }
        if self.startup.flight.is_some() {
            tx.wake_after(Duration::from_secs_f64(1.0 / 60.0));
        } else {
            tx.focus(EMBEDDED_STAGE, tx.by_keys() || tx.focus_visible());
            tx.wake_after(Duration::ZERO);
        }
    }

    pub(crate) fn poll_startup_intro(&mut self, now: f64, tx: &mut Tx) -> bool {
        if !self.startup.active() || (self.setup_gateway && !self.startup.awaiting_intro) {
            return false;
        }
        self.startup.begin(now);
        if let Some(wait) = self.startup.next_wake(now) {
            tx.wake_after(Duration::from_secs_f64(wait));
        } else if self.startup.awaiting_intro {
            self.startup.dock_setup(now);
            self.wake_for_setup_dock(tx);
        }
        // Render the final pose even when no subsequent frame is needed.
        true
    }

    pub(crate) fn poll_startup_wiring(&mut self, now: f64, tx: &mut Tx) -> bool {
        if !self.setup_gateway || !self.is_local_backend() || self.startup.flight.is_some() {
            return false;
        }
        let connected = self.connected();
        let registered = self.registration.context.is_some();
        let current_observation = self.startup_health_observed_at(now + WIRING_HOLD_SECONDS);
        let checks_pending = !current_observation && self.startup_health_checks_pending();
        let refresh_checks = !self.readiness_pending()
            && self.readiness_error.is_none()
            && !current_observation
            && matches!(self.readiness, Load::Idle | Load::Ready(_));
        let Some(wiring) = self.startup.wiring.as_mut() else {
            return false;
        };
        if !connected {
            let changed = wiring.stage != WiringStage::Connection;
            *wiring = Wiring::default();
            return changed;
        }
        if !registered && wiring.stage != WiringStage::Connection {
            let changed = wiring.stage != WiringStage::Registration;
            wiring.stage = WiringStage::Registration;
            wiring.hold_until = None;
            return changed;
        }
        if let WiringStage::HealthChecks { wait_until } = wiring.stage {
            if checks_pending && wiring.hold_until.is_none() && now < wait_until {
                tx.wake_after(Duration::from_secs_f64(wait_until - now));
                return false;
            }
        }
        let began_hold = wiring.hold_until.is_none();
        if wiring.hold(now, tx) {
            return began_hold && matches!(wiring.stage, WiringStage::HealthChecks { .. });
        }
        match wiring.stage {
            WiringStage::Connection => {
                wiring.stage = WiringStage::Registration;
                wiring.hold_until = None;
                if registered {
                    // Registration can finish while the connection checks are still visible.
                    // Schedule its own hold even when Reduced Motion produces no transition frames.
                    wiring.hold(now, tx);
                }
                tx.focus(EMBEDDED_STAGE, tx.focus_visible());
            }
            WiringStage::Registration => {
                wiring.stage = WiringStage::HealthChecks {
                    wait_until: now + HEALTH_CHECK_WAIT_SECONDS,
                };
                wiring.hold_until = None;
                if checks_pending || refresh_checks {
                    tx.wake_after(Duration::from_secs_f64(HEALTH_CHECK_WAIT_SECONDS));
                } else {
                    wiring.hold(now, tx);
                }
                tx.focus(EMBEDDED_STAGE, tx.focus_visible());
                if refresh_checks {
                    self.refresh_readiness();
                }
            }
            WiringStage::HealthChecks { .. } => {
                wiring.stage = WiringStage::ReadyToEnter;
                tx.focus(ENTER_WORKSTATION, tx.focus_visible());
            }
            WiringStage::ReadyToEnter => return false,
        }
        true
    }

    fn startup_health_checks_pending(&self) -> bool {
        self.readiness_pending()
            || matches!(self.readiness, Load::Idle | Load::Pending | Load::Loading)
    }

    fn startup_health_observed_at(&self, now: f64) -> bool {
        self.readiness_received_at.is_some()
            && startup_checks::scoped_readiness(self).is_some_and(|readiness| {
                readiness.checks.is_empty()
                    || readiness.checks.iter().any(|check| {
                        self.readiness_check_applies(check)
                            && self.readiness_check_fresh_at(check, now)
                    })
            })
    }

    /// Render the settled Embedded folder chooser with synthetic effects.
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

    /// Begin a connected Embedded fixture on the registration slide.
    pub fn fixture_show_embedded_registration(&mut self) {
        if self.fixture {
            self.startup.wiring = Some(Wiring {
                stage: WiringStage::Registration,
                hold_until: None,
            });
        }
    }

    /// Begin a connected Embedded setup fixture on the initial health slide.
    pub fn fixture_show_embedded_health_checks(&mut self) {
        if self.fixture {
            self.startup.wiring = Some(Wiring {
                stage: WiringStage::HealthChecks {
                    wait_until: self.clock + HEALTH_CHECK_WAIT_SECONDS,
                },
                hold_until: None,
            });
        }
    }

    /// Move the mark into the setup pane after a user or fixture chooses a target.
    pub(crate) fn choose_startup_destination(&mut self, kind: BackendKind) {
        if !self.startup.active() || self.setup_gateway {
            return;
        }
        // An outstanding profile read can populate the catalog but must respect the explicit choice.
        self.connection_profiles.initial_selection_applied = true;
        self.startup.awaiting_intro =
            kind == BackendKind::Local && self.startup.next_wake(self.clock).is_some();
        if !self.startup.awaiting_intro {
            self.startup.dock_setup(self.clock);
        }
        self.set_backend_kind(kind);
        self.setup_gateway = true;
        self.prefill_startup_distributed_url();
    }

    pub(crate) fn return_to_startup_choice(&mut self, tx: &mut Tx) {
        if !self.startup.active()
            || !self.setup_gateway
            || self.connected() && !self.is_local_backend()
        {
            return;
        }
        // Leaving a pending attempt uses the same cancellation and stale-response fence as setup.
        if self.is_local_backend()
            || self.connection.status == ConnectionStatus::Connecting
            || self.pairing.busy()
            || self.pairing.paused
        {
            self.disconnect();
        }
        self.setup_gateway = false;
        self.startup.flight = None;
        self.startup.reset_setup();
        if self.startup.reduced_motion.get() {
            tx.settle();
        }
        tx.focus(folder_key(self.startup.target_id.as_deref()), tx.by_keys());
    }

    pub(crate) fn finish_gateway_setup(&mut self, tx: &mut Tx) {
        if self.startup.active() {
            self.finish_startup(tx);
            return;
        }
        self.setup_gateway = false;
        if self.settings_open {
            self.settings_open = false;
            tx.focus("open-workstation-settings", tx.by_keys());
        }
    }

    pub(crate) fn finish_startup(&mut self, tx: &mut Tx) {
        if !self.startup.active() || !self.connected() || self.registration.context.is_none() {
            return;
        }
        if self.is_local_backend()
            && !self
                .startup
                .wiring
                .as_ref()
                .is_some_and(|wiring| wiring.stage == WiringStage::ReadyToEnter)
        {
            return;
        }
        self.startup.start_flight(Dock::Rail, self.clock);
        if self.startup.flight.is_some() {
            tx.wake_after(std::time::Duration::from_secs_f64(1.0 / 60.0));
        }
        self.startup.finish();
        self.setup_gateway = false;
        self.page = app::Page::Workstation;
        // Mount the shell on this commit, so it does not animate over the mark's own flight.
        tx.instant();
        tx.focus("nav-workstation", tx.by_keys() || tx.focus_visible());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accessibility::FixtureAccessibility;
    use crate::app::Intent;
    use fleetworks_contracts::connect::RegistrationReceipt;
    use fleetworks_contracts::context::RegisteredContext;
    use fleetworks_contracts::readiness::ReadinessCheck;
    use fleetworks_contracts::readiness::ReadinessCheckId;
    use fleetworks_contracts::readiness::ReadinessState;
    use fleetworks_contracts::readiness::WorkstationReadiness;

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
    fn folder_tiles_fade_in_and_return_does_not_replay_the_opening() {
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
            for (choice, name) in [(LOCAL, "Open Default folder"), (ADD_FOLDER, "Add folder")] {
                let ([x, y, w, h], alpha) = runtime.presented(choice).expect("first-frame tile");
                assert_eq!(alpha == 1.0, reduced);
                assert!(x >= 0.0 && y >= 0.0 && x + w <= size.0 && y + h <= size.1);
                assert!(runtime.elements().iter().any(|element| {
                    element.key.as_deref() == Some(choice)
                        && element
                            .label
                            .as_deref()
                            .is_some_and(|v| v.starts_with(name))
                }));
            }
            assert!(runtime.find("startup-remote").is_none());
            assert!(!runtime.state.intents.contains(&Intent::Connect));

            tick(&mut runtime, 0.7);
            assert_opaque(&runtime, LOCAL);
            assert_opaque(&runtime, ADD_FOLDER);
            key(&mut runtime, rune::Key::Tab);
            runtime.frame(0.03);
            key(&mut runtime, rune::Key::Enter);
            runtime.frame(0.30);
            tick(&mut runtime, 3.02);
            tick(&mut runtime, 3.25);
            assert_eq!(runtime.state.backend_kind, BackendKind::Local);
            assert_eq!(
                runtime.state.connection.status,
                ConnectionStatus::Connecting
            );
            assert!(runtime.find("startup-stage-starting").is_some());
            assert!(runtime.find("connect-local").is_none());
            assert!(runtime.state.intents.contains(&Intent::Connect));

            click(&mut runtime, "startup-back");
            if reduced {
                runtime.frame(0.0);
                assert_opaque(&runtime, LOCAL);
                assert_opaque(&runtime, ADD_FOLDER);
                assert_opaque(&runtime, "startup-orbit");
            }
            runtime.frame(0.60);
            assert_eq!(runtime.state.startup.seconds(0.6), INTRO_SECONDS);
            assert!(runtime.find("startup-orbit").is_some());
            assert!(runtime.find(ADD_FOLDER).is_some());

            key(&mut runtime, rune::Key::Tab);
            runtime.frame(0.62);
            key(&mut runtime, rune::Key::Text(" ".into()));
            runtime.frame(0.90);
            assert!(runtime.find("startup-folder-browse").is_some());
            assert!(runtime.find("startup-folder-path/value/input").is_some());
            assert!(runtime.find("nav-workstation").is_none());
            assert!(!runtime.state.connected());
            assert!(runtime.state.client.is_none());
            assert_eq!(
                runtime
                    .state
                    .intents
                    .iter()
                    .filter(|intent| matches!(intent, Intent::Connect))
                    .count(),
                1
            );
        }
    }

    fn tick(runtime: &mut rune::Runtime<State>, clock: f64) {
        runtime.update(|state, tx| {
            state.clock = clock;
            state.startup.begin(0.0);
            if !state.poll_startup_intro(clock, tx) {
                state.startup.poll_flight(clock, tx);
                state.poll_startup_wiring(clock, tx);
            }
        });
        runtime.frame(0.02);
    }

    #[test]
    fn early_folder_selection_finishes_the_checklists_and_waits_for_enter() {
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
            assert_eq!(
                runtime.state.connection.status,
                ConnectionStatus::Connecting
            );
            assert!(runtime.state.intents.contains(&Intent::Connect));
            if reduced {
                assert!(runtime.find("startup-stage-starting").is_some());
                assert_opaque(&runtime, "startup-embedded-stage");
            } else {
                assert!(runtime.find("startup-choose").is_some());
                assert!(runtime.find("startup-stage-starting").is_none());
                assert!(runtime.find("startup-orbit-flight").is_none());
            }

            // Embedded can complete both requests while the intro is still visible.
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
                observe_health_checks(state, receipt, ReadinessState::Ready);
            });
            let checklist_at = if reduced {
                1.05
            } else {
                tick(&mut runtime, 2.99);
                assert!(runtime.find("startup-choose").is_some());
                assert!(runtime.find("startup-orbit-flight").is_none());
                assert!(runtime.find("startup-wiring-connection-checks").is_none());
                tick(&mut runtime, 3.0);
                assert_eq!(runtime.state.startup.docked_pose, INTRO_SECONDS as f32);
                tick(&mut runtime, 3.11);
                let (dock, _) = runtime.presented(SETUP_MARK).expect("setup mark slot");
                let (in_flight, _) = runtime
                    .presented("startup-orbit-flight")
                    .expect("handoff frame");
                assert!(in_flight[2] > dock[2] && in_flight[2] < chooser[2]);
                assert!((in_flight[2] / in_flight[3] - chooser[2] / chooser[3]).abs() < 0.01);
                assert!(runtime.find("startup-wiring-connection-checks").is_none());
                3.23
            };
            tick(&mut runtime, checklist_at);
            assert_eq!(runtime.presented(SETUP_MARK).unwrap().1, 1.0);
            assert!(runtime
                .accessibility_json()
                .to_string()
                .contains("Private backend connection: Passed"));
            tick(&mut runtime, checklist_at + 1.99);
            assert!(runtime.find("startup-wiring-connection-checks").is_some());
            assert!(runtime.find("startup-wiring-registration-checks").is_none());

            let registering_at = checklist_at + 2.01;
            tick(&mut runtime, registering_at);
            let tree = runtime.accessibility_json().to_string();
            assert!(tree.contains("Registered workstation access: Passed"));
            assert!(tree.contains("Step 2 of 3"));
            tick(&mut runtime, registering_at + 1.99);
            assert!(runtime.find("startup-wiring-registration-checks").is_some());
            assert!(runtime.find("screen-workstation").is_none());

            let health_at = registering_at + 2.01;
            tick(&mut runtime, health_at);
            let tree = runtime.accessibility_json().to_string();
            assert!(tree.contains("Embedded health checks"));
            assert!(tree.contains("Gateway: Passed"));
            assert!(tree.contains("Actor catalog: Passed"));
            assert!(tree.contains("Step 3 of 3"));
            assert!(runtime.find("startup-embedded-actions").is_none());
            assert!(runtime.find("startup-back").is_none());
            assert!(runtime.find(ENTER_WORKSTATION).is_none());
            tick(&mut runtime, health_at + 1.99);
            assert!(runtime.find("startup-wiring-health-checklist").is_some());
            assert!(runtime.find("screen-workstation").is_none());
            assert!(runtime.find(ENTER_WORKSTATION).is_none());

            tick(&mut runtime, health_at + 2.01);
            assert!(runtime.find(ENTER_WORKSTATION).is_some());
            let workstation_at = health_at + 25.0;
            tick(&mut runtime, workstation_at);
            assert!(runtime.find("startup-wiring-health-checklist").is_some());
            assert!(runtime.find("screen-workstation").is_none());
            if reduced {
                click(&mut runtime, ENTER_WORKSTATION);
            } else {
                key(&mut runtime, rune::Key::Enter);
            }
            runtime.frame(0.02);
            assert!(!runtime.state.startup.active());
            assert_eq!(runtime.state.page, app::Page::Workstation);
            assert!(runtime.find("screen-workstation").is_some());
            assert!(runtime.find("inbox-qa-toggle").is_none());
            assert_eq!(runtime.find("startup-orbit-flight").is_some(), !reduced);
            tick(&mut runtime, workstation_at + 0.24);
            assert!(runtime.find("startup-orbit-flight").is_none());
            let (rail, opacity) = runtime.presented("brand-mark").expect("landed rail mark");
            assert_eq!(opacity, 1.0);
            assert!(rail[0] >= 0.0 && rail[1] >= 0.0);
        }
    }

    fn reach_embedded_health_checks(
        runtime: &mut rune::Runtime<State>,
        readiness: Load<WorkstationReadiness>,
    ) {
        tick(runtime, 1.0);
        click(runtime, LOCAL);
        runtime.frame(0.02);
        let registered = crate::tests::registered_fixture();
        runtime.update(|state, _| {
            state.connection.status = registered.connection.status;
            state.connection.capabilities = registered.connection.capabilities;
            state.registration.context = registered.registration.context;
            state.readiness = readiness;
        });
        tick(runtime, 2.0);
        tick(runtime, 4.01);
        tick(runtime, 6.02);
        assert!(runtime.find("startup-wiring-health-checklist").is_some());
        assert!(runtime.find("screen-workstation").is_none());
    }

    #[test]
    fn a_new_workstation_can_enter_with_catalog_and_search_setup_pending() {
        let mut runtime = runtime((640.0, 480.0), true);
        reach_embedded_health_checks(&mut runtime, Load::Idle);
        let tree = runtime.accessibility_json().to_string();
        assert!(runtime.state.intents.contains(&Intent::Readiness));
        assert!(tree.contains("Gateway: Checking"));
        assert!(tree.contains("Actor catalog: Checking"));
        assert!(tree.contains("Actor search: Checking"));
        assert!(runtime.find("startup-wiring-health-check-jobs").is_none());

        runtime.update(|state, _| {
            let context = state
                .registration
                .context
                .as_ref()
                .expect("registered fixture");
            let receipt = RegistrationReceipt::new(
                context.deployment_id.clone(),
                context.workstation_id.clone(),
                context.registration_id.clone(),
                context.workstation_ip,
                context.registration_revision,
            );
            observe_health_checks(state, receipt, ReadinessState::NotReady);
        });
        tick(&mut runtime, 7.0);
        let tree = runtime.accessibility_json().to_string();
        assert!(tree.contains("Actor catalog: Needs setup"));
        assert!(tree.contains("Actor search: Needs setup"));
        tick(&mut runtime, 8.99);
        assert!(runtime.find("screen-workstation").is_none());
        assert!(runtime.find(ENTER_WORKSTATION).is_none());
        tick(&mut runtime, 9.01);
        assert!(runtime.find("screen-workstation").is_none());
        click(&mut runtime, ENTER_WORKSTATION);
        runtime.frame(0.02);
        assert!(runtime.find("screen-workstation").is_some());
        assert!(!runtime.state.intents.iter().any(|intent| matches!(
            intent,
            Intent::Maintenance { .. }
                | Intent::UploadDataset { .. }
                | Intent::DownloadGithubDataset { .. }
        )));
    }

    #[test]
    fn unavailable_embedded_health_checks_allow_the_workstation_to_open_after_the_hold() {
        for readiness in [Load::Failed("Checks unreachable".into()), Load::Loading] {
            let mut runtime = runtime((640.0, 480.0), true);
            let pending = matches!(readiness, Load::Loading);
            reach_embedded_health_checks(&mut runtime, readiness);

            let unavailable_at = if pending {
                tick(&mut runtime, 16.01);
                assert!(runtime
                    .accessibility_json()
                    .to_string()
                    .contains("Gateway: Checking"));
                tick(&mut runtime, 16.03);
                16.03
            } else {
                6.02
            };
            assert!(runtime
                .accessibility_json()
                .to_string()
                .contains("Gateway: Unavailable"));
            tick(&mut runtime, unavailable_at + 1.99);
            assert!(runtime.find("startup-wiring-health-checklist").is_some());
            assert!(runtime.find(ENTER_WORKSTATION).is_none());
            tick(&mut runtime, unavailable_at + 2.01);
            assert!(runtime.find("screen-workstation").is_none());
            click(&mut runtime, ENTER_WORKSTATION);
            runtime.frame(0.02);
            assert!(runtime.find("screen-workstation").is_some());
        }
    }

    #[test]
    fn embedded_wiring_stays_on_pending_and_failed_checks_until_the_user_can_retry() {
        let mut runtime = runtime((640.0, 480.0), true);
        tick(&mut runtime, 1.0);
        click(&mut runtime, LOCAL);
        runtime.frame(0.02);
        runtime.update(|state, _| {
            state.local_backend.binary_status =
                BinaryDiscoveryStatus::Ready("/fixture/bundle/argos-api-host".into());
            state.connection.status =
                ConnectionStatus::Failed("The private backend timed out.".into());
        });
        tick(&mut runtime, 3.0);
        assert!(runtime
            .accessibility_json()
            .to_string()
            .contains("Private backend connection: Failed"));
        click(&mut runtime, "connect-local");
        runtime.frame(0.02);
        runtime.update(|state, _| {
            state.connection.status = ConnectionStatus::Connected;
            state.registration.receipt = Load::Loading;
        });
        tick(&mut runtime, 4.0);
        tick(&mut runtime, 6.1);
        tick(&mut runtime, 10.0);
        assert!(runtime
            .accessibility_json()
            .to_string()
            .contains("Registered workstation access: Checking"));
        assert!(runtime.find("screen-workstation").is_none());

        runtime.update(|state, _| {
            let mut capabilities = fleetworks_contracts::capabilities::Capabilities::default();
            capabilities
                .permissions
                .push(fleetworks_contracts::context::Permission::RegisterWorkstation);
            state.connection.capabilities = Some(capabilities);
            state.registration.receipt = Load::Failed("The workstation could not register.".into());
        });
        tick(&mut runtime, 11.0);
        assert!(runtime
            .accessibility_json()
            .to_string()
            .contains("Registered workstation access: Failed"));
        click(&mut runtime, "workstation-register");
        runtime.frame(0.02);
        assert!(runtime.state.intents.contains(&Intent::Register));

        let registration = crate::tests::registered_fixture().registration.context;
        runtime.update(|state, _| state.registration.context = registration);
        tick(&mut runtime, 11.2);
        click(&mut runtime, "startup-back");
        runtime.frame(0.02);
        tick(&mut runtime, 11.3);
        assert!(runtime.find(LOCAL).is_some());
        click(&mut runtime, LOCAL);
        runtime.frame(0.02);
        tick(&mut runtime, 12.3);
        assert!(runtime.find("startup-wiring-connection-checks").is_some());
        assert!(runtime.find("screen-workstation").is_none());
    }

    fn observe_health_checks(
        state: &mut State,
        receipt: RegistrationReceipt,
        actor_state: ReadinessState,
    ) {
        let observed = "2026-11-19T00:00:00Z";
        let checks = [
            (
                ReadinessCheckId::Gateway,
                ReadinessState::Ready,
                "gateway_reachable",
            ),
            (
                ReadinessCheckId::Jobs,
                ReadinessState::NotApplicable,
                "embedded_backend",
            ),
            (
                ReadinessCheckId::ActorCatalog,
                actor_state,
                "database_seed_incomplete",
            ),
            (
                ReadinessCheckId::ActorSearch,
                actor_state,
                "index_not_built",
            ),
        ]
        .into_iter()
        .map(|(id, observation, reason)| {
            let mut check = ReadinessCheck::new(
                id,
                observation,
                reason.into(),
                String::new(),
                observed.into(),
            );
            check.fresh_for_ms = 60_000;
            check
        })
        .collect();
        let mut readiness =
            WorkstationReadiness::new(receipt.deployment_id.clone(), observed.into(), checks);
        readiness.registration = Some(receipt);
        state.readiness = Load::Ready(readiness);
        state.readiness_received_at = Some(state.clock);
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
