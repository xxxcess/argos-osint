//! Storage home layout, config, credentials and locks (migration phase 4).
#![cfg(feature = "surreal")]

use argos_osint_core::storage::home::*;

#[test]
fn config_and_credentials_round_trip_privately() {
    let dir = tempfile::tempdir().unwrap();
    let layout = Layout::new(dir.path());
    assert_eq!(load_config_file(&layout).unwrap(), None);
    let cfg = StorageConfig {
        mode: Mode::Embedded,
        port: None,
    };
    save_config(&layout, &cfg).unwrap();
    assert_eq!(load_config_file(&layout).unwrap(), Some(cfg.clone()));
    assert_eq!("server".parse::<Mode>().unwrap(), Mode::Server);
    assert!("sqlite".parse::<Mode>().is_err());

    let a = load_or_create_credentials(&layout).unwrap();
    let b = load_or_create_credentials(&layout).unwrap();
    assert_eq!(a, b);
    assert_eq!(a.password.len(), 64);
    assert!(!format!("{a:?}").contains(&a.password));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for p in [layout.config(), layout.credentials()] {
            let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{}", p.display());
        }
    }
    // No temp files left behind.
    let names: Vec<_> = std::fs::read_dir(layout.storage())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert!(names.iter().all(|n| n.ends_with(".json")), "{names:?}");
}

#[test]
fn second_embedded_owner_fails_fast_and_lock_frees_on_drop() {
    let dir = tempfile::tempdir().unwrap();
    let layout = Layout::new(dir.path());
    let first = lock_embedded(&layout).unwrap();
    let err = lock_embedded(&layout).unwrap_err().to_string();
    assert_eq!(
        err,
        format!(
            "Argos home in use by PID {}; use server mode for concurrent CLI and TUI",
            std::process::id()
        )
    );
    drop(first);
    lock_embedded(&layout).unwrap();
}

#[cfg(feature = "embedded")]
#[tokio::test]
async fn open_embedded_bootstraps_and_refuses_a_second_owner() {
    let dir = tempfile::tempdir().unwrap();
    let layout = Layout::new(dir.path());
    let (opened, _guard) = open_embedded(&layout, "argos", "home").await.unwrap();
    assert_eq!(opened.bootstrap.previous, 0);
    assert!(layout.data().is_dir());
    let err = open_embedded(&layout, "argos", "home")
        .await
        .err()
        .expect("second owner refused")
        .to_string();
    assert!(err.starts_with("Argos home in use by PID"), "{err}");
}
