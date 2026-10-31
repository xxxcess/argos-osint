//! Atlas phase 6, "Index actors and articles" (contract `atlas-collection-v2`).
//!
//! Every retained article occurrence is bound through Brain (actors, mention
//! receipts, `mentioned_in` Links, ≤ [`crate::brain_actors::RELATED_PER_ARTICLE`]
//! Related records) and receives an `atlas_article_index` receipt in the same
//! transaction. Transactions hold at most [`BATCH`] occurrences; a failed unit
//! rolls back alone and leaves no receipt, so the run cannot report phase 6
//! complete and a resume redoes only the missing occurrences.
use anyhow::Result;

use crate::brain_actors::IDENTITY_INDEX_VERSION;

pub const BATCH: usize = 100;
pub const INDEX_CONTRACT: &str = "atlas-actor-index-v1";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexReport {
    pub total: u32,
    /// Receipts already current before this pass (resume / rerun).
    pub reused: u32,
    pub indexed: u32,
    /// Indexed occurrences with no bindable actor (still receipted).
    pub zero_actor: u32,
    pub failed: u32,
    pub errors: Vec<String>,
    /// Stopped early by pause/cancel; nothing after the last batch ran.
    pub stopped: bool,
}

impl IndexReport {
    pub fn complete(&self) -> bool {
        !self.stopped && self.failed == 0 && self.reused + self.indexed == self.total
    }
}

fn checksum(revision: &str, coverage: &str) -> String {
    crate::nlp::cache::hash(&format!(
        "{INDEX_CONTRACT}|{IDENTITY_INDEX_VERSION}|{revision}|{coverage}"
    ))
}

/// Phase 6 on the selected store: the legacy store runs [`index_run`]; SurrealDB runs
/// [`surreal::index_run`] on the repository.
pub async fn index_run_store(
    store: &crate::atlas_store::AtlasStore,
    run_id: &str,
    stop: &(dyn Fn() -> bool + Sync),
) -> Result<IndexReport> {
    let Some(repo) = store.repository() else {
        anyhow::bail!("SurrealDB storage is not configured");
    };
    surreal::index_run(repo, run_id, stop).await
}

/// Phase 6 on SurrealDB. Each occurrence is its own transaction (no
/// savepoints): a failed occurrence rolls back alone and leaves no receipt.
/// Lifecycle reconciliation runs once per batch of [`BATCH`].
#[cfg(feature = "surreal")]
pub mod surreal {
    use std::collections::{BTreeSet, HashMap};
    use std::sync::{Arc, Mutex};

    use anyhow::{anyhow, Result};
    use serde_json::{json, Value};

    use super::{checksum, IndexReport, BATCH};
    use crate::atlas_metadata::ArticleMetadata;
    use crate::brain_actors::{
        endpoint_key, hid, parse_subtype, subtype_str, text_of, BindRequest,
    };
    use crate::memory_metadata::{
        EndpointRef, LinkPayload, MemoryKind, MemoryPayload, RelatedPayload,
    };
    use crate::storage::lifecycle::{self, ArticleIndexRow};
    use crate::storage::memory::{put_typed_in, write_then_index, MemoryWrite, TypedMemory};
    use crate::storage::repo::Repository;
    use crate::storage::tx::{Tx, TxError};
    use crate::store::AtlasArticleRow;

    fn tx_err(op: &'static str) -> impl Fn(TxError) -> anyhow::Error {
        move |e| anyhow!("{op}: {e}")
    }

    /// `brain_backfill::atlas_run_units` on the repository: the same
    /// `[title, description, category, metadata_json]` revision.
    pub async fn run_units(
        repo: &Repository,
        run_id: &str,
    ) -> Result<Vec<(AtlasArticleRow, Option<ArticleMetadata>, String)>> {
        if run_id.is_empty() {
            return Ok(Vec::new());
        }
        let mut articles = crate::storage::atlas::list_articles(repo.db(), run_id)
            .await
            .map_err(tx_err("atlas run units"))?;
        articles.sort_by(|a, b| a.id.cmp(&b.id));
        let mut out = Vec::with_capacity(articles.len());
        for a in articles {
            let m = crate::storage::atlas_metadata::metadata(repo.db(), run_id, &a.id)
                .await
                .map_err(tx_err("atlas run unit metadata"))?;
            let raw = m.as_ref().map(serde_json::to_string).transpose()?;
            let rev = crate::brain_backfill::h(&json!([a.title, a.description, a.category, raw]));
            out.push((a, m, rev));
        }
        Ok(out)
    }

    /// Current, checksum-valid receipts for a run, keyed by article id.
    pub async fn receipts(repo: &Repository, run_id: &str) -> Result<HashMap<String, String>> {
        let rows = lifecycle::article_index_receipts(repo.db(), run_id)
            .await
            .map_err(tx_err("atlas article index receipts"))?;
        Ok(rows
            .into_iter()
            .filter(|r| r.index_checksum == checksum(&r.input_revision, &r.actor_coverage))
            .map(|r| (r.article_id, r.input_revision))
            .collect())
    }

    pub async fn index_run(
        repo: &Repository,
        run_id: &str,
        stop: &(dyn Fn() -> bool + Sync),
    ) -> Result<IndexReport> {
        let units = run_units(repo, run_id).await?;
        let have = receipts(repo, run_id).await?;
        let mut report = IndexReport {
            total: units.len() as u32,
            ..IndexReport::default()
        };
        let pending: Vec<_> = units
            .into_iter()
            .filter(|(a, _, rev)| {
                let current = have.get(&a.id) == Some(rev);
                if current {
                    report.reused += 1;
                }
                !current
            })
            .collect();
        for chunk in pending.chunks(BATCH) {
            if stop() {
                report.stopped = true;
                break;
            }
            let now = chrono::Utc::now().to_rfc3339();
            for (article, metadata, rev) in chunk {
                let unit = format!("{run_id}/{}", article.id);
                match index_occurrence(repo, article, metadata.as_ref(), rev, &now).await {
                    Ok(coverage) => {
                        report.indexed += 1;
                        if coverage == "zero actors" || coverage == "metadata missing" {
                            report.zero_actor += 1;
                        }
                    }
                    Err(err) => {
                        report.failed += 1;
                        report.errors.push(format!("{unit}: {err}"));
                    }
                }
            }
            let write: MemoryWrite = Arc::new(|tx: Tx| {
                Box::pin(async move { lifecycle::reconcile_lifecycle_in(&tx).await })
            });
            write_then_index(repo.publisher(), write)
                .await
                .map_err(tx_err("actor lifecycle reconcile"))?;
        }
        Ok(report)
    }

    /// `brain_backfill::index_article` for one occurrence, in one transaction.
    pub async fn index_occurrence(
        repo: &Repository,
        article: &AtlasArticleRow,
        metadata: Option<&ArticleMetadata>,
        rev: &str,
        now: &str,
    ) -> Result<String> {
        if article.run_id.is_empty() || article.id.is_empty() {
            return Err(anyhow!("actor index: empty article id"));
        }
        let article = Arc::new(article.clone());
        let metadata = Arc::new(metadata.cloned());
        let rev = rev.to_string();
        let now = now.to_string();
        let db = repo.db().clone();
        let coverage = Arc::new(Mutex::new(String::new()));
        let out = coverage.clone();
        let write: MemoryWrite = Arc::new(move |tx: Tx| {
            let article = article.clone();
            let metadata = metadata.clone();
            let rev = rev.clone();
            let now = now.clone();
            let db = db.clone();
            let out = out.clone();
            Box::pin(async move {
                let (cov, touched) =
                    write_occurrence(&tx, &db, &article, metadata.as_ref().as_ref(), &rev, &now)
                        .await?;
                let (run_id, article_id) = (article.run_id.as_str(), article.id.as_str());
                lifecycle::put_article_index_in(
                    &tx,
                    run_id,
                    article_id,
                    &ArticleIndexRow {
                        article_id: article_id.to_string(),
                        input_revision: rev.clone(),
                        index_checksum: checksum(&rev, &cov),
                        actor_coverage: cov.clone(),
                    },
                    &now,
                )
                .await?;
                // The same revision satisfies the historical backfill.
                crate::storage::entities::put_backfill_receipt_in(
                    &tx,
                    "atlas_articles",
                    &format!("{run_id}/{article_id}"),
                    &rev,
                    &cov,
                    &now,
                )
                .await?;
                *out.lock().unwrap_or_else(|e| e.into_inner()) = cov;
                Ok(touched)
            })
        });
        write_then_index(repo.publisher(), write)
            .await
            .map_err(tx_err("actor index occurrence"))?;
        let cov = coverage.lock().unwrap_or_else(|e| e.into_inner()).clone();
        Ok(cov)
    }

    async fn write_occurrence(
        tx: &Tx,
        db: &surrealdb::Surreal<surrealdb::engine::any::Any>,
        article: &AtlasArticleRow,
        metadata: Option<&ArticleMetadata>,
        revision: &str,
        now: &str,
    ) -> Result<(String, Vec<String>), TxError> {
        let (run_id, article_id) = (article.run_id.as_str(), article.id.as_str());
        let mut touched: BTreeSet<String> =
            lifecycle::retire_article_occurrence_in(tx, run_id, article_id)
                .await?
                .into_iter()
                .collect();
        let Some(metadata) = metadata else {
            return Ok(("metadata missing".into(), touched.into_iter().collect()));
        };
        let found = crate::atlas_metadata::article_actors(article, metadata);
        if found.is_empty() {
            return Ok(("zero actors".into(), touched.into_iter().collect()));
        }
        let scope = format!("atlas-article:{article_id}");
        let article_ep = EndpointRef::Article {
            run_id: run_id.into(),
            article_id: article_id.into(),
            revision: revision.into(),
        };
        let mut bound: Vec<(String, String, String)> = Vec::new();
        for a in &found {
            let subtype = parse_subtype(&a.type_hint);
            let key = crate::atlas_metadata::comparison_key(&a.display_name);
            let legacy = format!("{}:{key}", a.type_hint);
            let ext_scope = format!("atlas:{article_id}");
            let (binding, changed) = crate::storage::actor_bind::bind_in(
                tx,
                &BindRequest {
                    name: &a.display_name,
                    subtype,
                    scope: &scope,
                    identifiers: &[],
                    external: Some((&ext_scope, &legacy)),
                    provenance: json!({"atlas_run":run_id,"article":article_id}),
                },
                now,
            )
            .await?;
            touched.extend(changed);
            let spans: Vec<Value> = a
                .mentions
                .iter()
                .map(|m| json!({"field":m.field,"start_byte":m.start_byte,"end_byte":m.end_byte}))
                .collect();
            let fields: Vec<&str> = {
                let mut f: Vec<&str> = a.mentions.iter().map(|m| m.field.as_str()).collect();
                f.dedup();
                f
            };
            touched.extend(
                lifecycle::record_mention_in(
                    tx,
                    &binding.actor_id,
                    "article",
                    run_id,
                    article_id,
                    revision,
                    &Value::Array(spans).to_string(),
                    now,
                )
                .await?,
            );
            let link = LinkPayload {
                source: EndpointRef::Memory {
                    memory_id: binding.actor_id.clone(),
                    kind: MemoryKind::Actor,
                },
                target: article_ep.clone(),
                relation: "mentioned_in".into(),
                rationale: format!("article {} mentions {}", fields.join("/"), a.display_name),
                receipts: vec![format!(
                    "atlas_article_mentions:{run_id}/{article_id}:{revision}"
                )],
                promoted_from: None,
            };
            let id = hid(
                "link",
                &[
                    &link.relation,
                    &endpoint_key(&link.source),
                    &endpoint_key(&link.target),
                ],
            );
            touched.insert(put_memory(tx, &id, MemoryPayload::Link(link)).await?);
            bound.push((
                a.display_name.clone(),
                subtype_str(subtype).to_string(),
                binding.actor_id,
            ));
        }
        // Bounded tentative Related records, newest origins first.
        let mut written = 0usize;
        'outer: for (name, subtype, actor_id) in &bound {
            for other in lifecycle::name_candidates_in(tx, name, subtype, actor_id).await? {
                let mut origins = lifecycle::direct_mentions_in(tx, &other).await?;
                origins.retain(|(k, _, id, _)| k == "article" && id != article_id);
                origins.sort_by(|a, b| b.1.cmp(&a.1).then(a.2.cmp(&b.2)));
                for (_, orun, oid, orev) in origins {
                    if written >= crate::brain_actors::RELATED_PER_ARTICLE {
                        break 'outer;
                    }
                    let Some(candidate) = crate::storage::atlas::article(db, &orun, &oid).await?
                    else {
                        continue;
                    };
                    let cmeta = crate::storage::atlas_metadata::metadata(db, &orun, &oid).await?;
                    let shared = crate::related_articles::shared_entity_mentions_with(
                        article,
                        Some(metadata),
                        &candidate,
                        cmeta.as_ref(),
                    );
                    if shared.len() < crate::related_articles::MIN_SHARED_ENTITIES {
                        continue;
                    }
                    let related = RelatedPayload {
                        source: article_ep.clone(),
                        target: EndpointRef::Article {
                            run_id: orun,
                            article_id: oid,
                            revision: orev,
                        },
                        shared_actors: vec![],
                        basis: format!(
                            "shared mentions: {} (identity unconfirmed)",
                            shared.join(", ")
                        ),
                        interpretation: "Articles mention actors with the same name; identity is not established.".into(),
                        verification_gap: "Confirm both mentions refer to the same entity.".into(),
                        source_revisions: vec![format!("ingest:{run_id}/{article_id}:{revision}")],
                    };
                    let mut keys = [endpoint_key(&related.source), endpoint_key(&related.target)];
                    keys.sort();
                    let id = hid("related", &[&keys[0], &keys[1], &related.basis]);
                    touched.insert(put_memory(tx, &id, MemoryPayload::Related(related)).await?);
                    written += 1;
                }
            }
        }
        Ok((
            format!("{} actor(s), {written} related", bound.len()),
            touched.into_iter().collect(),
        ))
    }

    async fn put_memory(tx: &Tx, id: &str, payload: MemoryPayload) -> Result<String, TxError> {
        put_typed_in(
            tx,
            &TypedMemory {
                id: id.to_string(),
                text: text_of(&payload),
                kind: payload.kind(),
                payload: Some(payload),
                lifecycle: "active".into(),
            },
        )
        .await?;
        Ok(id.to_string())
    }
}

#[cfg(test)]
#[path = "atlas_actor_index_tests.rs"]
pub(crate) mod tests;
