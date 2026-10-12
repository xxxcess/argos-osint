use crate::brain::Memory;
use crate::memory_metadata::MemoryKind;

#[derive(Debug, Clone)]
pub struct BrainQuery {
    pub purpose: String,
    pub active_directive_ids: Vec<String>,
    pub current_question: String,
    pub confirmed_subjects: Vec<String>,
    pub requested_predicates: Vec<String>,
    pub allowed_kinds: Vec<MemoryKind>,
    pub candidate_limit: usize,
    pub token_budget: usize,
}

impl Default for BrainQuery {
    fn default() -> Self {
        Self {
            purpose: String::new(),
            active_directive_ids: Vec::new(),
            current_question: String::new(),
            confirmed_subjects: Vec::new(),
            requested_predicates: Vec::new(),
            allowed_kinds: Vec::new(),
            candidate_limit: 20,
            token_budget: 1500,
        }
    }
}

pub struct AdmissionPolicy {
    pub strict_mode: bool,
    pub allow_background: bool,
}

/// One bounded retrieval set per investigation. Resume and later assessment reuse
/// IDs, but load current memory revisions rather than caching editable text.
pub fn recall_candidates(
    store: &crate::store::Store,
    owner_kind: &str,
    owner_id: &str,
    query: &str,
) -> anyhow::Result<Vec<crate::brain::ScoredMemory>> {
    recall_candidates_with_limit(store, owner_kind, owner_id, query, 20)
}

pub fn recall_candidates_with_limit(
    store: &crate::store::Store,
    owner_kind: &str,
    owner_id: &str,
    query: &str,
    candidate_limit: usize,
) -> anyhow::Result<Vec<crate::brain::ScoredMemory>> {
    let candidate_limit = candidate_limit.clamp(10, 20);
    let generation = if owner_kind == "article" {
        crate::investigation::insight_persist::desired_generation(
            &store.conn,
            owner_kind,
            owner_id,
        )?
    } else {
        0
    };
    let key = format!("insight-recall:{owner_kind}:{owner_id}:{generation}");
    if !owner_id.is_empty() {
        if let Some(json) = store.app_state_get(&key)? {
            let ids: Vec<(String, f32)> = serde_json::from_str(&json)?;
            let mut hits = Vec::new();
            for (id, score) in ids.into_iter().take(candidate_limit) {
                if let Some(memory) = store.get_memory(&id)? {
                    hits.push(crate::brain::ScoredMemory { memory, score });
                }
            }
            return Ok(hits);
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    let hits: Vec<_> = store
        .recall(query, candidate_limit)?
        .into_iter()
        .filter(|hit| seen.insert(hit.memory.id.clone()))
        .take(candidate_limit)
        .collect();
    if !owner_id.is_empty() {
        store.app_state_set(
            &key,
            &serde_json::to_string(
                &hits
                    .iter()
                    .map(|hit| (&hit.memory.id, hit.score))
                    .collect::<Vec<_>>(),
            )?,
        )?;
    }
    Ok(hits)
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct InsightMetadata {
    pub insight_kind: String,
    pub assessment: crate::investigation::insights::Assessment,
    pub lifecycle: String,
    pub snapshot_id: String,
    pub revision: String,
}

pub fn insight_metadata(
    conn: &rusqlite::Connection,
    memory_id: &str,
) -> anyhow::Result<Option<InsightMetadata>> {
    use rusqlite::OptionalExtension;
    let row = conn.query_row("SELECT d.insight_kind,d.assessment_json,CASE WHEN h.active_snapshot_id=s.id THEN h.lifecycle ELSE 'previous' END,s.id,d.revision,m.text FROM insight_derivations d JOIN memories m ON m.id=d.memory_id JOIN insight_snapshots s ON s.id=d.snapshot_id JOIN insight_owner_state h ON h.owner_kind=s.owner_kind AND h.owner_id=s.owner_id WHERE d.memory_id=?1 AND d.status<>'deleted' ORDER BY (h.active_snapshot_id=s.id) DESC,s.created_at DESC,s.id DESC LIMIT 1",[memory_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?))).optional()?;
    row.map(
        |(insight_kind, assessment, lifecycle, snapshot_id, revision, text)| {
            let current_revision = crate::store::memory_revision(&text);
            let mut assessment: crate::investigation::insights::Assessment =
                serde_json::from_str(&assessment)?;
            if current_revision != revision {
                assessment.fidelity = "user_edited".into();
                assessment.relation = "unassessed".into();
                assessment.threshold_passed = false;
            }
            Ok(InsightMetadata {
                insight_kind,
                assessment,
                lifecycle,
                snapshot_id,
                revision: current_revision,
            })
        },
    )
    .transpose()
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct MemoryEvidence {
    pub memory_id: String,
    pub revision: String,
    pub kind: String,
    pub assessment: String,
    pub owner_kind: String,
    pub owner_id: String,
    #[serde(default)]
    pub entities: Vec<crate::nlp::EntityCandidate>,
    pub evidence: Vec<crate::investigation::insights::RetainedEvidence>,
}

/// Enriched recall is backed by persisted derivations, never guessed fields on Memory.
/// No network retrieval is performed to recover original evidence.
pub fn load_memory_evidence(
    conn: &rusqlite::Connection,
    memory_id: &str,
) -> anyhow::Result<Option<MemoryEvidence>> {
    use rusqlite::OptionalExtension;
    let row:Option<(String,String,String,String,String,String)>=conn.query_row("SELECT m.text,d.insight_kind,d.assessment_json,s.owner_kind,s.owner_id,d.id FROM memories m JOIN insight_derivations d ON d.memory_id=m.id JOIN insight_snapshots s ON s.id=d.snapshot_id WHERE m.id=?1 ORDER BY s.generation DESC LIMIT 1",[memory_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?;
    let Some((text, kind, assessment, owner_kind, owner_id, derivation)) = row else {
        return load_legacy_memory_evidence(conn, memory_id);
    };
    let entities_json: String = conn.query_row("SELECT s.entities_json FROM insight_snapshots s JOIN insight_derivations d ON d.snapshot_id=s.id WHERE d.id=?1",[&derivation],|r|r.get(0))?;
    let entities = serde_json::from_str(&entities_json)?;
    let mut stmt=conn.prepare("SELECT DISTINCT evidence_json FROM insight_derivation_evidence WHERE derivation_id=?1 ORDER BY source_id,source_revision")?;
    let refs = stmt
        .query_map([derivation], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut evidence = Vec::new();
    for json in refs {
        let reference: crate::investigation::insights::EvidenceRef = serde_json::from_str(&json)?;
        let artifact: Option<String> = conn
            .query_row(
                "SELECT text FROM nlp_artifacts WHERE hash=?1",
                [&reference.artifact_hash],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(text) = artifact {
            let native = if matches!(
                reference.location,
                crate::investigation::insights::EvidenceLocation::JsonPointer(_)
            ) {
                serde_json::from_str(&text).ok()
            } else {
                None
            };
            let retained = crate::investigation::insights::RetainedEvidence {
                reference,
                text,
                native,
                substantive: true,
                relevant: true,
                complete_story: false,
                known_truncation: false,
            };
            if retained.validate_ref(&retained.reference) {
                evidence.push(retained);
            }
        }
    }
    Ok(Some(MemoryEvidence {
        memory_id: memory_id.into(),
        revision: crate::store::memory_revision(&text),
        kind,
        assessment,
        owner_kind,
        owner_id,
        entities,
        evidence,
    }))
}

fn load_legacy_memory_evidence(
    conn: &rusqlite::Connection,
    memory_id: &str,
) -> anyhow::Result<Option<MemoryEvidence>> {
    use crate::investigation::insights::{EvidenceLocation, EvidenceRef, RetainedEvidence};
    use rusqlite::OptionalExtension;
    let base: Option<(String, String)> = conn.query_row(
        "SELECT m.text,c.classification FROM memories m JOIN insight_claims c ON c.memory_id=m.id WHERE m.id=?1",
        [memory_id], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
    let Some((text, kind)) = base else {
        return Ok(None);
    };
    let mut stmt = conn.prepare("SELECT DISTINCT s.call_id,s.source_url,COALESCE(s.run_id,'') FROM insight_sources s JOIN insight_claims c ON c.fingerprint=s.fingerprint WHERE c.memory_id=?1 ORDER BY s.call_id")?;
    let sources = stmt
        .query_map([memory_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut evidence = Vec::new();
    let mut owner_id = String::new();
    for (source_id, url, run_id) in sources {
        owner_id = run_id;
        let raw: Option<String> = conn
            .query_row(
                "SELECT result_json FROM osint_calls WHERE id=?1 AND status='completed'",
                [&source_id],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        if let Some(raw) = raw {
            if let Ok(result) = serde_json::from_str(&raw) {
                evidence.extend(crate::investigation::insights::evidence_from_tools(
                    conn,
                    &[(source_id.clone(), result)],
                )?);
            }
            continue;
        }
        // Legacy article records remain leads unless the retained acquisition
        // artifact is available. A rewritten body is never substituted for it.
        let article_id = source_id.strip_prefix("article:").unwrap_or(&source_id);
        let acquisition: Option<(String,String,String,String,String)> = conn.query_row(
            "SELECT a.text,r.source_id,r.revision,r.attempt_id,r.raw_hash FROM nlp_source_refs r JOIN nlp_artifacts a ON a.hash=r.raw_hash WHERE r.owner_kind='article' AND r.owner_id=?1 AND r.transform_version='validated-baseline-v1' ORDER BY r.rowid DESC LIMIT 1",
            [article_id], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
        if let Some((original, body_id, revision, acquisition_id, artifact_hash)) = acquisition {
            let retained = RetainedEvidence {
                reference: EvidenceRef {
                    source_id: format!("body:{body_id}"),
                    revision,
                    origin_root: format!("acquisition:{artifact_hash}"),
                    artifact_hash,
                    acquisition_id,
                    url,
                    published_at: None,
                    retrieved_at: String::new(),
                    location: EvidenceLocation::Range(crate::nlp::Span {
                        start_byte: 0,
                        end_byte: original.len(),
                    }),
                },
                text: original,
                native: None,
                substantive: true,
                relevant: true,
                complete_story: false,
                known_truncation: false,
            };
            if retained.validate_ref(&retained.reference) {
                evidence.push(retained);
            }
        }
    }
    if evidence.is_empty() {
        return Ok(None);
    }
    Ok(Some(MemoryEvidence {
        memory_id: memory_id.into(),
        revision: crate::store::memory_revision(&text),
        kind,
        assessment: "legacy/unassessed".into(),
        owner_kind: "legacy".into(),
        owner_id,
        entities: vec![],
        evidence,
    }))
}

pub fn admit_memory(query: &BrainQuery, memory: &Memory, policy: &AdmissionPolicy) -> bool {
    let memory_text: Vec<_> = memory
        .text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase)
        .collect();

    if policy.strict_mode && !query.confirmed_subjects.is_empty() {
        let mut match_found = false;
        for subject in &query.confirmed_subjects {
            let subject: Vec<_> = subject
                .split(|c: char| !c.is_alphanumeric())
                .filter(|s| !s.is_empty())
                .map(str::to_lowercase)
                .collect();
            if !subject.is_empty() && memory_text.windows(subject.len()).any(|w| w == subject) {
                match_found = true;
                break;
            }
        }
        if !match_found {
            return false;
        }
    }
    true
}

pub fn pack_context(admitted: &[Memory], budget: usize) -> String {
    let mut out = String::new();
    for memory in admitted.iter().take(5) {
        let entry=serde_json::json!({"memory_id":memory.id,"revision":crate::store::memory_revision(&memory.text),"text":memory.text,"provenance":memory.source,"qualification":"lead; verify against original evidence"}).to_string();
        // Four bytes/token is an estimate; the serialized byte limit is enforced too.
        if out.len() + entry.len() + 1 > budget.saturating_mul(4).min(6000) {
            continue;
        }
        out.push_str(&entry);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retrieval_is_bounded_and_replay_reloads_edits_and_deletions() {
        let store = crate::store::Store::memory().unwrap();
        let source = crate::brain::MemorySource {
            app: "fixture".into(),
            conversation_id: "bounded-recall".into(),
            message_id: None,
            reference: None,
        };
        for index in 0..25 {
            store
                .add_memory(
                    &format!("Acme factory report {index}"),
                    "investigation",
                    false,
                    source.clone(),
                )
                .unwrap();
        }
        let first = recall_candidates(&store, "recon", "run", "Acme factory").unwrap();
        assert_eq!(first.len(), 20);
        let ids: std::collections::BTreeSet<_> = first.iter().map(|h| &h.memory.id).collect();
        assert_eq!(ids.len(), 20);
        let bounded =
            recall_candidates_with_limit(&store, "recon", "small-run", "Acme factory", 10).unwrap();
        assert_eq!(bounded.len(), 10);
        let replay_bounded =
            recall_candidates_with_limit(&store, "recon", "small-run", "Acme factory", 20).unwrap();
        assert_eq!(
            replay_bounded.len(),
            10,
            "resume must not retrieve a second candidate set"
        );
        assert_eq!(
            recall_candidates_with_limit(&store, "recon", "minimum-run", "Acme factory", 1)
                .unwrap()
                .len(),
            10
        );
        store
            .update_memory(
                &first[0].memory.id,
                "Edited Acme assertion",
                "investigation",
                false,
            )
            .unwrap();
        store.delete_memory(&first[1].memory.id).unwrap();
        store
            .add_memory(
                "A newer Acme factory report",
                "investigation",
                false,
                source,
            )
            .unwrap();
        let replay = recall_candidates(&store, "recon", "run", "changed retrieval query").unwrap();
        assert_eq!(replay.len(), 19);
        assert_eq!(replay[0].memory.text, "Edited Acme assertion");
        assert!(replay.iter().all(|hit| ids.contains(&hit.memory.id)));
        let packed = pack_context(
            &replay.iter().map(|h| h.memory.clone()).collect::<Vec<_>>(),
            1500,
        );
        assert!(packed.len() <= 6000);
        assert!(packed.lines().count() <= 5);
        assert!(
            load_memory_evidence(&store.conn, &first[0].memory.id)
                .unwrap()
                .is_none(),
            "unsourced notes cannot verify a premise"
        );
    }
}
