//! The single storage entry point (migration phase 4): one runtime handle,
//! one repository per process, no `block_on` inside an async context.
//! Runs in the mode picked by `ARGOS_SURREAL_MODE`.
#![cfg(feature = "surreal")]

use argos_osint_core::storage::home::{self, Layout, Mode, StorageConfig};
use argos_osint_core::storage::{runtime, server};

fn mode() -> Option<Mode> {
    if std::env::var("ARGOS_SURREAL_MODE").as_deref() == Ok("embedded") {
        return cfg!(feature = "embedded").then_some(Mode::Embedded);
    }
    std::env::var_os("ARGOS_SURREAL_BIN").map(|_| Mode::Server)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_entry_point_serves_async_and_plain_thread_callers() {
    let Some(mode) = mode() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let layout = Layout::new(dir.path());
    home::save_config(&layout, &StorageConfig { mode, port: None }).unwrap();

    // Not initialized yet.
    assert!(runtime::repository().await.is_err());
    runtime::init(layout.clone()).unwrap();
    runtime::init(layout.clone()).unwrap();
    assert!(runtime::init(Layout::new(dir.path().join("other"))).is_err());

    let a = runtime::repository().await.unwrap();
    let b = runtime::repository().await.unwrap();
    assert_eq!(a.mode(), Some(mode));
    assert!(std::ptr::eq(a.db(), b.db()), "one repository per process");

    // Inside the runtime: refused, not a nested block_on.
    let err = runtime::blocking(|_r| async { Ok(()) })
        .unwrap_err()
        .to_string();
    assert!(err.contains("inside an async context"), "{err}");

    // A plain thread drives work on the same runtime.
    let version = std::thread::spawn(|| {
        runtime::blocking(|repo| async move {
            let mut r = repo
                .db()
                .query("SELECT VALUE version FROM storage_meta WHERE id = storage_meta:schema")
                .await?
                .check()?;
            Ok(r.take::<Vec<i64>>(0usize)?)
        })
    })
    .join()
    .unwrap()
    .unwrap();
    assert_eq!(version, vec![1]);

    // Bounded spawn_ordered preserves order and bounds under the shared runtime.
    let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    for i in 0..100 {
        let c = counter.clone();
        let accepted = runtime::spawn_ordered(async move {
            c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        assert!(accepted, "item {i} must be accepted within capacity");
    }
    runtime::drain().await;
    assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 100);

    drop((a, b));
    if mode == Mode::Server {
        // The process-wide repository still holds its lease.
        assert!(server::stop(&layout, true).await.unwrap());
    }
}

/// `open_home` bootstraps every module schema (`ensure_schema`) before it
/// hands out the store, in the configured mode; a reopen is a no-op.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_home_runs_ensure_schema() {
    use argos_osint_core::storage::bootstrap::{schemas, SCHEMA_VERSION};
    use argos_osint_core::storage::repo::Repository;
    let Some(mode) = mode() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let layout = Layout::new(dir.path());
    home::save_config(&layout, &StorageConfig { mode, port: None }).unwrap();

    let repo = Repository::open_home(&layout).await.unwrap();
    // Server mode already bootstraps once at spawn, before `open::open`.
    let fresh = if mode == Mode::Server {
        SCHEMA_VERSION
    } else {
        0
    };
    assert_eq!(repo.bootstrap().previous, fresh);
    assert_eq!(repo.bootstrap().version, SCHEMA_VERSION);
    let mut r = repo
        .db()
        .query("INFO FOR DB")
        .await
        .unwrap()
        .check()
        .unwrap();
    let info: serde_json::Value = r
        .take::<Option<surrealdb::types::Value>>(0usize)
        .unwrap()
        .map(|v| v.into_json_value())
        .expect("db info");
    let tables = info["tables"].as_object().expect("tables");
    for (module, schema) in schemas() {
        let defined = schema.split("DEFINE TABLE IF NOT EXISTS ").skip(1);
        let defined: Vec<&str> = defined
            .filter_map(|d| d.split_whitespace().next())
            .collect();
        // Index-only modules (e.g. `recon_extract`'s full-text index on
        // `memory`) define no table of their own.
        assert!(
            !defined.is_empty() || schema.contains("DEFINE INDEX IF NOT EXISTS "),
            "{module}: no tables parsed"
        );
        for table in defined {
            assert!(tables.contains_key(table), "{module}: {table} not defined");
        }
    }
    drop(repo);

    let again = Repository::open_home(&layout).await.unwrap();
    assert_eq!(again.bootstrap().previous, SCHEMA_VERSION);
    drop(again);
    if mode == Mode::Server {
        server::stop(&layout, true).await.unwrap();
    }
}
