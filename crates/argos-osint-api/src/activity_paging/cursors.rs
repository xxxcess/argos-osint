//! Ephemeral metadata-only page cursors. Expiry belongs to the first window,
//! not the latest continuation; no private row payload or cohort IDs are cached.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use argos_osint_core::storage::jobs_paging::Anchor;
use argos_osint_core::storage::jobs_paging::CreationWindow;
use argos_osint_core::storage::jobs_paging::Filters;
use fleetworks_contracts::context::ActivityScope;
use tokio::sync::Mutex;

const TTL: Duration = Duration::from_secs(120);
const MAX_RECORDS: usize = 256;
const MAX_BYTES: usize = 2 * 1024 * 1024;

pub(super) struct Snapshot {
    pub binding: String,
    pub scope: ActivityScope,
    pub workspace_filter: Option<String>,
    pub filters: Filters,
    pub window: CreationWindow,
    pub limit: usize,
    expires: Instant,
}
impl Snapshot {
    pub fn new(
        binding: String,
        scope: ActivityScope,
        workspace_filter: Option<String>,
        filters: Filters,
        window: CreationWindow,
        limit: usize,
    ) -> Self {
        Self {
            binding,
            scope,
            workspace_filter,
            filters,
            window,
            limit,
            expires: Instant::now() + TTL,
        }
    }
    fn bytes(&self) -> usize {
        self.binding.len()
            + self.scope.deployment_id.len()
            + self.scope.workstation_id.len()
            + self.workspace_filter.as_ref().map_or(0, String::len)
            + self.filters.app.len()
            + self.filters.operation.len()
            + self.filters.kind.len()
            + self.filters.correlation_id.len()
            + self.filters.text.len()
            + self.filters.since.len()
            + self.window.cutoff.len()
            + self.window.upper.len()
            + 1024
    }
}
#[derive(Clone)]
pub(super) struct Record {
    pub snapshot: Arc<Snapshot>,
    pub after: Option<Anchor>,
    issued: Instant,
}
impl Record {
    pub fn new(snapshot: Arc<Snapshot>, after: Option<Anchor>) -> Self {
        Self {
            snapshot,
            after,
            issued: Instant::now(),
        }
    }
    fn bytes(&self) -> usize {
        self.snapshot.bytes()
            + self
                .after
                .as_ref()
                .map_or(0, |after| after.created_at.len() + after.id.len())
            + 256
    }
}
#[derive(Default)]
struct Entries {
    rows: BTreeMap<String, Record>,
    bytes: usize,
}
impl Entries {
    fn remove(&mut self, token: &str) {
        if let Some(record) = self.rows.remove(token) {
            self.bytes = self.bytes.saturating_sub(record.bytes());
        }
    }
    fn prune_at(&mut self, now: Instant) {
        let expired: Vec<_> = self
            .rows
            .iter()
            .filter(|(_, record)| record.snapshot.expires <= now)
            .map(|(token, _)| token.clone())
            .collect();
        for token in expired {
            self.remove(&token);
        }
    }
}
#[derive(Default)]
pub struct JobsCursors {
    entries: Mutex<Entries>,
}
impl JobsCursors {
    pub fn new() -> Self {
        Self::default()
    }
    pub(super) async fn issue(&self, record: Record) -> Result<String, ()> {
        if record.bytes() > MAX_BYTES || record.snapshot.expires <= Instant::now() {
            return Err(());
        }
        let token = tokio::task::spawn_blocking(argos_osint_core::storage::home::random_secret)
            .await
            .map_err(|_| ())?
            .map_err(|_| ())?;
        let mut entries = self.entries.lock().await;
        entries.prune_at(Instant::now());
        while entries.rows.len() >= MAX_RECORDS || entries.bytes + record.bytes() > MAX_BYTES {
            let oldest = entries
                .rows
                .iter()
                .min_by_key(|(_, record)| record.issued)
                .map(|(token, _)| token.clone())
                .ok_or(())?;
            entries.remove(&oldest);
        }
        if entries.rows.contains_key(&token) {
            return Err(());
        }
        entries.bytes += record.bytes();
        entries.rows.insert(token.clone(), record);
        Ok(token)
    }
    pub(super) async fn get(&self, token: &str, binding: &str) -> Result<Record, ()> {
        self.get_at(token, binding, Instant::now()).await
    }
    async fn get_at(&self, token: &str, binding: &str, now: Instant) -> Result<Record, ()> {
        if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(());
        }
        let mut entries = self.entries.lock().await;
        entries.prune_at(now);
        let record = entries.rows.get(token).ok_or(())?;
        if record.snapshot.binding != binding {
            return Err(());
        }
        Ok(record.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot() -> Snapshot {
        Snapshot::new("fixture-proof".into(),serde_json::from_value(serde_json::json!({"deployment_id":"fixture-deployment","workstation_id":"fixture-node","scope_revision":1})).expect("scope"),None,Filters::default(),CreationWindow::at(chrono::Utc::now()),50)
    }
    #[tokio::test]
    async fn exact_expiry_is_anchored_to_first_window_and_proof_mismatch_never_widens() {
        let cache = JobsCursors::new();
        let snapshot = Arc::new(snapshot());
        let expires = snapshot.expires;
        let first = cache
            .issue(Record::new(snapshot.clone(), None))
            .await
            .expect("firstwindow cursor");
        let next = cache
            .issue(Record::new(
                snapshot,
                Some(
                    Anchor::new("2026-11-12T12:00:00Z".into(), "fixture-job".into())
                        .expect("anchor"),
                ),
            ))
            .await
            .expect("samewindow continuation");
        assert!(cache.get(&first, "wrong-proof").await.is_err());
        assert!(cache
            .get_at(&next, "fixture-proof", expires - Duration::from_nanos(1))
            .await
            .is_ok());
        assert!(cache
            .get_at(&first, "fixture-proof", expires)
            .await
            .is_err());
        assert!(
            cache.get_at(&next, "fixture-proof", expires).await.is_err(),
            "a continuation never extends120second lifetime"
        );
    }
    #[tokio::test]
    async fn oversized_metadata_is_rejected_and_eviction_is_bounded_without_payload_caching() {
        let cache = JobsCursors::new();
        let mut oversized = snapshot();
        oversized.filters.text = "x".repeat(MAX_BYTES);
        assert!(cache
            .issue(Record::new(Arc::new(oversized), None))
            .await
            .is_err());
        let snapshot = Arc::new(snapshot());
        let mut first = None;
        for _ in 0..=MAX_RECORDS {
            let token = cache
                .issue(Record::new(snapshot.clone(), None))
                .await
                .expect("bounded cursor");
            if first.is_none() {
                first = Some(token);
            }
        }
        let entries = cache.entries.lock().await;
        assert!(entries.rows.len() <= MAX_RECORDS && entries.bytes <= MAX_BYTES);
        assert!(!entries
            .rows
            .contains_key(&first.expect("firstissued token")));
    }
}
