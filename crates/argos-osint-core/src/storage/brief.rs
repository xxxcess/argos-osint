//! Intel brief publications on SurrealDB: the repository side of
//! `intel_brief_publications` and the `intel_brief_runs` fence that the
//! retained Intel display seeds read (`brief_pipeline::publish`,
//! `brief_pipeline::published_brief`, the retained-seed check in
//! `brain_topology_query`).
//!
//! Record keys: `intel_brief_publication` by article id (one current
//! publication per article, as SQLite's primary key), `intel_brief_run` by
//! job id. A publication only replaces one of an equal or older generation;
//! on replacement the shared fields move to the new job and only the
//! publishing stage's list (claims, inferences or leads) is replaced, so
//! earlier completed stages stay canonical while later ones run, exactly as
//! the SQLite upsert does. The JSON lists are stored as strings.
//!
//! Gaps (acceptance.md): `intel_inference_article_leads`, the stage receipts
//! (`brief_canonical::publish_stage_on`) and the brief chain itself stay on
//! SQLite; nothing in the brief pipeline calls these yet.

use anyhow::{anyhow, Result};
use serde_json::Value;
use surrealdb::types::SurrealValue;

use super::jobs::{rid, run, take};
use super::repo::Repository;
use super::tx::{run_tx, RetryPolicy, Tx, TxError};

/// Brief publication schema (idempotent).
pub const SCHEMA: &str = "DEFINE TABLE IF NOT EXISTS intel_brief_publication SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS job_id ON intel_brief_publication TYPE string;
DEFINE FIELD IF NOT EXISTS generation ON intel_brief_publication TYPE int;
DEFINE FIELD IF NOT EXISTS contract ON intel_brief_publication TYPE string;
DEFINE FIELD IF NOT EXISTS body_hash ON intel_brief_publication TYPE string;
DEFINE FIELD IF NOT EXISTS snapshot_hash ON intel_brief_publication TYPE string;
DEFINE FIELD IF NOT EXISTS claims_json ON intel_brief_publication TYPE string;
DEFINE FIELD IF NOT EXISTS inferences_json ON intel_brief_publication TYPE string;
DEFINE FIELD IF NOT EXISTS leads_json ON intel_brief_publication TYPE string;
DEFINE FIELD IF NOT EXISTS published_at ON intel_brief_publication TYPE string;

DEFINE TABLE IF NOT EXISTS intel_brief_run SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS article_id ON intel_brief_run TYPE string;
DEFINE FIELD IF NOT EXISTS generation ON intel_brief_run TYPE int;
DEFINE FIELD IF NOT EXISTS state ON intel_brief_run TYPE string
  ASSERT $value INSIDE ['queued', 'active', 'completed', 'failed', 'cancelled', 'superseded'];
DEFINE FIELD IF NOT EXISTS updated_at ON intel_brief_run TYPE string;
DEFINE INDEX IF NOT EXISTS intel_brief_run_article ON intel_brief_run FIELDS article_id, generation;";

/// Apply [`SCHEMA`] (idempotent). Until bootstrap registers it, callers and
/// tests apply it themselves.
pub async fn apply_schema(repo: &Repository) -> Result<()> {
    repo.db().query(SCHEMA).await?.check()?;
    Ok(())
}

/// The brief stage whose list a publication replaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Claims,
    Inferences,
    Links,
    /// The body (or any other stage): shared fields only.
    Other,
}

impl Stage {
    /// From the brief operation name (`intel_brief_claims`, ...).
    pub fn from_operation(operation: &str) -> Self {
        match operation {
            "intel_brief_claims" => Self::Claims,
            "intel_brief_inferences" => Self::Inferences,
            "intel_brief_links" => Self::Links,
            _ => Self::Other,
        }
    }
}

/// One publication write (`brief_pipeline::publish`).
#[derive(Debug, Clone)]
pub struct Publication {
    pub article_id: String,
    pub job_id: String,
    pub generation: i64,
    pub contract: String,
    pub body_hash: String,
    pub snapshot_hash: String,
    pub claims: Value,
    pub inferences: Value,
    /// The `leads` list of the links stage.
    pub leads: Value,
    pub stage: Stage,
}

#[derive(SurrealValue)]
struct Current {
    generation: i64,
}

/// Publish inside `tx`. Returns false (nothing written) when a newer
/// generation is already published for the article.
pub async fn publish_in(tx: &Tx, p: &Publication) -> Result<bool, TxError> {
    let key = rid("intel_brief_publication", &p.article_id);
    let mut r = run(
        tx,
        tx.query("SELECT generation FROM ONLY $p").bind(("p", key.clone())),
    )
    .await?;
    let current: Option<Current> = take(&mut r, 0)?;
    let now = chrono::Utc::now().to_rfc3339();
    match current {
        Some(c) if c.generation > p.generation => Ok(false),
        Some(_) => {
            let column = match p.stage {
                Stage::Claims => Some(("claims_json", p.claims.to_string())),
                Stage::Inferences => Some(("inferences_json", p.inferences.to_string())),
                Stage::Links => Some(("leads_json", p.leads.to_string())),
                Stage::Other => None,
            };
            let set_list = match &column {
                Some((name, _)) => format!(", {name} = $list"),
                None => String::new(),
            };
            run(
                tx,
                tx.query(format!(
                    "UPDATE $p SET job_id = $job, generation = $g, contract = $contract,
                       body_hash = $body, snapshot_hash = $snap, published_at = $now{set_list}
                       RETURN NONE"
                ))
                .bind(("p", key))
                .bind(("job", p.job_id.clone()))
                .bind(("g", p.generation))
                .bind(("contract", p.contract.clone()))
                .bind(("body", p.body_hash.clone()))
                .bind(("snap", p.snapshot_hash.clone()))
                .bind(("now", now))
                .bind(("list", column.map(|(_, v)| v).unwrap_or_default())),
            )
            .await?;
            Ok(true)
        }
        None => {
            run(
                tx,
                tx.query(
                    "CREATE $p SET job_id = $job, generation = $g, contract = $contract,
                       body_hash = $body, snapshot_hash = $snap, claims_json = $claims,
                       inferences_json = $inferences, leads_json = $leads, published_at = $now
                       RETURN NONE",
                )
                .bind(("p", key))
                .bind(("job", p.job_id.clone()))
                .bind(("g", p.generation))
                .bind(("contract", p.contract.clone()))
                .bind(("body", p.body_hash.clone()))
                .bind(("snap", p.snapshot_hash.clone()))
                .bind(("claims", p.claims.to_string()))
                .bind(("inferences", p.inferences.to_string()))
                .bind(("leads", p.leads.to_string()))
                .bind(("now", now)),
            )
            .await?;
            Ok(true)
        }
    }
}

/// [`publish_in`] as its own transaction.
pub async fn publish(repo: &Repository, p: &Publication) -> Result<bool> {
    let out = run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
        publish_in(&tx, p).await
    })
    .await
    .map_err(|e| anyhow!("publish brief {}: {e}", p.article_id))?;
    Ok(out.value)
}

/// Record (or move) a brief run's state for the retained-seed fence.
pub async fn record_run(
    repo: &Repository,
    job_id: &str,
    article_id: &str,
    generation: i64,
    state: &str,
) -> Result<()> {
    repo.db()
        .query(
            "UPSERT $r SET article_id = $a, generation = $g, state = $s, updated_at = $now
               RETURN NONE",
        )
        .bind(("r", rid("intel_brief_run", job_id)))
        .bind(("a", article_id.to_string()))
        .bind(("g", generation))
        .bind(("s", state.to_string()))
        .bind(("now", chrono::Utc::now().to_rfc3339()))
        .await?
        .check()?;
    Ok(())
}

#[derive(SurrealValue)]
struct Published {
    generation: i64,
    claims_json: String,
    inferences_json: String,
}

/// `brief_pipeline::published_brief`: generation, claims and inferences.
pub async fn published_brief(
    repo: &Repository,
    article_id: &str,
) -> Result<Option<(i64, Value, Value)>> {
    let mut r = repo
        .db()
        .query("SELECT generation, claims_json, inferences_json FROM ONLY $p")
        .bind(("p", rid("intel_brief_publication", article_id)))
        .await?
        .check()?;
    let row: Option<Published> = r.take(0usize)?;
    row.map(|p| {
        Ok((
            p.generation,
            serde_json::from_str(&p.claims_json)?,
            serde_json::from_str(&p.inferences_json)?,
        ))
    })
    .transpose()
}

/// Which retained list a seed names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetainedKind {
    Claim,
    Inference,
}

#[derive(SurrealValue)]
struct Fence {
    pub_job: Option<String>,
    list: Option<String>,
    run_article: Option<String>,
    run_state: Option<String>,
    run_generation: Option<i64>,
    newest: Option<i64>,
}

/// SurrealQL for [`retained_item`]; public for the plan test.
pub const RETAINED_SURQL: &str = "LET $pub = (SELECT * FROM ONLY $p);
LET $run = (SELECT * FROM ONLY $r);
RETURN {
  pub_job: $pub.job_id,
  list: IF $kind = 'inference' { $pub.inferences_json } ELSE { $pub.claims_json },
  run_article: $run.article_id,
  run_state: $run.state,
  run_generation: $run.generation,
  newest: math::max((SELECT VALUE generation FROM intel_brief_run WHERE article_id = $a))
};";

/// The retained-seed fence: true when `job_id` is the article's current
/// publication, its run belongs to the article in an `active`, `completed`
/// or `failed` state with no newer run, and the item `local_id` with exactly
/// `text` is in the published list.
pub async fn retained_item(
    repo: &Repository,
    article_id: &str,
    job_id: &str,
    kind: RetainedKind,
    local_id: &str,
    text: &str,
) -> Result<bool> {
    let mut r = repo
        .db()
        .query(RETAINED_SURQL)
        .bind(("p", rid("intel_brief_publication", article_id)))
        .bind(("r", rid("intel_brief_run", job_id)))
        .bind(("a", article_id.to_string()))
        .bind((
            "kind",
            match kind {
                RetainedKind::Claim => "claim",
                RetainedKind::Inference => "inference",
            },
        ))
        .await?
        .check()?;
    let fence: Option<Fence> = r.take(2usize)?;
    let Some(f) = fence else {
        return Ok(false);
    };
    let live = f.pub_job.as_deref() == Some(job_id)
        && f.run_article.as_deref() == Some(article_id)
        && matches!(f.run_state.as_deref(), Some("active" | "completed" | "failed"))
        && f.run_generation.is_some()
        && f.newest.unwrap_or(i64::MIN) <= f.run_generation.unwrap_or(i64::MIN);
    if !live {
        return Ok(false);
    }
    let list = match kind {
        RetainedKind::Claim => "claims",
        RetainedKind::Inference => "inferences",
    };
    let parsed: Value = f
        .list
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(Value::Null);
    Ok(parsed[list]
        .as_array()
        .into_iter()
        .flatten()
        .any(|c| c["id"] == *local_id && c["text"] == *text))
}
