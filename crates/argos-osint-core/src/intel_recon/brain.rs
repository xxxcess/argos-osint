//! Upsert Recon assessments into Brain insight tables and refresh article claims.

use anyhow::Result;

use crate::store::{insight_fingerprint, AtlasInsightClaim, PublishOptions, Store};

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

    // One transaction through the durable publisher: memories, claim/source
    // links, intel-recon provenance (new and reused memories) and index-outbox
    // rows commit together; indexing is acknowledged through leased tasks. No
    // Atlas receipt is written so the cycle's own receipt is never shadowed.
    store.publish_atlas_insights(
        run_id,
        &claims,
        relations,
        "",
        &PublishOptions {
            index_now: true,
            retag_app: Some("intel-recon".into()),
            skip_receipt: true,
            ..Default::default()
        },
    )?;

    // Record recon assessments where applicable.
    for update in updates {
        if let (Some(inv), Some(element_id)) = (
            update.investigation_id.as_deref(),
            update.element_id.as_deref(),
        ) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn update(entity: &str, object: &str, claim: &str) -> ReconInsightUpdate {
        ReconInsightUpdate {
            entity: entity.into(),
            namespace: "news".into(),
            predicate: "announces".into(),
            object: object.into(),
            topic: "military".into(),
            claim: claim.into(),
            classification: "fact".into(),
            confidence: 0.7,
            article_id: "art-1".into(),
            source_url: "https://ex.com".into(),
            published_at: "2026-10-01".into(),
            reliability: "B".into(),
            info_credibility: 2,
            admiralty: "B2".into(),
            rsp_status: String::new(),
            element_id: None,
            investigation_id: None,
            stance: "supported".into(),
            rationale: String::new(),
        }
    }

    #[test]
    fn intel_recon_publishes_durably_with_provenance_and_no_atlas_receipt() {
        let _fake = crate::embed::testing::fake();
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("argos.db")).unwrap();
        // An Atlas cycle published the first claim.
        let u = update("NATO", "aid", "NATO announces aid");
        let atlas_claim = AtlasInsightClaim {
            fingerprint: String::new(),
            entity: u.entity,
            namespace: u.namespace,
            predicate: u.predicate,
            object: u.object,
            topic: u.topic,
            claim: u.claim,
            classification: u.classification,
            confidence: u.confidence,
            article_id: u.article_id,
            source_url: u.source_url,
            published_at: u.published_at,
            reliability: u.reliability,
            info_credibility: u.info_credibility,
            admiralty: u.admiralty,
            rsp_status: u.rsp_status,
        };
        let atlas = store
            .publish_atlas_insights(
                "atlas-run",
                &[atlas_claim],
                &[],
                "",
                &PublishOptions::default(),
            )
            .unwrap();
        let seq = store.memories_changed_seq().unwrap();
        let n = upsert_recon_insights(
            &store,
            "recon-run",
            &[
                update("NATO", "aid", "NATO announces aid"),
                update("NATO", "drills", "NATO announces drills"),
            ],
            &[],
        )
        .unwrap();
        assert_eq!(n, 2);
        assert!(store
            .atlas_publication_receipt("recon-run")
            .unwrap()
            .is_none());
        let kept = store
            .atlas_publication_receipt("atlas-run")
            .unwrap()
            .unwrap();
        assert_eq!(
            (kept.revision, kept.created_memory_ids),
            (atlas.revision, atlas.created_memory_ids),
            "the cycle's own receipt is untouched"
        );
        assert!(store.memories_changed_seq().unwrap() > seq);
        let memories = store.list_memories().unwrap();
        assert_eq!(memories.len(), 2, "the shared claim is reused");
        for memory in &memories {
            assert_eq!(
                memory.source.reference.as_deref(),
                Some("atlas+intel-recon:recon-run"),
                "{memory:?}"
            );
        }
        let ids: Vec<String> = memories.into_iter().map(|m| m.id).collect();
        assert!(store.verify_memory_coverage(&ids).unwrap().complete());
    }

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
