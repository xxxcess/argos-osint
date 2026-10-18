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

#[test]
fn schema_41_upgrade_adds_actor_maintenance_without_changing_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("argos.db");
    let s = Store::open(&path).unwrap();
    let evidence = s
        .add_memory(
            "keep-evidence",
            "fact",
            false,
            crate::brain::MemorySource {
                app: "test".into(),
                conversation_id: "migration".into(),
                message_id: None,
                reference: None,
            },
        )
        .unwrap();
    s.conn.execute_batch("DROP TABLE brain_actor_dedup_receipts; DROP TRIGGER brain_article_deleted; DROP TRIGGER brain_article_changed; CREATE TABLE user_extra(value TEXT); INSERT INTO user_extra VALUES('preserve'); PRAGMA user_version=41").unwrap();
    drop(s);
    let s = Store::open(&path).unwrap();
    assert_eq!(s.get_memory(&evidence.id).unwrap(), Some(evidence));
    assert_eq!(
        s.conn
            .query_row("SELECT value FROM user_extra", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "preserve"
    );
    assert_eq!(
        s.conn
            .query_row("SELECT COUNT(*) FROM brain_actor_dedup_receipts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    for trigger in ["brain_article_deleted", "brain_article_changed"] {
        assert!(s
            .conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='trigger' AND name=?1)",
                [trigger],
                |r| r.get::<_, bool>(0)
            )
            .unwrap());
    }
}

fn user_version(conn: &rusqlite::Connection) -> i64 {
    conn.query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap()
}

fn table_exists(conn: &rusqlite::Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
        [name],
        |r| r.get::<_, i64>(0),
    )
    .unwrap()
        > 0
}

#[test]
fn schema_is_43_on_a_fresh_database_without_the_graph_explanation_table() {
    let (_d, store) = store();
    assert_eq!(crate::store::SCHEMA_VERSION, 43);
    assert_eq!(user_version(&store.conn), 43);
    assert!(!table_exists(&store.conn, "argos_graph_explanations"));
}

#[test]
fn a_42_database_migrates_to_43_and_drops_the_orphan_table() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("argos.db");
    {
        let store = Store::open(&path).unwrap();
        store
            .conn
            .execute_batch(
                "CREATE TABLE argos_graph_explanations (memory_id TEXT PRIMARY KEY, summary TEXT);
                 INSERT INTO argos_graph_explanations VALUES ('m1','old');
                 PRAGMA user_version = 42;",
            )
            .unwrap();
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(user_version(&store.conn), 43);
    assert!(!table_exists(&store.conn, "argos_graph_explanations"));
    assert!(table_exists(&store.conn, "memory_graph_summaries"));
}

#[test]
fn a_database_already_at_43_opens_cleanly_and_keeps_unrelated_tables() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("argos.db");
    {
        let store = Store::open(&path).unwrap();
        // Shape left by an earlier, reverted schema 43.
        store
            .conn
            .execute_batch(
                "CREATE TABLE topology_scope_summaries (digest TEXT PRIMARY KEY, seed_ref TEXT NOT NULL DEFAULT '');
                 INSERT INTO topology_scope_summaries VALUES ('d1','m1');
                 CREATE TABLE argos_graph_explanations (memory_id TEXT PRIMARY KEY);",
            )
            .unwrap();
        assert_eq!(user_version(&store.conn), 43);
    }
    for _ in 0..2 {
        let store = Store::open(&path).unwrap();
        assert_eq!(user_version(&store.conn), 43);
        assert!(!table_exists(&store.conn, "argos_graph_explanations"));
        let kept: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM topology_scope_summaries", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(kept, 1, "the leftover topology table is not touched");
    }
}
