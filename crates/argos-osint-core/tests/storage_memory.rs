//! Memory publication tests (migration phase 2) against a real SurrealDB
//! 3.3.4 disk server or the embedded RocksDB engine (see `support`).
//! Run through `python3 scripts/test_surreal.py --mode both`.
#![cfg(feature = "surreal")]

mod support;

use std::collections::HashSet;

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
    let publisher = Publisher::spawn(server.root("argos", name).await);
    (db, publisher)
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
