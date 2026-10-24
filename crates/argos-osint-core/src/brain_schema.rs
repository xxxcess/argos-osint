//! Schema-37/38 Brain migration: typed columns on `memory_metadata` and the
//! claim/inference kind migration preserving memory IDs (spec §4).

/// Typed columns added to `memory_metadata` (additive, idempotent).
const METADATA_COLUMNS: [(&str, &str); 6] = [
    ("payload_json", "payload_json TEXT NOT NULL DEFAULT '{}'"),
    (
        "payload_schema",
        "payload_schema INTEGER NOT NULL DEFAULT 0",
    ),
    ("revision", "revision INTEGER NOT NULL DEFAULT 1"),
    ("lifecycle", "lifecycle TEXT NOT NULL DEFAULT 'active'"),
    ("created_at", "created_at TEXT NOT NULL DEFAULT ''"),
    ("updated_at", "updated_at TEXT NOT NULL DEFAULT ''"),
];

#[cfg(test)]
#[path = "brain_schema_tests.rs"]
mod tests;
