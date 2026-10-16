//! Current source support is separate from retained evidence and identity history.
use anyhow::Result;
use rusqlite::Connection;

/// Join the caller's deletion/maintenance transaction. Historical snapshots and
/// typed rows survive, but unsupported evidence is no longer searchable.
pub(crate) fn reconcile_sources(conn: &Connection, now: &str) -> Result<()> {
    conn.execute_batch("UPDATE OR IGNORE insight_sources SET
        run_id=(SELECT a.run_id FROM atlas_articles a WHERE a.article_id=CASE WHEN call_id LIKE 'article:%' THEN substr(call_id,9) ELSE call_id END ORDER BY a.seen_at DESC,a.run_id LIMIT 1),
        answer_id=CASE WHEN answer_id LIKE 'atlas-%' THEN 'atlas-'||(SELECT a.run_id FROM atlas_articles a WHERE a.article_id=call_id ORDER BY a.seen_at DESC,a.run_id LIMIT 1) ELSE answer_id END
        WHERE thread_id IS NULL AND (call_id LIKE 'article:%' OR answer_id LIKE 'atlas-%' OR answer_id IN (SELECT id FROM insight_snapshots WHERE owner_kind IN ('article','atlas')))
          AND NOT EXISTS(SELECT 1 FROM atlas_articles a WHERE a.run_id=insight_sources.run_id AND a.article_id=CASE WHEN call_id LIKE 'article:%' THEN substr(call_id,9) ELSE call_id END)
          AND EXISTS(SELECT 1 FROM atlas_articles a WHERE a.article_id=CASE WHEN call_id LIKE 'article:%' THEN substr(call_id,9) ELSE call_id END);
        DELETE FROM insight_sources WHERE thread_id IS NULL AND answer_id LIKE 'atlas-%'
          AND NOT EXISTS(SELECT 1 FROM atlas_articles a WHERE a.run_id=insight_sources.run_id AND a.article_id=call_id);")?;
    conn.execute_batch(
        "DELETE FROM insight_sources WHERE thread_id IS NULL AND (
        answer_id IN (SELECT s.id FROM insight_snapshots s WHERE s.owner_kind IN ('article','atlas')
          AND NOT EXISTS(SELECT 1 FROM atlas_articles a WHERE a.article_id=s.owner_id))
        OR ((call_id LIKE 'article:%' OR answer_id LIKE 'atlas-%')
          AND NOT EXISTS(SELECT 1 FROM atlas_articles a WHERE a.article_id=CASE
            WHEN call_id LIKE 'article:%' THEN substr(call_id,9) ELSE call_id END)));",
    )?;
    // Legacy insight writers can still produce rows without typed metadata.
    conn.execute("INSERT OR IGNORE INTO memory_metadata(memory_id,memory_kind,created_at,updated_at)
        SELECT c.memory_id,CASE WHEN lower(trim(c.classification))='inference' THEN 'inference' ELSE 'claim' END,m.created_at,?1
        FROM insight_claims c JOIN memories m ON m.id=c.memory_id
        WHERE json_extract(m.source_json,'$.app') IN ('article','atlas','recon','recon-deleted')
          AND NOT EXISTS(SELECT 1 FROM insight_sources s WHERE s.fingerprint=c.fingerprint AND s.deleted_origin=0)", [now])?;
    // Existing non-article sources (including Recon) keep a shared claim alive.
    conn.execute("UPDATE memory_metadata SET lifecycle='retired',revision=revision+1,updated_at=?1
        WHERE lifecycle='active' AND memory_kind IN ('claim','atomic_claim','inference')
          AND memory_id IN (SELECT c.memory_id FROM insight_claims c JOIN memories m ON m.id=c.memory_id
            WHERE json_extract(m.source_json,'$.app') IN ('article','atlas','recon','recon-deleted')
              AND NOT EXISTS(SELECT 1 FROM insight_sources s WHERE s.fingerprint=c.fingerprint AND s.deleted_origin=0))", [now])?;
    Ok(())
}

/// Durably fence retired vectors, including edges retired by SQLite triggers.
/// The latest operation prevents repeated maintenance from growing the outbox.
pub(crate) fn reconcile_index(conn: &Connection) -> Result<()> {
    let ids = conn.prepare("SELECT mm.memory_id FROM memory_metadata mm WHERE
        (mm.lifecycle<>'active' AND COALESCE((SELECT operation FROM argos_index_changes i WHERE i.record_id=mm.memory_id AND i.record_kind='memory' ORDER BY seq DESC LIMIT 1),'')<>'index_remove')
        OR (mm.lifecycle='active' AND (SELECT operation FROM argos_index_changes i WHERE i.record_id=mm.memory_id AND i.record_kind='memory' ORDER BY seq DESC LIMIT 1)='index_remove')")?
        .query_map([], |r| r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    crate::store::enqueue_memory_index_on(conn, &ids, None)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{brain::MemorySource, store::Store};
    use rusqlite::params;

    fn claim(store: &Store, kind: &str) -> String {
        let memory = store
            .add_memory(
                "FBI investigated the event",
                "intel",
                false,
                MemorySource {
                    app: "article".into(),
                    conversation_id: "a".into(),
                    message_id: None,
                    reference: None,
                },
            )
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE argos_index_changes SET state='completed' WHERE record_id=?1",
                [&memory.id],
            )
            .unwrap();
        store.conn.execute("INSERT INTO insight_claims(fingerprint,memory_id,entity_id,predicate,object_value,topic,classification,confidence,created_at,updated_at) VALUES(?1,?1,'fbi','investigated','event','security','fact',0.8,'now','now')",[&memory.id]).unwrap();
        store.conn.execute("INSERT INTO memory_metadata(memory_id,memory_kind,payload_json,lifecycle) VALUES(?1,?2,'{}','active')",params![memory.id,kind]).unwrap();
        for article in ["a", "b"] {
            store.conn.execute("INSERT INTO insight_sources(fingerprint,run_id,answer_id,call_id) VALUES(?1,'r','atlas-r',?2)",params![memory.id,article]).unwrap();
        }
        memory.id
    }
    fn lifecycle(store: &Store, id: &str) -> String {
        store
            .conn
            .query_row(
                "SELECT lifecycle FROM memory_metadata WHERE memory_id=?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
    }
    #[test]
    fn article_deletion_updates_shared_support_then_retires_all_evidence_kinds() {
        let store = Store::memory().unwrap();
        for id in ["a", "b"] {
            crate::atlas_actor_index::tests::seed(&store, "r", id, "FBI investigated", &["FBI"]);
        }
        let ids = [claim(&store, "claim"), claim(&store, "inference")];
        store.atlas_delete_article("r", "a").unwrap();
        for id in &ids {
            assert_eq!(lifecycle(&store, id), "active");
            let count:i64=store.conn.query_row("SELECT COUNT(*) FROM insight_sources WHERE fingerprint=?1 AND deleted_origin=0",[id],|r|r.get(0)).unwrap();
            assert_eq!(count, 1);
        }
        store.atlas_delete_article("r", "b").unwrap();
        for id in &ids {
            assert_eq!(lifecycle(&store, id), "retired");
            assert!(
                store.get_memory(id).unwrap().is_some(),
                "retain historical row"
            );
            assert!(!store.list_memories().unwrap().iter().any(|m| m.id == *id));
            assert!(!store
                .search_memories("FBI")
                .unwrap()
                .iter()
                .any(|m| m.id == *id));
            let operation:String=store.conn.query_row("SELECT operation FROM argos_index_changes WHERE record_id=?1 ORDER BY seq DESC LIMIT 1",[id],|r|r.get(0)).unwrap();
            assert_eq!(operation, "index_remove");
        }
        let before = store.conn.total_changes();
        reconcile_index(&store.conn).unwrap();
        assert_eq!(
            store.conn.total_changes(),
            before,
            "maintenance is idempotent"
        );
        store
            .conn
            .execute(
                "UPDATE memory_metadata SET lifecycle='active' WHERE memory_id=?1",
                [&ids[0]],
            )
            .unwrap();
        reconcile_index(&store.conn).unwrap();
        let operation:String=store.conn.query_row("SELECT operation FROM argos_index_changes WHERE record_id=?1 ORDER BY seq DESC LIMIT 1",[&ids[0]],|r|r.get(0)).unwrap();
        assert_eq!(
            operation, "index_upsert",
            "reactivation fences an older removal"
        );
    }
    #[test]
    fn retained_duplicate_article_and_recon_source_keep_memory_alive() {
        let store = Store::memory().unwrap();
        for (run, id) in [("r", "a"), ("other", "a"), ("r", "b")] {
            crate::atlas_actor_index::tests::seed(&store, run, id, "FBI investigated", &["FBI"]);
        }
        let id = claim(&store, "claim");
        store.atlas_delete_article("r", "a").unwrap();
        assert_eq!(
            store
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM insight_sources WHERE fingerprint=?1",
                    [&id],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            2
        );
        store.conn.execute("INSERT INTO insight_sources(fingerprint,thread_id,answer_id,call_id) VALUES(?1,'thread','recon-answer','call-1')",[&id]).unwrap();
        store.atlas_delete_article("other", "a").unwrap();
        store.atlas_delete_article("r", "b").unwrap();
        assert_eq!(lifecycle(&store, &id), "active");
    }

    #[test]
    fn canonical_body_source_deletion_retires_actor_and_dependent_edges() {
        let store = Store::memory().unwrap();
        crate::atlas_actor_index::tests::seed(&store, "r", "a", "FBI investigated", &["FBI"]);
        let id = claim(&store, "inference");
        store
            .conn
            .execute("DELETE FROM insight_sources WHERE fingerprint=?1", [&id])
            .unwrap();
        store.conn.execute("INSERT INTO insight_snapshots(id,owner_kind,owner_id,generation,input_key,inputs_json,entities_json,coverage_json,readiness_json,contract_version,policy_version,created_at) VALUES('snapshot','article','a',1,'key','{}','[]','{}','{}','test','test','now')",[]).unwrap();
        store.conn.execute("INSERT INTO insight_sources(fingerprint,run_id,answer_id,call_id) VALUES(?1,'r','snapshot','body:a')",[&id]).unwrap();
        let actor = store
            .bind_actor(&crate::brain_actors::BindRequest {
                name: "FBI",
                subtype: crate::memory_metadata::ActorSubtype::Organization,
                scope: "article:a",
                identifiers: &[],
                external: None,
                provenance: serde_json::json!({}),
            })
            .unwrap()
            .actor_id;
        crate::brain_actors::add_memory_actor_ref(
            &store.conn,
            &id,
            &actor,
            "subject",
            crate::memory_metadata::IdentityState::Provisional,
            &serde_json::json!({}),
        )
        .unwrap();
        let domain = store
            .bind_actor(&crate::brain_actors::BindRequest {
                name: "fbi.gov",
                subtype: crate::memory_metadata::ActorSubtype::Domain,
                scope: "article:a",
                identifiers: &["fbi.gov".into()],
                external: None,
                provenance: serde_json::json!({}),
            })
            .unwrap()
            .actor_id;
        crate::brain_actors::add_memory_actor_ref(
            &store.conn,
            &id,
            &domain,
            "subject",
            crate::memory_metadata::IdentityState::Resolved,
            &serde_json::json!({}),
        )
        .unwrap();
        for kind in ["link", "related"] {
            let edge = store
                .add_memory(
                    kind,
                    "structural",
                    false,
                    MemorySource {
                        app: "brain".into(),
                        conversation_id: "fixture".into(),
                        message_id: None,
                        reference: None,
                    },
                )
                .unwrap();
            store
                .conn
                .execute(
                    "INSERT INTO memory_metadata(memory_id,memory_kind) VALUES(?1,?2)",
                    params![edge.id, kind],
                )
                .unwrap();
            store.conn.execute("INSERT INTO brain_memory_endpoints(memory_id,side,endpoint_kind,ref_id,origin,revision) VALUES(?1,'source','memory',?2,'inference','')",params![edge.id,id]).unwrap();
        }
        store.atlas_delete_article("r", "a").unwrap();
        assert_eq!(lifecycle(&store, &id), "retired");
        assert_eq!(lifecycle(&store, &actor), "retired");
        assert_eq!(
            lifecycle(&store, &domain),
            "retired",
            "trusted identifiers also lose index eligibility with their final source"
        );
        assert_eq!(
            store
                .insight_for_memory(&id)
                .unwrap()
                .unwrap()
                .sources
                .len(),
            0
        );
        assert_eq!(store.conn.query_row("SELECT COUNT(*) FROM memory_metadata WHERE memory_kind IN ('link','related') AND lifecycle='active'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert_eq!(
            store
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM insight_snapshots WHERE id='snapshot'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
    }

    #[test]
    fn cycle_deletion_moves_legacy_source_to_retained_article_occurrence() {
        let store = Store::memory().unwrap();
        for (run, id) in [("r", "a"), ("other", "a"), ("r", "b")] {
            crate::atlas_actor_index::tests::seed(&store, run, id, "FBI investigated", &["FBI"]);
        }
        let id = claim(&store, "claim");
        store.atlas_delete_run("r").unwrap();
        assert_eq!(lifecycle(&store, &id), "active");
        assert_eq!(
            store.article_origins_for_memory(&id).unwrap(),
            vec![crate::related_articles::ArticleOrigin {
                run_id: "other".into(),
                article_id: "a".into()
            }]
        );
        assert_eq!(
            store
                .insight_for_memory(&id)
                .unwrap()
                .unwrap()
                .sources
                .len(),
            1
        );
        store.atlas_delete_run("other").unwrap();
        assert!(!store.list_memories().unwrap().iter().any(|m| m.id == id));
    }
}
