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
