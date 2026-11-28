//! Scoped, asynchronous retention of Inbox job events.

use std::collections::BTreeSet;
use std::sync::Arc;

use chrono::DateTime;
use chrono::Utc;
use fleetworks_contracts::context::Permission;
use fleetworks_workstation::native::canonical_target_origin;
use fleetworks_workstation::native::JobNotice;
use fleetworks_workstation::native::JobNoticeChange;
use fleetworks_workstation::native::JobNoticeHistory;
use fleetworks_workstation::native::JobNoticeKey;
use fleetworks_workstation::native::JobNoticeKind;
use fleetworks_workstation::native::NativeContainer;
use rune::Tx;
use sha2::Digest;
use sha2::Sha256;

use super::describe_job_state;
use super::item_order;
use super::Inbox;
use super::Item;
use super::Topic;
use super::MAX_ITEMS;
use crate::backend::ActivityJob;
use crate::connection::GatewayEffects;
use crate::connection::RequestTask;
use crate::theme::StatusTone;
use crate::State;

#[derive(Clone, PartialEq, Eq)]
struct Binding {
    key: String,
    origin_revision: u64,
    context_revision: u64,
}

#[derive(Default)]
pub(super) struct History {
    binding: Option<Binding>,
    snapshot: Option<JobNoticeHistory>,
    queued: Vec<JobNoticeChange>,
    request: Option<RequestTask>,
    retry_at: f64,
    pub(super) deleting: Option<u64>,
    pub(super) error: Option<&'static str>,
}

impl History {
    pub(super) fn bound(&self) -> bool {
        self.binding.is_some()
    }

    fn record(&mut self, key: JobNoticeKey, observed_at_ms: i64) {
        if self
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.contains_event(&key))
        {
            return;
        }
        self.queue(JobNoticeChange::Record(JobNotice {
            key,
            observed_at_ms,
            read: false,
        }));
    }

    pub(super) fn mark_read(&mut self, item: &mut Item) {
        if std::mem::replace(&mut item.read, true) || !self.bound() {
            return;
        }
        if let Some(key) = &item.job_notice {
            self.queue(JobNoticeChange::MarkRead(key.clone()));
        }
    }

    fn queue(&mut self, change: JobNoticeChange) {
        match &change {
            JobNoticeChange::Record(new) => {
                let records = self.queued.iter().filter_map(|change| match change {
                    JobNoticeChange::Record(record) => Some(record),
                    _ => None,
                });
                if records.clone().any(|record| record.key.matches(&new.key)) {
                    return;
                }
                if records.count() >= MAX_ITEMS {
                    if let Some(index) = self
                        .queued
                        .iter()
                        .position(|change| matches!(change, JobNoticeChange::Record(_)))
                    {
                        self.queued.remove(index);
                    }
                }
            }
            JobNoticeChange::MarkRead(key) => {
                let mut reads = self
                    .queued
                    .iter()
                    .enumerate()
                    .filter_map(|(index, change)| match change {
                        JobNoticeChange::MarkRead(read) => Some((index, read)),
                        _ => None,
                    });
                if reads.clone().any(|(_, read)| read == key) {
                    return;
                }
                if let Some((oldest, _)) = reads.next() {
                    if reads.count() + 1 >= MAX_ITEMS {
                        self.queued.remove(oldest);
                    }
                }
            }
            JobNoticeChange::Delete(_) => {}
        }
        self.queued.push(change);
    }
}

pub(super) fn event_key(job: &ActivityJob) -> Option<JobNoticeKey> {
    if job.id.is_empty() || job.id.len() > 512 || job.id.chars().any(char::is_control) {
        return None;
    }
    let (kind, timestamp) = match describe_job_state(&job.state).as_str() {
        "running" => (JobNoticeKind::Started, &job.active_since),
        "failed" => (JobNoticeKind::Failed, &job.finished_at),
        _ => return None,
    };
    let transition = DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .and_then(|time| time.timestamp_nanos_opt())
        .filter(|time| *time > 0);
    Some(JobNoticeKey {
        job_id: job.id.clone(),
        job_digest: format!("{:x}", Sha256::digest(job.id.as_bytes())),
        kind,
        transition,
    })
}

impl Inbox {
    pub(super) fn observe_retained_job(
        &mut self,
        key: JobNoticeKey,
        observed_at_ms: i64,
        past_baseline: bool,
    ) {
        let previous = self
            .known_notice_keys
            .insert(key.job_id.clone(), key.clone());
        if previous
            .as_ref()
            .is_some_and(|previous| previous.matches(&key))
        {
            return;
        }
        let was_active = self
            .known_jobs
            .get(&key.job_id)
            .is_some_and(|state| crate::backend::job_is_active(state));
        let observed_new_failure = previous.is_some_and(|previous| {
            previous.kind == JobNoticeKind::Failed
                && previous.transition.is_some()
                && key.transition > previous.transition
        });
        if key.kind == JobNoticeKind::Started
            || (past_baseline && (was_active || observed_new_failure))
        {
            self.history.record(key, observed_at_ms);
        }
    }

    pub(super) fn refresh_retained_labels(
        &mut self,
        jobs: &[(String, String, String)],
        private: &BTreeSet<String>,
    ) -> bool {
        if !self.history.bound() {
            return false;
        }
        let mut changed = false;
        for item in self
            .items
            .iter_mut()
            .filter(|item| item.job_notice.is_some())
        {
            let Some(key) = &item.job_notice else {
                continue;
            };
            let title = jobs
                .iter()
                .find_map(|(id, title, _)| (id == &key.job_id).then_some(title));
            let allowed = title.is_some() && private.contains(&key.job_id);
            let group_title = title.filter(|title| allowed && !title.trim().is_empty());
            let display = group_title
                .cloned()
                .unwrap_or_else(|| generic_job_label(&key.job_id));
            if item.group_title != display
                || item.private_job != allowed
                || item.current_job_visibility != title.is_some()
            {
                item.group_title = display;
                item.private_job = allowed;
                item.current_job_visibility = title.is_some();
                changed = true;
            }
        }
        changed
    }

    fn replace_retained(&mut self, snapshot: &JobNoticeHistory, clock: f64) {
        let now_ms = Utc::now().timestamp_millis();
        self.items.retain(|item| {
            item.job_notice
                .as_ref()
                .is_none_or(|key| snapshot.retained().iter().any(|notice| &notice.key == key))
        });
        for notice in snapshot.retained() {
            if let Some(item) = self
                .items
                .iter_mut()
                .find(|item| item.job_notice.as_ref() == Some(&notice.key))
            {
                item.read |= notice.read;
            } else {
                self.append_retained(
                    notice,
                    clock - now_ms.saturating_sub(notice.observed_at_ms).max(0) as f64 / 1000.0,
                );
            }
        }
        self.items.make_contiguous().sort_by(item_order);
        while self.items.len() > MAX_ITEMS {
            self.items.pop_front();
        }
        self.retain_visible_selection();
    }

    fn append_retained(&mut self, notice: &JobNotice, at: f64) {
        let key = &notice.key;
        let (title, detail, tone) = match key.kind {
            JobNoticeKind::Started => (
                "Job started",
                "This workstation observed the job start. Open Jobs & Logs for current status.",
                StatusTone::Info,
            ),
            JobNoticeKind::Failed => (
                "Job failed",
                "This workstation observed the job fail. Open Jobs & Logs for details.",
                StatusTone::Err,
            ),
        };
        self.next_id += 1;
        self.items.push_back(Item {
            id: self.next_id,
            topic: Topic::Job(key.job_id.clone()),
            group_title: generic_job_label(&key.job_id),
            title: title.into(),
            detail: detail.into(),
            at,
            read: notice.read,
            tone,
            private_job: false,
            job_notice: Some(key.clone()),
            current_job_visibility: false,
        });
    }

    fn retain_visible_selection(&mut self) {
        self.expanded
            .retain(|topic| self.items.iter().any(|item| &item.topic == topic));
        if self
            .selected
            .is_some_and(|id| self.items.iter().all(|item| item.id != id))
        {
            self.selected = None;
        }
    }
}

fn generic_job_label(job_id: &str) -> String {
    let start = job_id
        .char_indices()
        .rev()
        .nth(7)
        .map_or(0, |(start, _)| start);
    format!("Job · {}", &job_id[start..])
}

impl State {
    /// Add two synthetic retained events to the production drawer for visual review.
    #[doc(hidden)]
    pub fn seed_inbox_retained_job_fixture(&mut self) {
        if !self.fixture {
            return;
        }
        let job_id = "fixture-retained-work";
        let now = Utc::now().timestamp_millis();
        for (kind, age) in [
            (JobNoticeKind::Started, 120_i64),
            (JobNoticeKind::Failed, 90),
        ] {
            self.inbox.append_retained(
                &JobNotice {
                    key: JobNoticeKey {
                        job_id: job_id.into(),
                        job_digest: format!("{:x}", Sha256::digest(job_id.as_bytes())),
                        kind,
                        transition: None,
                    },
                    observed_at_ms: now - age * 1000,
                    read: false,
                },
                self.clock - age as f64,
            );
        }
        self.inbox.expanded.insert(Topic::Job(job_id.into()));
        self.inbox.open = true;
    }

    fn job_notice_binding(&self) -> Option<Binding> {
        if self.fixture
            || !self.permission(Permission::ReadWorkstationActivity)
            || self.workspaces.container.is_none()
        {
            return None;
        }
        let context = self.registration.context.as_ref()?;
        let origin = canonical_target_origin(&self.connection.origin.text).ok()?;
        let scoped = serde_json::to_vec(&(
            self.backend_kind,
            origin,
            &context.deployment_id,
            &context.registration_id,
            &context.workstation_id,
            context.registration_revision,
            context.scope_revision,
            &context.configuration_epoch,
            &context.access_epoch,
        ))
        .ok()?;
        Some(Binding {
            key: format!("{:x}", Sha256::digest(scoped)),
            origin_revision: self.origin_revision,
            context_revision: context.context_revision,
        })
    }

    pub(crate) fn reset_inbox_for_authority(&mut self) {
        self.inbox.history = History::default();
        self.inbox.reset();
        self.inbox.scope = None;
    }

    pub(super) fn bind_job_notice_history(&mut self) -> bool {
        let binding = self.job_notice_binding();
        if self.inbox.history.binding == binding {
            return false;
        }
        let previously_bound = self.inbox.history.bound();
        if previously_bound {
            self.reset_inbox_for_authority();
        }
        self.inbox.history = History {
            binding,
            ..History::default()
        };
        previously_bound
    }

    pub(super) fn drive_job_notice_history(&mut self) {
        let history = &mut self.inbox.history;
        if history.request.is_some()
            || history.retry_at > self.clock
            || (history.snapshot.is_some() && history.queued.is_empty())
        {
            return;
        }
        let (Some(binding), Some(container), Some(services)) = (
            history.binding.clone(),
            self.workspaces.container.clone(),
            &self.services,
        ) else {
            return;
        };
        let Some(ui) = services.ui.get().cloned() else {
            return;
        };
        let changes = if let Some(delete) = history
            .queued
            .iter()
            .position(|change| matches!(change, JobNoticeChange::Delete(_)))
        {
            vec![history.queued.remove(delete)]
        } else {
            std::mem::take(&mut history.queued)
        };
        history.request = Some(services.update_job_notice_history(ui, container, binding, changes));
    }

    fn complete_job_notice_history(
        &mut self,
        binding: Binding,
        result: Result<JobNoticeHistory, String>,
        changes: Vec<JobNoticeChange>,
        tx: &mut Tx,
    ) {
        if self.job_notice_binding().as_ref() != Some(&binding)
            || self.inbox.history.binding.as_ref() != Some(&binding)
        {
            return;
        }
        let history = &mut self.inbox.history;
        history.request = None;
        let deleting = if changes
            .iter()
            .any(|change| matches!(change, JobNoticeChange::Delete(_)))
        {
            history.deleting.take()
        } else {
            None
        };
        let focus = deleting.and_then(|id| {
            self.inbox
                .items
                .iter()
                .find(|item| item.id == id)
                .map(|item| item.topic.clone())
        });
        match result {
            Ok(snapshot) => {
                self.inbox.history.error = None;
                self.inbox.replace_retained(&snapshot, self.clock);
                self.inbox.history.snapshot = Some(snapshot);
                if let Some(topic) = focus {
                    self.return_from_deleted_notice(&topic, tx);
                }
            }
            Err(error) => {
                tracing::warn!("Could not update the protected Inbox file: {error}");
                let history = &mut self.inbox.history;
                history.error = Some(if deleting.is_some() {
                    "The notification could not be deleted. Try again."
                } else {
                    "Saved job notifications are temporarily unavailable."
                });
                history.retry_at = if deleting.is_some() {
                    0.0
                } else {
                    self.clock + 5.0
                };
                let concurrent = std::mem::take(&mut history.queued);
                for change in changes {
                    if !matches!(change, JobNoticeChange::Delete(_)) {
                        history.queue(change);
                    }
                }
                for change in concurrent {
                    history.queue(change);
                }
            }
        }
    }

    pub(super) fn delete_job_notice(&mut self, id: u64, tx: &mut Tx) {
        if !self.inbox.open || self.inbox.history.deleting.is_some() {
            return;
        }
        let Some(item) = self.inbox.items.iter().find(|item| item.id == id) else {
            return;
        };
        let Some(key) = item.job_notice.clone() else {
            return;
        };
        if !self.inbox.expanded.contains(&item.topic) {
            return;
        }
        if self.fixture {
            let topic = item.topic.clone();
            self.inbox.items.retain(|item| item.id != id);
            self.inbox.retain_visible_selection();
            self.return_from_deleted_notice(&topic, tx);
        } else if self
            .job_notice_binding()
            .is_some_and(|binding| self.inbox.history.binding.as_ref() == Some(&binding))
        {
            self.inbox.history.deleting = Some(id);
            self.inbox.history.error = None;
            self.inbox.history.retry_at = 0.0;
            self.inbox.history.queue(JobNoticeChange::Delete(key));
            self.drive_job_notice_history();
        }
    }

    fn return_from_deleted_notice(&self, topic: &Topic, tx: &mut Tx) {
        if !self.inbox.open {
            return;
        }
        let key = if self
            .inbox
            .filtered_groups()
            .iter()
            .any(|group| group.topic == topic)
        {
            format!("{}-toggle", topic.key())
        } else {
            "inbox-close".into()
        };
        tx.return_focus(key.clone(), tx.focus_visible());
        tx.keep_on_screen(key);
    }
}

impl GatewayEffects {
    fn update_job_notice_history(
        &self,
        ui: rune::AppHandle<State>,
        container: Arc<NativeContainer>,
        binding: Binding,
        changes: Vec<JobNoticeChange>,
    ) -> RequestTask {
        let key = binding.key.clone();
        let task = self.runtime.spawn(async move {
            let work = changes.clone();
            let result = tokio::task::spawn_blocking(move || {
                container.update_job_notice_history(&key, &work)
            })
            .await
            .map_err(|error| error.to_string())
            .and_then(|result| result.map_err(|error| error.to_string()));
            ui.update(move |state, tx| {
                state.complete_job_notice_history(binding, result, changes, tx)
            });
        });
        RequestTask(task.abort_handle())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_running_is_recorded_and_new_failure_time_distinguishes_retries() {
        use crate::inbox::Link;
        use crate::inbox::Observation;

        let running = JobNoticeKey {
            job_id: "job-running".into(),
            job_digest: "a".repeat(64),
            kind: JobNoticeKind::Started,
            transition: Some(100),
        };
        let failed = JobNoticeKey {
            job_id: "job-existing-fail".into(),
            job_digest: "b".repeat(64),
            kind: JobNoticeKind::Failed,
            transition: Some(200),
        };
        let observe = |inbox: &mut Inbox, now, data: &[(JobNoticeKey, &str)]| {
            inbox.observe(
                now,
                Observation {
                    scope: 1,
                    link: Link::Connected,
                    jobs: data
                        .iter()
                        .map(|(key, state)| {
                            (key.job_id.clone(), "Safe title".into(), (*state).into())
                        })
                        .collect(),
                    job_notice_keys: data
                        .iter()
                        .map(|(key, _)| (key.job_id.clone(), key.clone()))
                        .collect(),
                    observed_at_ms: 1000,
                    ..Observation::default()
                },
            );
        };
        let mut inbox = Inbox::default();
        observe(
            &mut inbox,
            0.0,
            &[(running.clone(), "running"), (failed.clone(), "failed")],
        );
        assert_eq!(inbox.history.queued.len(), 1, "old failures are quiet");
        observe(
            &mut inbox,
            1.0,
            &[(running.clone(), "running"), (failed.clone(), "failed")],
        );
        assert_eq!(inbox.history.queued.len(), 1, "repeated statuses are quiet");
        let new_failure = JobNoticeKey {
            transition: Some(300),
            ..failed
        };
        let running_failed = JobNoticeKey {
            kind: JobNoticeKind::Failed,
            transition: Some(300),
            ..running
        };
        observe(
            &mut inbox,
            2.0,
            &[(new_failure, "failed"), (running_failed, "failed")],
        );
        assert_eq!(
            inbox.history.queued.len(),
            3,
            "new failures reach the persistence worker"
        );
    }
}
