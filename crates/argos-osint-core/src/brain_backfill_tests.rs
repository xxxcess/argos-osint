use super::*;
use crate::atlas_metadata::{ArticleMention, ArticleMetadata, MetadataState};
use crate::store::AtlasArticleRow;
use rusqlite::params;

fn article(run: &str, id: &str, title: &str) -> AtlasArticleRow {
    AtlasArticleRow {
        run_id: run.into(),
        id: id.into(),
        title: title.into(),
        description: String::new(),
        url: format!("https://x.test/{id}"),
        source_name: "X".into(),
        source_domain: "x.test".into(),
        published_at: "2026-10-12".into(),
        provider: "fixture".into(),
        category: "unk".into(),
        ..Default::default()
    }
}

fn seed(store: &Store, run: &str, id: &str, title: &str, names: &[&str]) {
    let _ = store.atlas_insert_run(run, "{}", "{}");
    let title = if names.is_empty() {
        title.to_string()
    } else {
        format!("{title} with Acme")
    };
    let a = article(run, id, &title);
    store.atlas_upsert_article(&a).unwrap();
    let mut m = ArticleMetadata::pending(&a);
    m.entity_state = MetadataState::Complete;
    let mut names = names.to_vec();
    if !names.is_empty() {
        names.push("Acme");
    }
    for n in &names {
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
