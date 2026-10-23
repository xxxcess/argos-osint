//! `argos storage init|status|reset --yes` (migration phase 4) in the mode
//! picked by `ARGOS_SURREAL_MODE`. Run through `scripts/test_surreal.py`.
#![cfg(feature = "surreal")]

use argos_osint_core::storage::commands::{self, ResetPhase};
use argos_osint_core::storage::home::{self, Layout, Mode};
use argos_osint_core::storage::server::{self, DB, NS};

fn mode() -> Option<Mode> {
    let embedded = std::env::var("ARGOS_SURREAL_MODE").as_deref() == Ok("embedded");
    if embedded {
        return cfg!(feature = "embedded").then_some(Mode::Embedded);
    }
    std::env::var_os("ARGOS_SURREAL_BIN").map(|_| Mode::Server)
}

async fn count_seed(layout: &Layout, mode: Mode) -> i64 {
    let q = "RETURN array::len(SELECT VALUE id FROM storage_meta:seed)";
    let rows: Option<i64> = match mode {
        Mode::Server => {
            let (o, lease, _) = server::connect_server(layout).await.unwrap();
            let v =
                o.db.query(q)
                    .await
                    .unwrap()
                    .check()
                    .unwrap()
                    .take(0usize)
                    .unwrap();
            drop(lease);
            v
        }
        Mode::Embedded => {
            #[cfg(feature = "embedded")]
            {
                let (o, _g) = home::open_embedded(layout, NS, DB).await.unwrap();
                o.db.query(q)
                    .await
                    .unwrap()
                    .check()
                    .unwrap()
                    .take(0usize)
                    .unwrap()
            }
            #[cfg(not(feature = "embedded"))]
            unreachable!()
        }
    };
    rows.unwrap_or(0)
}

async fn seed(layout: &Layout, mode: Mode) {
    let q = "CREATE storage_meta:seed SET version = 7 RETURN NONE";
    match mode {
        Mode::Server => {
            let (o, lease, _) = server::connect_server(layout).await.unwrap();
            o.db.query(q).await.unwrap().check().unwrap();
            drop(lease);
        }
        Mode::Embedded => {
            #[cfg(feature = "embedded")]
            {
                let (o, _g) = home::open_embedded(layout, NS, DB).await.unwrap();
                o.db.query(q).await.unwrap().check().unwrap();
            }
        }
    }
}

#[tokio::test]
async fn init_status_and_resumable_reset() {
    let Some(mode) = mode() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let layout = Layout::new(dir.path());

    let st = commands::status(&layout).await.unwrap();
    assert!(st.mode.is_none() && !st.ready);
    assert!(
        commands::reset(&layout, true).await.is_err(),
        "reset needs init"
    );

    let st = commands::init(&layout, mode, None).await.unwrap();
    assert_eq!(st.mode, Some(mode));
    assert!(st.ready, "{st:?}");
    assert_eq!(st.schema_version, Some(1));
    let other = match mode {
        Mode::Server => Mode::Embedded,
        Mode::Embedded => Mode::Server,
    };
    assert!(commands::init(&layout, other, None).await.is_err());

    // Legacy data, keepers, and one store record.
    for p in commands::legacy_paths(&layout) {
        if p.ends_with("memory_lancedb") {
            std::fs::create_dir_all(p.join("x.lance")).unwrap();
        } else {
            std::fs::write(&p, b"old").unwrap();
        }
    }
    std::fs::write(layout.home.join("settings.toml"), b"keep").unwrap();
    std::fs::create_dir_all(layout.home.join("models")).unwrap();
    seed(&layout, mode).await;
    assert_eq!(count_seed(&layout, mode).await, 1);

    let err = commands::reset(&layout, false)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("--yes"), "{err}");
    assert!(
        layout.home.join("argos.db").exists(),
        "no wipe without --yes"
    );

    let st = commands::reset(&layout, true).await.unwrap();
    assert!(st.ready, "{st:?}");
    let epoch1 = st.reset_epoch.clone().expect("epoch");
    for p in commands::legacy_paths(&layout) {
        assert!(!p.exists(), "{} survived", p.display());
    }
    assert!(layout.home.join("settings.toml").exists());
    assert!(layout.home.join("models").is_dir());
    assert!(layout.config().exists());
    assert_eq!(layout.credentials().exists(), mode == Mode::Server);
    assert_eq!(count_seed(&layout, mode).await, 0);

    // An interrupted reset is reported and resumes from its phase.
    std::fs::write(layout.home.join("argos.db"), b"old").unwrap();
    home::write_json(
        &layout.storage().join("reset.json"),
        &serde_json::json!({"phase": "bootstrap", "started_at": "t"}),
    )
    .unwrap();
    let st = commands::status(&layout).await.unwrap();
    assert_eq!(st.pending_reset, Some(ResetPhase::Bootstrap));
    assert!(!st.ready);
    let st = commands::reset(&layout, true).await.unwrap();
    assert!(st.ready && st.pending_reset.is_none(), "{st:?}");
    assert_ne!(st.reset_epoch.as_deref(), Some(epoch1.as_str()));
    // Resumed past the legacy phase: that file was not part of it.
    assert!(layout.home.join("argos.db").exists());

    if mode == Mode::Server {
        assert!(server::stop(&layout, true).await.unwrap());
    }
}
