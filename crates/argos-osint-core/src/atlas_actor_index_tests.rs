use super::*;
use crate::atlas_metadata::{ArticleMention, ArticleMetadata, MetadataState};
use crate::store::AtlasArticleRow;

pub(crate) fn seed(store: &Store, run: &str, id: &str, title: &str, names: &[&str]) {
    let _ = store.atlas_insert_run(run, "{}", "{}");
    let a = AtlasArticleRow {
        run_id: run.into(),
        id: id.into(),
        title: title.into(),
        url: format!("https://x.test/{id}"),
        source_name: "X".into(),
        source_domain: "x.test".into(),
        published_at: "2026-10-12".into(),
        provider: "fixture".into(),
        category: "unk".into(),
        ..Default::default()
    };
    store.atlas_upsert_article(&a).unwrap();
    let mut m = ArticleMetadata::pending(&a);
    m.entity_state = MetadataState::Complete;
    for n in names {
        let start = title.find(n).unwrap();
        m.mentions.push(ArticleMention {
            type_hint: "person".into(),
            comparison_key: n.to_lowercase(),
            field: "title".into(),
            start_byte: start,
            end_byte: start + n.len(),
            original_text: (*n).into(),
        });
    }
    assert!(store.atlas_save_metadata(&m).unwrap());
}

#[test]
fn collection_contracts_are_recognised_centrally() {
    use crate::atlas_work::*;
    assert!(is_collection_contract(COLLECTION_CONTRACT_V1));
    assert!(is_collection_contract(COLLECTION_CONTRACT));
    assert_eq!(COLLECTION_CONTRACT, "atlas-collection-v2");
    assert!(!is_collection_contract(""));
    assert!(!is_collection_contract("atlas-memory-v3"));
    assert_eq!(stage_label(STAGE_ACTOR_ARTICLES), "actor_articles");
    assert_eq!(stage_unit(STAGE_ACTOR_ARTICLES), "article");
    assert_eq!(crate::atlas::phase_title(6), "Index actors and articles");
    assert_eq!(crate::atlas::phase_title(5), "Related article leads");
}
