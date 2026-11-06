//! Explicit bounded review diagnostics. Saved receipts and exact-pin replay
//! remain separate records; model behavior never fills human review labels.
use super::repo::Repository;
use crate::classifier::readiness::ClassifierHome;
use crate::classifier::review::{ReviewExchange, MAX_REVIEW_BYTES, MAX_REVIEW_ITEMS};
use crate::classifier::{ClassificationReceipt, ClassifyInput, ModelSnapshot, WorkControl};
use serde::Serialize;
use std::collections::BTreeMap;
use surrealdb::types::SurrealValue;

#[derive(Serialize)]
pub struct ReviewExport {
    #[serde(flatten)]
    pub review: ReviewExchange,
    pub diagnostics: RunDiagnostics,
}
#[derive(Serialize)]
pub struct RunDiagnostics {
    pub schema_version: String,
    pub run_id: String,
    pub interpretation: String,
    pub pin_state: String,
    pub model: Option<ModelSnapshot>,
    pub manifest_sha256: Option<String>,
    pub committed_receipts: Vec<CommittedReceipt>,
    pub legacy_receipts: Vec<serde_json::Value>,
    pub explanations_available: bool,
    pub receipt_cohort_truncated: bool,
    pub replays: Vec<Replay>,
    pub duplicate_review_leads: Vec<DuplicateLead>,
}
#[derive(Serialize)]
pub struct CommittedReceipt {
    pub receipt_key_sha256: String,
    pub source_revision: String,
    pub input_revision: u64,
    pub artifact_sha256: String,
    pub manifest_sha256: String,
    pub receipt: ClassificationReceipt,
}
#[derive(Serialize)]
pub struct Replay {
    pub observation_id: String,
    pub payload_sha256: String,
    pub ranking_reason: String,
    pub receipt: ClassificationReceipt,
}
#[derive(Serialize)]
pub struct DuplicateLead {
    pub observation_id: String,
    pub candidate_family_id: String,
    pub candidate_payload_sha256: String,
    pub source_family_id: String,
    pub cross_partition: bool,
    pub method: String,
    pub dice: Option<f64>,
    pub jaro_winkler: Option<f64>,
    pub decision: String,
}
fn bounded(value: &impl Serialize) -> anyhow::Result<()> {
    if serde_json::to_vec(value)?.len() > MAX_REVIEW_BYTES {
        anyhow::bail!("review_diagnostics_byte_bound");
    }
    Ok(())
}
/// Returns at most512 bounded sources and their model/lexical review hints.
pub async fn export(
    repo: &Repository,
    home: &ClassifierHome,
    run: &str,
    limit: usize,
) -> anyhow::Result<ReviewExport> {
    if limit == 0 || limit > MAX_REVIEW_ITEMS {
        anyhow::bail!("review_limit_must_be_1_to_512");
    }
    let mut review = super::classifier_review::export(repo, run).await?;
    let mut diagnostics = diagnostics(repo, home, &review, limit).await?;
    let mut strata: BTreeMap<String, Vec<_>> = BTreeMap::new();
    for item in review.items.drain(..) {
        let replay = diagnostics
            .replays
            .iter()
            .find(|replay| replay.observation_id == item.observation.observation_id);
        let outcome = replay
            .map(|replay| {
                format!(
                    "{}:{}:{}",
                    replay.receipt.is_admitted(),
                    replay.ranking_reason,
                    replay
                        .receipt
                        .selected_domains
                        .iter()
                        .map(|id| id.as_str())
                        .collect::<Vec<_>>()
                        .join("+")
                )
            })
            .unwrap_or_else(|| "unavailable".into());
        strata
            .entry(format!(
                "{}:{outcome}",
                item.observation.publisher.as_deref().unwrap_or("unknown")
            ))
            .or_default()
            .push(item);
    }
    while review.items.len() < limit {
        let mut added = false;
        for group in strata.values_mut() {
            if let Some(item) = group.pop() {
                review.items.push(item);
                added = true;
                if review.items.len() == limit {
                    break;
                }
            }
        }
        if !added {
            break;
        }
    }
    diagnostics.replays.retain(|replay| {
        review
            .items
            .iter()
            .any(|item| item.observation.observation_id == replay.observation_id)
    });
    if let (Some(snapshot), Some(manifest)) = (&diagnostics.model, &diagnostics.manifest_sha256) {
        if let Ok(pin) = home.pin(Some(snapshot)).await {
            if &pin.manifest_sha256 == manifest {
                let items = review.items.clone();
                if let Ok(Ok(replays)) =
                    tokio::task::spawn_blocking(move || replay(&pin, &items, true)).await
                {
                    diagnostics.replays = replays;
                    diagnostics.explanations_available = true;
                }
            }
        }
    }
    diagnostics.duplicate_review_leads = duplicate_leads(repo, &review, limit).await?;
    review.sampling_bias.push_str(&format!(" Export selects at most {limit} items by publisher and saved-pin admission/domain/tie/OOV strata where replay is available. Missing strata are absent; this is not a prevalence sample. Diagnostic replay may cover additional fetched items; complete labels remain unreviewed."));
    let result = ReviewExport {
        review,
        diagnostics,
    };
    bounded(&result)?;
    Ok(result)
}
/// Committed bounded run receipts plus clearly identified exact-pin replays.
pub async fn diagnostics(
    repo: &Repository,
    home: &ClassifierHome,
    review: &ReviewExchange,
    limit: usize,
) -> anyhow::Result<RunDiagnostics> {
    if limit == 0 || limit > MAX_REVIEW_ITEMS {
        anyhow::bail!("review_limit_must_be_1_to_512");
    }
    #[derive(SurrealValue)]
    struct Row {
        key: String,
        receipt_json: Option<String>,
        receipt_sha256: String,
    }
    let mut response=repo.db().query("SELECT record::id(id) AS key,IF bytes::len(type::bytes(receipt_json))<=131072 {receipt_json} ELSE {NONE} AS receipt_json,receipt_sha256 FROM atlas_classification_receipt WHERE run_id=$run ORDER BY article_id,input_revision LIMIT $limit;")
        .bind(("run",review.run_id.clone())).bind(("limit",(limit+1) as i64)).await?.check()?;
    let mut rows: Vec<Row> = response.take(0)?;
    let truncated = rows.len() > limit;
    rows.truncate(limit);
    let mut committed = Vec::new();
    let mut legacy = Vec::new();
    for row in rows {
        let Some(json) = row.receipt_json else {
            anyhow::bail!("run_receipt_export_bound");
        };
        if crate::classifier::artifact::sha256_hex(json.as_bytes()) != row.receipt_sha256 {
            anyhow::bail!("run_receipt_export_integrity");
        }
        let raw: serde_json::Value = serde_json::from_str(&json)?;
        if let crate::classifier::DecodedReceipt::LegacyExactTwo(receipt) =
            crate::classifier::decode_receipt(&serde_json::to_string(&raw["receipt"])?)?
        {
            legacy.push(serde_json::json!({"receipt_key_sha256":crate::classifier::artifact::sha256_hex(row.key.as_bytes()),"authority":"historical_exact_two","key":raw["key"],"receipt":receipt}));
            continue;
        }
        let receipt: crate::atlas_classify::StoredReceipt = serde_json::from_str(&json)?;
        receipt.validate()?;
        if receipt.key.record_id() != row.key || receipt.key.occurrence.run_id != review.run_id {
            anyhow::bail!("run_receipt_export_identity");
        }
        committed.push(CommittedReceipt {
            receipt_key_sha256: crate::classifier::artifact::sha256_hex(row.key.as_bytes()),
            source_revision: receipt.key.source_revision,
            input_revision: receipt.key.input_revision,
            artifact_sha256: receipt.key.artifact_sha256,
            manifest_sha256: receipt.key.manifest_sha256,
            receipt: receipt.receipt,
        });
    }
    #[derive(SurrealValue)]
    struct Saved {
        pin_json: String,
        manifest_sha256: String,
    }
    let mut response = repo
        .db()
        .query("SELECT pin_json,manifest_sha256 FROM ONLY $run;")
        .bind((
            "run",
            super::jobs::rid("atlas_classifier_run", &review.run_id),
        ))
        .await?
        .check()?;
    let saved: Option<Saved> = response.take(0)?;
    let mut result=RunDiagnostics {schema_version:"classifier-run-diagnostics-v1".into(),run_id:review.run_id.clone(),interpretation:"Committed receipts describe saved admission. Replays describe model behavior under the saved collection pin; reviewers supply independent complete memberships. Lexical leads require human review and preserve existing family ownership/partitions.".into(),pin_state:"missing_saved_run_pin".into(),model:None,manifest_sha256:None,committed_receipts:committed,legacy_receipts:legacy,explanations_available:false,receipt_cohort_truncated:truncated,replays:Vec::new(),duplicate_review_leads:Vec::new()};
    if let Some(saved) = saved {
        if let Ok(snapshot) = serde_json::from_str::<ModelSnapshot>(&saved.pin_json) {
            result.model = Some(snapshot.clone());
            result.manifest_sha256 = Some(saved.manifest_sha256.clone());
            match home.pin(Some(&snapshot)).await {
                Ok(pin) if pin.manifest_sha256 == saved.manifest_sha256 => {
                    let items = review.items.clone();
                    let replays =
                        tokio::task::spawn_blocking(move || replay(&pin, &items, false)).await?;
                    match replays {
                        Ok(replays) => {
                            result.replays = replays;
                            result.pin_state = "verified_saved_pin".into();
                        }
                        Err(_) => result.pin_state = "saved_pin_replay_unavailable".into(),
                    }
                }
                _ => result.pin_state = "saved_pin_unavailable_or_changed".into(),
            }
        }
    }

    bounded(&result)?;
    Ok(result)
}
fn replay(
    pin: &crate::classifier::readiness::PinnedClassifier,
    items: &[crate::classifier::review::ReviewItem],
    explain: bool,
) -> Result<Vec<Replay>, crate::classifier::ClassifierError> {
    let mut replays = Vec::new();
    let selected: Vec<_> = items
        .iter()
        .filter(|item| item.split != crate::classifier::training::Split::Test)
        .collect();
    for batch in selected.chunks(32) {
        let inputs: Vec<_> = batch
            .iter()
            .map(|item| {
                let mut input = ClassifyInput::new(
                    &item.observation.observation_id,
                    &item.observation.title,
                    &item.observation.description,
                    item.observation.language.clone(),
                );
                input.explain = explain;
                input
            })
            .collect();
        let receipts = pin.service.classify_batch(inputs)?;
        if receipts.len() != batch.len() {
            return Err(crate::classifier::ClassifierError::WorkerStopped);
        }
        for (item, receipt) in batch.iter().zip(receipts) {
            replays.push(Replay {
                observation_id: item.observation.observation_id.clone(),
                payload_sha256: item.payload_sha256.clone(),
                ranking_reason: receipt.ranking_reason().into(),
                receipt,
            });
        }
    }
    Ok(replays)
}

fn excerpt(observation: &crate::classifier::ProviderObservation) -> String {
    let joined = format!("{} {}", observation.title, observation.description);
    let mut end = joined.len().min(1024);
    while !joined.is_char_boundary(end) {
        end -= 1;
    }
    joined[..end].into()
}
async fn duplicate_leads(
    repo: &Repository,
    review: &ReviewExchange,
    limit: usize,
) -> anyhow::Result<Vec<DuplicateLead>> {
    #[derive(SurrealValue)]
    struct Candidate {
        family_id: String,
        split: String,
        content_sha256: String,
        url_sha256: Option<String>,
        payloads: Vec<String>,
    }
    let mut leads = Vec::new();
    let mut pairs = Vec::new();
    let mut pair_indices = Vec::new();
    for item in review.items.iter().take(limit) {
        let signature = crate::atlas_calibration::family_signature(
            &item.observation,
            &std::sync::atomic::AtomicBool::new(false),
        )
        .map_err(|_| anyhow::anyhow!("review_family_signature_unavailable"))?;
        let projection="record::id(id) AS family_id,split,content_sha256,url_sha256,(SELECT VALUE payload_json FROM classifier_observation WITH INDEX classifier_observation_family WHERE family_id=record::id($parent.id) AND bytes::len(type::bytes(payload_json))<=131072 ORDER BY first_seen LIMIT 1) AS payloads";
        let mut sql=format!("SELECT {projection} FROM classifier_story_family WITH INDEX classifier_family_content WHERE content_sha256=$content LIMIT 9; SELECT {projection} FROM classifier_story_family WITH INDEX classifier_family_url WHERE url_sha256=$url AND $url!=NONE LIMIT 9;");
        for index in 0..8 {
            sql.push_str(&format!("SELECT {projection} FROM classifier_story_family WITH INDEX classifier_family_bands WHERE bands CONTAINS $band{index} LIMIT 9;"));
        }
        let mut query = repo
            .db()
            .query(sql)
            .bind(("content", signature.content_sha256.clone()))
            .bind(("url", signature.url_sha256.clone()));
        for (index, band) in signature.bands.iter().enumerate() {
            query = query.bind((format!("band{index}"), band.clone()));
        }
        let mut response = query.await?.check()?;
        let mut candidates = BTreeMap::new();
        for index in 0..10 {
            for candidate in response.take::<Vec<Candidate>>(index)? {
                if candidate.family_id != item.family_id.as_str() {
                    candidates
                        .entry(candidate.family_id.clone())
                        .or_insert(candidate);
                }
            }
        }
        let mut candidates: Vec<_> = candidates.into_values().collect();
        let exact = |candidate: &Candidate| {
            signature.content_sha256 == candidate.content_sha256
                || signature
                    .url_sha256
                    .as_ref()
                    .is_some_and(|hash| Some(hash) == candidate.url_sha256.as_ref())
        };
        candidates.sort_by(|a, b| {
            exact(b)
                .cmp(&exact(a))
                .then_with(|| a.family_id.cmp(&b.family_id))
        });
        for candidate in candidates.into_iter().take(8) {
            let Some(json) = candidate.payloads.first() else {
                continue;
            };
            let observation: crate::classifier::ProviderObservation = serde_json::from_str(json)?;
            let exact = signature.content_sha256 == candidate.content_sha256
                || signature
                    .url_sha256
                    .as_ref()
                    .is_some_and(|hash| Some(hash) == candidate.url_sha256.as_ref());
            let index = leads.len();
            leads.push(DuplicateLead {
                observation_id: item.observation.observation_id.clone(),
                candidate_family_id: candidate.family_id,
                source_family_id: item.family_id.to_string(),
                candidate_payload_sha256: crate::classifier::artifact::sha256_hex(json.as_bytes()),
                cross_partition: item.split.as_str() != candidate.split,
                method: if exact {
                    "exact_url_or_content"
                } else {
                    "natural_dice_jaro_winkler"
                }
                .into(),
                dice: None,
                jaro_winkler: None,
                decision: "independent_review_required; permanent partitions unchanged".into(),
            });
            if !exact {
                pairs.push(crate::classifier::similarity::Pair {
                    left: excerpt(&item.observation),
                    right: excerpt(&observation),
                });
                pair_indices.push(index);
            }
        }
    }
    if !pairs.is_empty() {
        let scores = tokio::task::spawn_blocking(move || {
            crate::classifier::similarity::compare(&pairs, &WorkControl::new())
        })
        .await??;
        for (index, score) in pair_indices.into_iter().zip(scores) {
            leads[index].dice = Some(score.dice);
            leads[index].jaro_winkler = Some(score.jaro_winkler);
        }
    }
    leads.retain(|lead| {
        lead.method == "exact_url_or_content"
            || lead.dice.is_some_and(|n| n >= 0.8) && lead.jaro_winkler.is_some_and(|n| n >= 0.9)
    });
    Ok(leads)
}
