//! Bounded human review import and mapping approval. Immutable observations,
//! source hashes and permanent story partitions remain the authority.
use std::collections::{BTreeMap, BTreeSet};

use chrono::Utc;
use surrealdb::types::SurrealValue;

use super::{classifier_calibration, jobs};
use crate::classifier::provider_labels::{self, MappingApproval, RuleValidation};
use crate::classifier::review::{
    ReviewDecision, ReviewExchange, ReviewItem, MAX_REVIEW_BYTES, MAX_REVIEW_ITEMS, REVIEW_SCHEMA,
};
use crate::classifier::training::{FamilyId, Split};
use crate::storage::repo::Repository;
use crate::storage::tx::{run_tx, RetryPolicy, Tx, TxError};

/// Maximum completed news cycles in one reviewed BERT recalibration cohort.
pub const MAX_RECALIBRATION_CYCLES: usize = 8;
const MAPPING_OBSERVATION_PAGE_SIZE: usize = 32;

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

fn valid_cycle_id(run: &str) -> bool {
    !run.is_empty() && run.len() <= 512 && !run.chars().any(char::is_control)
}

/// Capture the same bounded origin population as the review export, at the
/// latest completed monitoring watermark. Import and scoring both recheck it.
async fn completed_cycle_origins(tx: &Tx, cycle: &str) -> Result<Vec<String>, TxError> {
    #[derive(SurrealValue)]
    struct Completed {
        watermark: i64,
        report_json: Option<String>,
    }
    let mut result = tx
        .query(
            "SELECT watermark, IF bytes::len(type::bytes(report_json)) <= 16384
                { report_json } ELSE { NONE } AS report_json
             FROM classifier_calibration_job WHERE cycle_id = $cycle
             ORDER BY watermark DESC LIMIT 1;",
        )
        .bind(("cycle", cycle.to_owned()))
        .await?
        .check()?;
    let latest: Vec<Completed> = result.take(0)?;
    let latest = latest
        .first()
        .ok_or_else(|| invalid("review_cycle_monitoring_missing"))?;
    let report: classifier_calibration::CalibrationReport = serde_json::from_str(
        latest
            .report_json
            .as_deref()
            .ok_or_else(|| invalid("review_cycle_monitoring_bound"))?,
    )
    .map_err(|_| invalid("review_cycle_monitoring_corrupt"))?;
    if report.cycle_id != cycle
        || report.state != "completed"
        || u64::try_from(latest.watermark).ok() != Some(report.watermark)
    {
        return Err(invalid("review_cycle_monitoring_not_complete"));
    }
    let mut origins = tx
        .query(
            "SELECT VALUE observation_id FROM classifier_observation_origin
             WHERE cycle_id = $cycle AND sequence <= $watermark
             ORDER BY sequence LIMIT 512;",
        )
        .bind(("cycle", cycle.to_owned()))
        .bind(("watermark", latest.watermark))
        .await?
        .check()?;
    Ok(origins.take(0)?)
}

fn validated_cycles(mut cycles: Vec<String>) -> anyhow::Result<Vec<String>> {
    if cycles.is_empty()
        || cycles.len() > MAX_RECALIBRATION_CYCLES
        || !cycles.iter().all(|cycle| valid_cycle_id(cycle))
    {
        anyhow::bail!("invalid_recalibration_cycles");
    }
    cycles.sort_unstable();
    if cycles.windows(2).any(|pair| pair[0] == pair[1]) {
        anyhow::bail!("duplicate_recalibration_cycle");
    }
    Ok(cycles)
}

async fn completed_observation_ids(
    tx: &Tx,
    cycles: &[String],
) -> Result<BTreeSet<String>, TxError> {
    let mut ids = BTreeSet::new();
    for cycle in cycles {
        ids.extend(completed_cycle_origins(tx, cycle).await?);
    }
    Ok(ids)
}

/// Deterministic bounded monitoring sample. Provider/category/model choices
/// remain hints: the export starts unreviewed and explicitly discloses bias.
pub async fn export(repo: &Repository, run: &str) -> anyhow::Result<ReviewExchange> {
    if !valid_cycle_id(run) {
        anyhow::bail!("invalid_review_run");
    }
    let mut response=repo.db().query("SELECT payload_json,payload_sha256,family_id,(SELECT VALUE split FROM ONLY type::record('classifier_story_family',$parent.family_id)) AS split,((SELECT VALUE revision FROM ONLY type::record('classifier_review',record::id($parent.id))) ?? 0)+1 AS revision FROM classifier_observation WHERE record::id(id) IN (SELECT VALUE observation_id FROM classifier_observation_origin WHERE cycle_id=$run ORDER BY sequence LIMIT 512) AND family_id!='' AND bytes::len(type::bytes(payload_json))<=131072 ORDER BY family_id LIMIT 512;")
        .bind(("run",run.to_owned())).await?.check()?;
    let rows: Vec<ExportRow> = response.take(0)?;
    export_rows(run, rows)
}

/// Export unreviewed examples from a completed cycle for operator labeling.
/// Neither provider tags nor saved model selections fill in ground truth.
pub async fn export_completed_cycle(
    repo: &Repository,
    run: &str,
) -> anyhow::Result<ReviewExchange> {
    if !valid_cycle_id(run) {
        anyhow::bail!("invalid_review_run");
    }
    let cycle = run.to_owned();
    let read = run_tx(repo.db(), RetryPolicy::default(), move |tx| {
        let cycle = cycle.clone();
        async move {
            let ids: Vec<_> = completed_cycle_origins(&tx, &cycle)
                .await?
                .into_iter()
                .map(|id| jobs::rid("classifier_observation", &id))
                .collect();
            let mut response = tx
                .query(
                    "SELECT payload_json,payload_sha256,family_id,
                     (SELECT VALUE split FROM ONLY type::record('classifier_story_family', $parent.family_id)) AS split,
                     ((SELECT VALUE revision FROM ONLY type::record('classifier_review', record::id($parent.id))) ?? 0)+1 AS revision
                     FROM $observations
                     WHERE family_id != '' AND bytes::len(type::bytes(payload_json)) <= 131072
                     ORDER BY family_id LIMIT 512;",
                )
                .bind(("observations", ids))
                .await?
                .check()?;
            Ok::<Vec<ExportRow>, TxError>(response.take(0)?)
        }
    })
    .await?;
    export_rows(run, read.value)
}

fn export_rows(run: &str, rows: Vec<ExportRow>) -> anyhow::Result<ReviewExchange> {
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
    if serde_json::to_vec(&exchange)?.len() > MAX_REVIEW_BYTES {
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
    if !valid_cycle_id(&exchange.run_id) {
        anyhow::bail!("invalid_review_run");
    }
    let _writer = review_writer(home).await?;
    exchange.validate(reference)?;
    let corpus = exchange.sha256()?;
    let reference = reference.to_owned();
    let exchange = exchange.clone();
    let committed=run_tx(repo.db(),RetryPolicy::default(),move|tx| {
        let (exchange,reference,corpus)=(exchange.clone(),reference.clone(),corpus.clone());async move {
            let mut imported=0;
            let mut development_changed=false;
            let origins: BTreeSet<_> = completed_cycle_origins(&tx, &exchange.run_id).await?.into_iter().collect();
            for item in exchange.items {
                if item.decision==ReviewDecision::Unreviewed {continue;}
                if !origins.contains(&item.observation.observation_id) {
                    return Err(invalid("review_observation_outside_completed_cycle"));
                }
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

#[derive(SurrealValue)]
struct RecalibrationRow {
    observation_id: String,
    family_id: String,
    split: String,
    payload_sha256: String,
    review_reference: String,
    corpus_sha256: String,
    revision: i64,
    item_json: Option<String>,
    source_valid: bool,
}

/// Freeze the current complete human labels for the selected completed cycles.
/// Calibration and Test families remain disjoint. Observations, review
/// provenance, and family partitions are revalidated in the same read snapshot.
pub async fn recalibration_cohort(
    repo: &Repository,
    cycles: Vec<String>,
) -> anyhow::Result<ReviewExchange> {
    let cycles = validated_cycles(cycles)?;
    let requested_cycles = cycles.clone();
    let read = run_tx(repo.db(), RetryPolicy::default(), move |tx| {
        let cycles = requested_cycles.clone();
        async move {
            let reviews: Vec<_> = completed_observation_ids(&tx, &cycles)
                .await?
                .into_iter()
                .map(|id| jobs::rid("classifier_review", &id))
                .collect();
            let mut response = tx
                .query(
                    "SELECT observation_id,family_id,split,payload_sha256,review_reference,corpus_sha256,revision,
                     IF bytes::len(type::bytes(item_json)) <= 131072 { item_json } ELSE { NONE } AS item_json,
                     ((SELECT VALUE payload_sha256 FROM ONLY type::record('classifier_observation', $parent.observation_id)) = payload_sha256
                     AND (SELECT VALUE family_id FROM ONLY type::record('classifier_observation', $parent.observation_id)) = family_id
                     AND (SELECT VALUE split FROM ONLY type::record('classifier_story_family', $parent.family_id)) = split) AS source_valid
                     FROM $reviews WHERE decision = 'complete' AND split IN ['calibration', 'test']
                     ORDER BY family_id, observation_id LIMIT $limit;",
                )
                .bind(("reviews", reviews))
                .bind(("limit", (MAX_REVIEW_ITEMS + 1) as i64))
                .await?
                .check()?;
            Ok::<Vec<RecalibrationRow>, TxError>(response.take(0)?)
        }
    })
    .await?;
    let rows = read.value;
    if rows.len() > MAX_REVIEW_ITEMS {
        anyhow::bail!("recalibration_review_item_bound");
    }
    let mut byte_budget = MAX_REVIEW_BYTES;
    let mut families = BTreeMap::<FamilyId, ReviewItem>::new();
    let mut provenance = Vec::new();
    for row in rows {
        let body = row
            .item_json
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("recalibration_review_row_bound"))?;
        byte_budget = byte_budget
            .checked_sub(body.len())
            .ok_or_else(|| anyhow::anyhow!("recalibration_review_byte_bound"))?;
        let item: ReviewItem = serde_json::from_str(body)?;
        if !row.source_valid
            || item.observation.observation_id != row.observation_id
            || item.family_id.as_str() != row.family_id
            || item.split.as_str() != row.split
            || item.payload_sha256 != row.payload_sha256
            || i64::try_from(item.revision).ok() != Some(row.revision)
            || item.decision != ReviewDecision::Complete
            || !matches!(item.split, Split::Calibration | Split::Test)
            || row.review_reference.trim().is_empty()
            || row.review_reference.len() > 1024
            || row.review_reference.chars().any(char::is_control)
            || !valid_sha256(&row.corpus_sha256)
        {
            anyhow::bail!("recalibration_review_source_or_revision_changed");
        }
        if let Some(prior) = families.get(&item.family_id) {
            if prior.split != item.split || prior.membership() != item.membership() {
                anyhow::bail!("recalibration_conflicting_family_reviews");
            }
        } else {
            provenance.push((
                row.observation_id,
                row.corpus_sha256,
                row.review_reference,
                row.revision,
            ));
            families.insert(item.family_id.clone(), item);
        }
    }
    let items: Vec<_> = families.into_values().collect();
    for split in [Split::Calibration, Split::Test] {
        if crate::atlas_taxonomy::DomainId::ALL.iter().any(|domain| {
            !items
                .iter()
                .any(|item| item.split == split && item.domains.contains(domain))
        }) {
            anyhow::bail!("recalibration_requires_all_six_reviewed_domains_in_each_split");
        }
    }
    let cohort_id =
        crate::classifier::artifact::sha256_hex(&serde_json::to_vec(&(cycles, provenance))?);
    let exchange = ReviewExchange {
        schema_version: REVIEW_SCHEMA.into(),
        run_id: format!("reviewed-news-cycles:{cohort_id}"),
        sampling_bias: "Complete human reviews of bounded completed-cycle exports, one representative per permanent story family. Calibration and Test are disjoint.".into(),
        items,
    };
    exchange.validate("persisted completed-cycle reviews")?;
    Ok(exchange)
}

#[derive(SurrealValue)]
struct MappingObservation {
    observation_id: String,
    payload_json: String,
    payload_sha256: String,
    family_id: String,
    split: Option<String>,
}

/// Freeze a bounded cohort for assessment against the operator-reviewed
/// mapping predicates. The exchange retains immutable source observations
/// with empty labels. Existing per-article reviews do not enter this cohort.
///
/// # Errors
/// Returns an error when the cycles are invalid or unsettled, the captured
/// source is corrupt, or the eligible cohort exceeds the review exchange limits.
pub async fn mapping_cohort(
    repo: &Repository,
    cycles: Vec<String>,
) -> anyhow::Result<ReviewExchange> {
    let cycles = validated_cycles(cycles)?;
    let requested_cycles = cycles.clone();
    let read = run_tx(repo.db(), RetryPolicy::default(), move |tx| {
        let cycles = requested_cycles.clone();
        async move {
            let ids: Vec<_> = completed_observation_ids(&tx, &cycles).await?.into_iter().collect();
            let mut families = BTreeMap::new();
            let mut bytes = MAX_REVIEW_BYTES;
            // Ascending observation identity picks the first eligible
            // observation per permanent story family, across all cycles.
            for page in ids.chunks(MAPPING_OBSERVATION_PAGE_SIZE) {
                let records: Vec<_> = page
                    .iter()
                    .map(|id| jobs::rid("classifier_observation", id))
                    .collect();
                let mut response = tx
                    .query(
                        "SELECT record::id(id) AS observation_id,payload_json,payload_sha256,family_id,
                         (SELECT VALUE split FROM ONLY type::record('classifier_story_family', $parent.family_id)) AS split
                         FROM $observations
                         WHERE family_id != '' AND bytes::len(type::bytes(payload_json)) <= $observation_limit
                         ORDER BY observation_id;",
                    )
                    .bind(("observations", records))
                    .bind(("observation_limit", provider_labels::MAX_OBSERVATION_BYTES as i64))
                    .await?
                    .check()?;
                let rows: Vec<MappingObservation> = response.take(0)?;
                for row in rows {
                    let split = match row.split.as_deref() {
                        Some("calibration") => Split::Calibration,
                        Some("test") => Split::Test,
                        _ => continue,
                    };
                    let family_id = FamilyId::parse(&row.family_id)
                        .map_err(|_| invalid("mapping_cohort_family_invalid"))?;
                    let std::collections::btree_map::Entry::Vacant(slot) =
                        families.entry(family_id.clone())
                    else {
                        continue;
                    };
                    let observation: provider_labels::ProviderObservation =
                        serde_json::from_str(&row.payload_json)
                            .map_err(|_| invalid("mapping_cohort_observation_corrupt"))?;
                    if observation.observation_id != row.observation_id {
                        return Err(invalid("mapping_cohort_observation_identity"));
                    }
                    let reference = provider_labels::mapping_agreement_reference(&observation)
                        .map_err(|_| invalid("mapping_cohort_observation_bound"))?;
                    if reference.is_none() {
                        continue;
                    }
                    bytes = bytes.checked_sub(row.payload_json.len())
                        .ok_or_else(|| invalid("mapping_cohort_byte_bound"))?;
                    slot.insert(ReviewItem {
                        observation,
                        payload_sha256: row.payload_sha256,
                        family_id,
                        split,
                        revision: 1,
                        decision: ReviewDecision::Unreviewed,
                        domains: Vec::new(),
                        review_note: String::new(),
                    });
                    if families.len() > MAX_REVIEW_ITEMS {
                        return Err(invalid("mapping_cohort_item_bound"));
                    }
                }
            }
            Ok::<_, TxError>(families.into_values().collect::<Vec<_>>())
        }
    })
    .await?;
    let cycle_selection = crate::classifier::artifact::sha256_hex(&serde_json::to_vec(&cycles)?);
    let exchange = ReviewExchange {
        schema_version: REVIEW_SCHEMA.into(),
        run_id: format!("provider-mapping-news-cycles:{cycle_selection}"),
        sampling_bias: "Bounded earliest captured origins from the selected completed cycles. One supported provider-mapped observation per permanent Calibration or Test family. Provider and mapping coverage bias this cohort.".into(),
        items: read.value,
    };
    exchange.validate("captured provider mapping cohort")?;
    Ok(exchange)
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
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
