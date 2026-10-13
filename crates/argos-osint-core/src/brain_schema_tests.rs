use crate::store::Store;

fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("argos.db")).unwrap();
    (dir, store)
}

fn memory(store: &Store, id: &str) {
    store
        .conn
        .execute(
            "INSERT INTO memories(id,text,category,pinned,created_at,source_json) VALUES (?1,'t','investigation',0,'2026-01-01T00:00:00Z','{}')",
            [id],
        )
        .unwrap();
}

fn kind(store: &Store, id: &str) -> Option<String> {
    store
        .conn
        .query_row(
            "SELECT memory_kind FROM memory_metadata WHERE memory_id=?1",
            [id],
            |r| r.get(0),
        )
        .ok()
}

#[test]
fn fresh_store_has_schema_37_tables_and_typed_columns() {
    let (_d, store) = store();
    let version: i64 = store
        .conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, crate::store::SCHEMA_VERSION);
    assert_eq!(version, crate::store::SCHEMA_VERSION);
    for table in [
        "brain_actors",
        "brain_actor_aliases",
        "brain_actor_mentions",
        "brain_actor_external_refs",
        "brain_actor_redirects",
        "brain_memory_actor_refs",
        "brain_memory_endpoints",
        "brain_legacy_map",
        "intel_brief_stage_timings",
    ] {
        let n: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [table],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "{table}");
    }
    for column in [
        "payload_json",
        "payload_schema",
        "revision",
        "lifecycle",
        "updated_at",
    ] {
        let n: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('memory_metadata') WHERE name=?1",
                [column],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "{column}");
    }
}

#[test]
fn atomic_claims_migrate_to_claim_or_inference_preserving_ids() {
    let (_d, store) = store();
    for id in ["m-fact", "m-inf", "m-deriv", "m-manual", "m-untyped"] {
        memory(&store, id);
    }
    let c = &store.conn;
    for (id, k) in [
        ("m-fact", "atomic_claim"),
        ("m-inf", "atomic_claim"),
        ("m-deriv", "atomic_claim"),
        ("m-manual", "manual_note"),
    ] {
        c.execute(
            "INSERT INTO memory_metadata(memory_id,memory_kind) VALUES (?1,?2)",
            [id, k],
        )
        .unwrap();
    }
    let claim = |fp: &str, id: &str, class: &str| {
        c.execute(
            "INSERT INTO insight_claims(fingerprint,memory_id,entity_id,predicate,object_value,topic,classification,confidence,created_at,updated_at) VALUES (?1,?2,'e','p','o','t',?3,0.5,'x','x')",
            [fp, id, class],
        )
        .unwrap();
    };
    claim("f1", "m-fact", "fact");
    claim("f2", "m-inf", "inference");
    claim("f3", "m-untyped", "inference");
    c.execute(
        "INSERT INTO insight_snapshots(id,owner_kind,owner_id,generation,input_key,inputs_json,entities_json,coverage_json,readiness_json,contract_version,policy_version,created_at) VALUES ('snap','k','o',1,'i','{}','[]','{}','{}','c','p','x')",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO insight_derivations(id,snapshot_id,memory_id,insight_kind,fingerprint,item_json,assessment_json,status,revision) VALUES ('d1','snap','m-deriv','inference','fp','{}','{}','s','r')",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO memory_brief_memberships(brief_id,member_id,member_type) VALUES ('m-manual','m-fact','atomic_claim')",
        [],
    )
    .unwrap();
    // Re-run the (idempotent) migration on these legacy rows.
    crate::brain_schema::migrate(c).unwrap();
    crate::brain_schema::migrate(c).unwrap();
    assert_eq!(kind(&store, "m-fact").as_deref(), Some("claim"));
    assert_eq!(kind(&store, "m-inf").as_deref(), Some("inference"));
    assert_eq!(kind(&store, "m-deriv").as_deref(), Some("inference"));
    assert_eq!(kind(&store, "m-manual").as_deref(), Some("manual_note"));
    assert_eq!(kind(&store, "m-untyped").as_deref(), Some("inference"));
    let member: String = c
        .query_row(
            "SELECT member_type FROM memory_brief_memberships WHERE member_id='m-fact'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(member, "claim");
    // Every migrated kind decodes through the canonical enum.
    for id in ["m-fact", "m-inf", "m-manual", "m-untyped"] {
        assert!(crate::memory_metadata::MemoryKind::parse(&kind(&store, id).unwrap()).is_some());
    }
}
