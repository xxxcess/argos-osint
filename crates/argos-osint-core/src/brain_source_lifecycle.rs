//! Current source support is separate from retained evidence and identity history.
use anyhow::Result;
use rusqlite::Connection;

/// Join the caller's deletion/maintenance transaction. Historical snapshots and
/// typed rows survive, but unsupported evidence is no longer searchable.
pub(crate) fn reconcile_sources(conn: &Connection, now: &str) -> Result<()> {
    conn.execute_batch("UPDATE OR IGNORE insight_sources SET
        run_id=(SELECT a.run_id FROM atlas_articles a WHERE a.article_id=CASE WHEN call_id LIKE 'article:%' THEN substr(call_id,9) ELSE call_id END ORDER BY a.seen_at DESC,a.run_id LIMIT 1),
        answer_id=CASE WHEN answer_id LIKE 'atlas-%' THEN 'atlas-'||(SELECT a.run_id FROM atlas_articles a WHERE a.article_id=call_id ORDER BY a.seen_at DESC,a.run_id LIMIT 1) ELSE answer_id END
        WHERE thread_id IS NULL AND (call_id LIKE 'article:%' OR answer_id LIKE 'atlas-%' OR answer_id IN (SELECT id FROM insight_snapshots WHERE owner_kind IN ('article','atlas')))
          AND NOT EXISTS(SELECT 1 FROM atlas_articles a WHERE a.run_id=insight_sources.run_id AND a.article_id=CASE WHEN call_id LIKE 'article:%' THEN substr(call_id,9) ELSE call_id END)
          AND EXISTS(SELECT 1 FROM atlas_articles a WHERE a.article_id=CASE WHEN call_id LIKE 'article:%' THEN substr(call_id,9) ELSE call_id END);
        DELETE FROM insight_sources WHERE thread_id IS NULL AND answer_id LIKE 'atlas-%'
          AND NOT EXISTS(SELECT 1 FROM atlas_articles a WHERE a.run_id=insight_sources.run_id AND a.article_id=call_id);")?;
    conn.execute_batch(
        "DELETE FROM insight_sources WHERE thread_id IS NULL AND (
        answer_id IN (SELECT s.id FROM insight_snapshots s WHERE s.owner_kind IN ('article','atlas')
          AND NOT EXISTS(SELECT 1 FROM atlas_articles a WHERE a.article_id=s.owner_id))
        OR ((call_id LIKE 'article:%' OR answer_id LIKE 'atlas-%')
          AND NOT EXISTS(SELECT 1 FROM atlas_articles a WHERE a.article_id=CASE
            WHEN call_id LIKE 'article:%' THEN substr(call_id,9) ELSE call_id END)));",
    )?;
    // Legacy insight writers can still produce rows without typed metadata.
    conn.execute("INSERT OR IGNORE INTO memory_metadata(memory_id,memory_kind,created_at,updated_at)
        SELECT c.memory_id,CASE WHEN lower(trim(c.classification))='inference' THEN 'inference' ELSE 'claim' END,m.created_at,?1
        FROM insight_claims c JOIN memories m ON m.id=c.memory_id
        WHERE json_extract(m.source_json,'$.app') IN ('article','atlas','recon','recon-deleted')
          AND NOT EXISTS(SELECT 1 FROM insight_sources s WHERE s.fingerprint=c.fingerprint AND s.deleted_origin=0)", [now])?;
    // Existing non-article sources (including Recon) keep a shared claim alive.
    conn.execute("UPDATE memory_metadata SET lifecycle='retired',revision=revision+1,updated_at=?1
        WHERE lifecycle='active' AND memory_kind IN ('claim','atomic_claim','inference')
          AND memory_id IN (SELECT c.memory_id FROM insight_claims c JOIN memories m ON m.id=c.memory_id
            WHERE json_extract(m.source_json,'$.app') IN ('article','atlas','recon','recon-deleted')
              AND NOT EXISTS(SELECT 1 FROM insight_sources s WHERE s.fingerprint=c.fingerprint AND s.deleted_origin=0))", [now])?;
    Ok(())
}

/// Durably fence retired vectors, including edges retired by SQLite triggers.
/// The latest operation prevents repeated maintenance from growing the outbox.
pub(crate) fn reconcile_index(conn: &Connection) -> Result<()> {
    let ids = conn.prepare("SELECT mm.memory_id FROM memory_metadata mm WHERE
        (mm.lifecycle<>'active' AND COALESCE((SELECT operation FROM argos_index_changes i WHERE i.record_id=mm.memory_id AND i.record_kind='memory' ORDER BY seq DESC LIMIT 1),'')<>'index_remove')
        OR (mm.lifecycle='active' AND (SELECT operation FROM argos_index_changes i WHERE i.record_id=mm.memory_id AND i.record_kind='memory' ORDER BY seq DESC LIMIT 1)='index_remove')")?
        .query_map([], |r| r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    crate::store::enqueue_memory_index_on(conn, &ids, None)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{brain::MemorySource, store::Store};
    use rusqlite::params;
}
