//! Global operational retention. Results and canonical consistency outboxes
//! are durable data; job journals are disposable, fenced execution history.

pub const RETENTION_HOURS: i64 = 72;
pub const BATCH: usize = 256;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CleanupCounts {
    pub cleared: usize,
    pub stopped: usize,
}

pub const OWNER_MISSING: &str = "EXISTS(SELECT 1 FROM argos_job_owners o WHERE o.job_id=j.id AND o.required=1 AND (
    (o.owner_kind='article' AND NOT EXISTS(SELECT 1 FROM atlas_articles a WHERE a.article_id=o.owner_id)) OR
    (o.owner_kind='atlas_run' AND NOT EXISTS(SELECT 1 FROM atlas_runs a WHERE a.id=o.owner_id)) OR
    (o.owner_kind='thread' AND NOT EXISTS(SELECT 1 FROM recon_threads t WHERE t.id=o.owner_id AND t.deleted=0)) OR
    (o.owner_kind='recon_run' AND NOT EXISTS(SELECT 1 FROM recon_runs r JOIN recon_threads t ON t.id=r.thread_id WHERE r.id=o.owner_id AND t.deleted=0)) OR
    (o.owner_kind='memory' AND NOT EXISTS(SELECT 1 FROM memories m WHERE m.id=o.owner_id))))";

/// View predicate used consistently by lists, counts, details and controls.
pub fn eligible(alias: &str) -> String {
    format!("julianday({alias}.created_at)>=julianday('now')-3 AND NOT EXISTS(SELECT 1 FROM argos_job_retirements x WHERE x.job_id={alias}.id) AND NOT ({})",OWNER_MISSING.replace("j.id",&format!("{alias}.id")))
}

#[cfg(test)]
mod tests {}
