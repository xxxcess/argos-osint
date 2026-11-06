//! API fixtures obtain authority through real local scoring and publication.
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::store::AtlasArticleRow;

#[path = "../../../argos-osint-core/tests/fixtures/classifier/admission-publication.rs"]
mod publication;

pub async fn admit(repo: &Repository, articles: &[AtlasArticleRow]) {
    let _serial = super::classifier_fixture::serial().await;
    let installed = tokio::task::spawn_blocking(|| {
        super::classifier_fixture::install_with_titles([
            "Harbor signal",
            "Harbor Union merger",
            "River sensor",
        ])
    })
    .await
    .unwrap();
    publication::admit(repo, installed.directory.path(), articles).await;
    installed.home.shutdown().await.unwrap();
}
