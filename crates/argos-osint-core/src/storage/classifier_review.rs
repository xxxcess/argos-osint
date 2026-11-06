//! Bounded human review import and mapping approval. Immutable observations,
//! source hashes and permanent story partitions remain the authority.
use super::jobs;
use crate::classifier::provider_labels::{self, MappingApproval, RuleValidation};
use crate::classifier::review::{
    ReviewDecision, ReviewExchange, ReviewItem, MAX_REVIEW_ITEMS, REVIEW_SCHEMA,
};
use crate::classifier::training::{FamilyId, Split};
use crate::storage::repo::Repository;
use crate::storage::tx::{run_tx, RetryPolicy, TxError};
use chrono::Utc;
use std::collections::BTreeMap;
use surrealdb::types::SurrealValue;

pub const SCHEMA: &str = "
DEFINE TABLE IF NOT EXISTS classifier_review SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS observation_id ON classifier_review TYPE string;
DEFINE FIELD IF NOT EXISTS family_id ON classifier_review TYPE string;
DEFINE FIELD IF NOT EXISTS split ON classifier_review TYPE string;
DEFINE FIELD IF NOT EXISTS payload_sha256 ON classifier_review TYPE string;
DEFINE FIELD IF NOT EXISTS review_reference ON classifier_review TYPE string;
DEFINE FIELD IF NOT EXISTS corpus_sha256 ON classifier_review TYPE string;
DEFINE FIELD IF NOT EXISTS revision ON classifier_review TYPE int;
DEFINE FIELD IF NOT EXISTS item_json ON classifier_review TYPE string;
DEFINE FIELD IF NOT EXISTS decision ON classifier_review TYPE string;
DEFINE FIELD IF NOT EXISTS created_at ON classifier_review TYPE datetime;
DEFINE INDEX IF NOT EXISTS classifier_review_partition ON classifier_review FIELDS split,family_id;
DEFINE TABLE IF NOT EXISTS classifier_mapping_approval SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS approval_json ON classifier_mapping_approval TYPE string;
DEFINE FIELD IF NOT EXISTS updated_at ON classifier_mapping_approval TYPE datetime;
";
fn invalid(code: &str) -> TxError {
    TxError::Abort(code.into())
}

#[derive(SurrealValue)]
struct ExportRow {
    payload_json: String,
    payload_sha256: String,
    family_id: String,
    split: String,
    revision: i64,
}

/// Deterministic bounded monitoring sample. Provider/category/model choices
/// remain hints: the export starts unreviewed and explicitly discloses bias.
pub async fn export(repo: &Repository, run: &str) -> anyhow::Result<ReviewExchange> {
    if run.is_empty() || run.len() > 512 || run.chars().any(char::is_control) {
        anyhow::bail!("invalid_review_run");
    }
    let mut response=repo.db().query("SELECT payload_json,payload_sha256,family_id,(SELECT VALUE split FROM ONLY type::record('classifier_story_family',$parent.family_id)) AS split,((SELECT VALUE revision FROM ONLY type::record('classifier_review',record::id($parent.id))) ?? 0)+1 AS revision FROM classifier_observation WHERE record::id(id) IN (SELECT VALUE observation_id FROM classifier_observation_origin WHERE cycle_id=$run ORDER BY sequence LIMIT 512) AND family_id!='' AND bytes::len(type::bytes(payload_json))<=131072 ORDER BY family_id LIMIT 512;")
        .bind(("run",run.to_owned())).await?.check()?;
    let rows: Vec<ExportRow> = response.take(0)?;
    let mut publishers: BTreeMap<String, Vec<ReviewItem>> = BTreeMap::new();
    for row in rows {
        let observation: provider_labels::ProviderObservation =
            serde_json::from_str(&row.payload_json)?;
        let item = ReviewItem {
            observation,
            payload_sha256: row.payload_sha256,
            family_id: FamilyId::parse(&row.family_id)?,
            split: serde_json::from_value(row.split.into())?,
            revision: u64::try_from(row.revision)?,
            decision: ReviewDecision::Unreviewed,
            domains: Vec::new(),
            review_note: String::new(),
        };
        publishers
            .entry(item.observation.publisher.clone().unwrap_or_default())
            .or_default()
            .push(item);
    }
    let mut items = Vec::new();
    // Round-robin publishers within the bounded fetched cohort; no class hints
    // are copied into human ground truth and no whole-corpus scan is performed.
    while items.len() < MAX_REVIEW_ITEMS {
        let mut added = false;
        for group in publishers.values_mut() {
            if let Some(item) = group.pop() {
                items.push(item);
                added = true;
                if items.len() == MAX_REVIEW_ITEMS {
                    break;
                }
            }
        }
        if !added {
            break;
        }
    }
    let exchange=ReviewExchange {schema_version:REVIEW_SCHEMA.into(),run_id:run.into(),sampling_bias:"At most 512 earliest captured origins with settled family assignments; publisher round-robin within that cohort. Provider collection, language, filtering and monitoring availability bias this sample. Uncaptured and unsettled records are absent. Review admitted, rejected, OOV and tied examples independently; no class balance is guaranteed.".into(),items};
    if serde_json::to_vec(&exchange)?.len() > crate::classifier::review::MAX_REVIEW_BYTES {
        anyhow::bail!("review_export_byte_bound");
    }
    Ok(exchange)
}

/// Atomic bounded import. Replays preserve the original review; conflicting or
/// stale revisions roll back the entire exchange, including its provenance.
pub async fn import(
    repo: &Repository,
    home: &crate::classifier::readiness::ClassifierHome,
    exchange: &ReviewExchange,
    reference: &str,
) -> anyhow::Result<usize> {
    let _writer = review_writer(home).await?;
    exchange.validate(reference)?;
    let corpus = exchange.sha256()?;
    let reference = reference.to_owned();
    let exchange = exchange.clone();
    let committed=run_tx(repo.db(),RetryPolicy::default(),move|tx| {
        let (exchange,reference,corpus)=(exchange.clone(),reference.clone(),corpus.clone());async move {
            let mut imported=0;
            let mut development_changed=false;
            for item in exchange.items {
                if item.decision==ReviewDecision::Unreviewed {continue;}
                let id=item.observation.observation_id.clone();jobs::bump(&tx,"classifier_review_guard",&id).await?;
                let mut response=tx.query("RETURN (SELECT VALUE payload_sha256 FROM ONLY $observation)=$payload AND (SELECT VALUE family_id FROM ONLY $observation)=$family AND (SELECT VALUE split FROM ONLY $family_row)=$split; SELECT revision,item_json,review_reference FROM ONLY $review;")
                    .bind(("observation",jobs::rid("classifier_observation",&id))).bind(("payload",item.payload_sha256.clone())).bind(("family",item.family_id.to_string())).bind(("family_row",jobs::rid("classifier_story_family",item.family_id.as_str()))).bind(("split",item.split.as_str())).bind(("review",jobs::rid("classifier_review",&id))).await?.check()?;
                if response.take::<Option<bool>>(0)?!=Some(true) {return Err(invalid("review_source_or_partition_changed"));}
                #[derive(SurrealValue)] struct Prior {revision:i64,item_json:String,review_reference:String}
                let prior:Option<Prior>=response.take(1)?;
                let encoded=serde_json::to_string(&item).map_err(|_|invalid("review_serialization"))?;
                if let Some(prior)=prior {
                    if prior.revision==item.revision as i64 && prior.item_json==encoded && prior.review_reference==reference {continue;}
                    if prior.revision+1!=item.revision as i64 {return Err(invalid("review_revision_conflict"));}
                } else if item.revision!=1 {return Err(invalid("review_revision_conflict"));}
                tx.query("UPSERT $review SET observation_id=$id,family_id=$family,split=$split,payload_sha256=$payload,review_reference=$reference,corpus_sha256=$corpus,revision=$revision,item_json=$item,decision=$decision,created_at=<datetime>$now RETURN NONE;")
                    .bind(("review",jobs::rid("classifier_review",&id))).bind(("id",id)).bind(("family",item.family_id.to_string())).bind(("split",item.split.as_str())).bind(("payload",item.payload_sha256)).bind(("reference",reference.clone())).bind(("corpus",corpus.clone())).bind(("revision",item.revision as i64)).bind(("item",encoded)).bind(("decision",if item.decision==ReviewDecision::Complete {"complete"}else{"abstain"})).bind(("now",Utc::now().to_rfc3339())).await?.check()?;
                development_changed|=item.split==Split::Development;
                imported+=1;
            }
            if development_changed {jobs::bump(&tx,"classifier_mapping_guard","active").await?;tx.query("DELETE classifier_mapping_approval:active RETURN NONE;").await?.check()?;}
            Ok(imported)
        }
    }).await?;
    Ok(committed.value)
}

/// Approval is computed from actual complete reviewed Development families,
/// disjoint from training. No supplied true/false counts are trusted.
pub async fn approve_mapping(
    repo: &Repository,
    home: &crate::classifier::readiness::ClassifierHome,
    reference: &str,
) -> anyhow::Result<MappingApproval> {
    let _writer = review_writer(home).await?;
    if reference.trim().is_empty()
        || reference.len() > 1024
        || reference.chars().any(char::is_control)
    {
        anyhow::bail!("invalid_mapping_review_reference");
    }
    let mut response=repo.db().query("SELECT VALUE item_json FROM classifier_review WHERE split='development' ORDER BY family_id LIMIT 513;").await?.check()?;
    let bodies: Vec<String> = response.take(0)?;
    if bodies.len() > MAX_REVIEW_ITEMS {
        anyhow::bail!("mapping_review_requires_bounded_selection");
    }
    let mut families: BTreeMap<FamilyId, ReviewItem> = BTreeMap::new();
    let mut conflicts = std::collections::BTreeSet::new();
    for body in &bodies {
        let item: ReviewItem = serde_json::from_str(body)?;
        if let Some(prior) = families.get(&item.family_id) {
            if prior.split != item.split || prior.membership() != item.membership() {
                conflicts.insert(item.family_id.clone());
            }
        } else {
            families.insert(item.family_id.clone(), item);
        }
    }
    families.retain(|family, item| {
        !conflicts.contains(family) && item.decision == ReviewDecision::Complete
    });
    let mut domains = crate::atlas_taxonomy::DomainId::ALL.map(|domain| RuleValidation {
        domain,
        true_positive_families: 0,
        false_positive_families: 0,
    });
    for item in families.values() {
        if item.split != Split::Development {
            anyhow::bail!("mapping_evaluation_partition");
        }
        let mapped = provider_labels::map_labels(&item.observation, None)?;
        for (index, rule) in domains.iter_mut().enumerate() {
            if mapped.membership[index] == crate::classifier::training::LabelMembership::Present {
                if item.domains.contains(&rule.domain) {
                    rule.true_positive_families += 1;
                } else {
                    rule.false_positive_families += 1;
                }
            }
        }
    }
    let approval = MappingApproval {
        mapping_version: provider_labels::MAPPING_VERSION.into(),
        predicates_sha256: provider_labels::mapping_predicates_sha256(),
        review_reference: reference.into(),
        reviewed_corpus_sha256: crate::classifier::artifact::sha256_hex(
            serde_json::to_vec(&families)?.as_slice(),
        ),
        domains,
    };
    provider_labels::validate_approval(&approval)?;
    let encoded = serde_json::to_string(&approval)?;
    run_tx(repo.db(),RetryPolicy::default(),move|tx| {let encoded=encoded.clone();async move {jobs::bump(&tx,"classifier_mapping_guard","active").await?;tx.query("UPSERT classifier_mapping_approval:active SET approval_json=$approval,updated_at=time::now() RETURN NONE;").bind(("approval",encoded)).await?.check()?;Ok(())}}).await?;
    Ok(approval)
}

pub(crate) async fn active_approval_in(
    tx: &crate::storage::tx::Tx,
) -> Result<Option<MappingApproval>, TxError> {
    let mut response=tx.query("SELECT VALUE IF bytes::len(type::bytes(approval_json))<=16384 {approval_json} ELSE {NONE} FROM ONLY classifier_mapping_approval:active;").await?.check()?;
    let body: Option<String> = response.take(0)?;
    body.map(|body| {
        let approval: MappingApproval =
            serde_json::from_str(&body).map_err(|_| invalid("mapping_approval_corrupt"))?;
        provider_labels::validate_approval(&approval)
            .map_err(|_| invalid("mapping_approval_invalid"))?;
        Ok(approval)
    })
    .transpose()
}

async fn review_writer(
    home: &crate::classifier::readiness::ClassifierHome,
) -> anyhow::Result<crate::classifier::WriterLease> {
    let path = home.home().to_path_buf();
    Ok(tokio::task::spawn_blocking(move || {
        crate::classifier::artifact::ArtifactStore::new(&path)?.try_writer()
    })
    .await??)
}

/// Revalidates the entire captured non-Test review set and active approval under
/// the artifact writer. Import/approval share that writer through COMMIT, so no
/// reviewed revision can change between this audit and activation/readback.
pub(crate) async fn verify_capture(
    repo: &Repository,
    input: &crate::classifier::learning::LearningInput,
) -> anyhow::Result<()> {
    let expected = input.clone();
    run_tx(repo.db(),RetryPolicy::default(),move |tx| {
        let expected=expected.clone();
        async move {
            let approval=active_approval_in(&tx).await?;
            if approval.as_ref()!=Some(&expected.approval) {return Err(invalid("learning_mapping_capture_changed"));}
            let mut response=tx.query("SELECT item_json,review_reference,corpus_sha256,((SELECT VALUE payload_sha256 FROM ONLY type::record('classifier_observation',$parent.observation_id))=payload_sha256 AND (SELECT VALUE family_id FROM ONLY type::record('classifier_observation',$parent.observation_id))=family_id AND (SELECT VALUE split FROM ONLY type::record('classifier_story_family',$parent.family_id))=split) AS source_valid FROM classifier_review WHERE split!='test' ORDER BY family_id,observation_id LIMIT 1025;").await?.check()?;
            #[derive(SurrealValue)] struct Row {item_json:String,review_reference:String,corpus_sha256:String,source_valid:bool}
            let rows:Vec<Row>=response.take(0)?;
            if rows.len()!=expected.reviews.len() || rows.len()>1024 || rows.iter().any(|row|!row.source_valid) {return Err(invalid("learning_review_capture_changed"));}
            let actual:Vec<_>=rows.into_iter().map(|row|Ok(crate::classifier::learning::ReviewedCapture {item:serde_json::from_str(&row.item_json).map_err(|_|invalid("review_capture_corrupt"))?,review_reference:row.review_reference,corpus_sha256:row.corpus_sha256})).collect::<Result<_,TxError>>()?;
            if serde_json::to_vec(&actual).map_err(|_|invalid("review_capture_json"))?!=serde_json::to_vec(&expected.reviews).map_err(|_|invalid("review_capture_json"))? {return Err(invalid("learning_review_capture_changed"));}
            Ok(())
        }
    }).await?;
    Ok(())
}
