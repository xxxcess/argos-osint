//! Native Inbox: a bell popover over allowlisted transitions this app already observed.
//!
//! It is not a notification service. Nothing is pushed, nothing leaves this process, and it makes
//! no claim about device-wide alerts. Items are built from fixed text plus a job title or a
//! readiness check name, never from error text, URLs or credentials. The whole list resets when
//! the connection authority changes.

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

#[derive(Debug, Default)]
pub struct Inbox {
    items: VecDeque<Item>,
    next_id: u64,
    scope: Option<u64>,
    link: Link,
    jobs: Vec<(String, String)>,
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
            .filter(|item| self.filter == Filter::All || !item.read)
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
        self.jobs.clear();
        self.checks.clear();
        self.seen_job_events.clear();
        self.selected = None;
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
            self.jobs = observation
                .jobs
                .into_iter()
                .map(|(id, _, state)| (id, state))
                .collect();
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

        for (id, title, state) in &observation.jobs {
            let previous = self.jobs.iter().find(|(known, _)| known == id);
            let was_active =
                previous.is_some_and(|(_, state)| crate::backend::job_is_active(state));
            if was_active && !crate::backend::job_is_active(state) {
                let key = format!("{id}:{}", state.to_ascii_lowercase());
                if !self.seen_job_events.contains(&key) {
                    self.seen_job_events.push_back(key);
                    while self.seen_job_events.len() > MAX_SEEN_JOB_EVENTS {
                        self.seen_job_events.pop_front();
                    }
                    self.push(
                        Kind::Job,
                        format!("Job {}", describe_job_state(state)),
                        format!(
                            "{title} is now {}. Open Jobs & Logs for details.",
                            describe_job_state(state)
                        ),
                        now,
                    );
                    changed = true;
                }
            }
        }
        self.jobs = observation
            .jobs
            .into_iter()
            .map(|(id, _, state)| (id, state))
            .collect();

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
        let jobs = self
            .scoped_jobs
            .page
            .as_ref()
            .filter(|_| self.registration.context.is_some())
            .map(|page| {
                page.rows
                    .iter()
                    .take(100)
                    .map(|row| (row.id.clone(), row.title.clone(), row.state.clone()))
                    .collect()
            })
            .unwrap_or_default();
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
        .description("Changes this app observed. Nothing is sent to your device.")
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
            rune::row()
                .w_full()
                .px(12.0)
                .py(8.0)
                .gap(8.0)
                .center_y()
                .child(filter)
                .child(rune::spacer())
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
        assert_eq!(inbox.visible().len(), 1);
        inbox.select(id);
        assert!(inbox.selected.is_none());
        inbox.mark_all_read();
        assert_eq!(inbox.unread(), 0);
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
}
