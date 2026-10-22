//! Storage contracts for the SurrealDB migration (phase 1, foundation).
//!
//! These tests run against a real, pinned SurrealDB 3.3.4 server on loopback,
//! with RocksDB on disk, sync on every commit and versioning off. Each test
//! starts its own server in a fresh temporary directory and stops it at the end.
//!
//! Two modes, both through `Surreal<Any>`, selected by `ARGOS_SURREAL_MODE`:
//!
//! - `server` (default): the server binary comes from `ARGOS_SURREAL_BIN`;
//!   `scripts/test_surreal.py` downloads, verifies and passes it. Without it
//!   every test prints a skip message and passes, unless
//!   `ARGOS_SURREAL_REQUIRED=1`, which turns a missing server into a failure
//!   (CI sets it).
//! - `embedded` (needs the `embedded` feature): an in-process RocksDB datastore
//!   in a temp dir; "restart" drops every client and reopens the directory.
//!   The authentication contract does not apply (no network sign-in).
//!
//! Nothing here touches an Argos home, `argos.db` or `memory_lancedb/`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::storage;
use support::*;
use surrealdb::engine::any::{connect, Any};
use surrealdb::opt::auth::Root;
use surrealdb::types::{QueryError, SurrealValue};
use surrealdb::Surreal;

#[tokio::test]
async fn authentication_rejects_bad_and_missing_credentials() {
    let Some(server) = Server::start() else {
        return;
    };
    if server.embedded {
        eprintln!("storage_contracts: authentication is not applicable in embedded mode");
        return;
    }
    let address = format!("ws://127.0.0.1:{}", server.port);

    let wrong = connect(address.as_str()).await.expect("connect");
    let error = wrong
        .signin(Root {
            username: "root".into(),
            password: "not-the-password".into(),
        })
        .await
        .expect_err("a wrong password must not sign in");
    eprintln!(
        "wrong password -> kind={} message={}",
        error.kind_str(),
        error.message()
    );
    assert!(
        error.is_not_allowed(),
        "wrong password kind: {}",
        error.kind_str()
    );

    let db = server.root("argos", "contracts").await;
    db.query("DEFINE TABLE probe SCHEMALESS")
        .await
        .unwrap()
        .check()
        .unwrap();

    let anonymous = connect(address.as_str()).await.expect("connect");
    anonymous
        .use_ns("argos")
        .use_db("contracts")
        .await
        .expect("use ns/db");
    let result = anonymous
        .query("CREATE probe:1 SET ok = true")
        .await
        .and_then(|r| r.check());
    let denied = result.expect_err("an unauthenticated client must not write");
    eprintln!(
        "anonymous write -> kind={} message={}",
        denied.kind_str(),
        denied.message()
    );
    assert!(
        denied.is_not_allowed(),
        "anonymous write kind {}",
        denied.kind_str()
    );

    assert_eq!(
        count(&db, "probe").await,
        0,
        "the rejected write must not land"
    );
    db.query("CREATE probe:1 SET ok = true")
        .await
        .unwrap()
        .check()
        .unwrap();
    assert_eq!(count(&db, "probe").await, 1);
}

#[tokio::test]
async fn cancelled_and_failed_transactions_roll_back() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = server.root("argos", "contracts").await;

    // SurrealDB 3 rejects reads of undefined tables, so schema comes first.
    db.query("DEFINE TABLE item SCHEMALESS")
        .await
        .unwrap()
        .check()
        .unwrap();

    // SDK transaction handle, cancelled explicitly.
    let tx = db.clone().begin().await.expect("begin");
    tx.query("CREATE item:1 SET n = 1; CREATE item:2 SET n = 2")
        .await
        .expect("query in tx")
        .check()
        .expect("statements in tx");
    tx.cancel().await.expect("cancel");
    assert_eq!(
        count(&db, "item").await,
        0,
        "cancel must roll back every write"
    );

    // An in-query transaction that fails part-way writes nothing, and the
    // failure is reported per statement even though the transport succeeded.
    let response = db
        .query("BEGIN; CREATE item:3 SET n = 3; THROW 'abort'; CREATE item:4 SET n = 4; COMMIT;")
        .await
        .expect("transport succeeds");
    let mut response = response;
    let errors = response.take_errors();
    assert!(
        !errors.is_empty(),
        "the failed transaction must report statement errors"
    );
    assert_eq!(
        count(&db, "item").await,
        0,
        "a failed transaction must roll back"
    );

    // Committed work stays.
    let tx = db.clone().begin().await.expect("begin");
    tx.query("CREATE item:5 SET n = 5")
        .await
        .unwrap()
        .check()
        .unwrap();
    tx.commit().await.expect("commit");
    assert_eq!(count(&db, "item").await, 1);
}

#[tokio::test]
async fn concurrent_writes_surface_a_retryable_conflict() {
    let Some(server) = Server::start() else {
        return;
    };
    let first = server.root("argos", "contracts").await;
    let second = server.root("argos", "contracts").await;
    first
        .query("CREATE lease:scheduler SET owner = NONE, epoch = 0")
        .await
        .unwrap()
        .check()
        .unwrap();

    let tx1 = first.clone().begin().await.expect("begin 1");
    let tx2 = second.clone().begin().await.expect("begin 2");
    // Both read the same guard record, then both claim it.
    tx1.query("SELECT * FROM lease:scheduler")
        .await
        .unwrap()
        .check()
        .unwrap();
    tx2.query("SELECT * FROM lease:scheduler")
        .await
        .unwrap()
        .check()
        .unwrap();
    tx1.query("UPDATE lease:scheduler SET owner = 'one', epoch += 1")
        .await
        .unwrap()
        .check()
        .unwrap();
    let late = tx2
        .query("UPDATE lease:scheduler SET owner = 'two', epoch += 1")
        .await
        .and_then(|r| r.check());
    tx1.commit().await.expect("first claimant commits");
    let conflict = match late {
        Err(error) => {
            let _ = tx2.cancel().await;
            error
        }
        Ok(_) => tx2
            .commit()
            .await
            .expect_err("the second claimant must not commit"),
    };
    eprintln!(
        "conflict -> kind={} message={}",
        conflict.kind_str(),
        conflict.message()
    );
    // Raw form differs per engine (server: Query/TransactionConflict;
    // embedded 3.3.4: Internal with only a "can be retried" message).
    // storage::error::normalize gives both the one structured form.
    if server.embedded {
        assert!(
            conflict.is_internal(),
            "raw embedded kind: {}",
            conflict.kind_str()
        );
    }
    assert!(
        storage::error::is_conflict(&conflict),
        "{}",
        conflict.message()
    );
    let conflict = storage::error::normalize(conflict);
    assert!(
        conflict.is_query(),
        "conflict kind: {}",
        conflict.kind_str()
    );
    assert_eq!(
        conflict.query_details(),
        Some(&QueryError::TransactionConflict),
        "a write conflict must be the structured, retryable TransactionConflict"
    );

    let mut response = first
        .query("SELECT VALUE owner FROM ONLY lease:scheduler; SELECT VALUE epoch FROM ONLY lease:scheduler")
        .await
        .unwrap()
        .check()
        .unwrap();
    assert_eq!(
        response.take::<Option<String>>(0).unwrap().as_deref(),
        Some("one")
    );
    assert_eq!(
        response.take::<Option<i64>>(1).unwrap(),
        Some(1),
        "exactly one claim landed"
    );
}

fn unit(seed: usize) -> Vec<f32> {
    // Deterministic, distinct, normalised 384-dimensional vectors.
    let mut v: Vec<f32> = (0..DIM)
        .map(|i| (((i * 31 + seed * 17) % 97) as f32) - 48.0)
        .collect();
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.iter_mut().for_each(|x| *x /= norm);
    v
}

fn cosine_distance(a: &[f32], b: &[f32]) -> f32 {
    1.0 - a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>()
}

#[derive(Debug, SurrealValue)]
struct Hit {
    key: String,
    kind: String,
    d: f64,
}

async fn define_embeddings(db: &Surreal<Any>) {
    db.query(
        "DEFINE TABLE memory_embeddings SCHEMAFULL;
         DEFINE FIELD key ON memory_embeddings TYPE string;
         DEFINE FIELD kind ON memory_embeddings TYPE string;
         DEFINE FIELD generation ON memory_embeddings TYPE int;
         DEFINE FIELD embedding ON memory_embeddings TYPE array<float, 384>;
         DEFINE INDEX memory_embedding_diskann ON memory_embeddings
           FIELDS embedding DISKANN DIMENSION 384 DIST COSINE TYPE F32;",
    )
    .await
    .unwrap()
    .check()
    .expect("define DISKANN index");
}

#[tokio::test]
async fn diskann_f32_384_cosine_filtered_search() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = server.root("argos", "contracts").await;
    define_embeddings(&db).await;

    let kinds = ["claim", "actor", "inference"];
    for i in 0..60 {
        db.query("CREATE memory_embeddings SET key = $key, kind = $kind, generation = $generation, embedding = $embedding")
            .bind(("key", format!("m{i:02}")))
            .bind(("kind", kinds[i % 3].to_string()))
            .bind(("generation", if i % 5 == 0 { 1i64 } else { 2i64 }))
            .bind(("embedding", unit(i)))
            .await
            .unwrap()
            .check()
            .unwrap();
    }

    let info = db
        .query("INFO FOR TABLE memory_embeddings")
        .await
        .unwrap()
        .check()
        .unwrap();
    let info = format!("{info:?}");
    assert!(
        info.contains("DISKANN DIMENSION 384 DIST COSINE TYPE F32"),
        "index: {info}"
    );

    // m09 is a generation-2 claim, so it must be its own nearest neighbour.
    let query = unit(9);
    let sql = "SELECT key, kind, vector::distance::knn() AS d FROM memory_embeddings
               WHERE kind = 'claim' AND generation = 2 AND embedding <|5,100|> $q ORDER BY d";
    let mut plan = db
        .query(format!("{sql} EXPLAIN"))
        .bind(("q", query.clone()))
        .await
        .unwrap()
        .check()
        .unwrap();
    let plan: Option<surrealdb::types::Value> = plan.take(0).unwrap();
    let plan = format!("{plan:?}");
    assert!(
        plan.contains("KnnScan") && plan.contains("memory_embedding_diskann"),
        "plan: {plan}"
    );

    let mut response = db
        .query(sql)
        .bind(("q", query.clone()))
        .await
        .unwrap()
        .check()
        .unwrap();
    let hits: Vec<Hit> = response.take(0).unwrap();
    assert_eq!(
        hits.len(),
        5,
        "filtered KNN must still return k hits: {hits:?}"
    );

    // Exact reference: brute-force cosine over the same filtered set.
    let mut exact: Vec<(String, f32)> = (0..60)
        .filter(|i| i % 3 == 0 && i % 5 != 0)
        .map(|i| (format!("m{i:02}"), cosine_distance(&unit(i), &query)))
        .collect();
    exact.sort_by(|a, b| a.1.total_cmp(&b.1));
    for (hit, (key, distance)) in hits.iter().zip(&exact) {
        assert_eq!(hit.kind, "claim");
        assert_eq!(&hit.key, key, "ranked order matches exact cosine: {hits:?}");
        assert!(
            (hit.d as f32 - distance).abs() < 1e-4,
            "distance {} vs exact {distance}",
            hit.d
        );
    }
    assert_eq!(hits[0].key, "m09");
    assert!(
        hits.iter().all(|h| h.key != "m15" && h.key != "m30"),
        "generation-1 claims are filtered out"
    );
    assert!(hits[0].d.abs() < 1e-5, "self-distance is ~0");
}

#[tokio::test]
async fn committed_data_and_indexes_survive_a_server_restart() {
    let Some(mut server) = Server::start() else {
        return;
    };
    {
        let db = server.root("argos", "contracts").await;
        define_embeddings(&db).await;
        let tx = db.clone().begin().await.unwrap();
        tx.query("CREATE note:keep SET body = 'survives'")
            .await
            .unwrap()
            .check()
            .unwrap();
        tx.query("CREATE memory_embeddings SET key = 'm1', kind = 'claim', generation = 1, embedding = $e")
            .bind(("e", unit(1)))
            .await
            .unwrap()
            .check()
            .unwrap();
        tx.commit().await.unwrap();
        let tx = db.clone().begin().await.unwrap();
        tx.query("CREATE note:drop SET body = 'cancelled'")
            .await
            .unwrap()
            .check()
            .unwrap();
        tx.cancel().await.unwrap();
    }
    server.restart();
    assert_on_disk(&server.data_path());
    let log = server.log();
    assert!(
        server.embedded || log.contains("Sync mode: every transaction commit"),
        "sync mode not applied:\n{log}"
    );

    let db = server.root("argos", "contracts").await;
    let mut response = db
        .query("SELECT VALUE body FROM ONLY note:keep; RETURN count(SELECT * FROM note);")
        .await
        .unwrap()
        .check()
        .unwrap();
    assert_eq!(
        response.take::<Option<String>>(0).unwrap().as_deref(),
        Some("survives")
    );
    assert_eq!(
        response.take::<Option<i64>>(1).unwrap(),
        Some(1),
        "the cancelled write stays gone"
    );
    let mut response = db
        .query("SELECT key, kind, vector::distance::knn() AS d FROM memory_embeddings WHERE embedding <|1,40|> $q")
        .bind(("q", unit(1)))
        .await
        .unwrap()
        .check()
        .unwrap();
    let hits: Vec<Hit> = response.take(0).unwrap();
    assert_eq!(hits.len(), 1, "the DISKANN index answers after restart");
    assert_eq!(hits[0].key, "m1");
}

#[tokio::test]
async fn statement_errors_are_structured() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = server.root("argos", "contracts").await;
    db.query(
        "DEFINE TABLE actor SCHEMAFULL;
         DEFINE FIELD name ON actor TYPE string;
         DEFINE FIELD alias_key ON actor TYPE string;
         DEFINE INDEX actor_alias_key ON actor FIELDS alias_key UNIQUE;",
    )
    .await
    .unwrap()
    .check()
    .unwrap();

    // Outer transport success is not statement success.
    let mut response = db
        .query("CREATE actor:a SET name = 'A', alias_key = 'k'; CREATE actor:b SET name = 'B', alias_key = 'k'; CREATE actor:c SET name = 3, alias_key = 'c';")
        .await
        .expect("transport succeeds");
    let errors = response.take_errors();
    let duplicate = errors
        .get(&1)
        .expect("duplicate unique key fails statement 1");
    let wrong_type = errors.get(&2).expect("wrong field type fails statement 2");
    eprintln!(
        "duplicate -> kind={} message={}",
        duplicate.kind_str(),
        duplicate.message()
    );
    eprintln!(
        "wrong type -> kind={} message={}",
        wrong_type.kind_str(),
        wrong_type.message()
    );
    assert!(!errors.contains_key(&0), "the first statement succeeded");
    // Server 3.3.4 reports unique-index and field-coercion failures as kind
    // `Internal` with no details, so repositories cannot match a typed variant
    // for them. Pin that here: if a later server adds structured details, this
    // test fails and the repository error mapping should switch to them.
    assert!(
        duplicate.is_internal(),
        "duplicate kind {}",
        duplicate.kind_str()
    );
    assert!(
        duplicate
            .message()
            .contains("`actor_alias_key` already contains"),
        "{}",
        duplicate.message()
    );
    assert!(
        wrong_type.is_internal(),
        "wrong type kind {}",
        wrong_type.kind_str()
    );
    assert!(
        wrong_type
            .message()
            .contains("Couldn't coerce value for field `name`"),
        "{}",
        wrong_type.message()
    );
    assert!(!duplicate.is_connection() && !wrong_type.is_connection());

    // Undefined tables carry typed details.
    let missing = db
        .query("SELECT * FROM never_defined")
        .await
        .unwrap()
        .check()
        .expect_err("undefined table");
    eprintln!(
        "undefined table -> kind={} details={:?}",
        missing.kind_str(),
        missing.not_found_details()
    );
    assert!(
        missing.is_not_found(),
        "undefined table kind {}",
        missing.kind_str()
    );
    assert!(missing.not_found_details().is_some());

    // check() turns any statement error into an Err.
    let checked = db
        .query("CREATE actor:d SET name = 'D', alias_key = 'k'")
        .await
        .unwrap()
        .check();
    assert!(checked.is_err(), "check() must surface the statement error");
    assert_eq!(count(&db, "actor").await, 1);
}
