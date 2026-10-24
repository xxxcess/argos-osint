//! Per-home server lifecycle (migration phase 4): start, adopt, stale and
//! wrong-identity `server.json`, leases, last client stops the server.
//! Server mode only; run through `python3 scripts/test_surreal.py`.
#![cfg(feature = "surreal")]

use argos_osint_core::storage::home::{self, Layout};
use argos_osint_core::storage::server::{self, ServerInfo};

fn server_mode() -> bool {
    let embedded = std::env::var("ARGOS_SURREAL_MODE").as_deref() == Ok("embedded");
    let bin = std::env::var_os("ARGOS_SURREAL_BIN").is_some();
    if !embedded && !bin && std::env::var("ARGOS_SURREAL_REQUIRED").as_deref() == Ok("1") {
        panic!("ARGOS_SURREAL_BIN is required");
    }
    !embedded && bin
}

#[tokio::test]
async fn lifecycle_adopts_refuses_stale_and_wrong_identity_and_last_client_stops() {
    if !server_mode() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let layout = Layout::new(dir.path());

    let (opened, lease, info) = server::connect_server(&layout).await.unwrap();
    assert_eq!(opened.bootstrap.version, 1);
    assert!(server::pid_alive(info.pid));
    assert!(info.version.contains("3.3.4"), "{}", info.version);
    assert_eq!(
        home::read_json::<ServerInfo>(&layout.server_json()).unwrap(),
        Some(info.clone())
    );
    assert!(!std::fs::read_to_string(layout.server_json())
        .unwrap()
        .contains(&home::load_or_create_credentials(&layout).unwrap().password));

    // A second start adopts the live server.
    let again = server::ensure_started(&layout).await.unwrap();
    assert_eq!(
        (again.pid, again.instance_id.clone()),
        (info.pid, info.instance_id.clone())
    );
    assert_eq!(
        server::live_clients(&layout).unwrap(),
        vec![std::process::id()]
    );

    // Wrong identity on a live port is refused and the record dropped.
    let mut wrong = info.clone();
    wrong.instance_id = "someone-else".into();
    home::write_json(&layout.server_json(), &wrong).unwrap();
    assert_eq!(server::discover(&layout).await.unwrap(), None);
    assert!(!layout.server_json().exists());
    // Restore so the real server can be stopped.
    home::write_json(&layout.server_json(), &info).unwrap();

    // Another live client keeps it running.
    let other = layout.clients().join("1");
    home::write_json(&other, &serde_json::json!({"pid": 1})).unwrap();
    assert!(!server::stop(&layout, false).await.unwrap());
    std::fs::remove_file(&other).unwrap();

    // Last client out stops it.
    drop(opened);
    assert!(server::release(&layout, lease).await.unwrap());
    assert!(!server::pid_alive(info.pid));
    assert!(!layout.server_json().exists());

    // A stale record (dead pid) is removed, then a fresh start keeps the
    // database identity.
    let mut stale = info.clone();
    stale.pid = 999_999;
    home::write_json(&layout.server_json(), &stale).unwrap();
    assert_eq!(server::discover(&layout).await.unwrap(), None);
    assert!(!layout.server_json().exists());
    let restarted = server::ensure_started(&layout).await.unwrap();
    assert_ne!(restarted.pid, info.pid);
    assert_eq!(restarted.instance_id, info.instance_id);
    assert!(server::stop(&layout, true).await.unwrap());
}
