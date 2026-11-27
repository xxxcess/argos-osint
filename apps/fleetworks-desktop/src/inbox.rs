//! Native Inbox: a drawer for backend answers and allowlisted workstation notifications.
//!
//! Notification items come from fixed text, job titles and readiness check names. The list
//! resets when the connection authority changes.

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::VecDeque;

use fleetworks_contracts::chat::ChatSourceKind;
use fleetworks_contracts::chat::ChatSourceReference;
use fleetworks_contracts::chat::QaInboxNotificationGroup;
use fleetworks_contracts::chat::QaInboxNotificationSeverity;
use fleetworks_contracts::chat::QaInboxNotificationTopic;
use fleetworks_contracts::chat::QA_MAX_INBOX_NOTIFICATION_GROUPS;
use fleetworks_contracts::chat::QA_MAX_INBOX_NOTIFICATION_ITEMS;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::readiness::ReadinessCheckId;
use fleetworks_contracts::readiness::ReadinessState;
use rune::Cx;
use rune::El;
use rune::Icon;
use rune::TextEdit;
use rune::Tx;
use rune_kit::ControlSize;
use rune_kit::IconButton;
use rune_kit::IconButtonVariant;
use rune_kit::Kit;

use crate::accessibility::ViewPreferences;
use crate::icons;
use crate::material;
use crate::material::GlassSurface;
use crate::theme;
use crate::theme::StatusTone;
use crate::State;

/// Retained items. The oldest is dropped first.
pub(crate) const MAX_ITEMS: usize = QA_MAX_INBOX_NOTIFICATION_ITEMS as usize;
/// Remembered job transitions, so one job transition is recorded once.
const MAX_SEEN_JOB_EVENTS: usize = 200;
/// Bounded map of tracked job states across pagination and filtering.
const MAX_TRACKED_JOBS: usize = 512;
const DAY_SECONDS: f64 = 24.0 * 60.0 * 60.0;
const DRAWER_WIDTH: f32 = 440.0;
pub(crate) const NOTIFICATIONS_SCROLL: &str = "inbox-notifications-scroll";
pub(crate) const SCROLL: &str = "inbox-scroll";
const BELL: Icon = Icon {
    name: "fw-inbox-bell",
    view: 16.0,
    body: r#"<path d="M5.9 13.1a2.2 2.2 0 0 0 4.2 0M13 11.2H3l1.2-1.9V6a3.8 3.8 0 0 1 7.6 0v3.3L13 11.2Z" stroke="currentColor" stroke-width="1.35" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#,
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Topic {
    Connection,
    Job(String),
    Readiness(ReadinessCheckId),
}

impl Topic {
    fn key(&self) -> String {
        match self {
            Self::Connection => "inbox-group-connection".into(),
            Self::Job(id) => format!("inbox-group-job-{id}"),
            Self::Readiness(check) => format!("inbox-group-readiness-{}", readiness_name(*check)),
        }
    }
}

struct Group<'a> {
    topic: &'a Topic,
    title: &'a str,
    items: Vec<&'a Item>,
    unread: usize,
}

impl Group<'_> {
    fn latest_item(&self) -> &Item {
        self.items[self.items.len() - 1]
    }
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
    tone: StatusTone,
    private_job: bool,
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
    pub private_jobs: BTreeSet<String>,
    /// (check identity, state) from the latest readiness read.
    pub checks: Vec<(ReadinessCheckId, ReadinessState)>,
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
    checks: Vec<(ReadinessCheckId, ReadinessState)>,
    seen_job_events: VecDeque<String>,
    pub(crate) open: bool,
    pub(crate) transcript_at_bottom: bool,
    focus_return: Option<&'static str>,
    query: TextEdit,
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

    pub(crate) fn assistant_notifications(&self) -> Vec<QaInboxNotificationGroup> {
        use QaInboxNotificationSeverity::{Critical, Informational, Warning};
        use QaInboxNotificationTopic::{Connection, Jobs, Readiness};

        let mut groups = BTreeMap::new();
        for item in self.items.iter().filter(|item| !item.private_job) {
            let topic = match &item.topic {
                Topic::Connection => Connection,
                Topic::Job(_) => Jobs,
                Topic::Readiness(check) => Readiness(*check),
            };
            let severity = match item.tone {
                StatusTone::Err => Critical,
                StatusTone::Warn => Warning,
                _ => Informational,
            };
            let group = groups
                .entry(topic)
                .or_insert_with(|| QaInboxNotificationGroup::new(topic, severity, 0, 0));
            group.total += 1;
            if !item.read {
                group.severity = if group.unread == 0 {
                    severity
                } else {
                    group.severity.max(severity)
                };
                group.unread += 1;
            } else if group.unread == 0 {
                group.severity = group.severity.max(severity);
            }
        }
        let mut groups: Vec<_> = groups.into_values().collect();
        groups.sort_by_key(|group| Reverse((group.unread > 0, group.severity, group.unread)));
        groups.truncate(QA_MAX_INBOX_NOTIFICATION_GROUPS);
        groups
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
        groups.sort_by(|left, right| item_order(right.latest_item(), left.latest_item()));
        groups
    }

    fn filtered_groups(&self) -> Vec<Group<'_>> {
        let query = self.query.text.trim().to_lowercase();
        let mut groups = self.groups();
        if !query.is_empty() {
            groups.retain(|group| {
                group.title.to_lowercase().contains(&query)
                    || group.items.iter().any(|item| {
                        item.title.to_lowercase().contains(&query)
                            || item.detail.to_lowercase().contains(&query)
                    })
            });
        }
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
            return;
        }
        let Some(id) = self.selected else {
            return;
        };
        if let Some(item) = self
            .items
            .iter_mut()
            .find(|item| item.id == id && &item.topic == topic)
        {
            item.read = true;
            self.selected = None;
        }
    }

    fn select(&mut self, id: u64) {
        let previous = self.selected.take();
        if let Some(previous) = previous {
            self.mark_read(previous);
        }
        self.selected = (previous != Some(id)).then_some(id);
    }

    fn mark_read(&mut self, id: u64) {
        if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
            item.read = true;
        }
    }

    pub(crate) fn close(&mut self) {
        self.open = false;
        self.query = TextEdit::new("");
        self.selected = None;
        self.focus_return = None;
    }

    fn mark_group_read(&mut self, topic: &Topic) {
        for item in &mut self.items {
            if &item.topic == topic {
                item.read = true;
            }
        }
    }

    fn reset(&mut self) {
        self.items.clear();
        self.query = TextEdit::new("");
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
        private: bool,
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
                let tone = match state_label.as_str() {
                    "completed" => StatusTone::Ok,
                    "failed" => StatusTone::Err,
                    "cancelled" => StatusTone::Warn,
                    _ => StatusTone::Neutral,
                };
                self.push(
                    Topic::Job(id.clone()),
                    title.to_owned(),
                    format!("Job {state_label}"),
                    format!("{title} is now {state_label}. Open Jobs & Logs for details."),
                    now,
                    tone,
                );
                if let Some(item) = self.items.back_mut() {
                    item.private_job = private;
                }
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
                false,
            ) {
                changed = true;
            }
        }
        changed
    }

    fn push(
        &mut self,
        topic: Topic,
        group_title: String,
        title: String,
        detail: String,
        at: f64,
        tone: StatusTone,
    ) {
        self.next_id += 1;
        self.items.push_back(Item {
            id: self.next_id,
            topic,
            group_title,
            title,
            detail,
            at,
            read: false,
            tone,
            private_job: false,
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
                        StatusTone::Ok,
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
                        StatusTone::Err,
                    );
                    changed = true;
                }
                _ => {}
            }
            self.link = observation.link;
        }

        for (id, title, state) in observation.jobs {
            let private = observation.private_jobs.contains(&id);
            if self.observe_job_update(now, id, &title, state, None, private) {
                changed = true;
            }
        }

        for (id, state) in &observation.checks {
            let Some((_, previous)) = self.checks.iter().find(|(known, _)| known == id) else {
                continue;
            };
            if previous == state {
                continue;
            }
            let actor_running =
                *id == ReadinessCheckId::ActorSearch && *state == ReadinessState::Running;
            if actor_running && observation.tracked_actor_reindex {
                continue;
            }
            if actor_running || is_notable(*previous) || is_notable(*state) {
                let name = readiness_name(*id);
                let title = if actor_running {
                    crate::readiness_cards::ACTOR_INDEX_CATCH_UP.to_owned()
                } else {
                    format!("{name} is {}", readiness_word(*state))
                };
                self.push(
                    Topic::Readiness(*id),
                    name.clone(),
                    title,
                    format!(
                        "{name} changed from {} to {}.",
                        readiness_word(*previous),
                        readiness_word(*state)
                    ),
                    now,
                    match state {
                        ReadinessState::Ready => StatusTone::Ok,
                        ReadinessState::Running => StatusTone::Info,
                        ReadinessState::Degraded => StatusTone::Warn,
                        ReadinessState::NotReady | ReadinessState::Unavailable => StatusTone::Err,
                        _ => StatusTone::Neutral,
                    },
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
fn readiness_name(id: ReadinessCheckId) -> String {
    humanize(&format!("{id:?}"))
}

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
    pub(crate) fn open_inbox(&mut self) {
        self.open_inbox_from("inbox-toggle");
    }

    pub(crate) fn open_inbox_from(&mut self, trigger: &'static str) {
        self.inbox_qa.reset_sources_view();
        self.inbox.open = true;
        self.inbox.focus_return = Some(trigger);
        self.inbox_qa.request_resume();
    }

    fn inbox_focus_return(&self) -> &'static str {
        self.inbox.focus_return.unwrap_or("inbox-toggle")
    }

    pub(crate) fn close_inbox(&mut self, tx: &mut Tx) {
        let trigger = self.inbox_focus_return();
        self.inbox.close();
        self.inbox_qa.reset_sources_view();
        tx.focus(trigger, tx.focus_visible());
    }

    pub(crate) fn dismiss_inbox(&mut self, tx: &mut Tx) {
        if !self.dismiss_inbox_qa_sources(tx) {
            self.close_inbox(tx);
        }
    }

    /// Populate the Inbox with a compact synthetic notification and room for a QA exchange.
    #[doc(hidden)]
    pub fn seed_inbox_qa_drawer_fixture(&mut self) {
        if self.fixture {
            self.inbox.push(
                Topic::Connection,
                "Connection".into(),
                "Connected".into(),
                "The connection to the backend is ready.".into(),
                self.clock,
                StatusTone::Ok,
            );
            self.inbox.open = true;
        }
    }

    /// Open a synthetic Inbox whose notification history overflows for visual review.
    #[doc(hidden)]
    pub fn seed_inbox_qa_overflow_fixture(&mut self) {
        if !self.fixture {
            return;
        }
        self.seed_inbox_qa_drawer_fixture();
        let checks = [
            ReadinessCheckId::Gateway,
            ReadinessCheckId::Datastore,
            ReadinessCheckId::Jobs,
            ReadinessCheckId::Brain,
            ReadinessCheckId::Control,
            ReadinessCheckId::Nlp,
            ReadinessCheckId::Classifier,
            ReadinessCheckId::Registration,
            ReadinessCheckId::ActorCatalog,
            ReadinessCheckId::ActorSearch,
            ReadinessCheckId::ClassifierInference,
            ReadinessCheckId::ClassifierTraining,
        ];
        for check in checks {
            let name = readiness_name(check);
            self.inbox.push(
                Topic::Readiness(check),
                name.clone(),
                format!("{name} is ready"),
                format!("{name} completed its readiness check."),
                self.clock,
                StatusTone::Ok,
            );
        }
    }

    /// Populate the production Inbox with synthetic topics for native visual review.
    pub fn seed_inbox_drawer_fixture(&mut self) {
        if self.fixture {
            seed_drawer(&mut self.inbox, self.clock);
            self.inbox.push(
                Topic::Connection,
                "Connection".into(),
                "Connecting".into(),
                "The workstation started connecting to the backend.".into(),
                self.clock - 25.0 * 3600.0 - 5.0,
                StatusTone::Info,
            );
            self.inbox.mark_read(self.inbox.next_id);
        }
    }

    /// Fold what this app currently shows into the Inbox. Cheap, bounded, and read-only.
    /// Open one synthetic retained notice in the production drawer.
    #[doc(hidden)]
    pub fn fixture_show_triage_notice(&mut self) {
        if !self.fixture {
            return;
        }
        let topic = Topic::Readiness(ReadinessCheckId::ActorSearch);
        self.inbox.push(
            topic.clone(),
            "Actor search".into(),
            "Actor search was unavailable".into(),
            "An earlier actor search check reported a failure.".into(),
            self.clock,
            StatusTone::Err,
        );
        self.inbox.expanded.insert(topic);
        self.inbox.select(self.inbox.next_id);
        self.inbox.open = true;
    }

    #[cfg(test)]
    pub(crate) fn fixture_show_triage_job_notice(&mut self) {
        let id = "fixture-failed-job".to_owned();
        self.inbox.record_known_job(id.clone(), "running".into());
        self.inbox.observe_job_update(
            self.clock,
            id.clone(),
            "Fixture job",
            "failed".into(),
            None,
            true,
        );
        self.inbox.expanded.insert(Topic::Job(id));
        self.inbox.open = true;
    }

    pub(crate) fn open_readiness_notice(&mut self, id: ReadinessCheckId, tx: &mut Tx) {
        self.navigate(crate::app::Page::Workstation);
        if let crate::app::Load::Ready(readiness) = &self.readiness {
            if readiness.checks.iter().any(|check| check.id == id) {
                self.reveal_readiness_check(id, tx);
            }
        }
    }

    pub(crate) fn notification_source(&self, notice_id: u64) -> Option<ChatSourceReference> {
        if !self.inbox.open {
            return None;
        }
        let item = self.inbox.items.iter().find(|item| item.id == notice_id)?;
        if !self.inbox.expanded.contains(&item.topic) {
            return None;
        }
        let (kind, id) = match &item.topic {
            Topic::Readiness(id) => (ChatSourceKind::Readiness, id.as_str().to_owned()),
            Topic::Job(id) if item.private_job && !self.triage_job_is_shared(id) => {
                (ChatSourceKind::Job, id.clone())
            }
            _ => return None,
        };
        ChatSourceReference::new(kind, id).ok()
    }

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
        let mut private_jobs = BTreeSet::new();
        if self.registration.context.is_some() {
            let mut seen_ids = BTreeSet::new();
            if let Some(page) = &self.scoped_jobs.page {
                for row in page.rows.iter().take(MAX_TRACKED_JOBS) {
                    if seen_ids.insert(row.id.clone()) {
                        jobs.push((row.id.clone(), row.title.clone(), row.state.clone()));
                        if row.private_details_available && !row.status_only {
                            private_jobs.insert(row.id.clone());
                        }
                    }
                }
            }
            if let Load::Ready(detail) = &self.backend.detail {
                if detail.private_details_available
                    && detail.job.private_details_available
                    && !detail.job.status_only
                {
                    private_jobs.insert(detail.job.id.clone());
                } else {
                    private_jobs.remove(&detail.job.id);
                }
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
                .map(|check| (check.id, check.state))
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
                private_jobs,
                checks,
                tracked_actor_reindex,
            },
        )
    }
}

pub(crate) fn toolbar_item(state: &State, kit: &Kit, preferences: ViewPreferences) -> El {
    tray(state, kit, "inbox-toggle", preferences)
}

fn tray(state: &State, kit: &Kit, key: &'static str, preferences: ViewPreferences) -> El {
    let unread = state.inbox.unread();
    let label = if unread == 0 {
        "Inbox".to_owned()
    } else {
        format!("Inbox, {unread} unread")
    };
    let bell = rune_kit::Button::new(label.clone())
        .variant(rune_kit::ButtonVariant::Ghost)
        .icon(&BELL)
        .expanded(state.inbox.open)
        .label_with(move |font, _| {
            if unread == 0 {
                return rune::el();
            }
            rune::el()
                .key("inbox-unread-badge")
                .center()
                .fill(theme::hex(theme::MOLTEN_BRIGHT))
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
                state.open_inbox();
            }
        })
        .build(kit)
        .key(key)
        .label(label)
        .h(28.0)
        .px(8.0)
        .radius(999.0);
    let bell = material::glass_control(bell, preferences);
    if state.inbox.open {
        bell.fill(
            theme::hex(theme::SURFACE_ELEVATED).alpha(if preferences.matte() { 1.0 } else { 0.82 }),
        )
    } else {
        bell
    }
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
    let [window_width, window_height] = cx.layout_size("app-shell").unwrap_or([640.0, 480.0]);
    let mut panel = body(state, kit, preferences, window_height * 0.5)
        .w(DRAWER_WIDTH)
        .max_w(DRAWER_WIDTH)
        .h_full()
        .min_h(0.0)
        .transition(crate::app::surface_transition(
            preferences.reduced_motion,
            rune::Transition::offset(DRAWER_WIDTH, 0.0),
        ))
        .instant_changes();
    for shadow in kit.shadow_layers(&kit.elevation.overlay) {
        panel = panel.box_shadow(shadow);
    }
    let panel = material::glass(panel, GlassSurface::Overlay, preferences);
    let scrim = rune::el()
        .key("inbox-scrim")
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .fill(theme::hex(theme::BACKGROUND).alpha(if window_width < 820.0 { 0.44 } else { 0.28 }))
        .a11y_hidden();
    Some(
        rune::el()
            .key("inbox-layer")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .role("dialog")
            .label("Inbox")
            .focus_return(state.inbox_focus_return())
            .modal_tx(false, State::dismiss_inbox)
            .child(scrim)
            .child(
                rune::row()
                    .key("inbox-panel-position")
                    .w_full()
                    .h_full()
                    .child(dismiss_area("inbox-backdrop-left").grow(1.0).h_full())
                    .child(panel),
            ),
    )
}

fn dismiss_area(key: &'static str) -> El {
    rune::el()
        .key(key)
        .a11y_hidden()
        .tab_index(-1)
        .on_press_tx(State::close_inbox)
}

pub(crate) fn enter_from_below(el: El, preferences: ViewPreferences) -> El {
    enter(el, preferences, rune::Transition::offset(0.0, 6.0))
}

fn enter(el: El, preferences: ViewPreferences, transition: rune::Transition) -> El {
    if preferences.reduced_motion {
        return el;
    }
    let mut transition = crate::app::surface_transition(false, transition);
    transition.springs.exit = rune::Spring::NONE;
    // Only insertion moves. The drawer still snaps retained layout and nested controls.
    el.animated_changes()
        .motion(rune::Spring::NONE)
        .transition(transition)
}

fn body(
    state: &State,
    kit: &Kit,
    preferences: ViewPreferences,
    max_notifications_height: f32,
) -> El {
    let inbox = &state.inbox;
    let close = rune_kit::Button::new("Close")
        .variant(rune_kit::ButtonVariant::Ghost)
        .on_press_tx(State::close_inbox)
        .build(kit)
        .key("inbox-close")
        .autofocus();
    let filter = rune_kit::TextInput::new(&inbox.query)
        .label("Filter notifications")
        .placeholder("Search Inbox")
        .on_edit(|state: &mut State| &mut state.inbox.query)
        .build(kit)
        .key("inbox-filter")
        .w_full();
    let mut rows = rune::col().key("inbox-list").gap(8.0);
    let groups = inbox.filtered_groups();
    if groups.is_empty() {
        rows = rows.child(
            rune::text(if inbox.query.text.trim().is_empty() {
                "Job, readiness and connection changes appear here."
            } else {
                "No notifications match this search."
            })
            .key("inbox-empty")
            .font(kit.ty.small)
            .color(theme::hex(theme::TEXT_MUTED))
            .wrap(),
        );
    }
    let mut last_section = "";
    for group in groups {
        let section = if state.clock - group.latest_item().at < DAY_SECONDS {
            "PAST 24 HOURS"
        } else {
            "EARLIER"
        };
        if section != last_section {
            rows = rows.child(
                rune::text(section)
                    .key(format!("inbox-time-{}", group.topic.key()))
                    .role("heading")
                    .font(kit.ty.small_medium)
                    .color(theme::hex(theme::TEXT_MUTED)),
            );
            last_section = section;
        }
        rows = rows.child(group_view(&group, state, kit, preferences));
    }
    let notifications = rune::col()
        .key(NOTIFICATIONS_SCROLL)
        .role("region")
        .label("Notifications")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .max_h(max_notifications_height)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .scroll_anchor()
        .child(rows.padding(12.0));
    let transcript = state.inbox_qa_visible().then(|| {
        let chat = crate::inbox_qa::exchange(state, kit, preferences);
        rune::col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .children(chat.is_some().then(|| {
                rune::rule(1.0, theme::hex(theme::BORDER_CONTROL))
                    .key("inbox-chat-divider")
                    .w_full()
                    .a11y_hidden()
            }))
            .child(
                rune::col()
                    .key(SCROLL)
                    .w_full()
                    .grow(1.0)
                    .min_h(0.0)
                    .scroll(rune::ScrollSpec { stick_bottom: true })
                    .scroll_anchor()
                    .on_at_bottom(|state: &mut State, at_bottom| {
                        state.inbox.transcript_at_bottom = at_bottom;
                    })
                    .child(rune::col().w_full().padding(12.0).children(chat)),
            )
    });
    rune::col()
        .key("inbox-body")
        .child(
            rune::col()
                .key("inbox-header")
                .w_full()
                .px(14.0)
                .py(8.0)
                .gap(6.0)
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
                        .child(material::glass_control(close, preferences))
                        .child(tray(state, kit, "inbox-toggle-close", preferences).tab_index(-1)),
                )
                .child(
                    rune::text("Only changes observed by this workstation.")
                        .key("inbox-helper")
                        .font(kit.ty.small)
                        .color(theme::hex(theme::TEXT_MUTED))
                        .wrap(),
                )
                .child(material::glass_control(filter, preferences)),
        )
        .child(notifications)
        .children(transcript)
        .children(crate::inbox_qa::footer(state, kit, preferences))
}

fn group_view(group: &Group<'_>, state: &State, kit: &Kit, preferences: ViewPreferences) -> El {
    let key = group.topic.key();
    let expanded = state.inbox.expanded.contains(group.topic);
    let toggle_key = format!("{key}-toggle");
    let group_toggle_key = toggle_key.clone();
    let topic = group.topic.clone();
    let mark_topic = topic.clone();
    let mark_focus = toggle_key.clone();
    let latest = group.latest_item();
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
        .grow(1.0)
        .min_w(0.0)
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
        .child(status_dot(latest.tone))
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
                            theme::MOLTEN_ACCENT
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
    let mark = IconButton::new(kit.glyph.check)
        .variant(IconButtonVariant::Ghost)
        .control_size(ControlSize::ExtraLarge)
        .label(format!("Mark {} notifications as read", group.title))
        .help("Mark group as read")
        .disabled(group.unread == 0)
        .on_press_tx(move |state: &mut State, tx| {
            state.inbox.mark_group_read(&mark_topic);
            tx.focus(mark_focus.clone(), tx.focus_visible());
            tx.keep_on_screen(mark_focus.clone());
        })
        .build(kit)
        .key(format!("{key}-mark-read"));
    let header = rune::row()
        .key("inbox-group-header")
        .w_full()
        .center_y()
        .pr(6.0)
        .child(trigger)
        .child(mark);
    let border = theme::hex(if preferences.matte() {
        theme::BORDER_CONTROL
    } else {
        theme::BORDER_SUBTLE
    });
    let card = rune::col()
        .key("card")
        .w_full()
        .radius(8.0)
        .instant_changes()
        .transition(rune::Transition::none())
        .child(header);
    // The popover samples the backdrop once. Its stacked cards share the elevated tint
    // and rim without repeatedly blurring the same scene.
    let mut card = material::glass(card, GlassSurface::Overlay, preferences).backdrop_blur(0.0);
    if expanded {
        for item in &group.items {
            card = card.child(row(item, state, kit, preferences));
        }
    }
    let mut entry = rune::col().key(key).w_full().child(card);
    if !expanded && group.items.len() > 1 {
        for (key, inset) in [("near-lip", 7.0), ("far-lip", 13.0)] {
            entry = entry.child(
                rune::row().key(key).w_full().px(inset).a11y_hidden().child(
                    rune::el()
                        .grow(1.0)
                        .h(3.0)
                        .radius(2.0)
                        .fill(
                            theme::hex(theme::SURFACE_RAISED).alpha(if preferences.matte() {
                                1.0
                            } else {
                                0.82
                            }),
                        )
                        .border_sides([0.0, 1.0, 1.0, 1.0], border),
                ),
            );
        }
    }
    if expanded {
        entry
    } else {
        enter_from_below(entry, preferences)
    }
}

fn status_dot(tone: StatusTone) -> El {
    rune::el()
        .size(6.0, 6.0)
        .radius(3.0)
        .fill(tone.fg())
        .a11y_hidden()
}

fn view_status_button(notice_id: u64, check: ReadinessCheckId, kit: &Kit) -> El {
    IconButton::new(&icons::SHIELD_CHECK)
        .variant(IconButtonVariant::Ghost)
        .control_size(ControlSize::Large)
        .label("View status")
        .help("View this readiness check")
        .on_press_tx(move |state: &mut State, tx| {
            state.close_inbox(tx);
            state.open_readiness_notice(check, tx);
        })
        .build(kit)
        .key(format!("inbox-item-{notice_id}-action"))
}

fn open_job_logs_button(notice_id: u64, job: String, kit: &Kit) -> El {
    IconButton::new(&rune_kit::glyphs::FILE_TEXT)
        .variant(IconButtonVariant::Ghost)
        .control_size(ControlSize::Large)
        .label("Open job logs")
        .help("View this job’s operational logs")
        .on_press_tx(move |state: &mut State, tx| {
            if !state.inbox.open
                || state.registration.context.is_none()
                || !state.permission(Permission::ReadWorkstationActivity)
                || !state.inbox.items.iter().any(|item| {
                    item.id == notice_id && matches!(&item.topic, Topic::Job(id) if id == &job)
                })
            {
                return;
            }
            state.close_inbox(tx);
            state.setup_gateway = false;
            state.open_job(&job);
            state.reveal_operational_logs();
        })
        .build(kit)
        .key(format!("inbox-item-{notice_id}-action"))
}

fn row(item: &Item, state: &State, kit: &Kit, preferences: ViewPreferences) -> El {
    let id = item.id;
    let expanded = state.inbox.selected == Some(id);
    let row_key = format!("inbox-item-{id}-row");
    let toggle_key = row_key.clone();
    let age = age_label(state.clock, item.at);
    let entry = rune::col()
        .key(format!("inbox-item-{id}"))
        .w_full()
        .px(10.0)
        .border_sides([1.0, 0.0, 0.0, 0.0], theme::hex(theme::BORDER_SUBTLE));
    let action = match &item.topic {
        Topic::Readiness(check) => Some(view_status_button(id, *check, kit)),
        Topic::Job(job)
            if state.registration.context.is_some()
                && state.permission(Permission::ReadWorkstationActivity) =>
        {
            Some(open_job_logs_button(id, job.clone(), kit))
        }
        _ => None,
    };
    let trigger = rune::row()
        .key(row_key)
        .role("button")
        .focusable(true)
        .label(format!(
            "{}, {}, {}",
            if item.read { "Read" } else { "Unread" },
            item.title,
            age
        ))
        .expanded(expanded)
        .grow(1.0)
        .min_w(0.0)
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
        .child(
            rune::el()
                .size(6.0, 6.0)
                .a11y_hidden()
                .children((!item.read).then(|| {
                    status_dot(StatusTone::Accent).key(format!("inbox-item-{id}-unread"))
                })),
        )
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
        );
    let mut content = rune::col()
        .w_full()
        .instant_changes()
        .transition(rune::Transition::none())
        .child(
            rune::row()
                .w_full()
                .center_y()
                .child(trigger)
                .children(action)
                .children(crate::activity_triage::explain_notice_button(
                    state,
                    id,
                    &item.title,
                    kit,
                )),
        );
    if !expanded {
        return enter_from_below(entry.child(content), preferences);
    }
    content = content.child(
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
            ),
    );
    enter_from_below(entry.child(content), preferences)
}

fn seed_drawer(inbox: &mut Inbox, now: f64) {
    let connection = Topic::Connection;
    let readiness = Topic::Readiness(ReadinessCheckId::ActorSearch);
    let job = Topic::Job("fixture-index-job".into());
    let job_title = "Scheduled maintenance for the synthetic regional actor catalogue";
    inbox.push(
        connection.clone(),
        "Connection".into(),
        "Connected".into(),
        "The connection to the backend is ready.".into(),
        now - 25.0 * 3600.0,
        StatusTone::Ok,
    );
    inbox.mark_group_read(&connection);
    inbox.push(
        readiness.clone(),
        "Actor search".into(),
        crate::readiness_cards::ACTOR_INDEX_CATCH_UP.into(),
        "The index is warming up after backend startup. You can keep using the other workstation tools.".into(),
        now - 120.0,
        StatusTone::Info,
    );
    inbox.push(
        job.clone(),
        job_title.into(),
        "Job completed".into(),
        format!("{job_title} is complete. Open Jobs & Logs for details."),
        now - 120.0,
        StatusTone::Ok,
    );
    inbox.push(
        readiness.clone(),
        "Actor search".into(),
        "Actor search is ready".into(),
        "Actor search changed from running to ready.".into(),
        now - 28.0,
        StatusTone::Ok,
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
    fn assistant_notification_summary_ranks_unread_severity_and_omits_private_jobs() {
        use QaInboxNotificationSeverity::{Critical, Informational, Warning};
        use QaInboxNotificationTopic::{Connection, Jobs, Readiness};

        let mut inbox = Inbox::default();
        for (topic, tone, read, private) in [
            (Topic::Connection, StatusTone::Err, true, false),
            (Topic::Connection, StatusTone::Warn, false, false),
            (
                Topic::Readiness(ReadinessCheckId::ActorSearch),
                StatusTone::Err,
                false,
                false,
            ),
            (
                Topic::Readiness(ReadinessCheckId::Registration),
                StatusTone::Ok,
                false,
                false,
            ),
            (
                Topic::Job("shared-1".into()),
                StatusTone::Info,
                false,
                false,
            ),
            (
                Topic::Job("shared-2".into()),
                StatusTone::Info,
                false,
                false,
            ),
            (Topic::Job("private-1".into()), StatusTone::Err, false, true),
        ] {
            inbox.push(
                topic,
                "Private display title".into(),
                "Private item title".into(),
                "Untrusted item detail".into(),
                0.0,
                tone,
            );
            let item = inbox.items.back_mut().unwrap();
            item.read = read;
            item.private_job = private;
        }
        let groups = inbox.assistant_notifications();
        assert_eq!(
            groups,
            [
                QaInboxNotificationGroup::new(
                    Readiness(ReadinessCheckId::ActorSearch),
                    Critical,
                    1,
                    1
                ),
                QaInboxNotificationGroup::new(Connection, Warning, 1, 2),
                QaInboxNotificationGroup::new(Jobs, Informational, 2, 2),
            ]
        );
        let serialized = serde_json::to_string(&groups).unwrap();
        for private_value in ["private-1", "shared-1", "Private", "Untrusted"] {
            assert!(!serialized.contains(private_value));
        }
    }

    #[test]
    fn first_observation_is_only_a_baseline() {
        let mut inbox = Inbox::default();
        let mut first = obs(1, Link::Connected);
        first.jobs = vec![job("a", "completed")];
        first.checks = vec![(ReadinessCheckId::Jobs, ReadinessState::NotReady)];
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
        base.checks = vec![(ReadinessCheckId::ActorSearch, ReadinessState::Ready)];
        inbox.observe(1.0, base);
        let mut next = obs(1, Link::Connected);
        next.checks = vec![(ReadinessCheckId::ActorSearch, ReadinessState::Degraded)];
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
        base.checks = vec![(ReadinessCheckId::Jobs, ReadinessState::Unknown)];
        inbox.observe(1.0, base);
        let mut next = obs(1, Link::Connected);
        next.checks = vec![(ReadinessCheckId::Jobs, ReadinessState::Running)];
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
    fn groups_use_latest_item_without_reordering_when_read() {
        let mut inbox = Inbox::default();
        seed_drawer(&mut inbox, 900.0);
        let actor = Topic::Readiness(ReadinessCheckId::ActorSearch);
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
        let latest_id = latest.id;
        inbox.toggle_group(&actor);
        assert_eq!(inbox.selected, None);
        assert_eq!(inbox.unread(), 2);
        let groups = inbox.groups();
        assert_eq!(groups[0].topic, &actor);
        assert_eq!(groups[1].topic, &job);
        assert_eq!(groups[0].unread, 1);
        assert!(!groups[0].items[1].read);

        inbox.toggle_group(&actor);
        inbox.select(latest_id);
        inbox.toggle_group(&job);
        assert_eq!(inbox.selected, Some(latest_id));
        assert_eq!(inbox.unread(), 2);
        inbox.select(earliest_id);
        assert_eq!(inbox.selected, Some(earliest_id));
        assert_eq!(inbox.unread(), 1);
        assert_eq!(inbox.groups()[0].topic, &actor);
        inbox.mark_group_read(&job);
        assert_eq!(inbox.unread(), 0);
        assert_eq!(inbox.groups()[0].topic, &actor);

        let latest_at = inbox.groups()[0].latest_item().at;
        inbox.push(
            Topic::Connection,
            "Connection".into(),
            "Connected".into(),
            "The connection to the backend is ready.".into(),
            latest_at,
            StatusTone::Ok,
        );
        inbox.mark_group_read(&Topic::Connection);
        inbox.push(
            job.clone(),
            "Actor index".into(),
            "Job completed".into(),
            "The actor index is ready.".into(),
            latest_at - 1.0,
            StatusTone::Ok,
        );
        let groups = inbox.groups();
        assert_eq!(
            groups.iter().map(|group| group.topic).collect::<Vec<_>>(),
            [&Topic::Connection, &actor, &job],
        );
        assert_eq!(groups[0].unread, 0);
        assert_eq!(groups[2].unread, 1);
    }

    #[test]
    fn background_actor_progress_is_a_readiness_item_only_when_untracked() {
        for tracked in [false, true] {
            let mut inbox = Inbox::default();
            let mut base = obs(1, Link::Connected);
            base.checks = vec![(ReadinessCheckId::ActorSearch, ReadinessState::Unknown)];
            inbox.observe(0.0, base.clone());

            base.tracked_actor_reindex = tracked;
            base.checks[0].1 = ReadinessState::Running;
            assert_eq!(inbox.observe(1.0, base), !tracked);
            if tracked {
                assert_eq!(inbox.unread(), 0);
            } else {
                let item = inbox.items().next().expect("actor readiness item");
                assert_eq!(item.topic, Topic::Readiness(ReadinessCheckId::ActorSearch));
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

    fn click_notification(runtime: &mut rune::Runtime<State>, key: &str, now: f64) {
        runtime.update(|_, tx| tx.scroll_into_view(key));
        runtime.frame(now);
        let (x, y) = runtime.find(key).unwrap_or_else(|| panic!("visible {key}"));
        assert!(
            runtime.elements().iter().any(|element| {
                element.key.as_deref() == Some(key)
                    && x >= element.visible[0]
                    && x < element.visible[0] + element.visible[2]
                    && y >= element.visible[1]
                    && y < element.visible[1] + element.visible[3]
                    && !element.exiting
            }),
            "{key} center must be visible before clicking"
        );
        click(runtime, key, now);
    }

    fn focused(runtime: &rune::Runtime<State>, key: &str) -> bool {
        runtime.elements().into_iter().any(|element| {
            element.key.as_deref() == Some(key) && element.focused && !element.exiting
        })
    }

    #[test]
    fn new_notification_rises_in_its_slot_without_moving_existing_rows() {
        for reduced_motion in [false, true] {
            let mut state = crate::tests::registered_fixture();
            state.accessibility.fixture_override =
                Some(crate::accessibility::FixtureAccessibility {
                    reduced_motion,
                    ..Default::default()
                });
            state.seed_inbox_qa_drawer_fixture();
            state.inbox.expanded.insert(Topic::Connection);
            let mut runtime = runtime(state, (640.0, 480.0));
            let first = runtime.presented("inbox-item-1-row").unwrap().0;
            runtime.update(|state, _| {
                state.inbox.push(
                    Topic::Connection,
                    "Connection".into(),
                    "Connecting".into(),
                    "The workstation started connecting to the backend.".into(),
                    state.clock + 1.0,
                    StatusTone::Info,
                );
            });
            runtime.frame(0.0);
            let entering = runtime.presented("inbox-item-2-row").unwrap().0;
            assert_eq!(runtime.presented("inbox-item-1-row").unwrap().0, first);
            runtime.frame(0.5);
            let settled = runtime.presented("inbox-item-2-row").unwrap().0;
            assert_eq!(runtime.presented("inbox-item-1-row").unwrap().0, first);
            if reduced_motion {
                assert_eq!(entering, settled);
            } else {
                assert!(entering[1] > settled[1] + 1.0);
            }
        }
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
        let (toolbar, _) = runtime.presented("toolbar").expect("toolbar geometry");
        let (bell, _) = runtime
            .presented("inbox-toggle")
            .expect("Inbox bell geometry");
        assert!(bell[0] >= toolbar[0] && bell[0] + bell[2] <= toolbar[0] + toolbar[2]);

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
            let (panel, _) = runtime.presented("inbox-body").expect("drawer geometry");
            for (coordinate, expected) in
                panel
                    .into_iter()
                    .zip([640.0 - DRAWER_WIDTH, 0.0, DRAWER_WIDTH, 480.0])
            {
                assert!((coordinate - expected).abs() < 0.5, "{panel:?}");
            }
            for key in [
                "inbox-close",
                "inbox-toggle-close",
                "inbox-group-connection-mark-read",
                "inbox-helper",
                "inbox-filter",
                NOTIFICATIONS_SCROLL,
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
    fn glass_and_opaque_chrome_and_inbox_keep_the_same_hit_bounds() {
        use crate::accessibility::FixtureAccessibility;

        for size in [(640.0, 480.0), (1280.0, 800.0)] {
            let build = |opaque| {
                let mut state = crate::tests::registered_fixture();
                state.seed_inbox_drawer_fixture();
                state.accessibility.fixture_override = Some(FixtureAccessibility {
                    reduced_transparency: opaque,
                    matte_fallback: opaque,
                    ..FixtureAccessibility::default()
                });
                runtime(state, size)
            };
            let glass = build(false);
            let opaque = build(true);
            let rail = if size.0 < 820.0 {
                "compact-icon-rail"
            } else {
                "sidebar"
            };
            for key in [
                "toolbar-glass",
                rail,
                "toolbar-telemetry",
                "inbox-toggle",
                "inbox-body",
                "inbox-filter",
            ] {
                let glass_bounds = glass.presented(key).unwrap_or_else(|| panic!("{key}"));
                assert_eq!(
                    Some(glass_bounds),
                    opaque.presented(key),
                    "{key} at {size:?}"
                );
            }
            assert_eq!(glass.accessibility_json(), opaque.accessibility_json());
        }
    }

    #[test]
    fn notifications_without_assistant_hold_half_the_drawer_height() {
        for size in [(640.0, 480.0), (1280.0, 800.0)] {
            let mut state = crate::tests::registered_fixture();
            state.seed_inbox_qa_overflow_fixture();
            let mut runtime = runtime(state, size);
            let (viewport, _) = runtime.presented(NOTIFICATIONS_SCROLL).unwrap();
            assert!((viewport[3] - size.1 * 0.5).abs() < 0.5);
            assert_eq!(runtime.scroll_at_bottom(NOTIFICATIONS_SCROLL), Some(false));

            runtime.update(|_, tx| tx.scroll_to_bottom_instant(NOTIFICATIONS_SCROLL));
            runtime.frame(1.0);
            assert_eq!(runtime.scroll_at_bottom(NOTIFICATIONS_SCROLL), Some(true));
            assert_eq!(runtime.presented(NOTIFICATIONS_SCROLL).unwrap().0, viewport);

            runtime.update(|state, _| {
                state.inbox.query = TextEdit::new("unmatched notification");
            });
            runtime.frame(1.25);
            assert!(runtime.find("inbox-empty").is_some());
            assert_eq!(runtime.presented(NOTIFICATIONS_SCROLL).unwrap().0, viewport);
        }
    }

    #[test]
    fn inbox_filter_keeps_unread_totals_and_restores_all_groups_on_reopen() {
        let mut state = crate::tests::registered_fixture();
        state.seed_inbox_drawer_fixture();
        let mut runtime = runtime(state, (1280.0, 800.0));
        assert!(runtime.find("inbox-time-inbox-group-connection").is_some());
        assert!(runtime.find("inbox-group-connection/near-lip").is_some());
        assert!(runtime.find("inbox-group-job-fixture-index-job").is_some());

        click(&mut runtime, "inbox-filter/value/input", 1.0);
        runtime.key(rune::KeyEvent {
            key: rune::Key::Text("WARMING UP".into()),
            mods: rune::Mods::default(),
        });
        runtime.frame(1.25);
        assert!(runtime.find("inbox-group-readiness-Actor search").is_some());
        assert!(runtime.find("inbox-group-job-fixture-index-job").is_none());
        assert!(runtime.find("inbox-group-connection").is_none());
        assert_eq!(runtime.state.inbox.unread(), 3);

        click(&mut runtime, "inbox-close", 1.5);
        click(&mut runtime, "inbox-toggle", 1.75);
        assert!(runtime.find("inbox-group-job-fixture-index-job").is_some());
        assert_eq!(runtime.state.inbox.unread(), 3);
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
        click_notification(&mut runtime, "inbox-item-2-action", 1.0);
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
    fn job_notice_opens_its_scoped_logs_after_the_matching_detail_arrives() {
        use crate::app::Intent;
        use crate::app::Page;
        use crate::backend::JobDetail;
        use crate::backend::ReadKind;
        use crate::backend::ReadResult;
        use crate::logs::ActivityTab;
        use crate::logs::LogAction;
        use crate::logs::LogTarget;

        const JOB: &str = "fixture-index-job";
        const ACTION: &str = "inbox-item-3-action";
        let mut state = crate::tests::registered_fixture();
        state.page = Page::Tools;
        state.backend.selected_job = Some("prior-job".into());
        state.scoped_logs.window_focused = true;
        seed_drawer(&mut state.inbox, 900.0);
        let mut runtime = runtime(state, (640.0, 480.0));
        runtime.update(|_, tx| tx.scroll_into_view(ACTION));
        runtime.frame(0.8);
        assert!(runtime.elements().iter().any(|element| {
            element.key.as_deref() == Some(ACTION)
                && element.label.as_deref() == Some("Open job logs")
                && element.visible == element.rect
        }));
        click(&mut runtime, ACTION, 1.0);

        let state = &runtime.state;
        assert!(!state.inbox.open);
        assert_eq!(state.page, Page::Jobs);
        assert_eq!(state.activity_tab, ActivityTab::Logs);
        assert_eq!(state.backend.selected_job.as_deref(), Some(JOB));
        assert_eq!(
            state.scoped_logs.target,
            LogTarget::Job {
                id: JOB.into(),
                descendants: false,
            }
        );
        assert!(state.intents.contains(&Intent::OperationalLogs {
            action: LogAction::Refresh
        }));

        let detail: JobDetail = serde_json::from_value(serde_json::json!({
            "job": {
                "id": JOB,
                "app": "Workstation",
                "title": "Fixture index job",
                "operation": "actor_reindex",
                "state": "completed",
                "phase": "completed",
                "error_summary": "",
                "private_details_available": true,
                "activity_actions_available": false,
                "status_only": false,
            },
            "stages": [],
            "private_details_available": true,
            "activity_actions_available": false,
        }))
        .unwrap();
        runtime.update(|state, _| {
            state.backend.revisions.insert(ReadKind::JobDetail, 1);
            state.complete_backend_read(
                ReadKind::JobDetail,
                1,
                Ok(ReadResult::JobDetail(Box::new(detail))),
            );
            assert_eq!(state.reveal.as_deref(), Some(crate::logs::SECTION_KEY));
        });
        runtime.poll = crate::app::spec().poll;
        runtime.frame(1.25);
        runtime.frame(1.75);
        assert!(runtime.elements().iter().any(|element| {
            element.key.as_deref() == Some(crate::logs::SECTION_KEY)
                && element.visible == element.rect
        }));
    }

    #[test]
    fn drawer_actions_update_badges_and_keep_the_selected_notification_on_screen() {
        let mut state = crate::tests::registered_fixture();
        seed_drawer(&mut state.inbox, 900.0);
        let mut runtime = runtime(state, (640.0, 480.0));
        let row = "inbox-item-2-row";
        let later_row = "inbox-item-4-row";
        let unread = "inbox-item-2-unread";
        let later_unread = "inbox-item-4-unread";
        let status = "inbox-item-2-action";
        let group = "inbox-group-job-fixture-index-job-toggle";
        let mark_job = "inbox-group-job-fixture-index-job-mark-read";
        let mark_connection = "inbox-group-connection-mark-read";
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
        assert!(label(&runtime, row).starts_with("Unread, "));
        assert!(runtime.find(unread).is_some());
        assert!(runtime.find("inbox-item-2-mark").is_none());

        click_notification(&mut runtime, row, 1.0);
        runtime.frame(1.3);
        assert_eq!(label(&runtime, "inbox-toggle-close"), "Inbox, 2 unread");
        assert_eq!(
            runtime.state.inbox.groups()[0].topic,
            &Topic::Readiness(ReadinessCheckId::ActorSearch)
        );
        assert!(runtime.find("inbox-item-2-detail").is_none());
        assert!(label(&runtime, row).starts_with("Read, "));
        assert!(runtime.find(unread).is_none());
        assert!(runtime.find(later_unread).is_some());

        runtime.update(|state, tx| {
            tx.focus(row, true);
            state.inbox.push(
                Topic::Connection,
                "Connection".into(),
                "Connected".into(),
                "The connection to the backend is ready.".into(),
                901.0,
                StatusTone::Ok,
            );
        });
        runtime.frame(1.5);
        assert_eq!(label(&runtime, "inbox-toggle-close"), "Inbox, 3 unread");
        assert_eq!(runtime.state.inbox.groups()[0].topic, &Topic::Connection);
        assert!(focused(&runtime, row));
        assert!(runtime.find(later_unread).is_some());

        click_notification(&mut runtime, later_row, 1.6);
        assert!(runtime.find("inbox-item-4-detail").is_some());
        assert!(runtime.find(later_unread).is_some());
        click_notification(
            &mut runtime,
            "inbox-group-readiness-Actor search-toggle",
            1.7,
        );
        assert!(runtime.find(later_row).is_none());
        assert_eq!(label(&runtime, "inbox-toggle-close"), "Inbox, 2 unread");
        assert_eq!(
            label(&runtime, mark_connection),
            "Mark Connection notifications as read"
        );
        click_notification(&mut runtime, mark_job, 1.8);
        assert_eq!(label(&runtime, "inbox-toggle-close"), "Inbox, 1 unread");
        assert!(label(&runtime, "inbox-group-connection-toggle").contains("1 unread"));
        assert!(label(&runtime, group).contains("0 unread"));
        assert!(focused(&runtime, group));
        assert!(runtime
            .state
            .inbox
            .expanded
            .contains(&Topic::Job("fixture-index-job".into())));
        click_notification(&mut runtime, mark_connection, 1.9);
        assert_eq!(label(&runtime, "inbox-toggle-close"), "Inbox");
        assert_eq!(runtime.state.inbox.unread(), 0);
    }
}
