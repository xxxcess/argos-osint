//! Batched Atlas classification. Partial valid answers survive retries; the
//! shared model executor owns the entire captured route/attempt budget.
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{atomic::AtomicBool, Arc, Mutex};

use anyhow::Result;
use serde_json::{json, Value};

use crate::atlas::{classification_request, FeedArticle, CATEGORY_IDS};
use crate::provider::{ChatMessage, DecisionsResponse, SettingsFile};
use crate::recon::model_exec::{self, DecisionsAdapter, OperationScope};
use crate::secrets::{AuthFile, ProviderSecret};
use crate::store::{AtlasArticleRow, Store};

pub const PROMPT_VERSION: &str = "atlas-classifier-batch-v1";
pub const BATCH_SIZE: usize = 8;
pub const REQUEST_BYTES: usize = 32 * 1024;

pub struct BatchAdapter {
    articles: Vec<AtlasArticleRow>,
    valid: Mutex<BTreeMap<String, String>>,
    cache: Option<(std::path::PathBuf, String)>,
}

impl BatchAdapter {
    pub fn new(articles: Vec<AtlasArticleRow>) -> Self {
        Self {
            articles,
            valid: Mutex::new(BTreeMap::new()),
            cache: None,
        }
    }

    pub fn receipt(&self) -> BTreeMap<String, String> {
        self.valid.lock().expect("classification receipt").clone()
    }

    fn pending(&self) -> Vec<&AtlasArticleRow> {
        let valid = self.receipt();
        self.articles
            .iter()
            .filter(|a| !valid.contains_key(&a.id))
            .collect()
    }

    fn accept(&self, rows: Vec<(String, String)>) -> Result<BTreeMap<String, String>, String> {
        let pending: BTreeSet<_> = self.pending().iter().map(|a| a.id.as_str()).collect();
        let mut counts = BTreeMap::new();
        for (id, _) in &rows {
            *counts.entry(id.as_str()).or_insert(0usize) += 1;
        }
        let mut valid = self.valid.lock().map_err(|_| "receipt lock poisoned")?;
        let mut invalid = false;
        for (id, category) in &rows {
            if !pending.contains(id.as_str())
                || counts[id.as_str()] != 1
                || !CATEGORY_IDS.contains(&category.as_str())
            {
                invalid = true;
                continue;
            }
            valid.insert(id.clone(), category.clone());
        }
        if let Some((db, key)) = &self.cache {
            let json = serde_json::to_string(&*valid).map_err(|e| e.to_string())?;
            match crate::recon::backend::sqlite_path(db) {
                Some(path) => {
                    let store = Store::open(path).map_err(|e| e.to_string())?;
                    store.app_state_set(key, &json).map_err(|e| e.to_string())?;
                }
                // `accept` is synchronous: under SurrealDB the batch write
                // is queued in order, ahead of the end-of-call write.
                None => cache_write_ordered(db.clone(), key.clone(), json),
            }
        }
        if invalid || valid.len() != self.articles.len() {
            Err(format!(
                "unresolved or invalid classification IDs ({} of {})",
                valid.len(),
                self.articles.len()
            ))
        } else {
            Ok(valid.clone())
        }
    }
}

impl DecisionsAdapter<BTreeMap<String, String>> for BatchAdapter {
    fn compile_native(&self) -> (Value, Value) {
        let mut states = Vec::new();
        let mut questions = serde_json::Map::new();
        for row in self.pending() {
            let article = FeedArticle {
                id: row.id.clone(),
                title: row.title.clone(),
                description: row.description.clone(),
                source_name: row.source_name.clone(),
                country: row.country.clone(),
                url: row.url.clone(),
                source_domain: row.source_domain.clone(),
                author: row.author.clone(),
                image_url: row.image_url.clone(),
                published_at: row.published_at.clone(),
                provider: row.provider.clone(),
                temperature: row.temperature,
                seen_at: row.seen_at.clone(),
                category: row.category.clone(),
            };
            let (mut state, template) = classification_request(&article);
            state["article_id"] = json!(row.id);
            states.push(state);
            let mut question = template["category"].clone();
            question["instructions"] = json!(format!(
                "Classify only article_id {}. Choose one category or unk when no category fits.",
                row.id
            ));
            questions.insert(row.id.clone(), question);
        }
        (json!({"articles":states}), Value::Object(questions))
    }

    fn parse_native(
        &self,
        response: &DecisionsResponse,
    ) -> Result<BTreeMap<String, String>, String> {
        self.accept(
            response
                .answers
                .iter()
                .map(|(id, a)| (id.clone(), a.choice.clone().unwrap_or_default()))
                .collect(),
        )
    }

    fn compile_chat(&self) -> Vec<ChatMessage> {
        let rows: Vec<_> = self.pending().iter().map(|a| json!({"article_id":a.id,"title":a.title,"description":a.description,"publisher":a.source_name})).collect();
        vec![ChatMessage { role:"system".into(), content:"Classify each article from its separate title and description. Reply only with a JSON array [{\"article_id\":\"exact input ID\",\"category\":\"geopolitical|economic|military|information|stability|technology|unk\"}]. unk is a valid no-match result. Return every input ID exactly once.".into(), tool_call_id:None, tool_calls:Vec::new() }, ChatMessage { role:"user".into(), content:json!(rows).to_string(), tool_call_id:None, tool_calls:Vec::new() }]
    }

    fn parse_chat(&self, raw: &str) -> Result<BTreeMap<String, String>, String> {
        let value: Value = serde_json::from_str(raw.trim()).map_err(|e| e.to_string())?;
        let rows = value.as_array().ok_or("expected classification array")?;
        self.accept(
            rows.iter()
                .map(|r| {
                    (
                        r["article_id"].as_str().unwrap_or_default().to_string(),
                        r["category"].as_str().unwrap_or_default().to_string(),
                    )
                })
                .collect(),
        )
    }
}

fn request_bytes(adapter: &BatchAdapter, model: &str) -> usize {
    let (state, questions) = adapter.compile_native();
    let native = serde_json::to_vec(&json!({"model":model,"state":state,"questions":questions}))
        .expect("native request")
        .len();
    let messages: Vec<_> = adapter
        .compile_chat()
        .iter()
        .map(|m| json!({"role":m.role,"content":m.content}))
        .collect();
    let chat = serde_json::to_vec(&json!({"model":model,"messages":messages,"stream":false}))
        .expect("chat request")
        .len();
    // Reserve bounded adapter envelope fields (token/transport options).
    native.max(chat).saturating_add(512)
}

/// Cut batches against serialized native and chat envelopes.
pub fn batches(articles: &[AtlasArticleRow]) -> Vec<Vec<AtlasArticleRow>> {
    batches_for_model(articles, "")
}
pub fn batches_for_model(articles: &[AtlasArticleRow], model: &str) -> Vec<Vec<AtlasArticleRow>> {
    let mut batches = Vec::new();
    let mut current = Vec::new();
    for article in articles {
        let mut proposed = current.clone();
        proposed.push(article.clone());
        if !current.is_empty()
            && (current.len() == BATCH_SIZE
                || request_bytes(&BatchAdapter::new(proposed), model) > REQUEST_BYTES)
        {
            batches.push(std::mem::take(&mut current));
        }
        current.push(article.clone());
    }
    if !current.is_empty() {
        batches.push(current);
    }
    batches
}

pub async fn execute(
    db: &Path,
    run: &str,
    secret: &ProviderSecret,
    articles: Vec<AtlasArticleRow>,
    cancel: &Arc<AtomicBool>,
) -> (BTreeMap<String, String>, Option<String>) {
    let mut auth = AuthFile::load_from(&db.with_file_name("auth.json")).unwrap_or_default();
    auth.set_account(secret.clone());
    let mut settings =
        SettingsFile::load_from(&db.with_file_name("config.toml")).unwrap_or_default();
    // Capture the configured Classifier fallbacks, never a different role.
    settings.defaults.classifier.provider = secret.kind.clone();
    settings.defaults.classifier.model = secret.model.clone();
    let mut adapter = BatchAdapter::new(articles);
    let (routes, _) = model_exec::resolve_role_routes(&auth, &settings, "classifier");
    let mut inputs: Vec<_> = adapter
        .articles
        .iter()
        .map(|a| (&a.id, crate::atlas_metadata::input_hash(a)))
        .collect();
    inputs.sort();
    let revision =
        crate::nlp::cache::hash(&serde_json::to_string(&inputs).expect("classification inputs"));
    let cache_key = format!(
        "{PROMPT_VERSION}:{run}:{revision}:{}",
        crate::nlp::cache::hash(&serde_json::to_string(&routes).unwrap_or_default())
    );
    let sqlite_db = crate::recon::backend::sqlite_path(db);
    // Per-batch cache writes run on either store (see `accept`).
    adapter.cache = Some((db.to_path_buf(), cache_key.clone()));
    let store = crate::atlas_store::AtlasStore::open(db).await.ok();
    if let Some(store) = &store {
        if let Ok(Some(raw)) = store.app_state_get(&cache_key).await {
            if let Ok(rows) = serde_json::from_str::<BTreeMap<String, String>>(&raw) {
                let _ = adapter.accept(rows.into_iter().collect());
                if adapter.pending().is_empty() {
                    return (adapter.receipt(), None);
                }
            }
        }
    }
    let longest_model = routes
        .iter()
        .map(|route| route.model.as_str())
        .max_by_key(|model| model.len())
        .unwrap_or(&secret.model);
    if request_bytes(&adapter, longest_model) > REQUEST_BYTES {
        return (
            adapter.receipt(),
            Some("article exceeds serialized classifier request ceiling".into()),
        );
    }
    let operation = format!("atlas-classify-{run}-{revision}");
    let scope = OperationScope::new(&operation, "classifier").with_run(run);
    let result = model_exec::execute_decisions_or_chat(
        &auth,
        &settings,
        &scope,
        &adapter,
        cancel,
        None,
        sqlite_db.as_deref(),
    )
    .await;
    let receipt = adapter.receipt();
    let json = serde_json::to_string(&receipt).unwrap_or_default();
    if sqlite_db.is_none() {
        // Behind any queued per-batch writes, so the final receipt wins.
        cache_write_ordered(db.to_path_buf(), cache_key, json);
    } else if let Some(store) = &store {
        let _ = store.app_state_set(&cache_key, &json).await;
    }
    (receipt, result.err().map(|e| e.to_string()))
}

/// Queue a classifier cache write on the runtime's ordered queue (SurrealDB:
/// `accept` cannot await). Best-effort, like the SQLite write it mirrors.
fn cache_write_ordered(db: std::path::PathBuf, key: String, json: String) {
    let label = key.clone();
    let queued = crate::storage::runtime::spawn_ordered(async move {
        if let Ok(store) = crate::atlas_store::AtlasStore::open(&db).await {
            if let Err(e) = store.app_state_set(&key, &json).await {
                eprintln!("classifier cache not written: {e:#} (key {key})");
            }
        }
    });
    if !queued {
        eprintln!("classifier cache not written: no runtime (key {label})");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn article(id: &str) -> AtlasArticleRow {
        serde_json::from_value(json!({"id":id,"run_id":"r","title":"Acme","description":"","url":"https://example.org","country":"us","source_name":"Example","source_domain":"example.org","author":"","image_url":"","published_at":"","provider":"newsapi","temperature":1.0,"seen_at":"","category":"unk"})).unwrap()
    }
    #[test]
    fn partial_valid_rows_survive_and_only_missing_ids_are_recompiled() {
        let adapter = BatchAdapter::new(vec![article("a"), article("b")]);
        assert!(adapter
            .parse_chat(
                r#"[{"article_id":"a","category":"unk"},{"article_id":"b","category":"invalid"}]"#
            )
            .is_err());
        assert_eq!(adapter.receipt()["a"], "unk");
        assert!(!adapter.compile_chat()[1]
            .content
            .contains("\"article_id\":\"a\""));
        assert!(adapter
            .parse_chat(r#"[{"article_id":"b","category":"economic"}]"#)
            .is_ok());
    }
    #[test]
    fn duplicate_and_unknown_ids_never_become_successful_unk() {
        let adapter = BatchAdapter::new(vec![article("a")]);
        assert!(adapter
            .parse_chat(
                r#"[{"article_id":"a","category":"unk"},{"article_id":"a","category":"economic"}]"#
            )
            .is_err());
        assert!(adapter.receipt().is_empty());
        assert!(adapter
            .parse_chat(r#"[{"article_id":"wrong","category":"unk"}]"#)
            .is_err());
    }
    #[test]
    fn native_duplicate_question_ids_are_rejected_before_map_conversion() {
        assert!(crate::provider::parse_decisions(r#"{"answers":{"a":{"type":"choice","choice":"unk"},"a":{"type":"choice","choice":"economic"}}}"#).is_err());
    }
    #[test]
    fn batch_count_is_bounded_and_empty_never_dispatches() {
        assert!(batches(&[]).is_empty());
        let articles: Vec<_> = (0..17).map(|i| article(&i.to_string())).collect();
        assert_eq!(
            batches(&articles).iter().map(Vec::len).collect::<Vec<_>>(),
            vec![8, 8, 1]
        );
    }
}
