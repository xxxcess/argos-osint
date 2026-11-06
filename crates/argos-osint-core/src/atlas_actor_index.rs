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
#[cfg(feature = "surreal")]
pub(crate) use surreal::process_derivative_claims;

pub const BATCH: usize = 64;
pub const INDEX_CONTRACT: &str = "atlas-actor-index-v6-metadata-first";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexReport {
    pub total: u32,
    /// Receipts already current before this pass (resume / rerun).
    pub reused: u32,
    pub indexed: u32,
    /// Indexed occurrences with no bindable actor (still receipted).
    pub zero_actor: u32,
    pub failed: u32,
    /// Retained legacy/rejected/pending rows preserved without new inventory.
    pub ineligible: u32,
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
    use std::collections::HashMap;
    use std::sync::Arc;

    use anyhow::{anyhow, Result};
    use serde_json::{json, Value};
    use surrealdb::types::SurrealValue;

    use super::{checksum, IndexReport};
    use crate::atlas_metadata::ArticleMetadata;
    use crate::brain_actors::{endpoint_key, hid, text_of};
    use crate::memory_metadata::{
        EndpointRef, LinkPayload, MemoryKind, MemoryPayload, RelatedPayload,
    };
    use crate::storage::lifecycle::{self, ArticleIndexRow};
    use crate::storage::memory::{put_typed_in, write_then_index, MemoryWrite, TypedMemory};
    use crate::storage::repo::Repository;
    use crate::storage::tx::{Tx, TxError};
    use crate::store::AtlasArticleRow;

    /// Bounded current native units. Backfill callers can preserve their own
    /// budget while advancing this occurrence keyset between pages.
    pub async fn run_units_page(
        repo: &Repository,
        run_id: &str,
        after: Option<&crate::atlas_classify::OccurrenceKey>,
    ) -> Result<Vec<(AtlasArticleRow, Option<ArticleMetadata>, String)>> {
        use crate::storage::classifier_admission::{self, DerivativeStage};
        let keys =
            crate::storage::atlas::current_occurrence_keys_page(repo, Some(run_id), None, after)
                .await?;
        let owner = classifier_admission::AdmissionOwner::new(repo.clone());
        let result = classifier_admission::owned_tx(
            &owner,
            crate::storage::tx::RetryPolicy::default(),
            move |tx| {
                let keys = keys.clone();
                async move {
                    let rows = classifier_admission::derivative_snapshots_in(
                        &tx,
                        &keys,
                        DerivativeStage::Actors,
                    )
                    .await?;
                    let mut out = Vec::new();
                    for (_, (snapshot, article, metadata)) in rows {
                        if snapshot.admission_state == "admitted" {
                            out.push((
                                article,
                                metadata,
                                snapshot.revision(DerivativeStage::Actors)?,
                            ));
                        }
                    }
                    Ok(out)
                }
            },
        )
        .await;
        let cleanup = owner.shutdown().await;
        let result = result?;
        cleanup?;
        Ok(result.value)
    }
    /// Compatibility collector for explicit backfill. Normal indexing pages
    /// through `run_units_page`/thin keys and does not retain the whole run.
    pub async fn run_units(
        repo: &Repository,
        run_id: &str,
    ) -> Result<Vec<(AtlasArticleRow, Option<ArticleMetadata>, String)>> {
        let mut out = Vec::new();
        let mut after = None;
        loop {
            let page = run_units_page(repo, run_id, after.as_ref()).await?;
            if page.is_empty() {
                break;
            }
            after = page.last().map(|(article, _, _)| {
                crate::atlas_classify::OccurrenceKey::new(&article.run_id, &article.id)
            });
            out.extend(page);
        }
        Ok(out)
    }

    pub async fn index_article_occurrences(repo: &Repository, article_id: &str) -> Result<()> {
        let mut after = None;
        let mut indexed = 0usize;
        loop {
            let keys = crate::storage::atlas::current_occurrence_keys_page(
                repo,
                None,
                Some(article_id),
                after.as_ref(),
            )
            .await?;
            if keys.is_empty() {
                break;
            }
            after = keys.last().cloned();
            let report = refresh_keys(repo, &keys, &|| false).await?;
            indexed += usize::try_from(report.indexed + report.reused).unwrap_or(usize::MAX);
            if !report.complete() {
                anyhow::bail!(
                    "native actor refresh is incomplete: {}",
                    report.errors.join("; ")
                );
            }
        }
        let mut response = repo
            .db()
            .query("RETURN count(SELECT id FROM atlas_article WHERE article_id=$article);")
            .bind(("article", article_id.to_owned()))
            .await?
            .check()?;
        let retained = response
            .take::<Option<i64>>(0)?
            .ok_or_else(|| anyhow!("missing retained occurrence count"))?;
        if usize::try_from(retained).ok() != Some(indexed) {
            anyhow::bail!(
                "native actor refresh ineligible for legacy, rejected or pending occurrences"
            );
        }
        Ok(())
    }

    /// Current, checksum-valid index receipts, keyed by original article ID.
    pub async fn receipts(repo: &Repository, run_id: &str) -> Result<HashMap<String, String>> {
        Ok(lifecycle::article_index_receipts(repo.db(), run_id)
            .await?
            .into_iter()
            .filter(|row| row.index_checksum == checksum(&row.input_revision, &row.actor_coverage))
            .map(|row| (row.article_id, row.input_revision))
            .collect())
    }

    #[derive(SurrealValue)]
    struct IndexReceiptRow {
        run_id: Option<String>,
        article_id: Option<String>,
        input_revision: Option<String>,
        index_checksum: Option<String>,
        actor_coverage: Option<String>,
    }

    const RECEIPTS_MANY_SQL: &str = "SELECT
        IF bytes::len(type::bytes(run_id))<=512 {run_id} ELSE {NONE} AS run_id,
        IF bytes::len(type::bytes(article_id))<=512 {article_id} ELSE {NONE} AS article_id,
        IF bytes::len(type::bytes(input_revision))<=512 {input_revision} ELSE {NONE} AS input_revision,
        IF bytes::len(type::bytes(index_checksum))<=64 {index_checksum} ELSE {NONE} AS index_checksum,
        IF bytes::len(type::bytes(actor_coverage))<=4096 {actor_coverage} ELSE {NONE} AS actor_coverage
        FROM $records;";

    fn receipt_records(
        keys: &[crate::atlas_classify::OccurrenceKey],
    ) -> std::result::Result<Vec<surrealdb::types::RecordId>, TxError> {
        if keys.len() > 64 {
            return Err(TxError::Abort("actor_receipt_key_bound".into()));
        }
        Ok(keys
            .iter()
            .map(|key| crate::storage::jobs::rid("atlas_article_index", &key.record_id()))
            .collect())
    }

    fn valid_receipts(
        keys: &[crate::atlas_classify::OccurrenceKey],
        rows: Vec<IndexReceiptRow>,
    ) -> std::result::Result<
        std::collections::BTreeMap<crate::atlas_classify::OccurrenceKey, String>,
        TxError,
    > {
        let mut out = std::collections::BTreeMap::new();
        for row in rows {
            let (Some(run), Some(article), Some(revision), Some(digest), Some(coverage)) = (
                row.run_id,
                row.article_id,
                row.input_revision,
                row.index_checksum,
                row.actor_coverage,
            ) else {
                // Invalid or oversized output is missing work, not proof that
                // the occurrence is already indexed. The native writer repairs it.
                continue;
            };
            let key = crate::atlas_classify::OccurrenceKey::new(run, article);
            if !keys.contains(&key) {
                return Err(TxError::Abort("actor_receipt_correlation".into()));
            }
            if digest == checksum(&revision, &coverage) {
                out.insert(key, revision);
            }
        }
        Ok(out)
    }

    /// Bounded equivalent of `receipts`, retaining the same checksum validator.
    pub async fn receipts_many(
        repo: &Repository,
        keys: &[crate::atlas_classify::OccurrenceKey],
    ) -> Result<std::collections::BTreeMap<crate::atlas_classify::OccurrenceKey, String>> {
        let records = receipt_records(keys)?;
        if records.is_empty() {
            return Ok(std::collections::BTreeMap::new());
        }
        let mut response =
            crate::storage::classifier_admission::db_query(repo.db(), RECEIPTS_MANY_SQL)
                .bind(("records", records))
                .await?
                .check()?;
        Ok(valid_receipts(keys, response.take(0)?)?)
    }

    /// Checks actual actor output in the same source-fenced transaction as the
    /// derivative claim or completion audit. At most 64 point reads are issued.
    pub(crate) async fn receipts_many_in(
        tx: &Tx,
        keys: &[crate::atlas_classify::OccurrenceKey],
    ) -> std::result::Result<
        std::collections::BTreeMap<crate::atlas_classify::OccurrenceKey, String>,
        TxError,
    > {
        let records = receipt_records(keys)?;
        if records.is_empty() {
            return Ok(std::collections::BTreeMap::new());
        }
        let mut response = crate::storage::classifier_admission::tx_query(tx, RECEIPTS_MANY_SQL)
            .bind(("records", records))
            .await?
            .check()?;
        valid_receipts(keys, response.take(0)?)
    }

    pub async fn index_run(
        repo: &Repository,
        run_id: &str,
        stop: &(dyn Fn() -> bool + Sync),
    ) -> Result<IndexReport> {
        let mut after = None;
        let mut report = IndexReport::default();
        loop {
            if stop() {
                report.stopped = true;
                break;
            }
            let keys = crate::storage::atlas::current_occurrence_keys_page(
                repo,
                Some(run_id),
                None,
                after.as_ref(),
            )
            .await?;
            if keys.is_empty() {
                break;
            }
            after = keys.last().cloned();
            let page = refresh_keys(repo, &keys, stop).await?;
            report.total += page.total;
            report.reused += page.reused;
            report.indexed += page.indexed;
            report.zero_actor += page.zero_actor;
            report.failed += page.failed;
            report.errors.extend(page.errors);
            report.stopped |= page.stopped;
            if report.stopped {
                break;
            }
        }
        let mut response = repo
            .db()
            .query("RETURN count(SELECT id FROM atlas_article WHERE run_id=$run);")
            .bind(("run", run_id.to_owned()))
            .await?
            .check()?;
        let retained = u32::try_from(
            response
                .take::<Option<i64>>(0)?
                .ok_or_else(|| anyhow!("missing native retained count"))?,
        )
        .map_err(|_| anyhow!("native retained count bound"))?;
        report.ineligible = retained.saturating_sub(report.total);
        report.total = retained;
        if report.ineligible > 0 {
            report.errors.push(
                "legacy, rejected or pending occurrences are ineligible for new native indexing"
                    .into(),
            );
        }
        Ok(report)
    }

    /// Existing native/backfill entry: all captured publication checks and
    /// lifecycle mutation are shared with foreground and maintenance workers.
    pub async fn index_occurrence(
        repo: &Repository,
        article: &AtlasArticleRow,
        _metadata: Option<&ArticleMetadata>,
        _revision: &str,
        _now: &str,
    ) -> Result<String> {
        let key = crate::atlas_classify::OccurrenceKey::new(&article.run_id, &article.id);
        let report = refresh_keys(repo, std::slice::from_ref(&key), &|| false).await?;
        if !report.complete() {
            anyhow::bail!(
                "native actor occurrence unavailable or ineligible: {}",
                report.errors.join("; ")
            );
        }
        let mut response = repo
            .db()
            .query("SELECT VALUE actor_coverage FROM ONLY $key;")
            .bind((
                "key",
                crate::storage::jobs::rid(
                    "atlas_article_index",
                    &crate::storage::codec::encode(&[&article.run_id, &article.id]),
                ),
            ))
            .await?
            .check()?;
        response
            .take::<Option<String>>(0)?
            .ok_or_else(|| anyhow!("native actor receipt missing"))
    }

    async fn refresh_keys(
        repo: &Repository,
        keys: &[crate::atlas_classify::OccurrenceKey],
        stop: &(dyn Fn() -> bool + Sync),
    ) -> Result<IndexReport> {
        use crate::storage::classifier_admission::{self, DerivativeDisposition, DerivativeStage};
        if keys.len() > 64 {
            anyhow::bail!("native actor refresh key bound");
        }
        let owner = classifier_admission::AdmissionOwner::new(repo.clone());
        let result=async {
            let(fence,work)=classifier_admission::native_refresh_work(&owner,keys).await?;
            let mut report=IndexReport {total:keys.len() as u32,ineligible:keys.len().saturating_sub(work.len()) as u32,..Default::default()};
            if work.is_empty(){report.errors.push("current admitted receipt required".into());return Ok::<_,anyhow::Error>(report);}
            let cancel=Arc::new(std::sync::atomic::AtomicBool::new(false));
            let mut changed=std::collections::BTreeSet::new();
            for stage in [DerivativeStage::Actors,DerivativeStage::Reconcile] {
                loop {
                    let processing=classifier_admission::process_derivative_stage(&owner,&fence,&work,stage,&cancel);
                    tokio::pin!(processing);
                    let result=loop{tokio::select! {
                        result=&mut processing=>break result?,
                        _=tokio::time::sleep(std::time::Duration::from_millis(50))=>{if stop(){cancel.store(true,std::sync::atomic::Ordering::Release);}},
                    }};
                    if cancel.load(std::sync::atomic::Ordering::Acquire){report.stopped=true;return Ok(report);}
                    if stage==DerivativeStage::Actors {for item in &result.items {if matches!(item.disposition,DerivativeDisposition::Current){changed.insert(item.work.occurrence.clone());}}}
                    let states:Vec<_>=result.states.iter().filter_map(|state|state.stages.iter().find(|state|state.stage==stage).and_then(|state|state.disposition.as_ref())).collect();
                    if states.len()==work.len()&&states.iter().all(|state|state.terminal()){break;}
                    let continuing=result.items.iter().any(|item|matches!(&item.disposition,DerivativeDisposition::Retryable {code} if code.ends_with("continuation")));
                    if !continuing {
                        let code=result.items.iter().find_map(|item|native_failure_code(&item.disposition))
                            .or_else(||states.iter().find_map(|state|native_failure_code(state)))
                            .unwrap_or("native_state_incomplete");
                        report.failed=work.len() as u32;
                        report.errors.push(format!("{} native work incomplete: {code}",stage.as_str()));
                        return Ok(report);
                    }
                }
            }
            report.indexed=changed.len() as u32;report.reused=(work.len()-changed.len()) as u32;
            let records:Vec<_>=work.iter().map(|work|crate::storage::jobs::rid("atlas_article_index",&work.occurrence.record_id())).collect();
            let mut response=owner.query("RETURN count(SELECT id FROM $records WHERE actor_coverage='zero actors');").bind(("records",records)).await?.check()?;
            report.zero_actor=u32::try_from(response.take::<Option<i64>>(0)?.ok_or_else(||anyhow!("missing native zero-actor count"))?).map_err(|_|anyhow!("negative native zero-actor count"))?;
            Ok(report)
        }.await;
        let cleanup = owner.shutdown().await;
        let result = result?;
        cleanup?;
        Ok(result)
    }

    /// Operational errors expose only bounded machine identifiers, never the
    /// publisher's SQL or captured source material.
    fn native_failure_code(
        disposition: &crate::storage::classifier_admission::DerivativeDisposition,
    ) -> Option<&str> {
        use crate::storage::classifier_admission::DerivativeDisposition;
        match disposition {
            DerivativeDisposition::Retryable { code }
                if !code.is_empty()
                    && code.len() <= 64
                    && code
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_') =>
            {
                Some(code)
            }
            DerivativeDisposition::Retryable { .. } => Some("native_retryable"),
            DerivativeDisposition::StaleInput => Some("stale_input"),
            _ => None,
        }
    }

    async fn catalog_matcher(
        repo: &Repository,
    ) -> Result<Option<Arc<crate::actor_catalog::Matcher>>> {
        let mut response = repo
            .db()
            .query("SELECT VALUE status FROM ONLY brain_actor_seed:catalog;")
            .await?
            .check()?;
        match response.take::<Option<String>>(0)?.as_deref() {
            Some("complete") => Ok(Some(repo.actor_matcher().await?)),
            Some(_) => anyhow::bail!("Actor catalog is incomplete; run Seed DB from Argos Venture"),
            None => Ok(None),
        }
    }

    /// Processes only captured, leased occurrences. Catalog/body unavailability
    /// never retires a valid inventory or creates an empty successful receipt.
    pub(crate) async fn process_derivative_claims(
        owner: &crate::storage::classifier_admission::AdmissionOwner,
        fence: &crate::storage::classifier_admission::DerivativeFence,
        claims: &[crate::storage::classifier_admission::DerivativeClaim],
        cancel: &Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<
        std::collections::BTreeMap<
            crate::atlas_classify::OccurrenceKey,
            crate::storage::classifier_admission::DerivativeDisposition,
        >,
    > {
        use crate::storage::classifier_admission::{self, DerivativeDisposition};
        if claims.len() > 64 {
            anyhow::bail!("actor derivative batch bound");
        }
        let repo = owner.repository();
        let mut out = std::collections::BTreeMap::new();
        let Some(catalog) = catalog_matcher(repo).await? else {
            for claim in claims {
                if claim.snapshot.admission_state == "admitted" {
                    let disposition = classifier_admission::release_derivative(
                        owner,
                        fence,
                        claim,
                        DerivativeDisposition::Retryable {
                            code: "actor_catalog_unavailable".into(),
                        },
                    )
                    .await?;
                    out.insert(claim.work.occurrence.clone(), disposition);
                }
            }
            return Ok(out);
        };
        let heads: std::collections::BTreeMap<_, _> = claims
            .iter()
            .filter_map(|claim| {
                claim
                    .snapshot
                    .body
                    .clone()
                    .map(|head| (claim.article.id.clone(), head))
            })
            .collect();
        let (materials, oversized) = classifier_admission::owned_tx(
            owner,
            crate::storage::tx::RetryPolicy::default(),
            move |tx| {
                let heads = heads.clone();
                async move {
                    crate::storage::actor_catalog::clean_body_materials_many_in(&tx, &heads).await
                }
            },
        )
        .await?
        .value;
        for claim in claims {
            if claim.snapshot.admission_state != "admitted" {
                continue;
            }
            if cancel.load(std::sync::atomic::Ordering::Acquire) {
                break;
            }
            if claim.snapshot.body.is_some() && !materials.contains_key(&claim.article.id) {
                let disposition = classifier_admission::release_derivative(
                    owner,
                    fence,
                    claim,
                    DerivativeDisposition::Retryable {
                        code: if oversized.contains(&claim.article.id) {
                            "body_material_payload_too_large"
                        } else {
                            "body_candidates_unavailable"
                        }
                        .into(),
                    },
                )
                .await?;
                out.insert(claim.work.occurrence.clone(), disposition);
                continue;
            }
            let article = claim.article.clone();
            let catalog = catalog.clone();
            let material = materials.get(&article.id).cloned();
            let (found, clean) = tokio::task::spawn_blocking(move || {
                let mut found = catalog.article_actors(&article);
                let clean = material.map(|(clean, candidates)| {
                    crate::actor_catalog::append_body_actors(
                        &mut found,
                        catalog.body_actors(&clean.text, &candidates),
                    );
                    clean
                });
                (found, clean)
            })
            .await?;
            // Deterministically bounded tentative candidates, hydrated in bulk.
            let candidates = tentative_candidates(owner, &found, &claim.work.occurrence).await?;
            let fence = fence.clone();
            let claim = claim.clone();
            let result_key = claim.work.occurrence.clone();
            let result_receipt = claim.work.receipt_key.clone();
            let release_claim = claim.clone();
            let release_fence = fence.clone();
            let live_cancel = cancel.clone();
            let write: MemoryWrite = Arc::new(move |tx: Tx| {
                let fence = fence.clone();
                let claim = claim.clone();
                let found = found.clone();
                let clean = clean.clone();
                let candidates = candidates.clone();
                let cancel = live_cancel.clone();
                Box::pin(async move {
                    if cancel.load(std::sync::atomic::Ordering::Acquire) {
                        return Err(TxError::Abort("actor_publication_cancelled".into()));
                    }
                    let now = chrono::Utc::now();
                    if !classifier_admission::check_derivative_in(&tx, &fence, &claim, now).await? {
                        return Err(TxError::Abort("actor_derivative_stale".into()));
                    }
                    if let Some(body) = &claim.snapshot.body {
                        if !crate::storage::actor_catalog::fence_body_candidates_in(
                            &tx,
                            std::slice::from_ref(body),
                        )
                        .await?
                        {
                            return Err(TxError::Abort("actor_candidates_changed".into()));
                        }
                    }
                    let scope = lifecycle::ReconcileScope {
                        keys: vec![lifecycle::ReconcileKey::Source {
                            run_id: claim.article.run_id.clone(),
                            article_id: claim.article.id.clone(),
                        }],
                    };
                    let cursor = if claim.checkpoint_json.is_empty() {
                        None
                    } else {
                        Some(
                            serde_json::from_str::<lifecycle::ReconcileCursor>(
                                &claim.checkpoint_json,
                            )
                            .map_err(|_| TxError::Abort("actor_capture_checkpoint".into()))?,
                        )
                    };
                    let capture =
                        lifecycle::capture_affected_in(&tx, &scope, cursor.as_ref()).await?;
                    if !capture.ready {
                        let checkpoint = serde_json::to_string(&capture.cursor)
                            .map_err(|_| TxError::Abort("actor_capture_checkpoint".into()))?;
                        if !classifier_admission::complete_derivative_in(
                            &tx,
                            &fence,
                            &claim,
                            &DerivativeDisposition::Retryable {
                                code: "actor_capture_continuation".into(),
                            },
                            &checkpoint,
                            now,
                        )
                        .await?
                        {
                            return Err(TxError::Abort("actor_capture_lease_lost".into()));
                        }
                        return Ok(Vec::new());
                    }
                    let batch = lifecycle::SupportBatch::for_capture(&tx, &capture)?;
                    let coverage = if found.is_empty() {
                        "zero actors".to_owned()
                    } else {
                        format!("{} actor(s)", found.len())
                    };
                    let mut touched = write_scoped_inventory_in(
                        &tx,
                        &claim.article,
                        &found,
                        clean.as_ref(),
                        &batch,
                        &now.to_rfc3339(),
                    )
                    .await?;
                    // Candidate source/metadata guards are acquired and compared
                    // before creating each tentative relation in this transaction.
                    let keys: Vec<_> = candidates.keys().cloned().collect();
                    let mut current = std::collections::BTreeMap::new();
                    for keys in keys.chunks(64) {
                        current.extend(
                            classifier_admission::derivative_snapshots_in(
                                &tx,
                                keys,
                                classifier_admission::DerivativeStage::Related,
                            )
                            .await?,
                        );
                    }
                    let mut written_related = 0usize;
                    for (key, (snapshot, article, metadata)) in &candidates {
                        let Some((actual, _, _)) = current.get(key) else {
                            continue;
                        };
                        if snapshot.revision(classifier_admission::DerivativeStage::Related)?
                            != actual.revision(classifier_admission::DerivativeStage::Related)?
                        {
                            continue;
                        }
                        let shared = crate::related_articles::shared_entity_mentions_with(
                            &claim.article,
                            claim.metadata.as_ref(),
                            article,
                            metadata.as_ref(),
                        );
                        if shared.len() < crate::related_articles::MIN_SHARED_ENTITIES {
                            continue;
                        }
                        let related=RelatedPayload {
                            source:EndpointRef::Article {run_id:claim.article.run_id.clone(),article_id:claim.article.id.clone(),revision:claim.snapshot.source.source_hash.clone()},
                            target:EndpointRef::Article {run_id:article.run_id.clone(),article_id:article.id.clone(),revision:snapshot.source.source_hash.clone()},shared_actors:Vec::new(),
                            basis:format!("shared mentions: {} (identity unconfirmed)",shared.join(", ")),
                            interpretation:"Articles mention actors with the same name; identity is not established.".into(),verification_gap:"Confirm both mentions refer to the same entity.".into(),
                            source_revisions:vec![format!("ingest:{}/{}:{}",claim.article.run_id,claim.article.id,claim.snapshot.source.source_hash)],
                        };
                        let mut keys =
                            [endpoint_key(&related.source), endpoint_key(&related.target)];
                        keys.sort();
                        let id = hid("related", &[&keys[0], &keys[1], &related.basis]);
                        if let Some(id) =
                            put_memory_in_batch(&tx, &id, MemoryPayload::Related(related), &batch)
                                .await?
                        {
                            touched.push(id);
                        }
                        written_related += 1;
                        if written_related >= crate::brain_actors::RELATED_PER_ARTICLE {
                            break;
                        }
                    }
                    lifecycle::put_article_index_in(
                        &tx,
                        &claim.article.run_id,
                        &claim.article.id,
                        &ArticleIndexRow {
                            article_id: claim.article.id.clone(),
                            input_revision: claim.input_revision.clone(),
                            index_checksum: checksum(&claim.input_revision, &coverage),
                            actor_coverage: coverage,
                        },
                        &now.to_rfc3339(),
                    )
                    .await?;
                    let progress = lifecycle::reconcile_captured_in(&tx, &scope, &capture).await?;
                    touched.extend(progress.changed_memory_ids);
                    if let Some(next) = progress.next {
                        let checkpoint = serde_json::to_string(&next)
                            .map_err(|_| TxError::Abort("actor_reconcile_checkpoint".into()))?;
                        crate::storage::jobs::run(&tx,tx.query("UPDATE $key SET checkpoint_json=$checkpoint,state='pending' WHERE owner='' RETURN NONE;").bind(("key",classifier_admission::derivative_record(&claim.work.receipt_key,classifier_admission::DerivativeStage::Reconcile))).bind(("checkpoint",checkpoint))).await?;
                    }
                    if cancel.load(std::sync::atomic::Ordering::Acquire) {
                        return Err(TxError::Abort("actor_publication_cancelled".into()));
                    }
                    if !classifier_admission::complete_derivative_in(
                        &tx,
                        &fence,
                        &claim,
                        &DerivativeDisposition::Current,
                        "",
                        now,
                    )
                    .await?
                    {
                        return Err(TxError::Abort("actor_completion_lease_lost".into()));
                    }
                    Ok(touched)
                })
            });
            let result = write_then_index(repo.publisher(), write).await;
            let disposition = if result.is_ok() {
                let mut response = owner
                    .query("SELECT VALUE state FROM ONLY $key")
                    .bind((
                        "key",
                        classifier_admission::derivative_record(
                            &result_receipt,
                            classifier_admission::DerivativeStage::Actors,
                        ),
                    ))
                    .await?
                    .check()?;
                if response.take::<Option<String>>(0)?.as_deref() == Some("complete") {
                    DerivativeDisposition::Current
                } else {
                    DerivativeDisposition::Retryable {
                        code: "actor_capture_continuation".into(),
                    }
                }
            } else {
                classifier_admission::release_derivative(
                    owner,
                    &release_fence,
                    &release_claim,
                    DerivativeDisposition::Retryable {
                        code: "actor_publication_failed".into(),
                    },
                )
                .await?
            };
            out.insert(result_key, disposition);
        }
        Ok(out)
    }

    async fn tentative_candidates(
        owner: &crate::storage::classifier_admission::AdmissionOwner,
        actors: &[crate::atlas_metadata::ArticleActor],
        seed: &crate::atlas_classify::OccurrenceKey,
    ) -> Result<
        std::collections::BTreeMap<
            crate::atlas_classify::OccurrenceKey,
            (
                crate::storage::classifier_admission::DerivativeSnapshot,
                AtlasArticleRow,
                Option<ArticleMetadata>,
            ),
        >,
    > {
        use crate::storage::classifier_admission::{self, DerivativeStage};
        let identities: Vec<_> = actors
            .iter()
            .map(|actor| {
                (
                    crate::brain_actors::normalize_alias(&actor.display_name),
                    actor.type_hint.clone(),
                )
            })
            .collect();
        let own_ids: Vec<_> = actors.iter().map(|actor| actor.id.clone()).collect();
        let mut response=owner.query("SELECT VALUE actor_id FROM brain_actor_alias WHERE alias_type='name' AND [normalized,(SELECT VALUE subtype FROM ONLY type::record('brain_actor',actor_id))] INSIDE $identities AND actor_id NOT IN $own ORDER BY actor_id LIMIT 64;").bind(("identities",identities)).bind(("own",own_ids)).await?.check()?;
        let ids: Vec<String> = response.take(0)?;
        let mentions = crate::storage::atlas::current_actor_mentions(
            owner.repository(),
            &ids,
            seed,
            100,
            None,
        )
        .await?;
        let keys: Vec<_> = mentions
            .into_iter()
            .map(|mention| {
                crate::atlas_classify::OccurrenceKey::new(mention.origin_scope, mention.origin_id)
            })
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        let committed = classifier_admission::owned_tx(
            owner,
            crate::storage::tx::RetryPolicy::default(),
            move |tx| {
                let keys = keys.clone();
                async move {
                    let mut out = std::collections::BTreeMap::new();
                    for keys in keys.chunks(64) {
                        out.extend(
                            classifier_admission::derivative_snapshots_in(
                                &tx,
                                keys,
                                DerivativeStage::Related,
                            )
                            .await?,
                        );
                    }
                    Ok(out)
                }
            },
        )
        .await?;
        Ok(committed.value)
    }

    async fn write_scoped_inventory_in(
        tx: &Tx,
        article: &AtlasArticleRow,
        actors: &[crate::atlas_metadata::ArticleActor],
        clean: Option<&crate::storage::actor_catalog::CleanBody>,
        batch: &lifecycle::SupportBatch,
        now: &str,
    ) -> Result<Vec<String>, TxError> {
        let revision = crate::atlas_metadata::input_hash(article);
        let mut touched = lifecycle::refresh_article_occurrence_in_batch(
            tx,
            &article.run_id,
            &article.id,
            &revision,
            batch,
        )
        .await?;
        let target = EndpointRef::Article {
            run_id: article.run_id.clone(),
            article_id: article.id.clone(),
            revision: revision.clone(),
        };
        for actor in actors {
            let spans:Vec<Value>=actor.mentions.iter().map(|mention|{
                let mut value=json!({"field":mention.field,"start_byte":mention.start_byte,"end_byte":mention.end_byte});
                if mention.field=="body" {if let Some(clean)=clean {value["source_id"]=json!(clean.source_id);value["revision"]=json!(clean.revision);value["artifact_hash"]=json!(clean.artifact_hash);}}value
            }).collect();
            touched.extend(
                lifecycle::record_mention_in_batch(
                    tx,
                    &actor.id,
                    "article",
                    &article.run_id,
                    &article.id,
                    &revision,
                    &Value::Array(spans).to_string(),
                    now,
                    batch,
                )
                .await?,
            );
            let link = LinkPayload {
                source: EndpointRef::Memory {
                    memory_id: actor.id.clone(),
                    kind: MemoryKind::Actor,
                },
                target: target.clone(),
                relation: "mentioned_in".into(),
                rationale: format!("article mentions {}", actor.display_name),
                receipts: vec![format!(
                    "atlas_article_mentions:{}/{}:{revision}",
                    article.run_id, article.id
                )],
                promoted_from: None,
                validation: None,
            };
            let id = hid(
                "link",
                &[
                    &link.relation,
                    &endpoint_key(&link.source),
                    &endpoint_key(&link.target),
                ],
            );
            if let Some(id) =
                put_memory_in_batch(tx, &id, MemoryPayload::Link(Box::new(link)), batch).await?
            {
                touched.push(id);
            }
        }
        Ok(touched)
    }

    async fn put_memory_in_batch(
        tx: &Tx,
        id: &str,
        payload: MemoryPayload,
        batch: &lifecycle::SupportBatch,
    ) -> Result<Option<String>, TxError> {
        if !lifecycle::ingest_memory_writable_in(tx, id).await? {
            return Ok(None);
        }
        let id = put_memory(tx, id, payload).await?;
        lifecycle::mark_ingest_memory_in(tx, &id, batch).await?;
        Ok(Some(id))
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
