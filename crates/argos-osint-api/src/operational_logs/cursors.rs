//! Opaque bounded read cursors; no rows, raw sequences or authority reach JS.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use argos_osint_core::storage::operational_logs::LogFilters;
use argos_osint_core::storage::operational_logs::LogSelection;
use fleetworks_contracts::context::ActivityScope;
use tokio::sync::Mutex;

const TTL: Duration = Duration::from_secs(120);
const MAX_RECORDS: usize = 128;
const MAX_BYTES: usize = 8 * 1024 * 1024;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Kind {
    Page,
    Export,
    Follow,
}
#[derive(Clone)]
pub(super) struct Snapshot {
    pub binding: String,
    pub scope: ActivityScope,
    pub workspace_filter: Option<String>,
    pub selection: LogSelection,
    pub filters: LogFilters,
    pub limit: usize,
    pub cohort_digest: String,
    pub filter_digest: String,
}
impl Snapshot {
    fn bytes(&self) -> usize {
        self.binding.len()
            + self.cohort_digest.len()
            + self.filter_digest.len()
            + self.scope.deployment_id.len()
            + self.scope.workstation_id.len()
            + self.workspace_filter.as_ref().map_or(0, String::len)
            + self.selection.job_id.as_ref().map_or(0, String::len)
            + self.filters.app.len()
            + self.filters.text.len()
            + self.filters.since.len()
            + 1024
    }
}
#[derive(Clone)]
pub(super) struct Record {
    pub snapshot: Arc<Snapshot>,
    pub kind: Kind,
    pub upper: Option<i64>,
    pub boundary: Option<i64>,
    expires: Instant,
    issued: Instant,
}
impl Record {
    pub fn next(
        snapshot: Arc<Snapshot>,
        kind: Kind,
        upper: Option<i64>,
        boundary: Option<i64>,
    ) -> Self {
        let now = Instant::now();
        Self {
            snapshot,
            kind,
            upper,
            boundary,
            issued: now,
            expires: now + TTL,
        }
    }
    fn bytes(&self) -> usize {
        self.snapshot.bytes() + 256
    }
}
#[derive(Default)]
struct Entries {
    rows: BTreeMap<String, Record>,
    bytes: usize,
}
/// Process-local read cache only. Restart/expiry requires an explicit restart
/// of pagination; neither creates a broader query or skips a bounded cohort.
#[derive(Default)]
pub struct LogCursors {
    entries: Mutex<Entries>,
}
impl LogCursors {
    pub fn new() -> Self {
        Self::default()
    }
    pub(super) async fn issue(&self, record: Record) -> Result<String, ()> {
        if record.bytes() > MAX_BYTES {
            return Err(());
        }
        let token = tokio::task::spawn_blocking(argos_osint_core::storage::home::random_secret)
            .await
            .map_err(|_| ())?
            .map_err(|_| ())?;
        let mut entries = self.entries.lock().await;
        entries.prune();
        while entries.rows.len() >= MAX_RECORDS || entries.bytes + record.bytes() > MAX_BYTES {
            let oldest = entries
                .rows
                .iter()
                .min_by_key(|(_, record)| record.issued)
                .map(|(key, _)| key.clone())
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
    pub(super) async fn get(&self, token: &str, binding: &str, kind: Kind) -> Result<Record, ()> {
        if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(());
        }
        let mut entries = self.entries.lock().await;
        entries.prune();
        let record = entries.rows.get(token).ok_or(())?;
        if record.kind != kind || record.snapshot.binding != binding {
            return Err(());
        }
        Ok(record.clone())
    }
}
impl Entries {
    fn remove(&mut self, key: &str) {
        if let Some(record) = self.rows.remove(key) {
            self.bytes = self.bytes.saturating_sub(record.bytes());
        }
    }
    fn prune(&mut self) {
        let now = Instant::now();
        let old: Vec<_> = self
            .rows
            .iter()
            .filter(|(_, record)| record.expires <= now)
            .map(|(key, _)| key.clone())
            .collect();
        for key in old {
            self.remove(&key);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot() -> Arc<Snapshot> {
        Arc::new(Snapshot {binding:"fixture-proof".into(),scope:serde_json::from_value(serde_json::json!({"deployment_id":"fixture-deployment","workstation_id":"fixture-node","workspace_id":null,"scope_revision":1})).expect("scope"),workspace_filter:None,selection:LogSelection::workstation(),filters:LogFilters::default(),limit:100,cohort_digest:"fixture-complete-cohort".into(),filter_digest:"fixture-filter".into()})
    }
    #[tokio::test]
    async fn cursor_is_opaque_proof_bound_kind_bound_and_expiration_never_widens_query() {
        let cache = LogCursors::new();
        let token = cache
            .issue(Record::next(
                snapshot(),
                Kind::Page,
                Some(987654),
                Some(123456),
            ))
            .await
            .expect("opaque token");
        assert_eq!(token.len(), 64);
        assert!(cache.get(&token, "other-proof", Kind::Page).await.is_err());
        assert!(cache
            .get(&token, "fixture-proof", Kind::Follow)
            .await
            .is_err());
        assert_eq!(
            cache
                .get(&token, "fixture-proof", Kind::Page)
                .await
                .expect("same scoped cursor")
                .boundary,
            Some(123456)
        );
        cache
            .entries
            .lock()
            .await
            .rows
            .get_mut(&token)
            .expect("owned record")
            .expires = Instant::now();
        assert!(cache
            .get(&token, "fixture-proof", Kind::Page)
            .await
            .is_err());
    }
    #[tokio::test]
    async fn count_and_bytes_are_bounded_without_storing_log_payloads() {
        let cache = LogCursors::new();
        let mut first = None;
        for _ in 0..=MAX_RECORDS {
            let token = cache
                .issue(Record::next(snapshot(), Kind::Export, Some(1), None))
                .await
                .expect("bounded record");
            if first.is_none() {
                first = Some(token);
            }
        }
        let entries = cache.entries.lock().await;
        assert!(entries.rows.len() <= MAX_RECORDS && entries.bytes <= MAX_BYTES);
        assert!(!entries.rows.contains_key(&first.expect("first token")));
    }
}
