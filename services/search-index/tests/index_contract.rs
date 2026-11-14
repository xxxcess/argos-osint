//! Real temporary disk indexes: mocks cannot establish commit/reopen semantics.

use std::fs;

use fleetworks_contracts::actor_index::ActorIndexAlias;
use fleetworks_contracts::actor_index::ActorIndexBatch;
use fleetworks_contracts::actor_index::ActorIndexMutation;
use fleetworks_contracts::actor_index::ActorIndexPromote;
use fleetworks_contracts::actor_index::ActorIndexQuery;
use fleetworks_contracts::actor_index::ActorIndexRebuildStart;
use fleetworks_contracts::actor_index::ActorIndexRepair;
use fleetworks_contracts::actor_index::ActorIndexRetire;
use fleetworks_contracts::actor_index::ActorProjection;
use fleetworks_contracts::actor_index::ACTOR_INDEX_PROTOCOL;
use fleetworks_contracts::actor_index::MAX_BATCH_DOCUMENTS;
use fleetworks_search_index::engine::Engine;
use fleetworks_search_index::engine::EngineConfig;
use sha2::Digest;
use tempfile::TempDir;

fn fixture() -> (TempDir, EngineConfig) {
    let temporary = TempDir::new().expect("isolated fixture root");
    let config = EngineConfig {
        directory: temporary
            .path()
            .canonicalize()
            .expect("canonical fixture root")
            .join("index-volume"),
        deployment_id: "synthetic-deployment".into(),
        source_epoch: "synthetic-database-epoch".into(),
        writer_memory_bytes: 64 * 1024 * 1024,
        writer_threads: 1,
        merge_threads: 1,
    };
    (temporary, config)
}

fn actor(id: &str, revision: u64, name: &str, aliases: &[&str]) -> ActorIndexMutation {
    ActorIndexMutation::new(
        id.into(),
        revision,
        Some(ActorProjection {
            display_name: name.into(),
            aliases: aliases
                .iter()
                .map(|value| ActorIndexAlias {
                    value: (*value).into(),
                    strength: "catalog".into(),
                    provenance: Some("synthetic-fixture".into()),
                })
                .collect(),
            summary: None,
            actor_type: Some("synthetic organization".into()),
            countries: Vec::new(),
            source_id: Some("fixture-catalog".into()),
            authority_scopes: vec!["synthetic-reader".into()],
            eligible: true,
        }),
    )
    .expect("valid synthetic projection")
}

fn batch(
    id: &str,
    generation: &str,
    from: u64,
    mutations: Vec<ActorIndexMutation>,
) -> ActorIndexBatch {
    ActorIndexBatch {
        protocol: ACTOR_INDEX_PROTOCOL,
        deployment_id: "synthetic-deployment".into(),
        source_epoch: "synthetic-database-epoch".into(),
        index_generation: generation.into(),
        batch_id: id.into(),
        writer_epoch: 1,
        from_checkpoint: from,
        through_checkpoint: from + 1,
        source_complete: true,
        mutations,
    }
}

fn ids(engine: &Engine, query: &str) -> Vec<String> {
    engine
        .query(&ActorIndexQuery {
            query: query.into(),
            limit: 25,
        })
        .expect("bounded query")
        .hits
        .into_iter()
        .map(|hit| hit.actor_id)
        .collect()
}

#[test]
fn disk_commit_reload_reopen_and_ambiguous_ack_replay_preserve_one_actor() {
    let (_temporary, config) = fixture();
    let request = batch(
        "seed-batch-1",
        "g1",
        0,
        vec![actor("actor:harbor", 1, "Harbor Union", &["Harbor"])],
    );
    {
        let engine = Engine::open(config.clone()).expect("new owned disk index");
        assert_eq!(
            engine
                .query(&ActorIndexQuery {
                    query: "Harbor".into(),
                    limit: 25
                })
                .expect_err("unbuilt is not empty")
                .code,
            "actor_index_not_built"
        );
        let accepted = engine.apply(&request).expect("commit and explicit reload");
        assert_eq!(accepted.visible_checkpoint, 1);
        assert_eq!(ids(&engine, "Harbor"), ["actor:harbor"]);
        // Treat the successful response as lost; the actual durable commit is
        // reopened and replayed rather than forged in a fake persistence layer.
    }
    let engine = Engine::open(config).expect("reopen committed disk index");
    let replay = engine
        .apply(&request)
        .expect("same batch after uncertain acknowledgement");
    assert!(replay.replayed);
    assert_eq!(ids(&engine, "Harbor"), ["actor:harbor"]);
    assert_eq!(
        engine
            .status()
            .expect("read status")
            .active
            .expect("active")
            .visible_checkpoint,
        1
    );
}

#[test]
fn tombstone_fences_stale_resurrection_after_restart() {
    let (_temporary, config) = fixture();
    {
        let engine = Engine::open(config.clone()).expect("disk index");
        engine
            .apply(&batch(
                "create",
                "g1",
                0,
                vec![actor("actor:a", 1, "Harbor", &[])],
            ))
            .expect("create");
        let tombstone = ActorIndexMutation::new("actor:a".into(), 2, None).expect("tombstone");
        engine
            .apply(&batch("delete", "g1", 1, vec![tombstone]))
            .expect("durable tombstone");
        assert!(ids(&engine, "Harbor").is_empty());
    }
    let engine = Engine::open(config).expect("reopen tombstone");
    let failure = engine
        .apply(&batch(
            "delayed-create",
            "g1",
            2,
            vec![actor("actor:a", 1, "Harbor", &[])],
        ))
        .expect_err("old actor cannot resurrect");
    assert_eq!(failure.code, "obsolete_projection_revision");
    assert!(ids(&engine, "Harbor").is_empty());
    assert_eq!(
        engine
            .status()
            .expect("status")
            .active
            .expect("active")
            .visible_checkpoint,
        2
    );
}

#[test]
fn equal_revision_conflicts_and_wrong_source_lease_or_cursor_do_not_commit() {
    let (_temporary, config) = fixture();
    let engine = Engine::open(config).expect("index");
    let mut initial = batch("first", "g1", 0, vec![actor("actor:a", 1, "Harbor", &[])]);
    initial.writer_epoch = 4;
    engine.apply(&initial).expect("initial higher lease");
    let mut conflicting = batch("second", "g1", 1, vec![actor("actor:a", 1, "Changed", &[])]);
    conflicting.writer_epoch = 4;
    assert_eq!(
        engine
            .apply(&conflicting)
            .expect_err("equal revision conflict")
            .code,
        "projection_revision_conflict"
    );
    conflicting.mutations = vec![actor("actor:a", 2, "Changed", &[])];
    conflicting.source_epoch = "different-database".into();
    assert_eq!(
        engine.apply(&conflicting).expect_err("wrong source").code,
        "source_identity_mismatch"
    );
    conflicting.source_epoch = initial.source_epoch.clone();
    conflicting.writer_epoch = 3;
    assert_eq!(
        engine.apply(&conflicting).expect_err("old lease").code,
        "stale_writer_epoch"
    );
    conflicting.writer_epoch = 4;
    conflicting.from_checkpoint = 7;
    conflicting.through_checkpoint = 8;
    assert_eq!(
        engine.apply(&conflicting).expect_err("cursor gap").code,
        "checkpoint_gap_or_expired_receipt"
    );
    let mut reused = initial.clone();
    reused.mutations = vec![actor("actor:a", 2, "Changed", &[])];
    assert_eq!(
        engine
            .apply(&reused)
            .expect_err("same batch ID differs")
            .code,
        "batch_identity_conflict"
    );
    assert_eq!(ids(&engine, "Harbor"), ["actor:a"]);
    assert_eq!(
        engine
            .status()
            .expect("status")
            .active
            .expect("active")
            .visible_checkpoint,
        1
    );
}

#[test]
fn exact_name_then_ambiguous_aliases_are_stable_and_unicode_prefixes_work() {
    let (_temporary, config) = fixture();
    let engine = Engine::open(config).expect("index");
    engine
        .apply(&batch(
            "unicode",
            "g1",
            0,
            vec![
                actor("actor:b", 1, "Second", &["Union"]),
                actor("actor:a", 1, "First", &["Union"]),
                actor("actor:c", 1, "Union", &[]),
                actor("actor:unicode", 1, "研究所", &["Café North"]),
            ],
        ))
        .expect("index names and aliases");
    assert_eq!(ids(&engine, "Union"), ["actor:c", "actor:a", "actor:b"]);
    assert_eq!(ids(&engine, "研究"), ["actor:unicode"]);
    assert_eq!(ids(&engine, "Cafe\u{301}"), ["actor:unicode"]);
    assert!(engine
        .query(&ActorIndexQuery {
            query: "*:*".into(),
            limit: 25
        })
        .is_err());
    assert!(engine
        .query(&ActorIndexQuery {
            query: "Union".into(),
            limit: 0
        })
        .is_err());
}

#[test]
fn shadow_generation_keeps_old_reader_until_verified_barrier_then_reopens() {
    let (_temporary, config) = fixture();
    {
        let engine = Engine::open(config.clone()).expect("index");
        engine
            .apply(&batch(
                "old",
                "g1",
                0,
                vec![actor("actor:old", 1, "Old Catalog", &[])],
            ))
            .expect("old generation");
        let start = ActorIndexRebuildStart {
            deployment_id: config.deployment_id.clone(),
            source_epoch: config.source_epoch.clone(),
            request_id: "rebuild-1".into(),
            expected_generation: "g1".into(),
            writer_epoch: 2,
            source_checkpoint: 1,
        };
        assert_eq!(
            engine.start_rebuild(&start).expect("new shadow").generation,
            "g2"
        );
        assert_eq!(
            engine
                .start_rebuild(&start)
                .expect("replay same start")
                .generation,
            "g2"
        );
        let mut changed_start = start.clone();
        changed_start.source_checkpoint = 9;
        assert!(engine.start_rebuild(&changed_start).is_err());
        let mut next = batch(
            "replacement",
            "g2",
            1,
            vec![actor("actor:new", 1, "New Catalog", &[])],
        );
        next.writer_epoch = 2;
        next.source_complete = false;
        engine
            .apply(&next)
            .expect("partial shadow visible only internally");
        assert_eq!(ids(&engine, "Old"), ["actor:old"]);
        assert!(ids(&engine, "New").is_empty());
        let promote = ActorIndexPromote {
            deployment_id: config.deployment_id.clone(),
            source_epoch: config.source_epoch.clone(),
            expected_generation: "g1".into(),
            replacement_generation: "g2".into(),
            writer_epoch: 2,
            catchup_checkpoint: 2,
        };
        assert_eq!(
            engine
                .promote(&promote)
                .expect_err("incomplete replacement")
                .code,
            "replacement_not_verified_at_barrier"
        );
        let mut barrier = batch("barrier", "g2", 2, Vec::new());
        barrier.writer_epoch = 2;
        engine.apply(&barrier).expect("catchup barrier visible");
        let promote = ActorIndexPromote {
            catchup_checkpoint: 3,
            ..promote
        };
        engine
            .promote(&promote)
            .expect("atomic serving-generation switch");
        assert_eq!(ids(&engine, "New"), ["actor:new"]);
        assert!(ids(&engine, "Old").is_empty());
    }
    let engine = Engine::open(config.clone()).expect("reopen promoted generation");
    assert_eq!(engine.status().expect("status").retained_generations, 1);
    assert_eq!(ids(&engine, "New"), ["actor:new"]);
    let next = ActorIndexRebuildStart {
        deployment_id: config.deployment_id.clone(),
        source_epoch: config.source_epoch.clone(),
        request_id: "rebuild-2".into(),
        expected_generation: "g2".into(),
        writer_epoch: 3,
        source_checkpoint: 3,
    };
    assert_eq!(
        engine
            .start_rebuild(&next)
            .expect("another single-budget shadow")
            .generation,
        "g3"
    );
    assert_eq!(ids(&engine, "New"), ["actor:new"]);
    let status = engine.status().expect("retained identity");
    let retirement = ActorIndexRetire {
        deployment_id: config.deployment_id.clone(),
        source_epoch: config.source_epoch.clone(),
        request_id: "retire-reviewed-g1".into(),
        expected_generation: "g2".into(),
        generation: "g1".into(),
        expected_manifest_checksum: status
            .manifest_checksum
            .expect("physical manifest checksum"),
        writer_epoch: 3,
    };
    let mut stale = retirement.clone();
    stale.expected_manifest_checksum = "0".repeat(64);
    assert!(engine.retire(&stale).is_err());
    assert_eq!(
        engine
            .retire(&retirement)
            .expect("explicit guarded retirement")
            .retained_generations,
        0
    );
    assert_eq!(
        engine
            .retire(&retirement)
            .expect("replay retirement receipt")
            .retained_generations,
        0
    );
    assert_eq!(ids(&engine, "New"), ["actor:new"]);
}

fn generation_path(config: &EngineConfig, generation: &str) -> std::path::PathBuf {
    config
        .directory
        .join(format!(
            "{:x}",
            sha2::Sha256::digest(config.deployment_id.as_bytes())
        ))
        .join(generation)
}

#[test]
fn corrupt_active_repair_preserves_evidence_and_replays_without_claiming_a_built_index() {
    let (_temporary, config) = fixture();
    drop(Engine::open(config.clone()).expect("owned root"));
    let meta = generation_path(&config, "g1").join("meta.json");
    let evidence = b"synthetic corrupted active metadata";
    fs::write(&meta, evidence).expect("isolated corruption");
    assert!(Engine::open(config.clone()).is_err());
    let status = Engine::recovery_status(&config, "index_storage_unavailable");
    assert!(status.recovery_available);
    let repair = ActorIndexRepair {
        deployment_id: config.deployment_id.clone(),
        source_epoch: config.source_epoch.clone(),
        request_id: "explicit-repair".into(),
        expected_manifest_checksum: status.manifest_checksum.expect("owned manifest evidence"),
        writer_epoch: 2,
    };
    let engine = Engine::repair(&config, &repair).expect("explicit current-source recovery");
    assert_eq!(
        engine
            .status()
            .expect("recovered baseline")
            .active
            .expect("active")
            .generation,
        "g2"
    );
    assert_eq!(
        engine
            .query(&ActorIndexQuery {
                query: "Harbor".into(),
                limit: 10
            })
            .expect_err("not built is not no-match")
            .code,
        "actor_index_not_built"
    );
    assert_eq!(fs::read(&meta).expect("retained corruption"), evidence);
    drop(engine);
    let replay = Engine::repair(&config, &repair).expect("uncertain repair response replay");
    assert_eq!(
        replay
            .status()
            .expect("same generation")
            .active
            .expect("active")
            .generation,
        "g2"
    );
}

#[test]
fn corrupt_shadow_keeps_old_reader_and_explicit_restart_can_promote_a_verified_replacement() {
    let (_temporary, config) = fixture();
    let engine = Engine::open(config.clone()).expect("owned index");
    engine
        .apply(&batch(
            "old",
            "g1",
            0,
            vec![actor("actor:old", 1, "Old Catalog", &[])],
        ))
        .expect("old visible");
    engine
        .start_rebuild(&ActorIndexRebuildStart {
            deployment_id: config.deployment_id.clone(),
            source_epoch: config.source_epoch.clone(),
            request_id: "first-shadow".into(),
            expected_generation: "g1".into(),
            writer_epoch: 2,
            source_checkpoint: 1,
        })
        .expect("shadow");
    drop(engine);
    let meta = generation_path(&config, "g2").join("meta.json");
    let evidence = b"synthetic corrupted shadow metadata";
    fs::write(&meta, evidence).expect("isolated shadow corruption");
    let engine = Engine::open(config.clone()).expect("healthy old reader survives damaged shadow");
    assert_eq!(ids(&engine, "Old"), ["actor:old"]);
    let status = engine.status().expect("diagnosis");
    assert!(status.recovery_available);
    engine
        .repair_shadow(&ActorIndexRepair {
            deployment_id: config.deployment_id.clone(),
            source_epoch: config.source_epoch.clone(),
            request_id: "repair-shadow".into(),
            expected_manifest_checksum: status.manifest_checksum.expect("fenced repair"),
            writer_epoch: 3,
        })
        .expect("preserve failed shadow and old reader");
    assert_eq!(ids(&engine, "Old"), ["actor:old"]);
    assert_eq!(fs::read(meta).expect("retained failed shadow"), evidence);
    engine
        .start_rebuild(&ActorIndexRebuildStart {
            deployment_id: config.deployment_id.clone(),
            source_epoch: config.source_epoch.clone(),
            request_id: "replacement-shadow".into(),
            expected_generation: "g1".into(),
            writer_epoch: 4,
            source_checkpoint: 1,
        })
        .expect("new shadow");
    let mut replacement = batch(
        "new",
        "g3",
        1,
        vec![actor("actor:new", 1, "New Catalog", &[])],
    );
    replacement.writer_epoch = 4;
    engine.apply(&replacement).expect("verified replacement");
    engine
        .promote(&ActorIndexPromote {
            deployment_id: config.deployment_id.clone(),
            source_epoch: config.source_epoch.clone(),
            expected_generation: "g1".into(),
            replacement_generation: "g3".into(),
            writer_epoch: 4,
            catchup_checkpoint: 2,
        })
        .expect("verified switch");
    assert_eq!(ids(&engine, "New"), ["actor:new"]);
    assert!(ids(&engine, "Old").is_empty());
}

#[test]
fn writer_lock_and_corrupt_generation_fail_without_deleting_or_resetting_data() {
    let (_temporary, config) = fixture();
    let engine = Engine::open(config.clone()).expect("index");
    assert_eq!(
        Engine::open(config.clone())
            .err()
            .expect("second writer rejected")
            .code,
        "index_writer_locked"
    );
    drop(engine);
    let scope = fs::read_dir(&config.directory)
        .expect("scope directory")
        .next()
        .expect("scope")
        .expect("entry")
        .path();
    let meta = scope.join("g1/meta.json");
    fs::write(&meta, b"invalid synthetic index metadata").expect("corrupt isolated metadata");
    assert!(Engine::open(config).is_err());
    assert_eq!(
        fs::read(meta).expect("corrupt evidence retained"),
        b"invalid synthetic index metadata"
    );
}

#[test]
fn protocol_rejects_count_bytes_checksums_unknown_fields_and_duplicate_actors() {
    let mut request = batch("bounds", "g1", 0, vec![actor("actor:a", 1, "Harbor", &[])]);
    request.mutations[0].checksum = "0".repeat(64);
    assert!(request.validate().is_err());
    request.mutations = vec![actor("actor:a", 1, "Harbor", &[]); MAX_BATCH_DOCUMENTS + 1];
    assert_eq!(
        request.validate().expect_err("count bounded").0,
        "batch_document_limit"
    );
    request.mutations.truncate(2);
    assert_eq!(
        request.validate().expect_err("duplicate actor").0,
        "duplicate_batch_actor"
    );
    let mut value =
        serde_json::to_value(batch("forged", "g1", 0, Vec::new())).expect("request JSON");
    value["index_dir"] = serde_json::json!("/arbitrary/path");
    assert!(serde_json::from_value::<ActorIndexBatch>(value).is_err());
    request.mutations = (0..300)
        .map(|number| {
            let mut projection = actor(&format!("actor:{number}"), 1, "Harbor", &[])
                .projection
                .expect("projection");
            projection.aliases = (0..32)
                .map(|_| ActorIndexAlias {
                    value: "x ".repeat(128),
                    strength: "catalog".into(),
                    provenance: None,
                })
                .collect();
            ActorIndexMutation::new(format!("actor:{number}"), 1, Some(projection))
                .expect("bounded individual projection")
        })
        .collect();
    assert_eq!(
        request.validate().expect_err("batch bytes bounded").0,
        "batch_byte_limit"
    );
}
