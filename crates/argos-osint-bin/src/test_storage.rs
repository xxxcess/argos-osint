//! One embedded SurrealDB home per test process: the Store dispatches to
//! the storage runtime, so tests that touch it need it initialized.

use std::cell::RefCell;
use std::sync::{Mutex, MutexGuard, OnceLock};

use argos_osint_core::storage::home::{self, Layout, Mode, StorageConfig};
use argos_osint_core::storage::runtime;

struct Shared {
    _rt: tokio::runtime::Runtime,
    _dir: tempfile::TempDir,
}

static SHARED: OnceLock<Shared> = OnceLock::new();
/// Serializes the tests that share the one home.
static LOCK: Mutex<()> = Mutex::new(());

thread_local! {
    /// Held until the test's thread exits, so the next test starts after it.
    static HELD: RefCell<Option<MutexGuard<'static, ()>>> = const { RefCell::new(None) };
}

/// Initialize the process storage runtime on a temporary embedded home,
/// take the shared-home lock for the rest of this test thread and start
/// from empty tables.
pub(crate) fn ensure() {
    let first = HELD.with(|h| h.borrow().is_none());
    if !first {
        return;
    }
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    HELD.with(|h| *h.borrow_mut() = Some(guard));
    SHARED.get_or_init(|| {
        let dir = tempfile::tempdir().expect("storage tempdir");
        let layout = Layout::new(dir.path());
        home::save_config(
            &layout,
            &StorageConfig {
                mode: Mode::Embedded,
                port: None,
            },
        )
        .expect("storage config");
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("storage runtime");
        rt.block_on(async { runtime::init(layout) })
            .expect("storage init");
        Shared { _rt: rt, _dir: dir }
    });
    drain();
    let mut last = None;
    for _ in 0..20 {
        let wiped = std::thread::spawn(|| {
            runtime::blocking(|repo| async move {
                repo.db()
                    .query(
                        "FOR $t IN object::keys((INFO FOR DB).tables) { DELETE type::table($t); };",
                    )
                    .await?
                    .check()?;
                Ok(())
            })
        })
        .join()
        .expect("wipe thread");
        match wiped {
            Ok(()) => return,
            Err(e) => last = Some(e),
        }
        drain();
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    panic!("wipe test tables: {:#}", last.expect("an error"));
}

/// Wait for every background storage write started so far.
pub(crate) fn drain() {
    if SHARED.get().is_none() {
        return;
    }
    std::thread::spawn(|| {
        runtime::blocking(|_repo| async move {
            runtime::drain().await;
            Ok(())
        })
    })
    .join()
    .expect("drain thread")
    .expect("drain storage writes");
}
