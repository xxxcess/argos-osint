//! Atlas memory-phase reads on SurrealDB: the stored publication receipt
//! (`Store::atlas_publication_receipt`) and its phase-completion check
//! (`Store::verify_atlas_publication`), used by `atlas_memory`'s refresh of
//! finished runs whose indexing was incomplete.
//!
//! Coverage differs from SQLite in one way: there is no separate serving
//! index to probe, so a memory counts as indexed when
//! `memory_index_state` records its current revision.

use std::collections::BTreeSet;

use anyhow::Result;
use surrealdb::engine::any::Any;
use surrealdb::Surreal;

use super::insight::atlas_answer_id;
use super::jobs::rid;
use crate::store::{memory_revision, CoverageReport, PublicationReceipt, PublicationVerification};

/// Latest stored receipt for a run.
pub async fn publication_receipt(
    db: &Surreal<Any>,
    run_id: &str,
) -> Result<Option<PublicationReceipt>> {
    let mut r = db
        .query(
            "SELECT VALUE receipt_json FROM atlas_publication WHERE run_id = $run
               ORDER BY updated_at DESC LIMIT 1",
        )
        .bind(("run", run_id.to_string()))
        .await?
        .check()?;
    let rows: Vec<String> = r.take(0usize)?;
    Ok(match rows.into_iter().next() {
        Some(json) => Some(serde_json::from_str(&json)?),
        None => None,
    })
}

async fn memory_text(db: &Surreal<Any>, id: &str) -> Result<Option<String>> {
    let mut r = db
        .query("SELECT VALUE text FROM $m")
        .bind(("m", rid("memory", id)))
        .await?
        .check()?;
    let rows: Vec<String> = r.take(0usize)?;
    Ok(rows.into_iter().next())
}

async fn indexed_revision(db: &Surreal<Any>, id: &str) -> Result<Option<String>> {
    let mut r = db
        .query("SELECT VALUE revision FROM $s")
        .bind(("s", rid("memory_index_state", id)))
        .await?
        .check()?;
    let rows: Vec<String> = r.take(0usize)?;
    Ok(rows.into_iter().next())
}

/// `Store::verify_memory_coverage`.
pub async fn verify_memory_coverage(db: &Surreal<Any>, ids: &[String]) -> Result<CoverageReport> {
    let unique: BTreeSet<&String> = ids.iter().collect();
    let mut report = CoverageReport {
        required: unique.len(),
        ..Default::default()
    };
    for id in unique {
        let Some(text) = memory_text(db, id).await? else {
            report.missing_rows.push(id.clone());
            continue;
        };
        let rev = memory_revision(&text);
        match indexed_revision(db, id).await? {
            Some(recorded) if recorded == rev => report.indexed.push(id.clone()),
            Some(_) => report
                .pending
                .push((id.clone(), "an older revision is indexed".into())),
            None => report.pending.push((id.clone(), "not indexed yet".into())),
        }
    }
    Ok(report)
}

/// `Store::verify_atlas_publication`.
pub async fn verify_publication(
    db: &Surreal<Any>,
    receipt: &PublicationReceipt,
) -> Result<PublicationVerification> {
    let answer = atlas_answer_id(&receipt.run_id);
    let mut out = PublicationVerification {
        brief_ok: true,
        reconciled: receipt.reconciles(),
        ..Default::default()
    };
    for (fingerprint, memory_id) in &receipt.claim_memory_ids {
        let mut r = db
            .query(
                "SELECT VALUE memory_id FROM type::record('insight_claim', $fp);
                 RETURN count(SELECT VALUE id FROM insight_source
                   WHERE fingerprint = $fp AND run_id = $run AND answer_id = $answer);",
            )
            .bind(("fp", fingerprint.clone()))
            .bind(("run", receipt.run_id.clone()))
            .bind(("answer", answer.clone()))
            .await?
            .check()?;
        let claim_memory: Vec<String> = r.take(0usize)?;
        let linked: Option<i64> = r.take(1usize)?;
        let row_ok = memory_text(db, memory_id).await?.is_some();
        if (claim_memory.first() != Some(memory_id) || !row_ok)
            && !out.missing_claim_memories.contains(memory_id)
        {
            out.missing_claim_memories.push(memory_id.clone());
        }
        if linked.unwrap_or(0) == 0 && !out.broken_source_links.contains(fingerprint) {
            out.broken_source_links.push(fingerprint.clone());
        }
    }
    let mut r = db
        .query(
            "SELECT VALUE fingerprint FROM insight_source WHERE run_id = $run AND answer_id = $answer
               AND !record::exists(type::record('insight_claim', fingerprint));",
        )
        .bind(("run", receipt.run_id.clone()))
        .bind(("answer", answer))
        .await?
        .check()?;
    let orphans: Vec<String> = r.take(0usize)?;
    for fp in orphans {
        if !out.broken_source_links.contains(&fp) {
            out.broken_source_links.push(fp);
        }
    }
    if let Some(brief) = &receipt.brief_memory_id {
        out.brief_ok = memory_text(db, brief).await?.is_some();
    }
    out.coverage = verify_memory_coverage(db, &receipt.affected_memory_ids).await?;
    Ok(out)
}

// ---------------------------------------------------------------------------
// Checkpoints, repair status and the repair job's reads/writes
// (`argos_atlas_checkpoints`, `argos_atlas_repairs`).

/// `atlas_memory::save_checkpoint`: insert or replace the payload of one
/// `(run, revision)`, as SQLite's `ON CONFLICT DO UPDATE` does.
pub async fn put_checkpoint(
    db: &Surreal<Any>,
    run_id: &str,
    revision: &str,
    payload_json: &str,
    accepted: i64,
) -> Result<()> {
    db.query(
        "UPSERT type::record('atlas_checkpoint', $key) SET run_id = $run, revision = $rev,
           payload_json = $payload, accepted = $acc, rejected = rejected ?? 0,
           created_at = $now RETURN NONE;",
    )
    .bind(("key", super::codec::encode(&[run_id, revision])))
    .bind(("run", run_id.to_string()))
    .bind(("rev", revision.to_string()))
    .bind(("payload", payload_json.to_string()))
    .bind(("acc", accepted))
    .bind(("now", chrono::Utc::now().to_rfc3339()))
    .await?
    .check()?;
    Ok(())
}

/// Latest checkpoint payload of a run (`load_checkpoint`).
pub async fn latest_checkpoint(db: &Surreal<Any>, run_id: &str) -> Result<Option<String>> {
    let mut r = db
        .query(
            "SELECT VALUE payload_json FROM atlas_checkpoint WHERE run_id = $run
               ORDER BY created_at DESC LIMIT 1",
        )
        .bind(("run", run_id.to_string()))
        .await?
        .check()?;
    let rows: Vec<String> = r.take(0usize)?;
    Ok(rows.into_iter().next())
}

/// Drop a run's checkpoints (`prepare_reextraction`).
pub async fn delete_checkpoints(db: &Surreal<Any>, run_id: &str) -> Result<()> {
    db.query("DELETE atlas_checkpoint WHERE run_id = $run RETURN NONE")
        .bind(("run", run_id.to_string()))
        .await?
        .check()?;
    Ok(())
}

/// Stored repair detail JSON of a run (`repair_status`).
pub async fn repair_detail(db: &Surreal<Any>, run_id: &str) -> Result<Option<String>> {
    let mut r = db
        .query("SELECT VALUE detail_json FROM type::record('atlas_repair', $run)")
        .bind(("run", run_id.to_string()))
        .await?
        .check()?;
    let rows: Vec<String> = r.take(0usize)?;
    Ok(rows.into_iter().next())
}

/// `Store::atlas_has_insights`: the run has Atlas source links.
pub async fn has_insights(db: &Surreal<Any>, run_id: &str) -> Result<bool> {
    let mut r = db
        .query(
            "RETURN count(SELECT VALUE id FROM insight_source
               WHERE run_id = $run AND answer_id = $answer LIMIT 1);",
        )
        .bind(("run", run_id.to_string()))
        .bind(("answer", atlas_answer_id(run_id)))
        .await?
        .check()?;
    let n: Option<i64> = r.take(0usize)?;
    Ok(n.unwrap_or(0) > 0)
}

/// `Store::is_tombstoned`.
pub async fn is_tombstoned(db: &Surreal<Any>, memory_id: &str) -> Result<bool> {
    let mut r = db
        .query("RETURN count(SELECT VALUE id FROM memory_tombstone WHERE memory_id = $m LIMIT 1);")
        .bind(("m", memory_id.to_string()))
        .await?
        .check()?;
    let n: Option<i64> = r.take(0usize)?;
    Ok(n.unwrap_or(0) > 0)
}

/// Article ids with a first-sighting telemetry row (`intel_article`).
pub async fn seen_article_ids(db: &Surreal<Any>) -> Result<std::collections::HashSet<String>> {
    let mut r = db
        .query("SELECT VALUE article_id FROM telemetry_event WHERE event_type = 'intel_article'")
        .await?
        .check()?;
    let rows: Vec<String> = r.take(0usize)?;
    Ok(rows.into_iter().filter(|id| !id.is_empty()).collect())
}

/// `Store::requeue_uncovered`: queue index work for every coverage gap
/// (missing or stale revision). Returns the number of queued changes.
/// Nothing a user sees changes, so `memories_changed` is not bumped.
pub async fn requeue_uncovered(
    publisher: &super::publisher::Publisher,
    coverage: &CoverageReport,
) -> Result<usize> {
    let ids: Vec<String> = coverage.pending.iter().map(|(id, _)| id.clone()).collect();
    if ids.is_empty() || coverage.disabled {
        return Ok(0);
    }
    let count = std::sync::Arc::new(std::sync::Mutex::new(0usize));
    let out = count.clone();
    let ids = std::sync::Arc::new(ids);
    let nonce = format!("requeue-{}", chrono::Utc::now().timestamp_micros());
    let w: super::publisher::Write = std::sync::Arc::new(move |tx| {
        let (ids, out, nonce) = (ids.clone(), out.clone(), nonce.clone());
        Box::pin(async move {
            let changes = super::memory::index_changes_in(&tx, &ids, &nonce).await?;
            *out.lock().expect("requeue count") = changes.len();
            Ok(super::publisher::Wrote {
                changes,
                changed: false,
            })
        })
    });
    publisher
        .publish(w)
        .await
        .map_err(|e| anyhow::anyhow!("atlas requeue: {e}"))?;
    let n = *count.lock().expect("requeue count");
    Ok(n)
}

/// Bump `memories_changed` after a repair restored memories.
pub async fn bump_memories_changed(publisher: &super::publisher::Publisher) -> Result<()> {
    let w: super::publisher::Write = std::sync::Arc::new(|_tx| {
        Box::pin(async move {
            Ok(super::publisher::Wrote {
                changes: Vec::new(),
                changed: true,
            })
        })
    });
    publisher
        .publish(w)
        .await
        .map_err(|e| anyhow::anyhow!("atlas memories_changed: {e}"))?;
    Ok(())
}
