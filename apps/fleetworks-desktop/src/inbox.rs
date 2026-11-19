//! Native Inbox: a bell popover over allowlisted transitions this app already observed.
//!
//! It is not a notification service. Nothing is pushed, nothing leaves this process, and it makes
//! no claim about device-wide alerts. Items are built from fixed text plus a job title or a
//! readiness check name, never from error text, URLs or credentials. The whole list resets when
//! the connection authority changes.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::VecDeque;

use fleetworks_contracts::readiness::ReadinessState;
use rune::El;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Filter {
    #[default]
    All,
    Unread,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Connection,
    Job,
    Readiness,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Self::Connection => "Connection",
            Self::Job => "Job",
            Self::Readiness => "Readiness",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Item {
    pub id: u64,
    pub kind: Kind,
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
    pub(crate) filter: Filter,
    pub(crate) selected: Option<u64>,
}

impl Inbox {
    pub(crate) fn items(&self) -> impl Iterator<Item = &Item> {
        self.items.iter().rev()
    }

    pub(crate) fn unread(&self) -> usize {
        self.items.iter().filter(|item| !item.read).count()
    }

    pub(crate) fn visible(&self) -> Vec<&Item> {
        self.items()
            .filter(|item| {
                self.filter == Filter::All || !item.read || self.selected == Some(item.id)
            })
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn active_known_job_ids(&self) -> Vec<String> {
        self.known_jobs
            .iter()
            .filter(|(_, state)| crate::backend::job_is_active(state))
            .map(|(id, _)| id.clone())
            .collect()
    }

    pub(crate) fn select(&mut self, id: u64) {
        if self.selected == Some(id) {
            self.selected = None;
            return;
        }
        if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
            item.read = true;
            self.selected = Some(id);
        }
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
                    Kind::Job,
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

    fn push(&mut self, kind: Kind, title: String, detail: String, at: f64) {
        self.next_id += 1;
        self.items.push_back(Item {
            id: self.next_id,
            kind,
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
                        Kind::Connection,
                        "Connected".into(),
                        "The connection to the backend is ready.".into(),
                        now,
                    );
                    changed = true;
                }
                Link::Failed => {
                    self.push(
                        Kind::Connection,
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
            if previous != state && (is_notable(*previous) || is_notable(*state)) {
                self.push(
                    Kind::Readiness,
                    format!("{name} is {}", readiness_word(*state)),
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
        self.inbox.observe(
            now,
            Observation {
                scope: self.origin_revision,
                link,
                jobs,
                checks,
            },
        )
    }
}

/// The bell with its unread count, as the popover trigger.
fn bell(inbox: &Inbox, kit: &Kit) -> El {
    let unread = inbox.unread();
    let label = if unread == 0 {
        "Inbox".to_owned()
    } else {
        format!("Inbox, {unread} unread")
    };
    let button = rune_kit::Button::new(if unread == 0 {
        String::new()
    } else if unread > 9 {
        "9+".to_owned()
    } else {
        unread.to_string()
    })
    .variant(rune_kit::ButtonVariant::Ghost)
    .icon(&icons::BELL)
    .on_press(|state: &mut State| {
        state.inbox.open = !state.inbox.open;
    });
    button.build(kit).key("inbox-toggle").label(label)
}

/// The toolbar bell and its popover. The popover is pushed to `layers`.
pub(crate) fn toolbar_item(state: &State, kit: &Kit, layers: &mut Vec<El>) -> El {
    let inbox = &state.inbox;
    let popover = rune_kit::Popover::new("inbox")
        .trigger(bell(inbox, kit))
        .title("Inbox")
        .label("Inbox")
        .placement(rune_kit::Placement::BottomEnd)
        .open(inbox.open)
        .padding(0.0)
        .on_close(|state: &mut State| {
            state.inbox.open = false;
            state.inbox.selected = None;
        })
        .child(body(state, kit));
    popover.build(kit, layers)
}

fn body(state: &State, kit: &Kit) -> El {
    let inbox = &state.inbox;
    let visible = inbox.visible();
    let filter = rune_kit::SegmentedControl::new(
        &["All", "Unread"],
        Some(match inbox.filter {
            Filter::All => 0,
            Filter::Unread => 1,
        }),
    )
    .label("Inbox filter")
    .on_pick(|state: &mut State, index, _| {
        state.inbox.filter = if index == 1 {
            Filter::Unread
        } else {
            Filter::All
        };
    })
    .build(kit)
    .key("inbox-filter");
    let mark = rune_kit::Button::new("Mark all read")
        .variant(rune_kit::ButtonVariant::Ghost)
        .disabled(inbox.unread() == 0)
        .on_press(|state: &mut State| state.inbox.mark_all_read())
        .build(kit)
        .key("inbox-mark-all-read");
    let mut rows = rune::col().key("inbox-list").w_full().gap(2.0);
    if visible.is_empty() {
        rows = rows.child(
            rune::text(match inbox.filter {
                Filter::All => "Nothing yet. Job, readiness and connection changes appear here.",
                Filter::Unread => "No unread items.",
            })
            .font(kit.ty.small)
            .color(theme::hex(theme::TEXT_MUTED))
            .wrap()
            .padding(12.0),
        );
    }
    for item in visible {
        rows = rows.child(row(item, inbox.selected == Some(item.id), state.clock, kit));
    }
    rune::col()
        .key("inbox-body")
        .w(320.0)
        .min_h(0.0)
        .child(
            // The helper wraps inside the popover instead of running past its edge.
            rune::col().w_full().px(12.0).pt(10.0).child(
                rune::text("Changes this app observed. Nothing is sent to your device.")
                    .key("inbox-helper")
                    .font(kit.ty.small)
                    .color(theme::hex(theme::TEXT_MUTED))
                    .wrap()
                    .min_w(0.0),
            ),
        )
        .child(
            // Filter and action stack, so the action stays inside the popover at any text width.
            rune::col()
                .w_full()
                .px(12.0)
                .py(8.0)
                .gap(6.0)
                .child(filter)
                .child(mark),
        )
        .child(rune_kit::popover::fit_scroll(rows))
}

fn row(item: &Item, expanded: bool, now: f64, kit: &Kit) -> El {
    let id = item.id;
    let mut header = rune::row()
        .w_full()
        .gap(8.0)
        .center_y()
        .child(
            rune::text(if item.read { " " } else { "●" })
                .font(kit.ty.small)
                .color(theme::hex(theme::ACCENT_HOVER))
                .a11y_hidden(),
        )
        .child(
            rune::col()
                .grow(1.0)
                .min_w(0.0)
                .child(
                    rune::text(item.title.clone())
                        .font(if item.read {
                            kit.ty.small
                        } else {
                            kit.ty.small_medium
                        })
                        .color(theme::hex(theme::TEXT_PRIMARY))
                        .wrap(),
                )
                .child(
                    rune::text(format!(
                        "{} · {}",
                        item.kind.label(),
                        age_label(now, item.at)
                    ))
                    .font(kit.ty.small)
                    .color(theme::hex(theme::TEXT_MUTED)),
                ),
        );
    header = header.padding(8.0);
    let mut entry = rune::col().key(format!("inbox-item-{id}")).w_full().child(
        rune::row()
            .key(format!("inbox-item-{id}-row"))
            .w_full()
            .on_press(move |state: &mut State| state.inbox.select(id))
            .child(header),
    );
    if expanded {
        entry = entry.child(
            rune::text(item.detail.clone())
                .key(format!("inbox-item-{id}-detail"))
                .font(kit.ty.small)
                .color(theme::hex(theme::TEXT_SECONDARY))
                .wrap()
                .px(24.0)
                .pb(10.0),
        );
    }
    entry
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
    fn selecting_marks_read_and_filters_follow() {
        let mut inbox = Inbox::default();
        inbox.observe(0.0, obs(1, Link::Connecting));
        inbox.observe(1.0, obs(1, Link::Failed));
        inbox.observe(2.0, obs(1, Link::Connected));
        assert_eq!(inbox.unread(), 2);
        let id = inbox.items().next().expect("item").id;
        inbox.select(id);
        assert_eq!(inbox.unread(), 1);
        inbox.filter = Filter::Unread;
        // The expanded selected row stays visible in Unread until collapsed.
        assert_eq!(inbox.visible().len(), 2);
        inbox.select(id);
        assert!(inbox.selected.is_none());
        assert_eq!(inbox.visible().len(), 1);
        inbox.mark_all_read();
        assert_eq!(inbox.unread(), 0);
    }

    #[test]
    fn unread_filter_keeps_selected_row_visible_until_collapse_and_decrements_once() {
        let mut state = crate::tests::registered_fixture();
        state.inbox.observe(0.0, obs(1, Link::Connecting));
        state.inbox.observe(1.0, obs(1, Link::Connected));
        state.inbox.filter = Filter::Unread;
        state.inbox.open = true;
        let id = state.inbox.items().next().expect("item").id;

        let mut runtime = rune::Runtime::new(
            state,
            crate::app::view,
            (1180.0, 800.0),
            rune::Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        runtime.frame(0.0);
        runtime.frame(1.0 / 60.0);
        assert_eq!(runtime.state.inbox.unread(), 1);
        assert!(runtime.find(&format!("inbox-item-{id}")).is_some());

        runtime.update(|state, _| state.inbox.select(id));
        runtime.frame(2.0 / 60.0);
        assert_eq!(runtime.state.inbox.unread(), 0);
        assert!(
            runtime.find(&format!("inbox-item-{id}")).is_some(),
            "expanded row stays visible in Unread filter until collapsed"
        );
        assert!(
            runtime.find(&format!("inbox-item-{id}-detail")).is_some(),
            "expanded detail renders in Unread filter"
        );

        // Collapsing removes the read item from Unread and does not underflow unread count.
        runtime.update(|state, _| state.inbox.select(id));
        runtime.frame(3.0 / 60.0);
        assert_eq!(runtime.state.inbox.unread(), 0);
        assert!(runtime.find(&format!("inbox-item-{id}")).is_none());
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

    #[test]
    fn real_view_shows_the_bell_then_the_list_with_expandable_detail() {
        let mut state = crate::tests::registered_fixture();
        state.inbox.observe(0.0, obs(1, Link::Connecting));
        state.inbox.observe(1.0, obs(1, Link::Connected));
        let id = state.inbox.items().next().expect("item").id;
        let mut runtime = rune::Runtime::new(
            state,
            crate::app::view,
            (1180.0, 800.0),
            rune::Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        runtime.frame(0.0);
        assert!(runtime.find("inbox-toggle").is_some());
        assert!(runtime.find("inbox-body").is_none());

        runtime.update(|state, _| state.inbox.open = true);
        runtime.frame(1.0 / 60.0);
        runtime.frame(2.0 / 60.0);
        assert!(runtime.find("inbox-body").is_some());
        assert!(runtime.find(&format!("inbox-item-{id}")).is_some());
        assert!(runtime.find(&format!("inbox-item-{id}-detail")).is_none());
        assert_eq!(runtime.state.inbox.unread(), 1);

        runtime.update(|state, _| state.inbox.select(id));
        runtime.frame(3.0 / 60.0);
        assert!(runtime.find(&format!("inbox-item-{id}-detail")).is_some());
        assert_eq!(runtime.state.inbox.unread(), 0);
    }

    #[test]
    fn the_popover_helper_and_mark_all_read_stay_inside_it_at_wide_and_compact_sizes() {
        for size in [(1280.0, 800.0), (640.0, 480.0)] {
            let mut state = crate::tests::registered_fixture();
            state.inbox.observe(0.0, obs(1, Link::Connecting));
            state.inbox.observe(1.0, obs(1, Link::Connected));
            state.inbox.open = true;
            let mut runtime = rune::Runtime::new(
                state,
                crate::app::view,
                size,
                rune::Env {
                    system_dark: true,
                    scale: 1.0,
                },
            );
            for frame in 0..4 {
                runtime.frame(f64::from(frame) / 60.0);
            }
            let rect = |key: &str| {
                runtime
                    .elements()
                    .into_iter()
                    .find(|element| element.key.as_deref() == Some(key))
                    .unwrap_or_else(|| panic!("{key} at {size:?}"))
                    .rect
            };
            let (body, helper, mark) = (
                rect("inbox-body"),
                rect("inbox-helper"),
                rect("inbox-mark-all-read"),
            );
            let inside = |inner: [f32; 4]| {
                inner[0] >= body[0] - 0.5
                    && inner[0] + inner[2] <= body[0] + body[2] + 0.5
                    && inner[1] >= body[1] - 0.5
                    && inner[1] + inner[3] <= body[1] + body[3] + 0.5
            };
            assert!(
                inside(helper),
                "helper {helper:?} outside body {body:?} at {size:?}"
            );
            assert!(
                inside(mark),
                "action {mark:?} outside body {body:?} at {size:?}"
            );
            assert!(
                body[0] >= 0.0 && body[0] + body[2] <= size.0,
                "popover {body:?} leaves the {size:?} window"
            );
        }
    }
}
