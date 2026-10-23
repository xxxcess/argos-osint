//! Memory publication tests (migration phase 2) against a real SurrealDB
//! 3.3.4 disk server or the embedded RocksDB engine (see `support`).
//! Run through `python3 scripts/test_surreal.py --mode both`.
#![cfg(feature = "surreal")]

mod support;

use std::collections::HashSet;
use std::sync::Arc;

use argos_osint_core::storage::memory::{self, IndexedRow, MemoryWrite};
use argos_osint_core::storage::publisher::{self, Publisher};
use argos_osint_core::storage::tx::TxError;
use argos_osint_core::storage::{counters, outbox};
use support::*;
use surrealdb::engine::any::Any;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

/// Held by every test here so the printed publication latency is measured
/// with no other load on the database.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn pct(sorted: &[u32], p: f64) -> u32 {
    sorted[((sorted.len() as f64 - 1.0) * p).round() as usize]
}

async fn scalar<T: SurrealValue>(db: &Surreal<Any>, sql: &str) -> T {
    let mut r = db.query(sql).await.unwrap().check().unwrap();
    r.take::<Option<T>>(0usize)
        .unwrap()
        .expect("scalar query returned NONE")
}

async fn fresh(server: &Server, name: &str) -> (Surreal<Any>, Publisher) {
    let db = server.root("argos", name).await;
    outbox::apply_schema(&db).await.unwrap();
    memory::apply_schema(&db).await.unwrap();
    let publisher = Publisher::spawn(server.root("argos", name).await);
    (db, publisher)
}

/// Upsert `(id, text)` memories, or delete ids whose text is `None`.
fn put(rows: Vec<(&'static str, Option<&'static str>)>) -> MemoryWrite {
    Arc::new(move |tx| {
        let rows = rows.clone();
        Box::pin(async move {
            let mut ids = Vec::new();
            for (id, text) in rows {
                let q = match text {
                    Some(t) => tx
                        .query("UPSERT type::record('memory', $id) SET text = $t RETURN NONE")
                        .bind(("t", t.to_string())),
                    None => tx.query("DELETE type::record('memory', $id) RETURN NONE"),
                };
                q.bind(("id", id.to_string())).await?.check()?;
                ids.push(id.to_string());
            }
            Ok(ids)
        })
    })
}

async fn pending(db: &Surreal<Any>) -> Vec<(String, String, String)> {
    outbox::pending(db, 0, 100)
        .await
        .unwrap()
        .into_iter()
        .map(|c| (c.record_id, c.revision, c.operation))
        .collect()
}

#[tokio::test]
async fn a_write_and_its_index_work_commit_together() {
    let Some(server) = Server::start() else {
        return;
    };
    let _serial = SERIAL.lock().await;
    let (db, publisher) = fresh(&server, "mem_write").await;
    let p = memory::write_then_index(
        &publisher,
        put(vec![
            ("a", Some("alpha")),
            ("b", Some("beta")),
            ("a", Some("alpha")),
        ]),
    )
    .await
    .unwrap();
    assert_eq!(p.value.enqueued.len(), 2, "duplicate id queued once");
    assert_eq!(p.value.changed_seq, 1);
    let rev_a = memory::memory_revision("alpha");
    assert_eq!(
        pending(&db).await,
        vec![
            ("a".into(), rev_a.clone(), "index_upsert".into()),
            (
                "b".into(),
                memory::memory_revision("beta"),
                "index_upsert".into()
            ),
        ]
    );
    // A failing write leaves neither the memory nor its index work.
    let failing: MemoryWrite = Arc::new(|tx| {
        Box::pin(async move {
            tx.query("UPSERT memory:c SET text = 'gamma' RETURN NONE")
                .await?
                .check()?;
            Err(TxError::Abort("validation failed".into()))
        })
    });
    assert!(memory::write_then_index(&publisher, failing).await.is_err());
    let c: Option<i64> = {
        let mut r = db
            .query("RETURN count(SELECT * FROM memory:c)")
            .await
            .unwrap();
        r.take(0).unwrap()
    };
    assert_eq!(c, Some(0));
    assert_eq!(pending(&db).await.len(), 2);
    assert_eq!(
        counters::current(&db, memory_changed()).await.unwrap(),
        1,
        "failed write does not bump memories_changed"
    );
}

fn memory_changed() -> &'static str {
    argos_osint_core::storage::publisher::CHANGED_COUNTER
}

#[tokio::test]
async fn indexed_revisions_are_skipped_and_deletes_queue_removal() {
    let Some(server) = Server::start() else {
        return;
    };
    let _serial = SERIAL.lock().await;
    let (db, publisher) = fresh(&server, "mem_states").await;
    memory::write_then_index(
        &publisher,
        put(vec![("a", Some("alpha")), ("b", Some("beta"))]),
    )
    .await
    .unwrap();
    // Rebuild records both as indexed at their current text.
    let rows = vec![
        IndexedRow {
            id: "a".into(),
            text: "alpha".into(),
        },
        IndexedRow {
            id: "b".into(),
            text: "beta".into(),
        },
    ];
    memory::record_index_states(&db, &rows, "fp1", "g1")
        .await
        .unwrap();
    let states: Vec<(String, String, String)> = {
        let mut r = db
            .query("SELECT VALUE [record::id(id), fingerprint, generation] FROM memory_index_state ORDER BY id")
            .await
            .unwrap();
        r.take::<Vec<Vec<String>>>(0)
            .unwrap()
            .into_iter()
            .map(|v| (v[0].clone(), v[1].clone(), v[2].clone()))
            .collect()
    };
    assert_eq!(
        states,
        vec![
            ("a".into(), "fp1".into(), "g1".into()),
            ("b".into(), "fp1".into(), "g1".into())
        ]
    );
    // Same text again: already indexed, nothing queued, counter still bumps.
    let same = memory::write_then_index(&publisher, put(vec![("a", Some("alpha"))]))
        .await
        .unwrap();
    assert!(same.value.enqueued.is_empty());
    assert_eq!(same.value.changed_seq, 2);
    // Delete queues removal; an archived memory also queues removal.
    let del = memory::write_then_index(&publisher, put(vec![("b", None)]))
        .await
        .unwrap();
    assert_eq!(del.value.enqueued.len(), 1);
    let archive: MemoryWrite = Arc::new(|tx| {
        Box::pin(async move {
            tx.query("UPDATE memory:a SET lifecycle = 'archived' RETURN NONE")
                .await?
                .check()?;
            Ok(vec!["a".to_string()])
        })
    });
    memory::write_then_index(&publisher, archive).await.unwrap();
    let removals: Vec<_> = pending(&db)
        .await
        .into_iter()
        .filter(|(_, rev, op)| rev == memory::DELETED_REVISION && op == "index_remove")
        .map(|(id, _, _)| id)
        .collect();
    assert_eq!(removals, vec!["b".to_string(), "a".to_string()]);
}

fn probe_write(id: String, fail: bool) -> publisher::Write {
    std::sync::Arc::new(move |tx| {
        let id = id.clone();
        Box::pin(async move {
            if fail {
                return Err(TxError::Abort(format!("invalid write {id}")));
            }
            tx.query("UPSERT type::record('memory_probe', $id) SET at = time::now() RETURN NONE")
                .bind(("id", id.clone()))
                .await?
                .check()?;
            Ok(vec![publisher::Change {
                record_kind: "memory".into(),
                record_id: id.clone(),
                revision: "r1".into(),
                operation: "index_upsert".into(),
                generation: String::new(),
                content_hash: "h".into(),
                op_key: format!("pub:{id}"),
            }])
        })
    })
}

/// p99 above this prints a warning; latency never fails the test.
const WARN_P99_MS: f64 = 200.0;

/// Acceptance for group commit: 8 writers x 40 through one publisher commit
/// every write exactly once, keep outbox seqs gap-free, bump
/// `memories_changed` once per batch and never exhaust retries. p50/p99 are
/// printed per mode (holds SERIAL so no other test here loads the database).
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn eight_publishers_group_commit_every_write_once() {
    let _serial = SERIAL.lock().await;
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "publish").await.0;
    outbox::apply_schema(&db).await.unwrap();
    counters::apply_schema(&db).await.unwrap();
    let publisher = publisher::Publisher::spawn(server.root("argos", "publish").await);
    const PER_CLIENT: usize = 40;
    let mut handles = Vec::new();
    for c in 0..8 {
        let publisher = publisher.clone();
        handles.push(tokio::spawn(async move {
            let mut out = Vec::new();
            for n in 0..PER_CLIENT {
                let start = std::time::Instant::now();
                let r = publisher
                    .publish(probe_write(format!("m{c}-{n}"), false))
                    .await;
                out.push((r, start.elapsed().as_micros() as u32));
            }
            out
        }));
    }
    let mut ok = Vec::new();
    let mut exhausted = 0;
    for h in handles {
        for (r, lat) in h.await.unwrap() {
            match r {
                Ok(c) => ok.push((c, lat)),
                Err(TxError::Contention { .. }) => exhausted += 1,
                Err(e) => panic!("publish failed: {e}"),
            }
        }
    }
    let total = 8 * PER_CLIENT;
    assert_eq!(exhausted, 0, "publications exhausted their retries");
    let mut seqs: Vec<i64> = ok.iter().map(|(c, _)| c.value.enqueued[0].seq).collect();
    seqs.sort_unstable();
    assert_eq!(seqs, (1..=total as i64).collect::<Vec<_>>(), "gapless seqs");
    let written: HashSet<String> = ok
        .iter()
        .map(|(c, _)| c.value.enqueued[0].key.clone())
        .collect();
    assert_eq!(ok.len(), total, "every write committed");
    assert_eq!(written.len(), total, "no write committed twice");
    let probes: i64 = scalar(&db, "RETURN count(SELECT * FROM memory_probe)").await;
    assert_eq!(
        probes as usize, total,
        "every write's statements landed once"
    );
    let batches: HashSet<i64> = ok.iter().map(|(c, _)| c.value.changed_seq).collect();
    let changed: i64 = counters::current(&db, publisher::CHANGED_COUNTER)
        .await
        .unwrap();
    assert_eq!(changed as usize, batches.len(), "one bump per batch");
    let mut lat: Vec<u32> = ok.iter().map(|(_, l)| *l).collect();
    lat.sort_unstable();
    let mut retries: Vec<u32> = ok.iter().map(|(c, _)| c.retries).collect();
    retries.sort_unstable();
    let p99_ms = pct(&lat, 0.99) as f64 / 1000.0;
    // Latency is reported, never asserted (Code Architect): server p99 swung
    // 67-140 ms between runs with zero retries, per-query round-trip jitter.
    eprintln!(
        "PUBLISH-STATS mode={} writes={total} batches={} p50_ms={:.1} p99_ms={p99_ms:.1} \
         max_ms={:.1} retries_p99={} retries_max={} exhausted={exhausted}",
        if server.embedded {
            "embedded"
        } else {
            "server"
        },
        batches.len(),
        pct(&lat, 0.5) as f64 / 1000.0,
        *lat.last().unwrap() as f64 / 1000.0,
        pct(&retries, 0.99),
        retries.last().unwrap()
    );
    if p99_ms > WARN_P99_MS {
        eprintln!("PUBLISH-WARN p99_ms={p99_ms:.1} above {WARN_P99_MS} ms");
    }
}

#[tokio::test]
async fn an_invalid_write_is_split_out_of_its_batch() {
    let Some(server) = Server::start() else {
        return;
    };
    let _serial = SERIAL.lock().await;
    let db = fresh(&server, "publish_split").await.0;
    outbox::apply_schema(&db).await.unwrap();
    counters::apply_schema(&db).await.unwrap();
    let writes = vec![
        probe_write("a".into(), false),
        probe_write("bad".into(), true),
        probe_write("c".into(), false),
    ];
    let results = publisher::publish_batch(&db, &writes).await;
    assert!(results[0].is_ok() && results[2].is_ok());
    assert!(matches!(results[1], Err(TxError::Abort(_))));
    let probes: i64 = scalar(&db, "RETURN count(SELECT * FROM memory_probe)").await;
    assert_eq!(probes, 2);
    let seqs: Vec<i64> = [0, 2]
        .iter()
        .map(|&i| results[i].as_ref().unwrap().value.enqueued[0].seq)
        .collect();
    assert_eq!(seqs, vec![1, 2], "split commits stay gap-free");
    // Replaying a's op_key returns its row; a second write in the same batch
    // for the same work coalesces into it.
    let again = publisher::publish_batch(&db, &[writes[0].clone(), writes[0].clone()]).await;
    for r in &again {
        let e = &r.as_ref().unwrap().value.enqueued[0];
        assert_eq!((e.seq, e.created), (1, false));
    }
    assert_eq!(again[0].as_ref().unwrap().value.batch_size, 2);
    let mut other = probe_write("d".into(), false);
    // An op_key reused for other work aborts.
    other = {
        let inner = other;
        std::sync::Arc::new(move |tx| {
            let inner = inner.clone();
            Box::pin(async move {
                let mut v = inner(tx).await?;
                v[0].op_key = "pub:a".into();
                Ok(v)
            })
        })
    };
    let bad = publisher::publish_batch(&db, &[other]).await;
    assert!(matches!(&bad[0], Err(TxError::Abort(m)) if m.contains("already names")));
}

fn panicking_write() -> publisher::Write {
    Arc::new(|_tx| Box::pin(async move { panic!("boom in write") }))
}

#[tokio::test]
async fn a_panicking_write_fails_alone_and_the_publisher_keeps_running() {
    let Some(server) = Server::start() else {
        return;
    };
    let _serial = SERIAL.lock().await;
    let (db, publisher) = fresh(&server, "publish_panic").await;
    counters::apply_schema(&db).await.unwrap();
    // In a batch, the panic is split out and the good writes commit.
    let results = publisher::publish_batch(
        &db,
        &[
            probe_write("a".into(), false),
            panicking_write(),
            probe_write("c".into(), false),
        ],
    )
    .await;
    assert!(results[0].is_ok() && results[2].is_ok());
    assert!(matches!(&results[1], Err(TxError::Abort(m)) if m.contains("boom in write")));
    // Through the task: the waiter gets the error and the loop survives.
    let err = publisher.publish(panicking_write()).await.unwrap_err();
    assert!(
        matches!(&err, TxError::Abort(m) if m.contains("panicked")),
        "{err}"
    );
    let ok = publisher
        .publish(probe_write("d".into(), false))
        .await
        .unwrap();
    assert_eq!(ok.value.enqueued.len(), 1);
    let probes: i64 = scalar(&db, "RETURN count(SELECT * FROM memory_probe)").await;
    assert_eq!(probes, 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shutdown_drains_queued_writes_then_refuses_new_ones() {
    let Some(server) = Server::start() else {
        return;
    };
    let _serial = SERIAL.lock().await;
    let (db, publisher) = fresh(&server, "publish_shutdown").await;
    counters::apply_schema(&db).await.unwrap();
    let waiters: Vec<_> = (0..20)
        .map(|i| {
            let p = publisher.clone();
            tokio::spawn(async move { p.publish(probe_write(format!("s{i}"), false)).await })
        })
        .collect();
    // Let the writes reach the queue, then stop.
    tokio::task::yield_now().await;
    publisher.shutdown().await;
    let mut committed = 0i64;
    for w in waiters {
        match w.await.unwrap() {
            Ok(_) => committed += 1,
            Err(TxError::PublisherStopped) => {}
            Err(e) => panic!("unexpected {e}"),
        }
    }
    let probes: i64 = scalar(&db, "RETURN count(SELECT * FROM memory_probe)").await;
    assert_eq!(probes, committed, "every accepted write was published");
    let late = publisher.publish(probe_write("late".into(), false)).await;
    assert!(matches!(late, Err(TxError::PublisherStopped)));
    publisher.shutdown().await;
}

// --- graph projection ------------------------------------------------------

mod graph_projection {
    use super::*;
    use argos_osint_core::memory_metadata::{
        ActorPayload, ActorRef, ClaimPayload, EndpointRef, IdentityState, InferencePayload,
        LinkPayload, MemoryKind, MemoryPayload, RelatedPayload,
    };
    use argos_osint_core::storage::graph;
    use argos_osint_core::storage::memory::TypedMemory;

    fn actor_ref(id: &str, role: &str) -> ActorRef {
        ActorRef {
            actor_id: id.into(),
            role: role.into(),
            binding_state: IdentityState::Resolved,
        }
    }

    fn m(id: &str, kind: MemoryKind, payload: MemoryPayload) -> TypedMemory {
        TypedMemory {
            id: id.into(),
            text: format!("text of {id}"),
            kind,
            payload: Some(payload),
            lifecycle: "active".into(),
        }
    }

    fn ep(id: &str, kind: MemoryKind) -> EndpointRef {
        EndpointRef::Memory {
            memory_id: id.into(),
            kind,
        }
    }

    fn link(id: &str, from: &str, to: &str, relation: &str) -> TypedMemory {
        m(
            id,
            MemoryKind::Link,
            MemoryPayload::Link(LinkPayload {
                source: ep(from, MemoryKind::Actor),
                target: ep(to, MemoryKind::Actor),
                relation: relation.into(),
                rationale: String::new(),
                receipts: vec![],
                promoted_from: None,
            }),
        )
    }

    fn write(ms: Vec<TypedMemory>) -> MemoryWrite {
        Arc::new(move |tx| {
            let ms = ms.clone();
            Box::pin(async move {
                for x in &ms {
                    memory::put_typed_in(&tx, x).await?;
                }
                Ok(ms.iter().map(|x| x.id.clone()).collect())
            })
        })
    }

    async fn edges(db: &Surreal<Any>, meaning: &str) -> Vec<String> {
        let mut r = db
            .query(
                "SELECT VALUE record::id(id) FROM brain_graph_edge WHERE meaning = $m ORDER BY id",
            )
            .bind(("m", meaning.to_string()))
            .await
            .unwrap()
            .check()
            .unwrap();
        r.take(0).unwrap()
    }

    fn node(id: &str) -> surrealdb::types::RecordId {
        surrealdb::types::RecordId::new("brain_graph_node", graph::memory_node(id))
    }

    #[tokio::test]
    async fn memory_payloads_project_typed_edges_with_their_write() {
        let Some(server) = Server::start() else {
            return;
        };
        let _serial = SERIAL.lock().await;
        let (db, publisher) = fresh(&server, "graph_project").await;
        let actor = |id: &str| {
            m(
                id,
                MemoryKind::Actor,
                MemoryPayload::Actor(ActorPayload {
                    display_name: id.into(),
                    ..ActorPayload::default()
                }),
            )
        };
        let claim = m(
            "c",
            MemoryKind::Claim,
            MemoryPayload::Claim(ClaimPayload {
                actors: vec![actor_ref("x", "subject"), actor_ref("y", "object")],
                ..ClaimPayload::default()
            }),
        );
        let inference = m(
            "i",
            MemoryKind::Inference,
            MemoryPayload::Inference(InferencePayload {
                premise_claims: vec!["c".into()],
                actors: vec![actor_ref("x", "relevant")],
                support_state: "supported".into(),
                ..InferencePayload::default()
            }),
        );
        let related = m(
            "r",
            MemoryKind::Related,
            MemoryPayload::Related(RelatedPayload {
                source: ep("x", MemoryKind::Actor),
                target: EndpointRef::Article {
                    run_id: "run1".into(),
                    article_id: "a1".into(),
                    revision: "v1".into(),
                },
                shared_actors: vec!["x".into()],
                basis: String::new(),
                interpretation: String::new(),
                verification_gap: String::new(),
                source_revisions: vec![],
            }),
        );
        memory::write_then_index(
            &publisher,
            write(vec![
                actor("x"),
                actor("y"),
                claim.clone(),
                inference,
                link("l1", "x", "y", "supports"),
                link("l2", "x", "y", "contradicts"),
                related,
            ]),
        )
        .await
        .unwrap();
        // Parallel Link edges between the same endpoints keep the memory ids.
        assert_eq!(edges(&db, "link").await, vec!["l1", "l2"]);
        assert_eq!(edges(&db, "related").await, vec!["r"]);
        assert_eq!(edges(&db, "concerns").await.len(), 3);
        assert_eq!(edges(&db, "premise").await.len(), 1);
        // Native traversal from the claim reaches both actors.
        let mut r = db
            .query(
                "SELECT VALUE ->brain_graph_edge[WHERE meaning = 'concerns']->brain_graph_node.ref_id \
                 FROM ONLY $c",
            )
            .bind(("c", node("c")))
            .await
            .unwrap()
            .check()
            .unwrap();
        let mut reached: Vec<String> = r.take(0).unwrap();
        reached.sort();
        assert_eq!(reached, vec!["x", "y"]);
        // Article endpoint is an exact-revision node.
        let mut r = db
            .query("SELECT VALUE [kind, origin, ref_id, revision] FROM brain_graph_node WHERE kind = 'article'")
            .await
            .unwrap()
            .check()
            .unwrap();
        let art: Vec<Vec<String>> = r.take(0).unwrap();
        assert_eq!(art, vec![vec!["article", "run1", "a1", "v1"]]);

        // Retiring the claim drops the edges it owns, keeps the premise edge
        // the inference owns toward it, and marks its node.
        let mut retired = claim;
        retired.lifecycle = "archived".into();
        memory::write_then_index(&publisher, write(vec![retired]))
            .await
            .unwrap();
        assert_eq!(
            edges(&db, "concerns").await.len(),
            1,
            "inference's edge stays"
        );
        assert_eq!(edges(&db, "premise").await.len(), 1);
        let lc: String = scalar(
            &db,
            &format!(
                "SELECT VALUE lifecycle FROM ONLY brain_graph_node:⟨{}⟩",
                graph::memory_node("c")
            ),
        )
        .await;
        assert_eq!(lc, "archived");

        // Re-publishing a Link replaces its edge rather than adding one.
        memory::write_then_index(&publisher, write(vec![link("l1", "y", "x", "supports")]))
            .await
            .unwrap();
        assert_eq!(edges(&db, "link").await, vec!["l1", "l2"]);
        let mut r = db
            .query("SELECT VALUE record::id(out) FROM ONLY brain_graph_edge:l1")
            .await
            .unwrap()
            .check()
            .unwrap();
        let out: Option<String> = r.take(0).unwrap();
        assert_eq!(out, Some(graph::memory_node("x")));
    }
}

// --- recall ----------------------------------------------------------------

mod recall {
    use super::*;
    use argos_osint_core::memory_metadata::MemoryKind;
    use argos_osint_core::storage::embeddings::{self, Embedding};
    use argos_osint_core::storage::recall::{self, RecallQuery, RecallSource};

    fn near(i: usize, tilt: f32) -> Vec<f32> {
        let mut v = vec![0.0f32; embeddings::DIMENSION];
        v[0] = 1.0;
        v[i] = tilt;
        v
    }

    async fn mem(db: &Surreal<Any>, id: &str, text: &str, kind: &str, lc: &str) {
        db.query("UPSERT type::record('memory', $id) SET text = $t, memory_kind = $k, lifecycle = $lc RETURN NONE")
            .bind(("id", id.to_string()))
            .bind(("t", text.to_string()))
            .bind(("k", kind.to_string()))
            .bind(("lc", lc.to_string()))
            .await
            .unwrap()
            .check()
            .unwrap();
    }

    async fn emb(
        db: &Surreal<Any>,
        id: &str,
        of_text: &str,
        model: &str,
        generation: i64,
        tilt: usize,
    ) {
        let rev = memory::memory_revision(of_text);
        let v = near(tilt, 0.05);
        embeddings::put(
            db,
            &Embedding {
                key: id,
                kind: "claim",
                generation,
                revision: &rev,
                model,
                embedding: &v,
            },
        )
        .await
        .unwrap();
    }

    fn ids(hits: &[recall::RecallHit]) -> Vec<&str> {
        hits.iter().map(|h| h.id.as_str()).collect()
    }

    #[tokio::test]
    async fn vector_hits_are_fenced_and_lexical_fills_in() {
        let Some(server) = Server::start() else {
            return;
        };
        let _serial = SERIAL.lock().await;
        let (db, _publisher) = fresh(&server, "recall_fence").await;
        embeddings::apply_schema(&db).await.unwrap();
        let rows = [
            ("fresh", "alpha signal one", "claim", "active"),
            ("edited", "alpha signal two", "claim", "active"),
            ("othermodel", "alpha signal three", "claim", "active"),
            ("oldgen", "alpha signal four", "claim", "active"),
            ("archived", "alpha signal five", "claim", "archived"),
            ("actor", "alpha signal six", "actor", "active"),
        ];
        for (i, (id, text, kind, lc)) in rows.iter().enumerate() {
            mem(&db, id, text, kind, lc).await;
            let of_text = if *id == "edited" { "older text" } else { text };
            let model = if *id == "othermodel" { "mB" } else { "mA" };
            let generation = if *id == "oldgen" { 1 } else { 2 };
            emb(&db, id, of_text, model, generation, i + 1).await;
        }
        let q = RecallQuery {
            text: "alpha signal".into(),
            embedding: Some(near(1, 0.05)),
            model: "mA".into(),
            generation: 2,
            ..Default::default()
        };
        let hits = recall::recall(&db, &q).await.unwrap();
        assert_eq!(ids(&hits), vec!["fresh"], "stale hits never surface");
        assert_eq!(hits[0].source, RecallSource::Vector);
        assert!(hits[0].score > 0.9);

        // No hit survives fencing: lexical fallback over active, allowed kinds.
        let stale = RecallQuery {
            generation: 3,
            ..q.clone()
        };
        let hits = recall::recall(&db, &stale).await.unwrap();
        assert!(hits.iter().all(|h| h.source == RecallSource::Lexical));
        // Equal overlap ties order by id.
        assert_eq!(ids(&hits), vec!["edited", "fresh", "oldgen", "othermodel"]);

        // Explicit kinds override the default exclusions.
        let actors = RecallQuery {
            embedding: None,
            allowed_kinds: vec![MemoryKind::Actor],
            ..q.clone()
        };
        assert_eq!(
            ids(&recall::recall(&db, &actors).await.unwrap()),
            vec!["actor"]
        );

        let opted_out = RecallQuery {
            opt_out: true,
            ..q.clone()
        };
        assert!(recall::recall(&db, &opted_out).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn lexical_recall_pages_the_store_and_applies_caps() {
        let Some(server) = Server::start() else {
            return;
        };
        let _serial = SERIAL.lock().await;
        let (db, _publisher) = fresh(&server, "recall_lexical").await;
        // More than one page; the best matches sit on the last page.
        let total = recall::LEXICAL_PAGE + 40;
        let mut sql = String::from("BEGIN;");
        for i in 0..total {
            sql.push_str(&format!(
                "UPSERT memory:⟨m{i:04}⟩ SET text = 'filler entry {i} beacon', memory_kind = 'claim';"
            ));
        }
        sql.push_str("COMMIT;");
        db.query(sql).await.unwrap().check().unwrap();
        let long = "beacon harbor lantern ".repeat(400);
        for (id, text) in [
            ("zz1", "beacon harbor lantern"),
            ("zz2", "beacon harbor"),
            ("zz3", long.as_str()),
        ] {
            mem(&db, id, text, "claim", "active").await;
        }
        let q = RecallQuery {
            text: "beacon harbor lantern".into(),
            max_memories: 50,
            token_budget: 10_000,
            ..Default::default()
        };
        let hits = recall::recall(&db, &q).await.unwrap();
        assert_eq!(hits.len(), recall::MAX_MEMORIES, "count capped at 5");
        assert!(hits.iter().map(|h| h.tokens).sum::<usize>() <= recall::MAX_TOKENS);
        // zz3 matches as well as zz1 but is over the token budget.
        assert_eq!(&ids(&hits)[..2], &["zz1", "zz2"]);
        assert!(!ids(&hits).contains(&"zz3"));
        assert!(hits[2..].iter().all(|h| h.id.starts_with('m')));
    }
}

mod graph_edge_keys {
    use super::*;
    use argos_osint_core::memory_metadata::{
        ActorRef, ClaimPayload, IdentityState, InferencePayload, MemoryKind, MemoryPayload,
    };
    use argos_osint_core::storage::graph;
    use argos_osint_core::storage::memory::TypedMemory;

    fn actor(id: &str, role: &str) -> ActorRef {
        ActorRef {
            actor_id: id.into(),
            role: role.into(),
            binding_state: IdentityState::Resolved,
        }
    }

    #[tokio::test]
    async fn repeated_actors_and_premises_write_one_edge_each() {
        let Some(server) = Server::start() else {
            return;
        };
        let _serial = SERIAL.lock().await;
        let (db, publisher) = fresh(&server, "graph_dupes").await;
        let ms = vec![
            TypedMemory {
                id: "c".into(),
                text: "claim".into(),
                kind: MemoryKind::Claim,
                payload: Some(MemoryPayload::Claim(ClaimPayload {
                    actors: vec![
                        actor("x", "subject"),
                        actor("x", "subject"),
                        actor("x", "object"),
                    ],
                    ..ClaimPayload::default()
                })),
                lifecycle: "active".into(),
            },
            TypedMemory {
                id: "i".into(),
                text: "inference".into(),
                kind: MemoryKind::Inference,
                payload: Some(MemoryPayload::Inference(InferencePayload {
                    premise_claims: vec!["c".into(), "c".into()],
                    actors: vec![actor("x", "relevant"), actor("x", "relevant")],
                    ..InferencePayload::default()
                })),
                lifecycle: "active".into(),
            },
        ];
        let write: MemoryWrite = Arc::new(move |tx| {
            let ms = ms.clone();
            Box::pin(async move {
                for m in &ms {
                    memory::put_typed_in(&tx, m).await?;
                }
                Ok(ms.iter().map(|m| m.id.clone()).collect())
            })
        });
        memory::write_then_index(&publisher, write).await.unwrap();
        let count = |meaning: &'static str| {
            let db = db.clone();
            async move {
                let n: i64 = scalar(
                    &db,
                    &format!(
                        "RETURN count(SELECT * FROM brain_graph_edge WHERE meaning = '{meaning}')"
                    ),
                )
                .await;
                n
            }
        };
        assert_eq!(
            count("concerns").await,
            3,
            "x/subject, x/object, x/relevant"
        );
        assert_eq!(count("premise").await, 1);
    }

    #[tokio::test]
    async fn dropping_owned_edges_uses_the_owner_index() {
        let Some(server) = Server::start() else {
            return;
        };
        let _serial = SERIAL.lock().await;
        let (db, _publisher) = fresh(&server, "graph_owner_plan").await;
        let mut r = db
            .query(format!("{} EXPLAIN FULL", graph::DELETE_OWNED_SURQL))
            .bind(("id", "m1".to_string()))
            .await
            .unwrap()
            .check()
            .unwrap();
        let plan: surrealdb::types::Value = r.take(0).unwrap();
        let plan = format!("{plan:?}");
        // DELETE plans in the 3.3.4 iterator format: an index iteration on
        // brain_graph_edge_owner, never a table iteration.
        assert!(
            plan.contains(r#""operation": String("Iterate Index")"#),
            "{plan}"
        );
        assert!(
            plan.contains(r#""index": String("brain_graph_edge_owner")"#),
            "{plan}"
        );
        assert!(!plan.contains("Iterate Table"), "{plan}");
    }
}
