//! API fixtures obtain authority through real local scoring and publication.
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::store::AtlasArticleRow;

#[path = "../../../argos-osint-core/tests/fixtures/classifier/admission-publication.rs"]
mod publication;

pub async fn admit(repo: &Repository, articles: &[AtlasArticleRow]) {
    admit_with_titles(
        repo,
        articles,
        ["Harbor signal", "Harbor Union merger", "River sensor"],
    )
    .await;
}

/// The offline worker matches each configured cue as a contiguous substring
/// of the article title or description.
pub async fn admit_with_titles(
    repo: &Repository,
    articles: &[AtlasArticleRow],
    titles: [&'static str; 3],
) {
    let _serial = super::classifier_fixture::serial().await;
    let installed =
        tokio::task::spawn_blocking(move || super::classifier_fixture::install_with_titles(titles))
            .await
            .unwrap();
    publication::admit(repo, installed.directory.path(), articles).await;
    installed.home.shutdown().await.unwrap();
}
