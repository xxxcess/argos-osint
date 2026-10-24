//! Atlas work-unit manifests on SurrealDB: `atlas_work_unit`, the SurrealDB
//! side of `atlas_work_units` (`Store::atlas_save_unit_manifest`,
//! `atlas_get_unit_manifests`, `atlas_get_all_unit_manifests`).
//!
//! Record key: `codec::encode([run_id, unit_id])`. A save replaces the
//! whole manifest, like SQLite's `INSERT OR REPLACE`. The manifest is kept
//! as its JSON (`manifest_json`) beside the indexed `run_id` and `stage`.

use anyhow::{bail, Result};
use surrealdb::engine::any::Any;
use surrealdb::Surreal;

use super::codec::encode;
use crate::atlas_work::UnitManifest;

/// Work-unit schema (idempotent). Register in bootstrap as
/// `("atlas_work", atlas_work::SCHEMA)`.
pub const SCHEMA: &str = "DEFINE TABLE IF NOT EXISTS atlas_work_unit SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS run_id ON atlas_work_unit TYPE string;
DEFINE FIELD IF NOT EXISTS unit_id ON atlas_work_unit TYPE string;
DEFINE FIELD IF NOT EXISTS stage ON atlas_work_unit TYPE int;
DEFINE FIELD IF NOT EXISTS manifest_json ON atlas_work_unit TYPE string;
DEFINE INDEX IF NOT EXISTS atlas_work_unit_run_stage ON atlas_work_unit FIELDS run_id, stage;";

pub async fn apply_schema(db: &Surreal<Any>) -> Result<()> {
    db.query(SCHEMA).await?.check()?;
    Ok(())
}

/// `Store::atlas_save_unit_manifest` (replace).
pub async fn save_unit_manifest(db: &Surreal<Any>, m: &UnitManifest) -> Result<()> {
    if m.run_id.is_empty() || m.unit_id.is_empty() {
        bail!("atlas_work_unit: empty id");
    }
    db.query(
        "UPSERT type::record('atlas_work_unit', $key) CONTENT {
           run_id: $run, unit_id: $unit, stage: $stage, manifest_json: $json
         } RETURN NONE",
    )
    .bind(("key", encode(&[&m.run_id, &m.unit_id])))
    .bind(("run", m.run_id.clone()))
    .bind(("unit", m.unit_id.clone()))
    .bind(("stage", i64::from(m.stage)))
    .bind(("json", serde_json::to_string(m)?))
    .await?
    .check()?;
    Ok(())
}

fn decode(rows: Vec<String>) -> Result<Vec<UnitManifest>> {
    rows.iter()
        .map(|j| serde_json::from_str(j).map_err(Into::into))
        .collect()
}

/// `Store::atlas_get_unit_manifests`: one stage of one run, by unit id.
pub async fn unit_manifests(
    db: &Surreal<Any>,
    run_id: &str,
    stage: i32,
) -> Result<Vec<UnitManifest>> {
    let mut r = db
        .query(
            "SELECT VALUE manifest_json FROM atlas_work_unit
             WHERE run_id = $run AND stage = $stage ORDER BY unit_id",
        )
        .bind(("run", run_id.to_string()))
        .bind(("stage", i64::from(stage)))
        .await?
        .check()?;
    decode(r.take(0)?)
}

/// `Store::atlas_get_all_unit_manifests`: stage then unit id order.
pub async fn all_unit_manifests(db: &Surreal<Any>, run_id: &str) -> Result<Vec<UnitManifest>> {
    let mut r = db
        .query(
            "SELECT VALUE manifest_json FROM atlas_work_unit
             WHERE run_id = $run ORDER BY stage, unit_id",
        )
        .bind(("run", run_id.to_string()))
        .await?
        .check()?;
    decode(r.take(0)?)
}
