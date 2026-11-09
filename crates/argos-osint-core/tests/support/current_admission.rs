//! Current authority for discovery/index fixtures, through the live admission path.
use std::sync::OnceLock;

use argos_osint_core::storage::atlas;
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::store::AtlasArticleRow;

#[path = "../fixtures/classifier/admission-support.rs"]
mod fixture;
#[path = "../fixtures/classifier/admission-publication.rs"]
mod publication;

/// Fictional vocabulary selects a deterministic score pair through the worker protocol.
/// Reusing one persisted model also exercises restore and same-run resumption.
pub async fn admit(repo: &Repository, row: &mut AtlasArticleRow, vocabulary: &str) {
    static MODEL: OnceLock<tempfile::TempDir> = OnceLock::new();
    let _serial = fixture::serial().await;
    if MODEL.get().is_none() {
        let installed = tokio::task::spawn_blocking(fixture::install).await.unwrap();
        installed.home.shutdown().await.unwrap();
        MODEL.set(installed.directory).expect("one fixture model");
    }
    let cue = vocabulary.to_ascii_lowercase();
    assert!(!cue.is_empty(), "fixture admission needs its worker cue");
    // Cloned occurrences must retain exactly the same metadata. Reappending a
    // fixture cue would change their source revision and body acquisition key.
    if row.description != cue && !row.description.ends_with(&format!(" {cue}")) {
        row.description = format!("{} {cue}", row.description).trim().to_string();
    }
    publication::admit(repo, MODEL.get().unwrap().path(), std::slice::from_ref(row)).await;
    *row = atlas::article(repo.db(), &row.run_id, &row.id)
        .await
        .unwrap()
        .unwrap();
}
