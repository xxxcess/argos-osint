//! `storage::open` bootstraps the schema before returning a client
//! (migration phase 4). Run through `python3 scripts/test_surreal.py --mode both`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::storage::bootstrap::SCHEMA_VERSION;
use argos_osint_core::storage::open::{open, Target};
use support::*;

async fn has_schema(db: &surrealdb::Surreal<surrealdb::engine::any::Any>) {
    let mut r = db
        .query(
            "SELECT VALUE version FROM storage_meta:schema; SELECT count() FROM memory GROUP ALL",
        )
        .await
        .unwrap()
        .check()
        .unwrap();
    assert_eq!(r.take::<Vec<i64>>(0usize).unwrap(), vec![SCHEMA_VERSION]);
}

#[tokio::test]
async fn open_applies_schema_before_returning() {
    let Some(server) = Server::start() else {
        return;
    };
    let target = if server.embedded {
        #[cfg(feature = "embedded")]
        {
            Target::Embedded {
                path: server.dir.path().join("open-store"),
            }
        }
        #[cfg(not(feature = "embedded"))]
        unreachable!("embedded mode needs the embedded feature")
    } else {
        Target::Server {
            port: server.port,
            username: "root".into(),
            password: server.password.clone(),
        }
    };
    assert!(!format!("{target:?}").contains(&server.password) || server.embedded);
    let first = open(&target, "argos", "open").await.unwrap();
    assert_eq!(first.bootstrap.previous, 0);
    has_schema(&first.db).await;
    if !server.embedded {
        // A second open (another client) finds the schema already there.
        let second = open(&target, "argos", "open").await.unwrap();
        assert_eq!(second.bootstrap.previous, SCHEMA_VERSION);
        has_schema(&second.db).await;
    }
}
