//! Interactive Text Network Analysis: local co-occurrence graphs.

mod build;
mod corpus;
mod extract;
mod types;

pub use build::{build_snapshot, build_snapshot_blocking};
pub use corpus::{CorpusDoc, TnaCorpus};
pub use extract::{extract_occurrences, ip_is_blocked_label, Occurrence};
pub use types::{
    desk_key, report_key, TnaAnchor, TnaCluster, TnaClusterSummary, TnaEdge, TnaGap, TnaNode,
    TnaNodeKind, TnaScope, TnaSnapshot,
};

use anyhow::Result;

use crate::store::Store;

/// Rebuild the targeted snapshot for one filed report and upsert it.
pub fn rebuild_for_report(store: &Store, report_id: &str) -> Result<TnaSnapshot> {
    let corpus = TnaCorpus::targeted(store, report_id)?;
    let snap = build_snapshot(&corpus);
    store.upsert_tna_graph(&report_key(report_id), &snap)?;
    Ok(snap)
}

/// Rebuild the desk collection snapshot from every completed report.
pub fn rebuild_collection(store: &Store) -> Result<TnaSnapshot> {
    let corpus = TnaCorpus::collection(store)?;
    let snap = build_snapshot(&corpus);
    store.upsert_tna_graph(desk_key(), &snap)?;
    Ok(snap)
}

/// After a report is filed: refresh targeted + collection.
pub fn rebuild_after_file(store: &Store, report_id: &str) -> Result<()> {
    let _ = rebuild_for_report(store, report_id)?;
    let _ = rebuild_collection(store)?;
    Ok(())
}

/// After a report is deleted: drop targeted key and refresh collection.
pub fn rebuild_after_delete(store: &Store, report_id: &str) -> Result<()> {
    let _ = store.delete_tna_graph(&report_key(report_id));
    let _ = rebuild_collection(store)?;
    Ok(())
}
