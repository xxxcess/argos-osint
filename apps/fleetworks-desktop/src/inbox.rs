//! Native Inbox: a grouped drawer over allowlisted transitions this app already observed.
//!
//! It is not a notification service. Nothing is pushed, nothing leaves this process, and it makes
//! no claim about device-wide alerts. Items are built from fixed text plus a job title or a
//! readiness check name, never from error text, URLs or credentials. The whole list resets when
//! the connection authority changes.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::VecDeque;

use fleetworks_contracts::readiness::ReadinessState;
use rune::Cx;
use rune::El;
use rune::Tx;
use rune_kit::Kit;

use crate::icons;
use crate::theme;
use crate::State;

/// Retained items. The oldest is dropped first.
pub(crate) const MAX_ITEMS: usize = 50;
/// Remembered job transitions, so one job transition is recorded once.
const MAX_SEEN_JOB_EVENTS: usize = 200;
/// Bounded map of tracked job states across pagination and filtering.
const MAX_TRACKED_JOBS: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Topic {
    Connection,
    Job(String),
    Readiness(String),
}

impl Topic {
    fn key(&self) -> String {
        match self {
            Self::Connection => "inbox-group-connection".into(),
            Self::Job(id) => format!("inbox-group-job-{id}"),
            Self::Readiness(check) => format!("inbox-group-readiness-{check}"),
        }
    }

    fn action(&self) -> Option<&'static str> {
        match self {
            Self::Connection => None,
            Self::Job(_) => Some("Open job"),
            Self::Readiness(_) => Some("View status"),
        }
    }
}

struct Group<'a> {
    topic: &'a Topic,
    title: &'a str,
    items: Vec<&'a Item>,
    unread: usize,
}

fn item_order(left: &Item, right: &Item) -> std::cmp::Ordering {
    left.at.total_cmp(&right.at).then(left.id.cmp(&right.id))
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Item {
    pub id: u64,
    topic: Topic,
    group_title: String,
    pub title: String,
    pub detail: String,
    /// UI clock reading when the transition was observed.
    pub at: f64,
    pub read: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Link {
    #[default]
    Unknown,
    Connecting,
    Connected,
    Failed,
}

/// What this app observed on one poll. Built from state it already holds.
#[derive(Debug, Clone, Default)]
pub(crate) struct Observation {
    /// The connection authority. A different value resets the Inbox.
    pub scope: u64,
    pub link: Link,
    /// (job id, title, state) for the registered workstation's visible jobs.
    pub jobs: Vec<(String, String, String)>,
    /// (check name, state) from the latest readiness read.
    pub checks: Vec<(String, ReadinessState)>,
    pub tracked_actor_reindex: bool,
}

/// Authorized job status update used to reconcile bounded known job states.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JobStatusUpdate {
    pub id: String,
    pub title: String,
    pub state: String,
    /// Optional authoritative transition identity; defaults to canonical terminal state.
    pub transition_id: Option<String>,
}

#[derive(Debug, Default)]
pub struct Inbox {
    items: VecDeque<Item>,
    next_id: u64,
    scope: Option<u64>,
    link: Link,
    known_jobs: BTreeMap<String, String>,
    known_job_order: VecDeque<String>,
    checks: Vec<(String, ReadinessState)>,
    seen_job_events: VecDeque<String>,
    pub(crate) open: bool,
    expanded: BTreeSet<Topic>,
    selected: Option<u64>,
}

impl Inbox {
    #[cfg(test)]
    pub(crate) fn items(&self) -> impl Iterator<Item = &Item> {
        self.items.iter().rev()
    }

    pub(crate) fn unread(&self) -> usize {
        self.items.iter().filter(|item| !item.read).count()
    }

    fn groups(&self) -> Vec<Group<'_>> {
        let mut grouped = BTreeMap::new();
        for item in &self.items {
            grouped
                .entry(&item.topic)
                .or_insert_with(Vec::new)
                .push(item);
        }
        let mut groups: Vec<_> = grouped
            .into_iter()
            .map(|(topic, mut items)| {
                items.sort_by(|left, right| item_order(left, right));
                Group {
                    topic,
                    title: &items[items.len() - 1].group_title,
                    unread: items.iter().filter(|item| !item.read).count(),
                    items,
                }
            })
            .collect();
        groups.sort_by(|left, right| {
            let priority = (left.unread == 0).cmp(&(right.unread == 0));
            let oldest =
                |group: &Group<'_>| group.items.iter().position(|item| !item.read).unwrap_or(0);
            priority.then_with(|| item_order(left.items[oldest(left)], right.items[oldest(right)]))
        });
        groups
    }

    #[cfg(test)]
    pub(crate) fn active_known_job_ids(&self) -> Vec<String> {
        self.known_jobs
            .iter()
            .filter(|(_, state)| crate::backend::job_is_active(state))
            .map(|(id, _)| id.clone())
            .collect()
    }

    fn toggle_group(&mut self, topic: &Topic) {
        if !self.expanded.remove(topic) {
            self.expanded.insert(topic.clone());
        }
    }

    fn select(&mut self, id: u64) {
        self.selected = (self.selected != Some(id)).then_some(id);
    }

    fn mark_read(&mut self, id: u64) {
        if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
            item.read = true;
        }
    }

    pub(crate) fn close(&mut self) {
        self.open = false;
        self.selected = None;
    }

    pub(crate) fn mark_all_read(&mut self) {
        for item in &mut self.items {
            item.read = true;
        }
    }

    fn reset(&mut self) {
        self.items.clear();
        self.link = Link::Unknown;
        self.known_jobs.clear();
        self.known_job_order.clear();
        self.checks.clear();
        self.seen_job_events.clear();
        self.expanded.clear();
        self.selected = None;
    }

    fn record_known_job(&mut self, id: String, state: String) {
        if let Some(known_state) = self.known_jobs.get_mut(&id) {
            *known_state = state;
            return;
        }
        while self.known_jobs.len() >= MAX_TRACKED_JOBS {
            let evict_index = self
                .known_job_order
                .iter()
                .position(|known_id| {
                    self.known_jobs
                        .get(known_id)
                        .is_some_and(|known_state| !crate::backend::job_is_active(known_state))
                })
                .unwrap_or(0);
            if let Some(evicted) = self.known_job_order.remove(evict_index) {
                self.known_jobs.remove(&evicted);
            } else {
                break;
            }
        }
        self.known_job_order.push_back(id.clone());
        self.known_jobs.insert(id, state);
    }

    fn observe_job_update(
        &mut self,
        now: f64,
        id: String,
        title: &str,
        state: String,
        transition_id: Option<&str>,
    ) -> bool {
        let was_active = self
            .known_jobs
            .get(&id)
            .is_some_and(|previous| crate::backend::job_is_active(previous));
        let mut changed = false;
        if was_active && !crate::backend::job_is_active(&state) {
            let identity = transition_id
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_ascii_lowercase)
                .unwrap_or_else(|| describe_job_state(&state));
            let key = format!("{id}:{identity}");
            if !self.seen_job_events.contains(&key) {
                self.seen_job_events.push_back(key);
                while self.seen_job_events.len() > MAX_SEEN_JOB_EVENTS {
                    self.seen_job_events.pop_front();
                }
                let state_label = describe_job_state(&state);
                self.push(
                    Topic::Job(id.clone()),
                    title.to_owned(),
                    format!("Job {state_label}"),
                    format!("{title} is now {state_label}. Open Jobs & Logs for details."),
                    now,
                );
                changed = true;
            }
        }
        self.record_known_job(id, state);
        changed
    }

    /// Reconcile bounded known job states from authorized status updates without
    /// emitting a burst on initial hydration. Deduplicates by job + authoritative
    /// transition identity and ignores updates for a mismatched scope.
    #[cfg(test)]
    pub(crate) fn reconcile_job_statuses<I>(&mut self, now: f64, scope: u64, updates: I) -> bool
    where
        I: IntoIterator<Item = JobStatusUpdate>,
    {
        if self.scope != Some(scope) {
            return false;
        }
        let mut changed = false;
        for update in updates.into_iter().take(MAX_TRACKED_JOBS) {
            if self.observe_job_update(
                now,
                update.id,
                &update.title,
                update.state,
                update.transition_id.as_deref(),
            ) {
                changed = true;
            }
        }
        changed
    }

    fn push(&mut self, topic: Topic, group_title: String, title: String, detail: String, at: f64) {
        self.next_id += 1;
        self.items.push_back(Item {
            id: self.next_id,
            topic,
            group_title,
            title,
            detail,
            at,
            read: false,
        });
        while self.items.len() > MAX_ITEMS {
            if let Some(dropped) = self.items.pop_front() {
                if self.selected == Some(dropped.id) {
                    self.selected = None;
                }
                if !self.items.iter().any(|item| item.topic == dropped.topic) {
                    self.expanded.remove(&dropped.topic);
                }
            }
        }
    }

    /// Record the allowlisted transitions between the last observation and this one.
    /// The first observation after a reset only sets the baseline. Returns whether anything changed.
    pub(crate) fn observe(&mut self, now: f64, observation: Observation) -> bool {
        let mut changed = false;
        if self.scope != Some(observation.scope) {
            if self.scope.is_some() {
                changed = !self.items.is_empty();
            }
            self.reset();
            self.scope = Some(observation.scope);
            self.link = observation.link;
            for (id, _, state) in observation.jobs {
                self.record_known_job(id, state);
            }
            self.checks = observation.checks;
            return changed;
        }

        if observation.link != self.link {
            match observation.link {
                Link::Connected if self.link != Link::Unknown => {
                    self.push(
                        Topic::Connection,
                        "Connection".into(),
                        "Connected".into(),
                        "The connection to the backend is ready.".into(),
                        now,
                    );
                    changed = true;
                }
                Link::Failed => {
                    self.push(
                        Topic::Connection,
                        "Connection".into(),
                        "Connection failed".into(),
                        "The last connection attempt did not succeed. Open Setup Gateway for details."
                            .into(),
                        now,
                    );
                    changed = true;
                }
                _ => {}
            }
            self.link = observation.link;
        }

        for (id, title, state) in observation.jobs {
            if self.observe_job_update(now, id, &title, state, None) {
                changed = true;
            }
        }

        for (name, state) in &observation.checks {
            let Some((_, previous)) = self.checks.iter().find(|(known, _)| known == name) else {
                continue;
            };
            if previous == state {
                continue;
            }
            let actor_running = name == "Actor search" && *state == ReadinessState::Running;
            if actor_running && observation.tracked_actor_reindex {
                continue;
            }
            if actor_running || is_notable(*previous) || is_notable(*state) {
                let title = if actor_running {
                    crate::readiness_cards::ACTOR_INDEX_CATCH_UP.to_owned()
                } else {
                    format!("{name} is {}", readiness_word(*state))
                };
                self.push(
                    Topic::Readiness(name.clone()),
                    name.clone(),
                    title,
                    format!(
                        "{name} changed from {} to {}.",
                        readiness_word(*previous),
                        readiness_word(*state)
                    ),
                    now,
                );
                changed = true;
            }
        }
        self.checks = observation.checks;
        changed
    }
}

fn describe_job_state(state: &str) -> String {
    match state.to_ascii_lowercase().as_str() {
        "completed" | "succeeded" | "success" | "done" => "completed".into(),
        "failed" | "error" | "dead" => "failed".into(),
        "cancelled" | "canceled" => "cancelled".into(),
        other => other.to_owned(),
    }
}

/// A readiness change is worth an item when it enters or leaves a state a person acts on.
fn is_notable(state: ReadinessState) -> bool {
    matches!(
        state,
        ReadinessState::Ready
            | ReadinessState::NotReady
            | ReadinessState::Degraded
            | ReadinessState::Unavailable
    )
}

fn readiness_word(state: ReadinessState) -> &'static str {
    match state {
        ReadinessState::Ready => "ready",
        ReadinessState::NotReady => "not ready",
        ReadinessState::Running => "running",
        ReadinessState::Degraded => "degraded",
        ReadinessState::Unknown => "unknown",
        ReadinessState::Unavailable => "unavailable",
        ReadinessState::NotApplicable => "not applicable",
        _ => "changed",
    }
}

/// Split a camel-case id such as `ActorSearch` into `Actor search`.
fn humanize(id: &str) -> String {
    let mut out = String::new();
    for (index, ch) in id.chars().enumerate() {
        if ch.is_uppercase() && index > 0 {
            out.push(' ');
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

pub(crate) fn age_label(now: f64, at: f64) -> String {
    let seconds = (now - at).max(0.0) as u64;
    match seconds {
        0..=9 => "just now".into(),
        10..=59 => format!("{seconds}s ago"),
        60..=3599 => format!("{}m ago", seconds / 60),
        _ => format!("{}h ago", seconds / 3600),
    }
}

impl State {
    pub(crate) fn close_inbox(&mut self, tx: &mut Tx) {
        self.inbox.close();
        tx.focus("inbox-toggle", tx.focus_visible());
    }

    /// Populate the production drawer with synthetic topics for native visual review.
    pub fn seed_inbox_drawer_fixture(&mut self) {
        if self.fixture {
            seed_drawer(&mut self.inbox, self.clock);
        }
    }

    /// Fold what this app currently shows into the Inbox. Cheap, bounded, and read-only.
    pub(crate) fn observe_inbox(&mut self) -> bool {
        use crate::app::Load;
        use crate::connection::ConnectionStatus;
        if self.fixture {
            return false;
        }
        let link = match &self.connection.status {
            ConnectionStatus::Connected => Link::Connected,
            ConnectionStatus::Connecting => Link::Connecting,
            ConnectionStatus::Failed(_) => Link::Failed,
            ConnectionStatus::Disconnected => Link::Unknown,
        };
        let mut jobs = Vec::new();
        if self.registration.context.is_some() {
            let mut seen_ids = BTreeSet::new();
            if let Some(page) = &self.scoped_jobs.page {
                for row in page.rows.iter().take(MAX_TRACKED_JOBS) {
                    if seen_ids.insert(row.id.clone()) {
                        jobs.push((row.id.clone(), row.title.clone(), row.state.clone()));
                    }
                }
            }
            if let Load::Ready(detail) = &self.backend.detail {
                if jobs.len() < MAX_TRACKED_JOBS && seen_ids.insert(detail.job.id.clone()) {
                    jobs.push((
                        detail.job.id.clone(),
                        detail.job.title.clone(),
                        detail.job.state.clone(),
                    ));
                }
            }
        }
        let checks = match &self.readiness {
            Load::Ready(readiness) => readiness
                .checks
                .iter()
                .map(|check| (humanize(&format!("{:?}", check.id)), check.state))
                .collect(),
            _ => Vec::new(),
        };
        let now = self.clock;
        let tracked_actor_reindex = crate::views::active_actor_reindex(self).is_some()
            || crate::views::queued_actor_reindex_id(self).is_some();
        self.inbox.observe(
            now,
            Observation {
                scope: self.origin_revision,
                link,
                jobs,
                checks,
                tracked_actor_reindex,
            },
        )
    }
}

pub(crate) fn toolbar_item(state: &State, kit: &Kit) -> El {
    tray(state, kit, "inbox-toggle")
}

fn tray(state: &State, kit: &Kit, key: &'static str) -> El {
    let unread = state.inbox.unread();
    let label = if unread == 0 {
        "Inbox".to_owned()
    } else {
        format!("Inbox, {unread} unread")
    };
    rune_kit::Button::new(label.clone())
        .variant(rune_kit::ButtonVariant::Ghost)
        .icon(&icons::INBOX)
        .expanded(state.inbox.open)
        .label_with(move |font, _| {
            if unread == 0 {
                return rune::el();
            }
            rune::el()
                .key("inbox-unread-badge")
                .center()
                .fill(theme::hex(theme::ACCENT_HOVER))
                .radius(8.0)
                .px(5.0)
                .child(
                    rune::text(unread.to_string())
                        .font(font)
                        .color(theme::hex(theme::ON_ACCENT)),
                )
        })
        .on_press_tx(|state: &mut State, tx| {
            if state.inbox.open {
                state.close_inbox(tx);
            } else if !state.has_active_confirmation()
                && !state.settings_open
                && !state.commands.open
            {
                state.inbox.open = true;
            }
        })
        .build(kit)
        .key(key)
        .label(label)
}

pub(crate) fn drawer(
    state: &State,
    cx: &Cx<State>,
    kit: &Kit,
    preferences: crate::accessibility::ViewPreferences,
) -> Option<El> {
    if !state.inbox.open || state.has_active_confirmation() {
        return None;
    }
    let trigger = cx.layout_with("inbox-toggle", |rect| rect);
    let top = trigger.map_or(cx.titlebar_inset() + 50.0, |rect| rect[1] + rect[3] + 8.0);
    let boundary = theme::hex(theme::BORDER_SUBTLE);
    let panel = body(state, kit)
        .w(364.0)
        .max_w(364.0)
        .h_full()
        .min_h(0.0)
        .border_sides([1.0, 0.0, 0.0, 1.0], boundary)
        .fill(theme::hex(theme::SURFACE))
        .transition(crate::app::surface_transition(
            preferences.reduced_motion,
            rune::Transition::offset(8.0, 0.0),
        ))
        .instant_changes();

    let scrim = rune::el()
        .key("inbox-scrim")
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .fill(rune::Color {
            a: 0.4,
            ..theme::hex(theme::BACKGROUND)
        })
        .a11y_hidden();
    let scrim = if preferences.matte() {
        scrim
    } else {
        scrim.backdrop_blur(3.0)
    };
    let mut layer = rune::el()
        .key("inbox-layer")
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .role("dialog")
        .label("Inbox")
        .focus_return("inbox-toggle")
        .modal_tx(false, State::close_inbox)
        .child(scrim)
        .child(
            rune::col()
                .w_full()
                .h_full()
                .child(dismiss_area("inbox-backdrop-top").w_full().h(top))
                .child(
                    rune::row()
                        .key("inbox-panel-position")
                        .w_full()
                        .grow(1.0)
                        .min_h(0.0)
                        .child(dismiss_area("inbox-backdrop-left").grow(1.0).h_full())
                        .child(panel),
                ),
        );
    if let Some([x, y, width, height]) = trigger {
        // The modal makes the workspace inert. Keep its only shared control operable in place.
        layer = layer.child(
            rune::el()
                .key("inbox-modal-tray-position")
                .absolute(x, y)
                .size(width, height)
                .radius(5.0)
                .fill(theme::hex(theme::SURFACE))
                .child(
                    tray(state, kit, "inbox-toggle-close")
                        .w_full()
                        .h_full()
                        .tab_index(-1),
                ),
        );
    }
    Some(layer)
}

fn dismiss_area(key: &'static str) -> El {
    rune::el()
        .key(key)
        .a11y_hidden()
        .tab_index(-1)
        .on_press_tx(State::close_inbox)
}

fn body(state: &State, kit: &Kit) -> El {
    let inbox = &state.inbox;
    let close = rune_kit::Button::new("Close")
        .variant(rune_kit::ButtonVariant::Ghost)
        .on_press_tx(State::close_inbox)
        .build(kit)
        .key("inbox-close")
        .autofocus();
    let mark = rune_kit::Button::new("Mark all read")
        .variant(rune_kit::ButtonVariant::Ghost)
        .disabled(inbox.unread() == 0)
        .on_press_tx(|state: &mut State, tx| {
            state.inbox.mark_all_read();
            tx.focus("inbox-close", tx.focus_visible());
        })
        .build(kit)
        .key("inbox-mark-all-read");
    let mut rows = rune::col().key("inbox-list").gap(9.0);
    let groups = inbox.groups();
    if groups.is_empty() {
        rows = rows.child(
            rune::text("Job, readiness and connection changes appear here.")
                .font(kit.ty.small)
                .color(theme::hex(theme::TEXT_MUTED))
                .wrap(),
        );
    }
    for group in groups {
        rows = rows.child(group_view(&group, state, kit));
    }
    rune::col()
        .key("inbox-body")
        .child(
            rune::col()
                .key("inbox-header")
                .w_full()
                .px(14.0)
                .py(8.0)
                .gap(5.0)
                .border_sides([0.0, 0.0, 1.0, 0.0], theme::hex(theme::BORDER_SUBTLE))
                .child(
                    rune::row()
                        .gap(8.0)
                        .center_y()
                        .child(
                            rune::text("Inbox")
                                .font(kit.ty.body_medium)
                                .color(theme::hex(theme::TEXT_PRIMARY))
                                .grow(1.0)
                                .wrap(),
                        )
                        .child(mark)
                        .child(close),
                )
                .child(
                    rune::text("Only changes observed by this workstation.")
                        .key("inbox-helper")
                        .font(kit.ty.small)
                        .color(theme::hex(theme::TEXT_MUTED))
                        .wrap(),
                ),
        )
        .child(
            rune::col()
                .key("inbox-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                })
                .scroll_anchor()
                .child(rows.w_full().padding(12.0)),
        )
}

fn group_view(group: &Group<'_>, state: &State, kit: &Kit) -> El {
    let key = group.topic.key();
    let expanded = state.inbox.expanded.contains(group.topic);
    let toggle_key = format!("{key}-toggle");
    let group_toggle_key = toggle_key.clone();
    let topic = group.topic.clone();
    let latest = group.items[group.items.len() - 1];
    let count = format!("{} unread", group.unread);
    let label = format!(
        "{}, {}, {}",
        group.title,
        count,
        age_label(state.clock, latest.at)
    );
    let trigger = rune::row()
        .key(toggle_key)
        .role("button")
        .focusable(true)
        .label(label)
        .expanded(expanded)
        .w_full()
        .min_h(42.0)
        .px(10.0)
        .py(6.0)
        .gap(8.0)
        .center_y()
        .radius(5.0)
        .hover(theme::hex(theme::SURFACE_ELEVATED))
        .on_press_tx(move |state: &mut State, tx| {
            state.inbox.toggle_group(&topic);
            tx.keep_on_screen(group_toggle_key.clone());
        })
        .child(unread_dot(group.unread > 0))
        .child(
            rune::text(group.title.to_owned())
                .font(kit.ty.small_medium)
                .color(theme::hex(theme::TEXT_PRIMARY))
                .wrap()
                .min_w(0.0)
                .grow(1.0)
                .a11y_hidden(),
        )
        .child(
            rune::col()
                .gap(2.0)
                .a11y_hidden()
                .child(
                    rune::text(count)
                        .font(kit.ty.small_medium)
                        .color(theme::hex(if group.unread == 0 {
                            theme::TEXT_MUTED
                        } else {
                            theme::ACCENT_HOVER
                        })),
                )
                .child(
                    rune::text(age_label(state.clock, latest.at))
                        .font(kit.ty.small)
                        .color(theme::hex(theme::TEXT_MUTED)),
                ),
        )
        .child(
            rune::icon(
                if expanded {
                    &icons::CHEVRON_DOWN
                } else {
                    &icons::CHEVRON_RIGHT
                },
                12.0,
            )
            .color(theme::hex(theme::TEXT_SECONDARY))
            .a11y_hidden(),
        );
    let mut entry = rune::col()
        .key(key)
        .w_full()
        .radius(6.0)
        .fill(theme::hex(theme::SURFACE_RAISED))
        .border(1.0, theme::hex(theme::BORDER_SUBTLE))
        .child(trigger);
    if expanded {
        for item in &group.items {
            entry = entry.child(row(item, state, kit));
        }
    }
    entry
}

fn unread_dot(unread: bool) -> El {
    rune::el()
        .size(6.0, 6.0)
        .radius(3.0)
        .fill(if unread {
            theme::hex(theme::ACCENT_HOVER)
        } else {
            rune::Color::TRANSPARENT
        })
        .a11y_hidden()
}

fn row(item: &Item, state: &State, kit: &Kit) -> El {
    let id = item.id;
    let expanded = state.inbox.selected == Some(id);
    let row_key = format!("inbox-item-{id}-row");
    let toggle_key = row_key.clone();
    let age = age_label(state.clock, item.at);
    let mut entry = rune::col()
        .key(format!("inbox-item-{id}"))
        .w_full()
        .px(10.0)
        .border_sides([1.0, 0.0, 0.0, 0.0], theme::hex(theme::BORDER_SUBTLE))
        .child(
            rune::row()
                .key(row_key.clone())
                .role("button")
                .focusable(true)
                .label(format!(
                    "{}{}, {}",
                    if item.read { "" } else { "Unread, " },
                    item.title,
                    age
                ))
                .expanded(expanded)
                .w_full()
                .min_h(34.0)
                .px(8.0)
                .py(5.0)
                .gap(8.0)
                .center_y()
                .radius(5.0)
                .hover(theme::hex(theme::SURFACE_ELEVATED))
                .on_press_tx(move |state: &mut State, tx| {
                    state.inbox.select(id);
                    tx.keep_on_screen(toggle_key.clone());
                })
                .child(unread_dot(!item.read))
                .child(
                    rune::text(item.title.clone())
                        .font(if item.read {
                            kit.ty.small
                        } else {
                            kit.ty.small_medium
                        })
                        .color(theme::hex(theme::TEXT_PRIMARY))
                        .min_w(0.0)
                        .grow(1.0)
                        .wrap()
                        .a11y_hidden(),
                )
                .child(
                    rune::text(age)
                        .font(kit.ty.small)
                        .color(theme::hex(theme::TEXT_MUTED))
                        .a11y_hidden(),
                ),
        );
    if !expanded {
        return entry;
    }
    let mut actions = rune::row().gap(6.0);
    if !item.read {
        let topic = item.topic.clone();
        actions = actions.child(
            rune_kit::Button::new("Mark read")
                .variant(rune_kit::ButtonVariant::Ghost)
                .on_press_tx(move |state: &mut State, tx| {
                    let position = |inbox: &Inbox| {
                        inbox
                            .groups()
                            .iter()
                            .position(|group| group.topic == &topic)
                    };
                    let before = position(&state.inbox);
                    state.inbox.mark_read(id);
                    if position(&state.inbox) == before {
                        tx.focus(row_key.clone(), tx.focus_visible());
                        tx.keep_on_screen(row_key.clone());
                    } else {
                        // Keep the scroll viewport where the reader was when this group moves.
                        tx.focus("inbox-close", tx.focus_visible());
                    }
                })
                .build(kit)
                .key(format!("inbox-item-{id}-mark")),
        );
    }
    if let Some(label) = item.topic.action().filter(|_| {
        !matches!(item.topic, Topic::Job(_))
            || (state.connected() && state.registration.context.is_some())
    }) {
        let topic = item.topic.clone();
        actions = actions.child(
            rune_kit::Button::new(label)
                .variant(rune_kit::ButtonVariant::Ghost)
                .on_press_tx(move |state: &mut State, tx| {
                    state.close_inbox(tx);
                    match &topic {
                        Topic::Job(id) => {
                            state.setup_gateway = false;
                            state.open_job(id);
                        }
                        Topic::Readiness(name) => {
                            state.navigate(crate::app::Page::Workstation);
                            let check = match &state.readiness {
                                crate::app::Load::Ready(readiness) => readiness
                                    .checks
                                    .iter()
                                    .find(|check| humanize(&format!("{:?}", check.id)) == *name)
                                    .map(|check| check.id),
                                _ => None,
                            };
                            if let Some(id) = check {
                                state.reveal_readiness_check(id, tx);
                            }
                        }
                        Topic::Connection => {}
                    }
                })
                .build(kit)
                .key(format!("inbox-item-{id}-action")),
        );
    }
    entry = entry.child(
        rune::col()
            .key(format!("inbox-item-{id}-detail"))
            .gap(5.0)
            .px(12.0)
            .pb(6.0)
            .child(
                rune::text(item.detail.clone())
                    .font(kit.ty.small)
                    .color(theme::hex(theme::TEXT_SECONDARY))
                    .wrap(),
            )
            .child(actions),
    );
    entry
}

fn seed_drawer(inbox: &mut Inbox, now: f64) {
    let connection = Topic::Connection;
    let readiness = Topic::Readiness("Actor search".into());
    let job = Topic::Job("fixture-index-job".into());
    let job_title = "Scheduled maintenance for the synthetic regional actor catalogue";
    inbox.push(
        connection.clone(),
        "Connection".into(),
        "Connected".into(),
        "The connection to the backend is ready.".into(),
        now - 600.0,
    );
    inbox.mark_all_read();
    inbox.push(
        readiness.clone(),
        "Actor search".into(),
        crate::readiness_cards::ACTOR_INDEX_CATCH_UP.into(),
        "The index is warming up after backend startup. You can keep using the other workstation tools.".into(),
        now - 120.0,
    );
    inbox.push(
        job.clone(),
        job_title.into(),
        "Job completed".into(),
        format!("{job_title} is complete. Open Jobs & Logs for details."),
        now - 120.0,
    );
    inbox.push(
        readiness.clone(),
        "Actor search".into(),
        "Actor search is ready".into(),
        "Actor search changed from running to ready.".into(),
        now - 28.0,
    );
    inbox.expanded.extend([readiness, job]);
    inbox.selected = inbox.items.get(1).map(|item| item.id);
    inbox.open = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(scope: u64, link: Link) -> Observation {
        Observation {
            scope,
            link,
            ..Observation::default()
        }
    }

    fn job(id: &str, state: &str) -> (String, String, String) {
        (id.into(), format!("Title {id}"), state.into())
    }

    #[test]
    fn first_observation_is_only_a_baseline() {
        let mut inbox = Inbox::default();
        let mut first = obs(1, Link::Connected);
        first.jobs = vec![job("a", "completed")];
        first.checks = vec![("Jobs".into(), ReadinessState::NotReady)];
        assert!(!inbox.observe(1.0, first));
        assert_eq!(inbox.unread(), 0);
    }

    #[test]
    fn a_job_leaving_active_state_is_recorded_once() {
        let mut inbox = Inbox::default();
        let mut base = obs(1, Link::Connected);
        base.jobs = vec![job("a", "running")];
        inbox.observe(1.0, base.clone());
        let mut done = base.clone();
        done.jobs = vec![job("a", "completed")];
        assert!(inbox.observe(2.0, done.clone()));
        assert!(!inbox.observe(3.0, done));
        assert_eq!(inbox.unread(), 1);
        let item = inbox.items().next().expect("item");
        assert_eq!(item.title, "Job completed");
        assert!(item.detail.contains("Title a"));
        // The same job transition after a flap is not recorded twice.
        let mut again = base.clone();
        again.jobs = vec![job("a", "running")];
        inbox.observe(4.0, again);
        let mut finished = base;
        finished.jobs = vec![job("a", "completed")];
        assert!(!inbox.observe(5.0, finished));
        assert_eq!(inbox.items().count(), 1);
    }

    #[test]
    fn a_job_first_seen_finished_is_not_announced() {
        let mut inbox = Inbox::default();
        inbox.observe(1.0, obs(1, Link::Connected));
        let mut later = obs(1, Link::Connected);
        later.jobs = vec![job("old", "failed")];
        assert!(!inbox.observe(2.0, later));
    }

    #[test]
    fn connection_and_readiness_changes_use_fixed_text_only() {
        let mut inbox = Inbox::default();
        let mut base = obs(1, Link::Connecting);
        base.checks = vec![("Actor search".into(), ReadinessState::Ready)];
        inbox.observe(1.0, base);
        let mut next = obs(1, Link::Connected);
        next.checks = vec![("Actor search".into(), ReadinessState::Degraded)];
        assert!(inbox.observe(2.0, next));
        let titles: Vec<_> = inbox.items().map(|item| item.title.clone()).collect();
        assert_eq!(titles, ["Actor search is degraded", "Connected"]);
        let failed = obs(1, Link::Failed);
        inbox.observe(3.0, failed);
        let latest = inbox.items().next().expect("item");
        assert_eq!(latest.title, "Connection failed");
        assert!(!latest.detail.contains("http"));
    }

    #[test]
    fn running_and_unknown_readiness_flaps_stay_quiet() {
        let mut inbox = Inbox::default();
        let mut base = obs(1, Link::Connected);
        base.checks = vec![("Jobs".into(), ReadinessState::Unknown)];
        inbox.observe(1.0, base);
        let mut next = obs(1, Link::Connected);
        next.checks = vec![("Jobs".into(), ReadinessState::Running)];
        assert!(!inbox.observe(2.0, next));
    }

    #[test]
    fn a_new_authority_clears_items_and_selection() {
        let mut inbox = Inbox::default();
        let mut base = obs(1, Link::Connected);
        base.jobs = vec![job("a", "running")];
        inbox.observe(1.0, base.clone());
        let mut done = base.clone();
        done.jobs = vec![job("a", "completed")];
        inbox.observe(2.0, done);
        let id = inbox.items().next().expect("item").id;
        inbox.select(id);
        assert!(inbox.selected.is_some());
        let mut other = obs(2, Link::Connected);
        other.jobs = vec![job("a", "running")];
        assert!(inbox.observe(3.0, other));
        assert_eq!(inbox.items().count(), 0);
        assert!(inbox.selected.is_none());
    }

    #[test]
    fn retention_is_capped_and_oldest_goes_first() {
        let mut inbox = Inbox::default();
        let mut base = obs(1, Link::Connected);
        base.jobs = (0..MAX_ITEMS + 10)
            .map(|n| job(&n.to_string(), "running"))
            .collect();
        inbox.observe(0.0, base.clone());
        let mut done = base;
        done.jobs = (0..MAX_ITEMS + 10)
            .map(|n| job(&n.to_string(), "completed"))
            .collect();
        inbox.observe(1.0, done);
        assert_eq!(inbox.items().count(), MAX_ITEMS);
        assert!(inbox.items().all(|item| !item.detail.contains("Title 0 ")));
    }

    #[test]
    fn groups_use_oldest_unread_then_insertion_order_with_chronological_rows() {
        let mut inbox = Inbox::default();
        seed_drawer(&mut inbox, 900.0);
        let actor = Topic::Readiness("Actor search".into());
        let job = Topic::Job("fixture-index-job".into());

        let groups = inbox.groups();
        assert_eq!(
            groups.iter().map(|group| group.topic).collect::<Vec<_>>(),
            [&actor, &job, &Topic::Connection],
        );
        assert_eq!(groups[0].unread, 2);
        let [earliest, latest] = groups[0].items.as_slice() else {
            panic!("two ordered actor notifications");
        };
        assert!(earliest.at < latest.at);
        assert_eq!(earliest.at, groups[1].items[0].at);
        let earliest_id = earliest.id;
        inbox.mark_read(earliest_id);
        inbox.mark_read(earliest_id);
        assert_eq!(inbox.unread(), 2);
        let groups = inbox.groups();
        assert_eq!(groups[0].topic, &job);
        assert_eq!(groups[1].topic, &actor);
        assert_eq!(groups[1].unread, 1);

        inbox.mark_all_read();
        assert_eq!(inbox.unread(), 0);
        assert_eq!(inbox.groups()[0].topic, &Topic::Connection);
    }

    #[test]
    fn background_actor_progress_is_a_readiness_item_only_when_untracked() {
        for tracked in [false, true] {
            let mut inbox = Inbox::default();
            let mut base = obs(1, Link::Connected);
            base.checks = vec![("Actor search".into(), ReadinessState::Unknown)];
            inbox.observe(0.0, base.clone());

            base.tracked_actor_reindex = tracked;
            base.checks[0].1 = ReadinessState::Running;
            assert_eq!(inbox.observe(1.0, base), !tracked);
            if tracked {
                assert_eq!(inbox.unread(), 0);
            } else {
                let item = inbox.items().next().expect("actor readiness item");
                assert_eq!(item.topic, Topic::Readiness("Actor search".into()));
                assert_eq!(item.topic.action(), Some("View status"));
                assert_eq!(item.title, crate::readiness_cards::ACTOR_INDEX_CATCH_UP);
            }
        }
    }

    #[test]
    fn known_jobs_survive_pagination_and_reconcile_without_initial_hydration_burst() {
        let mut inbox = Inbox::default();
        let mut head = obs(1, Link::Connected);
        head.jobs = vec![job("j1", "running"), job("j0", "completed")];
        assert!(!inbox.observe(1.0, head));
        assert_eq!(inbox.items().count(), 0);

        // Pagination/filtering hides j1 from the current head page; j1 must stay remembered.
        let mut filtered_page = obs(1, Link::Connected);
        filtered_page.jobs = vec![job("j2", "completed"), job("j3", "failed")];
        assert!(
            !inbox.observe(2.0, filtered_page),
            "initial observation of terminal jobs on another page must not emit a burst"
        );
        assert!(inbox.active_known_job_ids().contains(&"j1".to_owned()));

        // Reconciling j1 from an authorized status update records its completion once.
        assert!(inbox.reconcile_job_statuses(
            3.0,
            1,
            [JobStatusUpdate {
                id: "j1".into(),
                title: "Title j1".into(),
                state: "completed".into(),
                transition_id: Some("rev-9".into()),
            }],
        ));
        assert_eq!(inbox.items().count(), 1);
        assert_eq!(inbox.items().next().expect("item").title, "Job completed");

        // Re-delivering the same job + authoritative transition identity is deduplicated.
        assert!(!inbox.reconcile_job_statuses(
            4.0,
            1,
            [JobStatusUpdate {
                id: "j1".into(),
                title: "Title j1".into(),
                state: "completed".into(),
                transition_id: Some("rev-9".into()),
            }],
        ));
        assert_eq!(inbox.items().count(), 1);

        // Mismatched scope updates are ignored.
        assert!(!inbox.reconcile_job_statuses(
            5.0,
            2,
            [JobStatusUpdate {
                id: "j4".into(),
                title: "Title j4".into(),
                state: "failed".into(),
                transition_id: None,
            }],
        ));
    }

    #[test]
    fn retry_scheduled_transition_does_not_emit_false_terminal_item() {
        let mut inbox = Inbox::default();
        let mut active = obs(1, Link::Connected);
        active.jobs = vec![job("j1", "running")];
        inbox.observe(1.0, active);

        let mut retrying = obs(1, Link::Connected);
        retrying.jobs = vec![job("j1", "retry_scheduled")];
        assert!(
            !inbox.observe(2.0, retrying),
            "active -> retry_scheduled is nonterminal and must not produce an Inbox item"
        );
        assert_eq!(inbox.items().count(), 0);

        let mut failed = obs(1, Link::Connected);
        failed.jobs = vec![job("j1", "failed")];
        assert!(inbox.observe(3.0, failed));
        assert_eq!(inbox.items().count(), 1);
        assert_eq!(inbox.items().next().expect("item").title, "Job failed");
    }

    #[test]
    fn humanizes_check_ids_and_ages() {
        assert_eq!(humanize("ActorSearch"), "Actor search");
        assert_eq!(humanize("Jobs"), "Jobs");
        assert_eq!(age_label(100.0, 99.0), "just now");
        assert_eq!(age_label(100.0, 70.0), "30s ago");
        assert_eq!(age_label(400.0, 100.0), "5m ago");
        assert_eq!(age_label(8000.0, 0.0), "2h ago");
    }

    fn runtime(state: State, size: (f32, f32)) -> rune::Runtime<State> {
        let mut runtime = rune::Runtime::new(
            state,
            crate::app::view,
            size,
            rune::Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        runtime.frame(0.0);
        runtime.frame(0.25);
        runtime.frame(0.5);
        runtime
    }

    fn click(runtime: &mut rune::Runtime<State>, key: &str, now: f64) {
        let (x, y) = runtime.find(key).unwrap_or_else(|| panic!("visible {key}"));
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(now);
    }

    fn focused(runtime: &rune::Runtime<State>, key: &str) -> bool {
        runtime.elements().into_iter().any(|element| {
            element.key.as_deref() == Some(key) && element.focused && !element.exiting
        })
    }

    #[test]
    fn modal_drawer_has_four_close_paths_and_preserves_the_workspace_at_minimum_size() {
        let mut state = crate::tests::registered_fixture();
        state.page = crate::app::Page::Workstation;
        state.assistant.enabled = true;
        state.assistant.preference = crate::assistant::Preference::Open;
        state.inbox.observe(0.0, obs(1, Link::Connecting));
        state.inbox.observe(1.0, obs(1, Link::Connected));
        let mut runtime = runtime(state, (640.0, 480.0));

        for (step, close) in [
            "inbox-toggle-close",
            "inbox-close",
            "Escape",
            "inbox-backdrop-left",
        ]
        .into_iter()
        .enumerate()
        {
            let now = 1.0 + step as f64 * 2.0;
            click(&mut runtime, "inbox-toggle", now);
            runtime.frame(now + 0.3);
            assert_eq!(runtime.state.inbox.unread(), 1);
            assert!(focused(&runtime, "inbox-close"));
            assert!(runtime.find("assistant-dock").is_some());
            assert!(runtime.find("screen-workstation").is_some());
            assert!(runtime.find("inbox-filter").is_none());
            let (panel, _) = runtime.presented("inbox-body").expect("drawer geometry");
            let (tray, _) = runtime.presented("inbox-toggle").expect("tray geometry");
            assert!(panel[0] >= 0.0 && (panel[0] + panel[2] - 640.0).abs() < 0.5);
            assert!(panel[1] >= tray[1] + tray[3]);
            assert!((panel[1] + panel[3] - 480.0).abs() < 0.5);
            for key in [
                "inbox-close",
                "inbox-mark-all-read",
                "inbox-helper",
                "inbox-scroll",
            ] {
                let (rect, _) = runtime.presented(key).expect(key);
                assert!(
                    rect[0] >= panel[0] && rect[0] + rect[2] <= 640.5,
                    "{key}: {rect:?}"
                );
                assert!(
                    rect[1] >= panel[1] && rect[1] + rect[3] <= 480.5,
                    "{key}: {rect:?}"
                );
            }
            let mut reached_group = false;
            for seq in 0..5 {
                runtime.key(rune::KeyEvent {
                    key: rune::Key::Tab,
                    mods: rune::Mods::default(),
                });
                runtime.frame(now + 0.4 + f64::from(seq) * 0.01);
                if focused(&runtime, "inbox-group-connection-toggle") {
                    reached_group = true;
                    if step == 0 {
                        runtime.key(rune::KeyEvent {
                            key: rune::Key::Text(" ".into()),
                            mods: rune::Mods::default(),
                        });
                        runtime.frame(now + 0.45 + f64::from(seq) * 0.01);
                        assert!(runtime.find("inbox-item-1-row").is_some());
                        break;
                    }
                }
                assert!(runtime
                    .elements()
                    .into_iter()
                    .any(|element| { element.focused && element.path.contains("inbox-body") }));
            }
            assert!(reached_group);
            if close == "Escape" {
                runtime.key(rune::KeyEvent {
                    key: rune::Key::Escape,
                    mods: rune::Mods::default(),
                });
                runtime.frame(now + 0.6);
            } else {
                click(&mut runtime, close, now + 0.6);
            }
            runtime.frame(now + 0.9);
            assert!(!runtime.state.inbox.open, "{close}");
            assert!(runtime.find("inbox-body").is_none(), "{close}");
            assert!(focused(&runtime, "inbox-toggle"), "{close}");
            assert_eq!(
                runtime.state.assistant.preference,
                crate::assistant::Preference::Open
            );
            assert_eq!(runtime.state.page, crate::app::Page::Workstation);
        }
    }

    #[test]
    fn drawer_view_status_reveals_the_matching_readiness_disclosure() {
        use fleetworks_contracts::readiness::ReadinessCheck;
        use fleetworks_contracts::readiness::ReadinessCheckId;
        use fleetworks_contracts::readiness::WorkstationReadiness;

        let mut state = crate::tests::registered_fixture();
        state.page = crate::app::Page::Tools;
        let stamp = "2026-01-01T00:00:00Z";
        let mut check = ReadinessCheck::new(
            ReadinessCheckId::ActorSearch,
            ReadinessState::Running,
            "index_catching_up".into(),
            "Synthetic actor index catch-up".into(),
            stamp.into(),
        );
        check.fresh_for_ms = 30_000;
        state.readiness = crate::app::Load::Ready(WorkstationReadiness::new(
            "synthetic-deployment".into(),
            stamp.into(),
            vec![check],
        ));
        state.readiness_received_at = Some(0.0);
        seed_drawer(&mut state.inbox, 900.0);
        let mut runtime = runtime(state, (640.0, 480.0));
        click(&mut runtime, "inbox-item-2-action", 1.0);
        runtime.frame(1.3);

        assert_eq!(runtime.state.page, crate::app::Page::Workstation);
        assert!(!runtime.state.inbox.open);
        assert!(runtime.state.workstation_status_checks_open);
        assert!(runtime.find("readiness-details-actor_search").is_some());
        assert!(runtime.elements().iter().any(|element| {
            element.key.as_deref() == Some("readiness-trigger-actor_search") && element.focused
        }));
        assert_eq!(runtime.state.inbox.unread(), 3);
    }

    #[test]
    fn drawer_actions_update_badges_and_keep_the_selected_notification_on_screen() {
        let mut state = crate::tests::registered_fixture();
        seed_drawer(&mut state.inbox, 900.0);
        let mut runtime = runtime(state, (640.0, 480.0));
        let row = "inbox-item-2-row";
        let later_row = "inbox-item-4-row";
        let mark = "inbox-item-2-mark";
        let status = "inbox-item-2-action";
        let group = "inbox-group-job-fixture-index-job-toggle";
        let label = |runtime: &rune::Runtime<State>, key: &str| {
            runtime
                .elements()
                .into_iter()
                .find(|element| element.key.as_deref() == Some(key))
                .and_then(|element| element.label)
                .unwrap_or_else(|| panic!("accessible {key}"))
        };
        assert_eq!(label(&runtime, "inbox-toggle-close"), "Inbox, 3 unread");
        assert_eq!(label(&runtime, status), "View status");
        assert!(label(&runtime, group).contains("1 unread"));

        click(&mut runtime, mark, 1.0);
        runtime.frame(1.3);
        assert_eq!(label(&runtime, "inbox-toggle-close"), "Inbox, 2 unread");
        assert!(focused(&runtime, "inbox-close"));
        assert_eq!(
            runtime.state.inbox.groups()[0].topic,
            &Topic::Job("fixture-index-job".into())
        );
        assert!(runtime.find("inbox-item-2-detail").is_some());
        assert!(runtime.find(mark).is_none());

        runtime.update(|state, tx| {
            tx.focus(row, true);
            state.inbox.push(
                Topic::Connection,
                "Connection".into(),
                "Connected".into(),
                "The connection to the backend is ready.".into(),
                901.0,
            );
        });
        runtime.frame(1.5);
        assert_eq!(label(&runtime, "inbox-toggle-close"), "Inbox, 3 unread");
        assert!(focused(&runtime, row));
        assert!(runtime.find("inbox-item-2-detail").is_some());

        click(&mut runtime, later_row, 1.6);
        click(&mut runtime, "inbox-item-4-mark", 1.7);
        assert!(focused(&runtime, "inbox-close"));
        click(&mut runtime, "inbox-mark-all-read", 1.8);
        assert_eq!(label(&runtime, "inbox-toggle-close"), "Inbox");
        assert_eq!(runtime.state.inbox.unread(), 0);
        assert!(focused(&runtime, "inbox-close"));
    }
}
