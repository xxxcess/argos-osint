//! Local retention for the Inbox's scoped job notifications.

use std::collections::BTreeMap;

use anyhow::Context;
use anyhow::Result;
use fleetworks_contracts::chat::QA_MAX_INBOX_NOTIFICATION_ITEMS;
use rustix::fs::AtFlags;
use serde::Deserialize;
use serde::Serialize;

use super::fs;
use super::fs::ObjectIdentity;
use super::NativeContainer;

const VERSION: u16 = 1;
const MAX_RETAINED: usize = QA_MAX_INBOX_NOTIFICATION_ITEMS as usize;

/// Lifecycle events retained by the workstation Inbox.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobNoticeKind {
    Started,
    Failed,
}

/// One observed lifecycle event. The transition is the server's UTC time in nanoseconds.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JobNoticeKey {
    pub job_id: String,
    /// Stable SHA-256 of the job ID. Deleted-event journals retain this digest only.
    pub job_digest: String,
    pub kind: JobNoticeKind,
    pub transition: Option<i64>,
}

impl JobNoticeKey {
    /// An undated legacy observation cannot distinguish separate events of the same kind.
    #[must_use]
    pub fn matches(&self, other: &Self) -> bool {
        self.job_id == other.job_id
            && self.kind == other.kind
            && (self.transition.is_none()
                || other.transition.is_none()
                || self.transition == other.transition)
    }

    fn valid(&self) -> bool {
        !self.job_id.is_empty()
            && self.job_id.len() <= 512
            && !self.job_id.chars().any(char::is_control)
            && super::valid_marker(&self.job_digest)
            && self.transition.is_none_or(|time| time > 0)
    }
}

/// Job identity, safe lifecycle status and local observation time. No private job content.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JobNotice {
    pub key: JobNoticeKey,
    pub observed_at_ms: i64,
    pub read: bool,
}

/// Changes that can be merged while holding the installation lock.
#[derive(Clone)]
pub enum JobNoticeChange {
    Record(JobNotice),
    MarkRead(JobNoticeKey),
    Delete(JobNoticeKey),
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JobNoticeHistory {
    retained: Vec<JobNotice>,
}

impl JobNoticeHistory {
    #[must_use]
    pub fn retained(&self) -> &[JobNotice] {
        &self.retained
    }

    #[must_use]
    pub fn contains_event(&self, key: &JobNoticeKey) -> bool {
        self.retained.iter().any(|notice| notice.key.matches(key))
    }

    fn apply(&mut self, change: &JobNoticeChange) -> Result<bool> {
        match change {
            JobNoticeChange::Record(notice) => {
                anyhow::ensure!(
                    notice.key.valid() && notice.observed_at_ms > 0,
                    "Invalid job notification"
                );
                if self.contains_event(&notice.key) {
                    return Ok(false);
                }
                self.retained.push(notice.clone());
                self.retained.sort_by_key(|notice| notice.observed_at_ms);
                if self.retained.len() > MAX_RETAINED {
                    self.retained.remove(0);
                }
                Ok(true)
            }
            JobNoticeChange::MarkRead(key) => {
                if let Some(notice) = self.retained.iter_mut().find(|notice| &notice.key == key) {
                    return Ok(!std::mem::replace(&mut notice.read, true));
                }
                Ok(false)
            }
            JobNoticeChange::Delete(_) => anyhow::bail!("Inbox deletion needs a separate update"),
        }
    }

    fn valid(&self) -> bool {
        self.retained.len() <= MAX_RETAINED
            && self
                .retained
                .iter()
                .all(|notice| notice.key.valid() && notice.observed_at_ms > 0)
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Journal<T> {
    version: u16,
    authority: String,
    installation_marker: String,
    root_object: ObjectIdentity,
    history: T,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Deletion {
    kind: JobNoticeKind,
    transition: Option<i64>,
}

impl Deletion {
    fn matches(&self, key: &JobNoticeKey) -> bool {
        self.kind == key.kind
            && (self.transition.is_none()
                || key.transition.is_none()
                || self.transition == key.transition)
    }
}

type DeletedEvents = (Journal<Vec<Deletion>>, Option<ObjectIdentity>);

impl NativeContainer {
    /// Load and optionally update Inbox history for the app's verified authority identity.
    /// Empty reads leave the filesystem unchanged. Call on a blocking worker.
    pub fn update_job_notice_history(
        &self,
        authority: &str,
        changes: &[JobNoticeChange],
    ) -> Result<JobNoticeHistory> {
        anyhow::ensure!(super::valid_marker(authority), "Invalid Inbox authority");
        let _guard = self.locked_settings()?;
        let name = format!(".native-inbox-jobs-{authority}.json");
        let (mut journal, object) =
            self.read_notice_journal::<JobNoticeHistory>(&name, authority)?;
        anyhow::ensure!(
            journal.history.valid(),
            "Inbox history format requires repair"
        );
        let mut deleted = BTreeMap::new();
        let before = journal.history.retained.len();
        let retained = std::mem::take(&mut journal.history.retained);
        for notice in retained {
            let deletions = self.notice_deletions(authority, &notice.key, &mut deleted)?;
            if !deletions
                .0
                .history
                .iter()
                .any(|entry| entry.matches(&notice.key))
            {
                journal.history.retained.push(notice);
            }
        }
        let mut changed = journal.history.retained.len() != before;

        if let [JobNoticeChange::Delete(key)] = changes {
            anyhow::ensure!(key.valid(), "Invalid job notification identity");
            let (deletions, current) = self.notice_deletions(authority, key, &mut deleted)?;
            if !deletions.history.iter().any(|entry| entry.matches(key)) {
                deletions.history.push(Deletion {
                    kind: key.kind,
                    transition: key.transition,
                });
                self.revalidate_root()?;
                fs::write_json(
                    &self.root_fd,
                    &deletions_name(authority, key),
                    deletions,
                    *current,
                )?;
            }
            journal
                .history
                .retained
                .retain(|notice| !notice.key.matches(key));
            // The durable marker commits the delete. A stopped or failed compaction is repaired
            // by the next read and never exposes the deleted item again.
            let _ = fs::write_json(&self.root_fd, &name, &journal, object);
            return Ok(journal.history);
        }

        for change in changes {
            if let JobNoticeChange::Record(notice) = change {
                let deletions = self.notice_deletions(authority, &notice.key, &mut deleted)?;
                if deletions
                    .0
                    .history
                    .iter()
                    .any(|entry| entry.matches(&notice.key))
                {
                    continue;
                }
            }
            changed |= journal.history.apply(change)?;
        }
        if changed {
            self.revalidate_root()?;
            fs::write_json(&self.root_fd, &name, &journal, object)?;
        }
        Ok(journal.history)
    }

    fn notice_deletions<'a>(
        &self,
        authority: &str,
        key: &JobNoticeKey,
        cache: &'a mut BTreeMap<String, DeletedEvents>,
    ) -> Result<&'a mut DeletedEvents> {
        anyhow::ensure!(key.valid(), "Invalid job notification identity");
        match cache.entry(key.job_digest.clone()) {
            std::collections::btree_map::Entry::Occupied(entry) => Ok(entry.into_mut()),
            std::collections::btree_map::Entry::Vacant(entry) => {
                let stored =
                    self.read_notice_journal(&deletions_name(authority, key), authority)?;
                Ok(entry.insert(stored))
            }
        }
    }

    fn read_notice_journal<T: serde::de::DeserializeOwned + Default>(
        &self,
        name: &str,
        authority: &str,
    ) -> Result<(Journal<T>, Option<ObjectIdentity>)> {
        let result = match rustix::fs::statat(&self.root_fd, name, AtFlags::SYMLINK_NOFOLLOW) {
            Err(rustix::io::Errno::NOENT) => (
                Journal {
                    version: VERSION,
                    authority: authority.to_owned(),
                    installation_marker: self.installation_marker.clone(),
                    root_object: self.catalog_root_object,
                    history: T::default(),
                },
                None,
            ),
            Ok(_) => {
                let (journal, object) = fs::read_json::<Journal<T>>(&self.root_fd, name)?;
                anyhow::ensure!(
                    journal.version == VERSION
                        && journal.authority == authority
                        && journal.installation_marker == self.installation_marker
                        && journal.root_object == self.catalog_root_object,
                    "Inbox history ownership or format requires repair"
                );
                (journal, Some(object))
            }
            Err(error) => return Err(error).context("Inspect protected Inbox history"),
        };
        Ok(result)
    }
}

fn deletions_name(authority: &str, key: &JobNoticeKey) -> String {
    format!(
        ".native-inbox-deletions-{authority}-{}.json",
        key.job_digest
    )
}

#[cfg(test)]
mod tests;
