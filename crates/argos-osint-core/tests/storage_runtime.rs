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

    drop((a, b));
    if mode == Mode::Server {
        // The process-wide repository still holds its lease.
        assert!(server::stop(&layout, true).await.unwrap());
    }
}
