//! Live-evidence validation inside the publishing transaction (P3 item 2,
//! part 3): the SurrealDB counterpart of
//! `insight_persist::validate_live_evidence`.
//!
//! [`validate_in`] reads the owner and every evidence source through the
//! publication's own transaction, so an edit or delete that commits first
//! makes the publish fail (or conflict and retry) instead of racing it.
//! [`step`] wraps it as the `validate` [`SnapshotStep`] that
//! `insight::publish_snapshot` and `refresh_exec::run_refresh_slice` take.
//!
//! Recon owners check the run (`recon_run`, not cancelled) and its thread
//! (`recon_thread`, not deleted); tool evidence (`call:`) checks the
//! completed `osint_call`'s result hash.

use std::sync::Arc;

use surrealdb::types::SurrealValue;

use super::insight::SnapshotStep;
use super::jobs::{rid, run, take};
use super::tx::{Tx, TxError};
use crate::investigation::insights::EvidenceSnapshot;

fn refuse(message: &str) -> TxError {
    TxError::Abort(message.to_string())
}

fn ensure(ok: bool, message: &str) -> Result<(), TxError> {
    if ok {
        Ok(())
    } else {
        Err(refuse(message))
    }
}

#[derive(SurrealValue)]
struct ArticleMeta {
    title: String,
    description: String,
    url: String,
    published_at: String,
}

#[derive(SurrealValue)]
struct Report {
    source_hash: String,
    completion_sequence: i64,
    job_id: String,
}

/// Validate `snapshot`'s owner and evidence against live rows, in `tx`.
pub async fn validate_in(tx: &Tx, snapshot: &EvidenceSnapshot) -> Result<(), TxError> {
    match snapshot.owner_kind.as_str() {
        "article" | "atlas" => {
            let mut r = run(
                tx,
                tx.query("SELECT VALUE id FROM atlas_article WHERE article_id = $a LIMIT 1")
                    .bind(("a", snapshot.owner_id.clone())),
            )
            .await?;
            let live: Vec<surrealdb::types::RecordId> = take(&mut r, 0)?;
            ensure(!live.is_empty(), "article owner no longer exists")?;
        }
        "recon" => {
            let mut r = run(
                tx,
                tx.query(
                    "LET $run = (SELECT thread_id, state FROM ONLY $r);
                     RETURN $run != NONE AND $run.state != 'cancelled'
                       AND (SELECT VALUE deleted FROM ONLY type::record('recon_thread', $run.thread_id)) = false;",
                )
                .bind(("r", rid("recon_run", &snapshot.owner_id))),
            )
            .await?;
            let last = r.num_statements() - 1;
            ensure(
                take::<Option<bool>>(&mut r, last)? == Some(true),
                "Recon owner no longer active",
            )?;
        }
        _ => return Err(refuse("unsupported publication owner")),
    }
    for reference in &snapshot.source_refs {
        let id = reference.source_id.as_str();
        if let Some(job) = id.strip_prefix("report:") {
            let mut r = run(
                tx,
                tx.query(
                    "RETURN record::exists(type::record('job_retirement', $job));
                     SELECT source_hash, completion_sequence, job_id FROM intel_report_result
                       WHERE article_id = $a ORDER BY completion_sequence DESC LIMIT 1;",
                )
                .bind(("job", job.to_string()))
                .bind(("a", snapshot.owner_id.clone())),
            )
            .await?;
            ensure(
                take::<Option<bool>>(&mut r, 0)? != Some(true),
                "Intel article report execution retired before publication",
            )?;
            let latest: Vec<Report> = take(&mut r, 1)?;
            ensure(
                latest.first().is_some_and(|l| {
                    l.job_id == job
                        && reference.revision
                            == format!("{}:{}", l.completion_sequence, l.source_hash)
                }),
                "report authority changed",
            )?;
        } else if let Some(call) = id.strip_prefix("call:") {
            let call = call.split("#page").next().unwrap_or(call);
            let mut r = run(
                tx,
                tx.query("RETURN IF (SELECT VALUE status FROM ONLY $c) = 'completed' THEN (SELECT VALUE result_json FROM ONLY $c) ELSE NONE END")
                    .bind(("c", rid("osint_call", call))),
            )
            .await?;
            let json: Option<String> = take(&mut r, 0)?;
            ensure(
                json.is_some_and(|s| crate::nlp::cache::hash(&s) == reference.revision),
                "tool evidence revision changed",
            )?;
        } else if let Some(article) = id.strip_prefix("article:") {
            let mut r = run(
                tx,
                tx.query(
                    "SELECT title, description, url, published_at FROM atlas_article
                       WHERE article_id = $a ORDER BY seen_at DESC LIMIT 1",
                )
                .bind(("a", article.to_string())),
            )
            .await?;
            let row: Vec<ArticleMeta> = take(&mut r, 0)?;
            ensure(
                row.first().is_some_and(|m| {
                    crate::nlp::cache::hash(&format!("{}\n{}", m.title, m.description))
                        == reference.revision
                        && m.url == reference.url
                        && reference.published_at.as_ref() == Some(&m.published_at)
                }),
                "article metadata revision changed",
            )?;
        } else if let Some(body) = id.strip_prefix("body:") {
            let mut r = run(
                tx,
                tx.query("SELECT VALUE body_version FROM ONLY $b")
                    .bind(("b", rid("article_body", body))),
            )
            .await?;
            let version: Option<i64> = take(&mut r, 0)?;
            ensure(
                version.is_some_and(|v| v.to_string() == reference.revision),
                "body evidence revision changed",
            )?;
        }
        let mut r = run(
            tx,
            tx.query("SELECT VALUE text FROM ONLY $h")
                .bind(("h", rid("nlp_artifact", &reference.artifact_hash))),
        )
        .await?;
        let text: Option<String> = take(&mut r, 0)?;
        ensure(
            text.is_some_and(|s| crate::nlp::cache::hash(&s) == reference.artifact_hash),
            "retained source artifact unavailable",
        )?;
    }
    for (id, revision) in &snapshot.memory_refs {
        let mut r = run(
            tx,
            tx.query("SELECT VALUE text FROM ONLY $m")
                .bind(("m", rid("memory", id))),
        )
        .await?;
        let text: Option<String> = take(&mut r, 0)?;
        ensure(
            text.is_some_and(|s| crate::store::memory_revision(&s) == *revision),
            "memory premise revision changed",
        )?;
    }
    Ok(())
}

/// [`validate_in`] as a publication step over a captured snapshot.
pub fn step(snapshot: EvidenceSnapshot) -> SnapshotStep {
    let snapshot = Arc::new(snapshot);
    Arc::new(move |tx, _| {
        let snapshot = snapshot.clone();
        Box::pin(async move { validate_in(&tx, &snapshot).await })
    })
}
