//! Engine-free compiled Rust local-IPC client contract probe.
//!
//! Spawns `argos-api-host --role local-ipc --state-dir <DIR>` with a cleaned
//! environment, validates `Capabilities::require_native_contract()` on both the
//! `HelloAck` handshake and the `Capabilities` RPC response, verifies
//! `Client::from_local_ipc` readiness, and gracefully shuts down the child.

use std::path::PathBuf;
use std::sync::Arc;

use fleetworks_contracts::capabilities::Capabilities;
use fleetworks_contracts::{Client, IpcOperation, LocalIpcClient};

struct ProbeArgs {
    host: PathBuf,
    state_dir: PathBuf,
}

fn parse_args() -> Result<ProbeArgs, String> {
    let mut host = None;
    let mut state_dir = None;
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--host" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--host requires a path argument".to_owned())?;
                host = Some(PathBuf::from(value));
            }
            "--state-dir" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--state-dir requires a path argument".to_owned())?;
                state_dir = Some(PathBuf::from(value));
            }
            other => return Err(format!("unknown argument `{other}`")),
        }
    }
    Ok(ProbeArgs {
        host: host.ok_or_else(|| "--host is required".to_owned())?,
        state_dir: state_dir.ok_or_else(|| "--state-dir is required".to_owned())?,
    })
}

fn build_command(args: &ProbeArgs) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(&args.host);
    cmd.args([
        "--role",
        "local-ipc",
        "--state-dir",
        &args.state_dir.to_string_lossy(),
    ]);
    cmd.env_clear();
    for key in ["PATH", "HOME", "TMPDIR", "LANG", "LC_ALL", "RUST_LOG"] {
        if let Some(value) = std::env::var_os(key) {
            cmd.env(key, value);
        }
    }
    cmd.env("ARGOS_SURREAL_REQUIRED", "1");
    cmd
}

async fn run_probe(args: ProbeArgs) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let mut cmd = build_command(&args);
    let ipc = Arc::new(
        LocalIpcClient::spawn_command(&mut cmd, "fleetworks-rust-probe", env!("CARGO_PKG_VERSION"))
            .await?,
    );
    let hello_caps = ipc
        .capabilities()
        .ok_or("companion did not advertise capabilities in hello_ack")?
        .clone();
    hello_caps.require_native_contract()?;

    let client = Client::from_local_ipc(Arc::clone(&ipc));
    let ready = client.ready().await?;
    let rpc_caps: Capabilities = ipc.request_json(None, IpcOperation::Capabilities).await?;
    rpc_caps.require_native_contract()?;
    if rpc_caps.deployment_id != hello_caps.deployment_id
        || rpc_caps.configuration_epoch != hello_caps.configuration_epoch
    {
        return Err("capabilities identity drifted between hello_ack and RPC".into());
    }

    let shutdown_ack = client.shutdown_local("rust_probe_complete").await?;
    Ok(serde_json::json!({
        "status": "pass",
        "api_version": rpc_caps.api_version,
        "deployment_id": rpc_caps.deployment_id,
        "configuration_epoch": rpc_caps.configuration_epoch,
        "ready": ready,
        "shutdown_checkpointed": shutdown_ack.checkpointed,
    }))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args().map_err(std::io::Error::other)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let summary = runtime.block_on(run_probe(args))?;
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}
