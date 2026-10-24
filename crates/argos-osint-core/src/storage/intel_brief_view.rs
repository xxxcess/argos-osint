//! The Intel brief page's article reads on SurrealDB (the TUI
//! `IntelBriefSnapshot` parts that do not depend on the brief chain):
//! actor metadata, insight lifecycle and refresh status, the active output
//! and its claim rows, assessed link explanations, claim relations and the
//! retained article body.

use std::collections::HashMap;

use anyhow::Result;
use surrealdb::types::SurrealValue;

use super::repo::Repository;
use crate::atlas_metadata::ArticleMetadata;
use crate::intel_recon::ArticleBodyRow;
use crate::investigation::insight_persist::{claim_rows_from_output, ClaimMemory};
use crate::investigation::insights::PipelineOutput;
use crate::store::{AtlasArticleClaim, AtlasArticleRow};

#[derive(Debug, Default)]
pub struct BriefParts {
    pub metadata: Option<ArticleMetadata>,
    pub lifecycle: String,
    pub status: String,
    pub output: Option<PipelineOutput>,
    pub claims: Vec<AtlasArticleClaim>,
    pub explanations: HashMap<(String, String), String>,
    pub relations: Vec<(String, String, String)>,
    pub body: Option<ArticleBodyRow>,
}

#[derive(SurrealValue)]
struct Owner {
    lifecycle: String,
    last_error: String,
}

#[derive(SurrealValue)]
struct ClaimMemoryRow {
    text: Option<String>,
    tombstoned: bool,
}

/// Read every repository-backed brief part for `article`.
pub async fn brief_parts(repo: &Repository, article: &AtlasArticleRow) -> Result<BriefParts> {
    let db = repo.db();
    let metadata = super::atlas_metadata::metadata(db, &article.run_id, &article.id).await?;
    let mut r = db
        .query(
            "SELECT lifecycle, last_error FROM ONLY type::record('insight_owner_state', ['article', $a]);
             SELECT lifecycle, last_error FROM ONLY type::record('insight_owner_state', ['atlas', $a]);",
        )
        .bind(("a", article.id.clone()))
        .await?
        .check()?;
    let article_owner: Option<Owner> = r.take(0usize)?;
    let atlas_owner: Option<Owner> = r.take(1usize)?;
    let lifecycle = article_owner
        .as_ref()
        .or(atlas_owner.as_ref())
        .map(|o| o.lifecycle.clone())
        .unwrap_or_else(|| "legacy/unassessed".into());
    let status = refresh_status(repo, &article.id, article_owner.as_ref()).await?;
    let mut output = super::insight_owner::active_output(repo, "article", &article.id).await?;
    let mut claim_lifecycle = article_owner.as_ref().map(|o| o.lifecycle.clone());
    if output.is_none() {
        output = super::insight_owner::active_output(repo, "atlas", &article.id).await?;
        claim_lifecycle = atlas_owner.as_ref().map(|o| o.lifecycle.clone());
    }
    let mut claims = Vec::new();
    if let Some(out) = &output {
        let mut memories = HashMap::new();
        for fp in out
            .claims
            .iter()
            .map(crate::investigation::insights::claim_fingerprint)
            .chain(
                out.inferences
                    .iter()
                    .map(crate::investigation::insights::inference_fingerprint),
            )
        {
            let mut r = db
                .query(
                    "LET $m = (SELECT VALUE memory_id FROM ONLY type::record('insight_claim', $fp));
                     RETURN {
                       text: IF $m THEN (SELECT VALUE text FROM ONLY type::record('memory', $m)) ELSE NONE END,
                       tombstoned: count(SELECT id FROM memory_tombstone WHERE fingerprint = $fp LIMIT 1) > 0
                     };",
                )
                .bind(("fp", fp.clone()))
                .await?
                .check()?;
            let row: Option<ClaimMemoryRow> = r.take(1usize)?;
            memories.insert(fp, row);
        }
        claims = claim_rows_from_output(
            out,
            claim_lifecycle.as_deref().unwrap_or(""),
            &article.id,
            |fp| {
                Ok(match memories.get(fp).and_then(Option::as_ref) {
                    Some(ClaimMemoryRow {
                        text: Some(text), ..
                    }) => ClaimMemory::Current(text.clone()),
                    Some(ClaimMemoryRow {
                        tombstoned: true, ..
                    }) => ClaimMemory::Tombstoned,
                    _ => ClaimMemory::Unpublished,
                })
            },
        )?;
    }
    let explanations = super::intel::link_explanations(db, &article.id).await?;
    let ids: Vec<String> = claims.iter().map(|c| c.fingerprint.clone()).collect();
    let relations = if ids.is_empty() {
        Vec::new()
    } else {
        let mut r = db
            .query(
                "SELECT VALUE [left_fingerprint, right_fingerprint, relation] FROM insight_relation
                   WHERE left_fingerprint IN $ids AND right_fingerprint IN $ids",
            )
            .bind(("ids", ids))
            .await?
            .check()?;
        r.take::<Vec<Vec<String>>>(0usize)?
            .into_iter()
            .filter(|v| v.len() == 3)
            .map(|v| (v[0].clone(), v[1].clone(), v[2].clone()))
            .collect()
    };
    let body = super::pipeline_inputs::article_body_for_article(repo, &article.id).await?;
    Ok(BriefParts {
        metadata,
        lifecycle,
        status,
        output,
        claims,
        explanations,
        relations,
        body,
    })
}

/// `insight_persist::article_refresh_status` on the repository, from the
/// owner state and its newest refresh task.
async fn refresh_status(
    repo: &Repository,
    article_id: &str,
    owner: Option<&Owner>,
) -> Result<String> {
    let Some(owner) = owner else {
        return Ok("Waiting · complete article details required".into());
    };
    if !owner.last_error.is_empty() {
        return Ok(format!("Previous · {}", owner.last_error));
    }
    let mut r = repo
        .db()
        .query(
            "SELECT VALUE (SELECT VALUE state FROM ONLY type::record('task', record::id($parent.id))) ?? ''
               FROM insight_refresh_task WHERE owner_scope = $scope ORDER BY created_at DESC LIMIT 1",
        )
        .bind(("scope", format!("article:{article_id}")))
        .await?
        .check()?;
    let task = r
        .take::<Vec<String>>(0usize)?
        .into_iter()
        .next()
        .unwrap_or_default();
    Ok(match task.as_str() {
        "running" => "Running extraction".into(),
        "queued" | "retry_scheduled" => "Queued".into(),
        _ if owner.lifecycle == "active" => "Published".into(),
        _ => format!("Waiting · {}", owner.lifecycle),
    })
}
