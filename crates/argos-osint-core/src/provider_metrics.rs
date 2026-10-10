//! Read-only contract for provider capacity, queue and quota records.
//!
//! Provider limits, shared admission, fair queuing and retry/fallback
//! orchestration are owned by `ARGOS_PROVIDER_REQUEST_ORCHESTRATION_SPEC.md`.
//! This module is the *consumer* seam: it publishes the snapshot DTOs the
//! Profile dashboard renders and the quota setting DTOs both the portable
//! configuration transfer and the companion's limit editors share.
//!
//! It deliberately contains no scheduler, no admission policy and no retry
//! policy. When the companion has not published a snapshot, `available` is
//! false and every rendered metric is reported as unavailable (never zero).

use serde::{Deserialize, Serialize};

/// One quota group's settings: the portable, user-editable rate policy.
///
/// This is the canonical DTO shared by the companion's limit editors, the
/// profile configuration transfer (`rate_limits`) and the Models widgets.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct QuotaSetting {
    /// Stable quota group identity, e.g. `openrouter`, `firecrawl`.
    pub quota_group_id: String,
    /// Scope selector: `*` for the whole account, or a model id.
    #[serde(default)]
    pub scope: String,
    /// Provider-verified sustained requests per minute, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_rpm: Option<u32>,
    /// Provider-verified tokens per minute, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_tpm: Option<u32>,
    /// Provider-verified requests per day, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_rpd: Option<u32>,
    /// Argos's own local RPM override. None inherits the verified limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_rpm: Option<u32>,
    /// Maximum simultaneous in-flight requests for this group. Must be >= 1.
    #[serde(default = "default_concurrency")]
    pub concurrency: u32,
    /// Where the verified numbers came from: `provider_docs`, `probe`,
    /// `desk_assumption`, `unset`.
    #[serde(default)]
    pub source: String,
    /// RFC3339 timestamp of the last verification.
    #[serde(default)]
    pub verified_at: String,
}

fn default_concurrency() -> u32 {
    1
}

impl QuotaSetting {
    pub fn is_unset(&self) -> bool {
        self.verified_rpm.is_none()
            && self.verified_tpm.is_none()
            && self.verified_rpd.is_none()
            && self.local_rpm.is_none()
            && (self.source.is_empty() || self.source == "unset")
    }

    /// Effective requests per minute: the local override wins over the verified
    /// limit. None when neither is configured.
    pub fn effective_rpm(&self) -> Option<u32> {
        self.local_rpm.or(self.verified_rpm)
    }
}

/// One row of the live provider capacity table.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CapacityRow {
    pub provider: String,
    pub quota_group: String,
    pub scope: String,
    pub sends_60s: u32,
    pub rpm_limit: Option<u32>,
    pub effective_rpm: Option<u32>,
    pub pace_per_min: Option<f64>,
    pub active: u32,
    pub max_concurrency: Option<u32>,
    pub queued: u32,
    pub oldest_wait_ms: Option<u64>,
    pub cooldown_ms: Option<u64>,
    pub quota_source: String,
}

/// One row of the queue-snapshot table (enqueue-to-send delay percentiles).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QueueRow {
    pub provider: String,
    pub scope: String,
    pub p50_ms: Option<u64>,
    pub p95_ms: Option<u64>,
    pub samples: u64,
}

/// Read-only capacity snapshot. `available == false` means the companion has
/// not published anything yet: the dashboard must render unavailable, not zero.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CapacitySnapshot {
    pub available: bool,
    pub captured_at: String,
    pub capacity: Vec<CapacityRow>,
    pub queue: Vec<QueueRow>,
}

impl CapacitySnapshot {
    pub fn unavailable() -> Self {
        Self::default()
    }
}

/// Rows read back from `telemetry_provider_capacity` by the aggregation worker.
#[derive(Clone, Debug, Default)]
pub struct StoredCapacityRow {
    pub captured_at: String,
    pub provider: String,
    pub quota_group: String,
    pub scope: String,
    pub sends_60s: u32,
    pub rpm_limit: Option<u32>,
    pub pace_per_min: Option<f64>,
    pub active: u32,
    pub max_concurrency: Option<u32>,
    pub queued: u32,
    pub oldest_wait_ms: Option<u64>,
    pub cooldown_ms: Option<u64>,
    pub quota_source: String,
}

/// Publish a snapshot supplied by the orchestration companion.
///
/// The companion is the only writer; this crate replaces the cache and returns
/// the row count so callers can confirm the write took effect.
pub fn cache_capacity_rows(
    conn: &rusqlite::Connection,
    rows: &[StoredCapacityRow],
) -> anyhow::Result<usize> {
    conn.execute("DELETE FROM telemetry_provider_capacity", [])?;
    let mut written = 0usize;
    for row in rows {
        conn.execute(
            "INSERT OR REPLACE INTO telemetry_provider_capacity
             (captured_at, provider, quota_group, scope, sends_60s, rpm_limit, pace_per_min,
              active, max_concurrency, queued, oldest_wait_ms, cooldown_ms, quota_source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            rusqlite::params![
                row.captured_at,
                row.provider,
                row.quota_group,
                row.scope,
                row.sends_60s,
                row.rpm_limit,
                row.pace_per_min,
                row.active,
                row.max_concurrency,
                row.queued,
                row.oldest_wait_ms,
                row.cooldown_ms,
                row.quota_source,
            ],
        )?;
        written += 1;
    }
    Ok(written)
}

/// Load the most recent cached snapshot. Returns an unavailable snapshot when
/// the companion has not published one.
pub fn capacity_snapshot(conn: &rusqlite::Connection) -> CapacitySnapshot {
    let mut stmt = match conn.prepare(
        "SELECT captured_at, provider, quota_group, scope, sends_60s, rpm_limit, pace_per_min,
                active, max_concurrency, queued, oldest_wait_ms, cooldown_ms, quota_source
         FROM telemetry_provider_capacity ORDER BY captured_at DESC, provider, quota_group, scope",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return CapacitySnapshot::unavailable(),
    };
    let rows = stmt.query_map([], |row| {
        Ok(StoredCapacityRow {
            captured_at: row.get(0)?,
            provider: row.get(1)?,
            quota_group: row.get(2)?,
            scope: row.get(3)?,
            sends_60s: row.get(4)?,
            rpm_limit: row.get(5)?,
            pace_per_min: row.get(6)?,
            active: row.get(7)?,
            max_concurrency: row.get(8)?,
            queued: row.get(9)?,
            oldest_wait_ms: row.get(10)?,
            cooldown_ms: row.get(11)?,
            quota_source: row.get(12)?,
        })
    });
    let Ok(rows) = rows else {
        return CapacitySnapshot::unavailable();
    };
    let mut capacity: Vec<CapacityRow> = Vec::new();
    let mut latest_captured_at = String::new();
    for row in rows.flatten() {
        if row.captured_at > latest_captured_at {
            latest_captured_at = row.captured_at.clone();
        }
        capacity.push(CapacityRow {
            provider: row.provider,
            quota_group: row.quota_group,
            scope: row.scope,
            sends_60s: row.sends_60s,
            rpm_limit: row.rpm_limit,
            effective_rpm: row.rpm_limit,
            pace_per_min: row.pace_per_min,
            active: row.active,
            max_concurrency: row.max_concurrency,
            queued: row.queued,
            oldest_wait_ms: row.oldest_wait_ms,
            cooldown_ms: row.cooldown_ms,
            quota_source: row.quota_source,
        });
    }
    if capacity.is_empty() {
        return CapacitySnapshot::unavailable();
    }
    CapacitySnapshot {
        available: true,
        captured_at: latest_captured_at,
        capacity,
        queue: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("schema_telemetry.sql"))
            .unwrap();
        conn
    }

    #[test]
    fn missing_companion_snapshot_is_unavailable_not_zero() {
        let conn = memory();
        let snapshot = capacity_snapshot(&conn);
        assert!(!snapshot.available);
        assert!(snapshot.capacity.is_empty());
    }

    #[test]
    fn cached_rows_become_an_available_snapshot() {
        let conn = memory();
        let rows = vec![StoredCapacityRow {
            captured_at: "2026-10-09T12:00:00Z".into(),
            provider: "openrouter".into(),
            quota_group: "openrouter".into(),
            scope: "*".into(),
            sends_60s: 12,
            rpm_limit: Some(60),
            pace_per_min: Some(12.0),
            active: 2,
            max_concurrency: Some(4),
            queued: 1,
            oldest_wait_ms: Some(320),
            cooldown_ms: None,
            quota_source: "provider_docs".into(),
        }];
        assert_eq!(cache_capacity_rows(&conn, &rows).unwrap(), 1);
        let snapshot = capacity_snapshot(&conn);
        assert!(snapshot.available);
        assert_eq!(snapshot.capacity.len(), 1);
        assert_eq!(snapshot.capacity[0].sends_60s, 12);
        assert_eq!(snapshot.capacity[0].rpm_limit, Some(60));
    }

    #[test]
    fn quota_setting_effective_rpm_prefers_the_local_override() {
        let mut setting = QuotaSetting {
            quota_group_id: "openrouter".into(),
            verified_rpm: Some(60),
            ..QuotaSetting::default()
        };
        assert_eq!(setting.effective_rpm(), Some(60));
        setting.local_rpm = Some(30);
        assert_eq!(setting.effective_rpm(), Some(30));
        assert!(!setting.is_unset());
    }

    #[test]
    fn unset_quota_setting_is_recognised() {
        assert!(QuotaSetting::default().is_unset());
    }
}
