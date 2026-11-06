//! Transactional late-input and policy invalidation. Discovery is keyset-paged;
//! body changes enumerate indexed subscriptions, not all retained runs.
use super::classifier_admission::{self as admission, AdmissionOwner};
use super::jobs;
use super::tx::{RetryPolicy, Tx, TxError};
use crate::atlas_classify::OccurrenceKey;
use chrono::{DateTime, Utc};
use surrealdb::types::SurrealValue;

pub const SCHEMA:&str="
DEFINE TABLE IF NOT EXISTS atlas_derivative_invalidation SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS scope ON atlas_derivative_invalidation TYPE string;
DEFINE FIELD IF NOT EXISTS article_id ON atlas_derivative_invalidation TYPE string;
DEFINE FIELD IF NOT EXISTS revision ON atlas_derivative_invalidation TYPE string;
DEFINE FIELD IF NOT EXISTS after_run ON atlas_derivative_invalidation TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS after_article ON atlas_derivative_invalidation TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS state ON atlas_derivative_invalidation TYPE string DEFAULT 'pending';
DEFINE FIELD IF NOT EXISTS updated_at ON atlas_derivative_invalidation TYPE datetime;
DEFINE INDEX IF NOT EXISTS atlas_derivative_invalidation_pending ON atlas_derivative_invalidation FIELDS state,updated_at;
DEFINE INDEX IF NOT EXISTS atlas_projection_article ON atlas_classification_projection FIELDS article_id,run_id;
DEFINE TABLE IF NOT EXISTS atlas_derivative_policy SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS signature ON atlas_derivative_policy TYPE string;
DEFINE FIELD IF NOT EXISTS epoch ON atlas_derivative_policy TYPE int DEFAULT 0;
DEFINE TABLE IF NOT EXISTS atlas_derivative_catalog SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS epoch ON atlas_derivative_catalog TYPE int DEFAULT 0;
DEFINE INDEX IF NOT EXISTS nlp_source_prose_owner ON nlp_source_ref FIELDS prose_hash,owner_kind,owner_id;
DEFINE TABLE IF NOT EXISTS atlas_derivative_input_head SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS revision ON atlas_derivative_input_head TYPE string;
DEFINE FIELD IF NOT EXISTS generation ON atlas_derivative_input_head TYPE int;
";
fn invalid(code: &str) -> TxError {
    TxError::Abort(code.into())
}
pub(crate) fn policy_signature() -> String {
    crate::classifier::artifact::sha256_hex(
        super::codec::encode(&[
            "atlas-independent-derivative-policy-v2",
            admission::PUBLICATION_VERSION,
            crate::atlas_metadata::ENTITY_VERSION,
            crate::atlas_metadata::PRIOR_VERSION,
            crate::atlas_actor_index::INDEX_CONTRACT,
            crate::brain_actors::IDENTITY_INDEX_VERSION,
            crate::related_articles::MATCH_VERSION,
            super::lifecycle::RECONCILE_VERSION,
        ])
        .as_bytes(),
    )
}
/// Idempotence compares the latest published revision, not all historical
/// revisions: changing A -> B -> A must create a fresh discovery generation.
async fn enqueue_scope_in(
    tx: &Tx,
    scope: &str,
    key: &str,
    revision: &str,
    now: DateTime<Utc>,
) -> Result<(), TxError> {
    if revision.is_empty() || revision.len() > 1024 || key.len() > 512 {
        return Err(invalid("derivative_invalidation_bound"));
    }
    let head =
        crate::classifier::artifact::sha256_hex(super::codec::encode(&[scope, key]).as_bytes());
    jobs::bump(tx, "job_guard", &format!("derivative-input:{head}")).await?;
    let mut response=tx.query("{LET $prior=(SELECT * FROM ONLY $head); IF $prior.revision=$revision {RETURN NONE;}; UPSERT $head SET revision=$revision,generation=($prior.generation ?? 0)+1 RETURN NONE; RETURN (SELECT VALUE generation FROM ONLY $head);};")
        .bind(("head",jobs::rid("atlas_derivative_input_head",&head))).bind(("revision",revision.to_owned())).await?.check()?;
    let Some(generation) = response.take::<Option<i64>>(0)? else {
        return Ok(());
    };
    if generation > 1 {
        let previous = crate::classifier::artifact::sha256_hex(
            super::codec::encode(&[&head, &(generation - 1).to_string()]).as_bytes(),
        );
        tx.query("DELETE $row RETURN NONE;")
            .bind(("row", jobs::rid("atlas_derivative_invalidation", &previous)))
            .await?
            .check()?;
    }
    let id = crate::classifier::artifact::sha256_hex(
        super::codec::encode(&[&head, &generation.to_string()]).as_bytes(),
    );
    tx.query("CREATE $row SET scope=$scope,article_id=$article,revision=$revision,after_run='',after_article='',state='pending',updated_at=<datetime>$now RETURN NONE;")
        .bind(("row",jobs::rid("atlas_derivative_invalidation",&id))).bind(("scope",scope.to_owned())).bind(("article",key.to_owned())).bind(("revision",revision.to_owned())).bind(("now",now.to_rfc3339())).await?.check()?;
    if scope == "catalog" {
        jobs::bump(tx, "job_guard", "atlas-derivative-catalog-input").await?;
        tx.query("UPSERT atlas_derivative_catalog:current SET epoch=(epoch ?? 0)+1 RETURN NONE;")
            .await?
            .check()?;
    }
    Ok(())
}
/// Joins the body/source/catalog publication transaction, never a read guard.
pub(crate) async fn enqueue_in(
    tx: &Tx,
    article: Option<&str>,
    revision: &str,
    now: DateTime<Utc>,
) -> Result<(), TxError> {
    if let Some(article) = article {
        let mut subscribers=tx.query("RETURN count(SELECT id FROM atlas_classification_projection WITH INDEX atlas_projection_article WHERE article_id=$article AND admission_state='admitted' LIMIT 1)>0;").bind(("article",article.to_owned())).await?.check()?;
        if subscribers.take::<Option<bool>>(0)? != Some(true) {
            return Ok(());
        }
    }
    enqueue_scope_in(
        tx,
        if article.is_some() {
            "article"
        } else {
            "catalog"
        },
        article.unwrap_or_default(),
        revision,
        now,
    )
    .await
}
/// NLP feature completion wakes every article subscribed to this prose hash.
/// Enumeration is deferred and bounded, including shared/syndicated bodies.
pub(crate) async fn enqueue_prose_in(
    tx: &Tx,
    prose: &str,
    revision: &str,
    now: DateTime<Utc>,
) -> Result<(), TxError> {
    let mut subscribers=tx.query("RETURN count(SELECT id FROM nlp_source_ref WITH INDEX nlp_source_prose_owner WHERE prose_hash=$prose AND owner_kind='article' LIMIT 1)>0;").bind(("prose",prose.to_owned())).await?.check()?;
    if subscribers.take::<Option<bool>>(0)? != Some(true) {
        return Ok(());
    }
    enqueue_scope_in(tx, "prose", prose, revision, now).await
}
#[derive(SurrealValue)]
struct Event {
    key: String,
    scope: String,
    article_id: String,
    revision: String,
    after_run: String,
    after_article: String,
}
#[derive(SurrealValue)]
struct Occurrence {
    run_id: String,
    article_id: String,
}
/// One discovery page and its durable handoffs/cursor commit atomically. The
/// worker alternates discovery and publication so catalog refresh cannot starve
/// older continuations. Empty pages still consume one bounded activation.
pub(crate) async fn discover_one(
    owner: &AdmissionOwner,
    now: DateTime<Utc>,
) -> Result<usize, TxError> {
    admission::owned_tx(owner,RetryPolicy::default(),move|tx|async move {
        jobs::bump(&tx,"job_guard","atlas-derivative-discovery").await?;
        let signature=policy_signature();
        let mut policy=tx.query("RETURN (SELECT VALUE signature FROM ONLY atlas_derivative_policy:current)=$signature;").bind(("signature",signature.clone())).await?.check()?;
        if policy.take::<Option<bool>>(0)?!=Some(true) {
            tx.query("UPSERT atlas_derivative_policy:current SET signature=$signature,epoch=(epoch ?? 0)+1 RETURN NONE;").bind(("signature",signature.clone())).await?.check()?;
            enqueue_in(&tx,None,&format!("policy:{signature}"),now).await?;
        }
        let mut response=tx.query("SELECT record::id(id) AS key,scope,article_id,revision,after_run,after_article FROM atlas_derivative_invalidation WITH INDEX atlas_derivative_invalidation_pending WHERE state='pending' ORDER BY updated_at LIMIT 1;").await?.check()?;
        let events:Vec<Event>=response.take(0)?;let Some(event)=events.into_iter().next()else{return Ok(0);};
        if event.scope=="prose" {
            #[derive(SurrealValue)] struct Source {owner_id:String}
            let mut response=tx.query("SELECT owner_id FROM nlp_source_ref WITH INDEX nlp_source_prose_owner WHERE prose_hash=$hash AND owner_kind='article' AND owner_id>$after ORDER BY owner_id LIMIT 9;").bind(("hash",event.article_id.clone())).bind(("after",event.after_article.clone())).await?.check()?;
            let mut rows:Vec<Source>=response.take(0)?;let more=rows.len()>8;rows.truncate(8);
            for row in &rows {enqueue_in(&tx,Some(&row.owner_id),&format!("feature:{}:{}",event.key,event.revision),now).await?;}
            checkpoint(&tx,&event,rows.last().map(|row|("",row.owner_id.as_str())),!more,now).await?;
            return Ok(rows.len().max(1));
        }
        let (sql,same_run)=if event.scope=="article" {
            ("SELECT run_id,article_id FROM atlas_classification_projection WITH INDEX atlas_projection_article WHERE article_id=$article AND run_id>$after_run ORDER BY run_id LIMIT 9;",false)
        } else if event.scope=="catalog" {
            ("SELECT run_id,article_id FROM atlas_classification_projection WITH INDEX atlas_projection_run WHERE run_id=$after_run AND article_id>$after_article ORDER BY article_id LIMIT 9;",true)
        } else {return Err(invalid("derivative_invalidation_scope"));};
        let mut response=tx.query(sql).bind(("article",event.article_id.clone())).bind(("after_run",event.after_run.clone())).bind(("after_article",event.after_article.clone())).await?.check()?;
        let mut rows:Vec<Occurrence>=response.take(0)?;
        if same_run && rows.is_empty() {
            let mut response=tx.query("SELECT run_id,article_id FROM atlas_classification_projection WITH INDEX atlas_projection_run WHERE run_id>$after_run ORDER BY run_id,article_id LIMIT 9;").bind(("after_run",event.after_run.clone())).await?.check()?;
            rows=response.take(0)?;
        }
        let more=rows.len()>8;rows.truncate(8);
        let keys:Vec<_>=rows.iter().map(|row|OccurrenceKey::new(&row.run_id,&row.article_id)).collect();
        admission::refresh_handoff_in(&tx,&keys,&format!("{}:{}",event.key,event.revision),now).await?;
        checkpoint(&tx,&event,rows.last().map(|row|(row.run_id.as_str(),row.article_id.as_str())),rows.is_empty()||(!same_run&&!more),now).await?;
        Ok(rows.len().max(1))
    }).await.map(|value|value.value).map_err(|_|invalid("derivative_discovery_failed"))
}
async fn checkpoint(
    tx: &Tx,
    event: &Event,
    last: Option<(&str, &str)>,
    done: bool,
    now: DateTime<Utc>,
) -> Result<(), TxError> {
    if let Some((run, article)) = last {
        tx.query("UPDATE $row SET after_run=$run,after_article=$article,updated_at=<datetime>$now RETURN NONE;").bind(("row",jobs::rid("atlas_derivative_invalidation",&event.key))).bind(("run",run.to_owned())).bind(("article",article.to_owned())).bind(("now",now.to_rfc3339())).await?.check()?;
    }
    if done {
        tx.query("DELETE $row RETURN NONE;")
            .bind((
                "row",
                jobs::rid("atlas_derivative_invalidation", &event.key),
            ))
            .await?
            .check()?;
    }
    Ok(())
}
