//! Upsert Recon assessments into Brain insight tables and refresh article claims.

use anyhow::Result;

use crate::brain::MemorySource;
use crate::store::{insight_fingerprint, AtlasInsightClaim, Store};

/// One recon-derived or revised insight to merge into Brain.
#[derive(Clone, Debug)]
pub struct ReconInsightUpdate {
    pub entity: String,
    pub namespace: String,
    pub predicate: String,
    pub object: String,
    pub topic: String,
    pub claim: String,
    pub classification: String,
    pub confidence: f64,
    pub article_id: String,
    pub source_url: String,
    pub published_at: String,
    pub reliability: String,
    pub info_credibility: u8,
    pub admiralty: String,
    pub rsp_status: String,
    /// When set, also store a recon revision assessment on this ledger element.
    pub element_id: Option<String>,
    pub investigation_id: Option<String>,
    pub stance: String,
    pub rationale: String,
}

/// Update matching Brain claims and insert new ones discovered during Intel Recon.
pub fn upsert_recon_insights(
    store: &Store,
    run_id: &str,
    updates: &[ReconInsightUpdate],
    relations: &[(String, String, String)],
) -> Result<usize> {
    if updates.is_empty() && relations.is_empty() {
        return Ok(0);
    }
    let claims: Vec<AtlasInsightClaim> = updates
        .iter()
        .map(|u| {
            let ns = u.namespace.trim().to_ascii_lowercase();
            let entity = u.entity.trim().to_ascii_lowercase();
            let predicate = u.predicate.trim().to_ascii_lowercase();
            let object = u.object.trim().to_ascii_lowercase();
            AtlasInsightClaim {
                fingerprint: insight_fingerprint(&ns, &entity, &predicate, &object),
                entity: u.entity.clone(),
                namespace: u.namespace.clone(),
                predicate: u.predicate.clone(),
                object: u.object.clone(),
                topic: u.topic.clone(),
                claim: u.claim.clone(),
                classification: u.classification.clone(),
                confidence: u.confidence,
                article_id: u.article_id.clone(),
                source_url: u.source_url.clone(),
                published_at: u.published_at.clone(),
                reliability: u.reliability.clone(),
                info_credibility: u.info_credibility,
                admiralty: u.admiralty.clone(),
                rsp_status: u.rsp_status.clone(),
            }
        })
        .collect();

    // persist_atlas_insights tags memories as app=atlas; rewrite provenance for new inserts
    // after the call for fingerprints that were freshly created by recon.
    store.persist_atlas_insights(run_id, &claims, relations, "", "")?;

    // Annotate sources / assessments with recon origin where applicable.
    for update in updates {
        let ns = update.namespace.trim().to_ascii_lowercase();
        let entity = update.entity.trim().to_ascii_lowercase();
        let predicate = update.predicate.trim().to_ascii_lowercase();
        let object = update.object.trim().to_ascii_lowercase();
        let fingerprint = insight_fingerprint(&ns, &entity, &predicate, &object);
        retag_memory_source(store, &fingerprint, run_id)?;
        if let (Some(inv), Some(element_id)) =
            (update.investigation_id.as_deref(), update.element_id.as_deref())
        {
            let revised = serde_json::json!({
                "confidence": update.confidence,
                "stance": update.stance,
                "classification": update.classification,
                "admiralty": update.admiralty,
            });
            let _ = store.insert_assessment(
                inv,
                element_id,
                "recon",
                &update.stance,
                &update.rationale,
                update.confidence,
                "[]",
                "{}",
                &revised.to_string(),
            );
            let status = if update.stance == "unresolved" {
                "unresolved"
            } else {
                "assessed"
            };
            let _ = store.update_element_assessment(
                element_id,
                status,
                &update.stance,
                &update.rationale,
                "",
                "[]",
            );
        }
    }
    Ok(claims.len())
}

fn retag_memory_source(store: &Store, fingerprint: &str, run_id: &str) -> Result<()> {
    let memory_id: Option<String> = store.conn.query_row(
        "SELECT memory_id FROM insight_claims WHERE fingerprint=?1",
        [fingerprint],
        |row| row.get(0),
    )
    .ok();
    let Some(memory_id) = memory_id else {
        return Ok(());
    };
    let source_json: String = store.conn.query_row(
        "SELECT source_json FROM memories WHERE id=?1",
        [&memory_id],
        |row| row.get(0),
    )?;
    let mut source: MemorySource = serde_json::from_str(&source_json).unwrap_or(MemorySource {
        app: "intel-recon".into(),
        conversation_id: run_id.into(),
        message_id: None,
        reference: Some(run_id.into()),
    });
    // Keep original atlas provenance; mark recon touch via reference suffix when already atlas.
    if source.app == "atlas" {
        source.reference = Some(format!("atlas+intel-recon:{run_id}"));
    } else {
        source.app = "intel-recon".into();
        source.conversation_id = run_id.into();
        source.reference = Some(run_id.into());
    }
    store.conn.execute(
        "UPDATE memories SET source_json=?1 WHERE id=?2",
        rusqlite::params![serde_json::to_string(&source)?, memory_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_adds_and_updates_claims() {
        let store = Store::memory().unwrap();
        let update = ReconInsightUpdate {
            entity: "NATO".into(),
            namespace: "news".into(),
            predicate: "announces".into(),
            object: "aid".into(),
            topic: "military".into(),
            claim: "NATO announces aid".into(),
            classification: "fact".into(),
            confidence: 0.77,
            article_id: "art-1".into(),
            source_url: "https://ex.com".into(),
            published_at: "2026-10-01".into(),
            reliability: "B".into(),
            info_credibility: 2,
            admiralty: "B2".into(),
            rsp_status: "gr".into(),
            element_id: None,
            investigation_id: None,
            stance: "supported".into(),
            rationale: "corroborated".into(),
        };
        let n = upsert_recon_insights(&store, "run-1", &[update], &[]).unwrap();
        assert_eq!(n, 1);
        let rows = store.atlas_claims_for_article("run-1", "art-1").unwrap();
        assert_eq!(rows.len(), 1);
        assert!((rows[0].confidence - 0.77).abs() < f64::EPSILON);
    }
}
