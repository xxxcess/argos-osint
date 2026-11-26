//! Embedded ownership and typed adaptation of the optional observation journal.

use std::fs;
use std::fs::OpenOptions;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::path::PathBuf;

use anyhow::ensure;
use anyhow::Result;
use argos_osint_api::inbox_qa::context;
use fleetworks_jobs::workstation_context as sqlite;

pub(crate) const DB_NAME: &str = "workstation-context.sqlite3";

/// Prepare the private file and check any retained sidecars under the state lock.
pub(crate) fn prepare_path(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => verify_file(&metadata)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)?;
        }
        Err(error) => return Err(error.into()),
    }
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut sidecar = path.as_os_str().to_os_string();
        sidecar.push(suffix);
        match fs::symlink_metadata(sidecar) {
            Ok(metadata) => verify_file(&metadata)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn verify_file(metadata: &fs::Metadata) -> Result<()> {
    // SAFETY: geteuid takes no arguments and always succeeds.
    let current_uid = unsafe { libc::geteuid() };
    ensure!(
        metadata.is_file()
            && metadata.nlink() == 1
            && metadata.uid() == current_uid
            && metadata.mode() & 0o077 == 0,
        "observation database path must be an owner-only regular file",
    );
    Ok(())
}

pub(crate) struct Journal(sqlite::Journal);

impl Journal {
    pub(crate) fn start(
        path: PathBuf,
        deployment: String,
        state_lock: impl Send + 'static,
    ) -> Self {
        Self(sqlite::Journal::start(path, deployment, state_lock, || {
            tracing::warn!(code = "embedded_workstation_context_unavailable");
        }))
    }

    pub(crate) async fn shutdown(&self) {
        self.0.shutdown().await;
    }
}

impl context::Journal for Journal {
    fn record(&self, record: context::Record) {
        let resource = record.value.resource();
        let (family, Some(key)) = resource.family_and_key() else {
            return;
        };
        if let Ok(payload) = serde_json::to_string(&record.value) {
            self.0.record(sqlite::Record {
                scope: record.scope,
                family,
                resource: key.to_owned(),
                generation: record.generation,
                revision: record.revision,
                observed_at_ms: record.observed_at_ms,
                payload,
                terminal: record.terminal,
            });
        }
    }

    fn query(&self, query: context::Query) -> context::QueryFuture<'_> {
        Box::pin(async move {
            let (family, key) = query.resource.family_and_key();
            let range = match query.range {
                context::Range::Current => sqlite::Range::Current,
                context::Range::History { since_ms, until_ms } => {
                    sqlite::Range::History { since_ms, until_ms }
                }
                _ => return None,
            };
            let page = self
                .0
                .query(sqlite::Lookup {
                    scope: query.scope,
                    family,
                    resource: key.map(str::to_owned),
                    range,
                })
                .await?;
            let count = page.entries.len();
            let entries: Vec<_> = page
                .entries
                .into_iter()
                .filter_map(|entry| {
                    serde_json::from_str(&entry.payload).ok().map(|value| {
                        context::Entry::new(
                            entry.sequence,
                            entry.observed_at_ms,
                            entry.recorded_at_ms,
                            value,
                        )
                    })
                })
                .collect();
            let incomplete = page.incomplete || entries.len() != count;
            Some(context::History::new(
                entries,
                page.coverage_started_at_ms,
                page.retrieved_at_ms,
                incomplete,
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;

    #[test]
    fn journal_open_validates_file_identity_and_sidecars_without_replacing_them() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DB_NAME);
        prepare_path(&path).unwrap();
        assert_eq!(fs::symlink_metadata(&path).unwrap().mode() & 0o777, 0o600);
        let alternate = dir.path().join("alternate");
        fs::write(&alternate, b"private state").unwrap();
        let wal = dir.path().join(format!("{DB_NAME}-wal"));
        symlink(&alternate, &wal).unwrap();
        assert!(prepare_path(&path).is_err());
        assert_eq!(fs::read(&alternate).unwrap(), b"private state");
    }
}
