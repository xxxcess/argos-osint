//! Immutable derivation history and transaction-scoped publication fencing.
use anyhow::Result;

use crate::tasks::ClaimedTask;

use super::insights::PipelineInput;

pub fn article_details_eligible(input: &PipelineInput) -> bool {
    input.required_tasks_complete
        && input.evidence.iter().any(|e| {
            e.reference.source_id.starts_with("body:")
                && e.substantive
                && e.complete_story
                && !e.known_truncation
        })
}

/// Why a new generation was needed, comparing the prior task's input with its
/// own captured settings against the new input with current settings.
pub fn supersession_reason(
    prior: &PipelineInput,
    prior_settings: &crate::provider::SettingsFile,
    next: &PipelineInput,
    settings: &crate::provider::SettingsFile,
) -> (String, serde_json::Value) {
    if prior.surface != next.surface && next.surface == "recon_report" {
        return (
            "completed report replaces article details".into(),
            serde_json::json!({"from":prior.surface,"to":next.surface}),
        );
    }
    let mut a = prior.snapshot.clone();
    let mut b = next.snapshot.clone();
    a.key.clear();
    b.key.clear();
    a.generation = 0;
    b.generation = 0;
    let sa = serde_json::to_value(&a).unwrap_or_default();
    let sb = serde_json::to_value(&b).unwrap_or_default();
    let (ea, eb) = (
        serde_json::to_value(&prior.evidence).unwrap_or_default(),
        serde_json::to_value(&next.evidence).unwrap_or_default(),
    );
    if sa != sb || ea != eb {
        let changed: Vec<String> = sb
            .as_object()
            .map(|m| {
                m.iter()
                    .filter(|(k, v)| sa.get(k.as_str()) != Some(v))
                    .map(|(k, _)| k.clone())
                    .collect()
            })
            .unwrap_or_default();
        return (
            "source changed".into(),
            serde_json::json!({"fields":changed,"evidence_changed":ea!=eb}),
        );
    }
    let roles = |s: &crate::provider::SettingsFile| {
        serde_json::to_value(
            super::insights::INSIGHT_ROLES
                .iter()
                .map(|r| (r.to_string(), s.defaults.resolve_role(r).0))
                .collect::<std::collections::BTreeMap<_, _>>(),
        )
        .unwrap_or_default()
    };
    let (ra, rb) = (roles(prior_settings), roles(settings));
    if ra != rb {
        let changed: Vec<String> = rb
            .as_object()
            .map(|m| {
                m.iter()
                    .filter(|(k, v)| ra.get(k.as_str()) != Some(v))
                    .map(|(k, _)| k.clone())
                    .collect()
            })
            .unwrap_or_default();
        return (
            "model routes changed".into(),
            serde_json::json!({"roles":changed}),
        );
    }
    ("preparation contract changed".into(), serde_json::json!({}))
}

/// What the store holds for one published claim fingerprint.
pub enum ClaimMemory {
    /// The claim's memory text now (possibly user-edited).
    Current(String),
    /// No memory row (not published, or not tracked by this store).
    Unpublished,
    /// Deleted by the user: hide the row.
    Tombstoned,
}

/// Intel brief claim rows from an owner's active output (faithful claims and
/// inferences), each labelled with its status. `memory` reports the stored
/// memory for a fingerprint, so user edits and deletions show.
pub fn claim_rows_from_output(
    output: &super::insights::PipelineOutput,
    lifecycle: &str,
    article_id: &str,
    mut memory: impl FnMut(&str) -> Result<ClaimMemory>,
) -> Result<Vec<crate::store::AtlasArticleClaim>> {
    let mut rows = Vec::new();
    let mut push = |fp: String,
                    subject: &str,
                    predicate: &str,
                    object: &str,
                    text: &str,
                    kind: &str,
                    refs: &[super::insights::EvidenceRef],
                    assessment: &super::insights::Assessment|
     -> Result<()> {
        if !assessment.faithful() {
            return Ok(());
        }
        let stored = match memory(&fp)? {
            ClaimMemory::Tombstoned => return Ok(()),
            ClaimMemory::Current(text) => Some(text),
            ClaimMemory::Unpublished => None,
        };
        let edited = stored.as_ref().is_some_and(|current| current != text);
        let text = stored.as_deref().unwrap_or(text);
        let status = if edited {
            "user_edited · unassessed".into()
        } else if lifecycle == "pending" || lifecycle == "previous" {
            format!("previous · {lifecycle}")
        } else {
            assessment.relation.clone()
        };
        let entity = output
            .entities
            .iter()
            .find(|e| e.id == subject)
            .and_then(|e| e.original_forms.first())
            .cloned()
            .unwrap_or_else(|| subject.into());
        rows.push(crate::store::AtlasArticleClaim {
            fingerprint: fp,
            entity,
            predicate: predicate.into(),
            object: object.into(),
            topic: "investigation".into(),
            classification: kind.into(),
            confidence: 0.0,
            claim: format!("[{kind} · {}] {text}", status.replace('_', " ")),
            source_url: refs.first().map(|r| r.url.clone()).unwrap_or_default(),
            published_at: refs
                .first()
                .and_then(|r| r.published_at.clone())
                .unwrap_or_default(),
            article_id: article_id.into(),
            reliability: String::new(),
            info_credibility: 0,
            admiralty: String::new(),
            rsp_status: String::new(),
        });
        Ok(())
    };
    for c in &output.claims {
        push(
            super::insights::claim_fingerprint(c),
            &c.subject_ref,
            &c.predicate,
            &c.object,
            &c.text,
            "claim",
            &c.evidence_refs,
            &c.assessment,
        )?;
    }
    for i in &output.inferences {
        push(
            super::insights::inference_fingerprint(i),
            i.entity_refs.first().map(String::as_str).unwrap_or(""),
            "derived",
            &i.text,
            &i.text,
            "inference",
            &i.evidence_refs,
            &i.assessment,
        )?;
    }
    Ok(rows)
}

/// [`validate_retry`] on the selected store: under SurrealDB it runs
/// `storage::insight_retry::validate_retry` and never opens SQLite.
pub(crate) async fn validate_retry_at(
    _db_path: &std::path::Path,
    input: &PipelineInput,
) -> Result<()> {
    let repo = crate::storage::runtime::repository().await?;
    crate::storage::insight_retry::validate_retry(repo.db(), input).await
}

pub struct PublicationFence<'a> {
    pub task: &'a ClaimedTask,
    pub now: &'a str,
    /// Live input key recomputed from source, premise, binding and memory revisions.
    pub live_input_key: &'a str,
}

#[cfg(test)]
mod tests {}
