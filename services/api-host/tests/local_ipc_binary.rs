#![cfg(feature = "embedded")]

use fleetworks_contracts::capabilities::Capabilities;
use fleetworks_contracts::ipc::{IpcOperation, LocalIpcClient};
use fleetworks_contracts::Client;
use tokio::process::Command;

const BINARY: &str = env!("CARGO_BIN_EXE_argos-api-host");

#[test]
fn built_api_host_shows_role_help() {
    let output = std::process::Command::new(BINARY)
        .arg("--help")
        .output()
        .expect("run the built service host with --help");
    assert!(output.status.success(), "service help failed: {output:?}");
    let help = std::str::from_utf8(&output.stdout).expect("UTF-8 service help");
    assert!(help.contains("--role <ROLE>"), "{help}");
    assert!(help.contains("bootstrap:"), "{help}");
    assert!(help.contains("--state-dir <STATE_DIR>"), "{help}");
}

#[tokio::test]
async fn built_local_ipc_binary_advertises_valid_contract_survives_restart_and_releases_lock_on_drop(
) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state_dir = dir.path().canonicalize().expect("canonical").join("state");
    std::fs::create_dir_all(&state_dir).expect("create state_dir");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&state_dir, std::fs::Permissions::from_mode(0o700))
            .expect("chmod state_dir 700");
    }

    // Session 1: boot real binary, validate HelloAck capabilities against production native
    // contract and transport-neutral Client endpoints, then shut down cleanly.
    let mut cmd1 = Command::new(BINARY);
    cmd1.args(["--role", "local-ipc", "--state-dir"])
        .arg(&state_dir);
    let ipc1 = LocalIpcClient::spawn_command(&mut cmd1, "fleetworks-desktop-test", "0.2.1")
        .await
        .expect("spawn session 1 local-ipc binary");
    let hello_caps1 = ipc1
        .capabilities()
        .expect("HelloAck must include capabilities")
        .clone();
    hello_caps1
        .require_native_contract()
        .expect("HelloAck capabilities must satisfy production native contract");
    let deployment_id = hello_caps1
        .deployment_id
        .clone()
        .expect("non-empty deployment_id");
    let config_epoch = hello_caps1
        .configuration_epoch
        .clone()
        .expect("non-empty configuration_epoch");

    let client1 = Client::from_local_ipc(std::sync::Arc::new(ipc1.clone()));
    assert_eq!(client1.capabilities(), Some(&hello_caps1));
    let fetched_caps1: Capabilities = ipc1
        .request_json(None, IpcOperation::Capabilities)
        .await
        .expect("capabilities RPC over local IPC");
    fetched_caps1
        .require_native_contract()
        .expect("RPC capabilities satisfy native contract");
    assert_eq!(
        fetched_caps1.deployment_id.as_deref(),
        Some(deployment_id.as_str())
    );
    assert_eq!(
        fetched_caps1.configuration_epoch.as_deref(),
        Some(config_epoch.as_str())
    );

    let ack = ipc1.shutdown("restart_test").await.expect("shutdown 1");
    assert!(ack.checkpointed);
    drop(client1);
    drop(ipc1);

    // Session 2: immediately reopen on the exact same --state-dir, assert stable deployment_id
    // and lock availability right after shutdown, then drop without calling shutdown().
    let mut cmd2 = Command::new(BINARY);
    cmd2.args(["--role", "local-ipc", "--state-dir"])
        .arg(&state_dir);
    let ipc2 = LocalIpcClient::spawn_command(&mut cmd2, "fleetworks-desktop-test", "0.2.1")
        .await
        .expect("reopen session 2 on same state_dir after shutdown");
    let hello_caps2 = ipc2
        .capabilities()
        .expect("HelloAck 2 capabilities")
        .clone();
    hello_caps2
        .require_native_contract()
        .expect("HelloAck 2 capabilities satisfy native contract");
    assert_eq!(
        hello_caps2.deployment_id.as_deref(),
        Some(deployment_id.as_str())
    );
    assert_eq!(
        hello_caps2.configuration_epoch.as_deref(),
        Some(config_epoch.as_str())
    );
    drop(ipc2);

    // Session 3: immediately reopen after dropping last LocalIpcClient without shutdown();
    // broken Arc cycle + child kill/reap must have already released state.lock / RocksDB LOCK.
    let mut cmd3 = Command::new(BINARY);
    cmd3.args(["--role", "local-ipc", "--state-dir"])
        .arg(&state_dir);
    let ipc3 = LocalIpcClient::spawn_command(&mut cmd3, "fleetworks-desktop-test", "0.2.1")
        .await
        .expect("reopen session 3 immediately after last LocalIpcClient drop");
    let hello_caps3 = ipc3
        .capabilities()
        .expect("HelloAck 3 capabilities")
        .clone();
    hello_caps3
        .require_native_contract()
        .expect("HelloAck 3 capabilities satisfy native contract");
    assert_eq!(
        hello_caps3.deployment_id.as_deref(),
        Some(deployment_id.as_str())
    );
    let _ = ipc3.shutdown("binary_test_done").await;
}
