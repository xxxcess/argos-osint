//! Launch the workspace's RabbitMQ server as an owned foreground process.

use std::env;
use std::fs::File;
use std::io::Write;
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{ensure, Context, Result};
use fleetworks_workspace::{Environment, Workspace};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::process::{Child, Command};

use crate::host_storage::{private_dir, private_log};

const DEFAULT_BIND: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 5672);
const BROKER_USER: &str = "fleetworks";
const BROKER_VHOST: &str = "fleetworks";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    pub(crate) bind: SocketAddrV4,
    binary: PathBuf,
}

impl Config {
    pub(crate) fn resolve(
        workspace: &Workspace,
        environment: &Environment,
        prior: Option<&Self>,
    ) -> Result<Self> {
        let bind = environment
            .get("FLEETWORKS_HOST_BROKER_BIND")
            .map(|value| {
                value
                    .to_str()
                    .and_then(|value| value.parse().ok())
                    .context("FLEETWORKS_HOST_BROKER_BIND must be an IPv4 address and port")
            })
            .transpose()?
            .or_else(|| prior.map(|prior| prior.bind))
            .unwrap_or(DEFAULT_BIND);
        let binary = match environment.get("FLEETWORKS_RABBITMQ_BIN") {
            Some(path) => workspace.resolve_path(Path::new(&path)),
            None => prior
                .map(|prior| prior.binary.clone())
                .or_else(|| {
                    env::var_os("PATH").and_then(|paths| {
                        env::split_paths(&paths)
                            .map(|path| path.join("rabbitmq-server"))
                            .find(|path| path.is_file())
                    })
                })
                .context("rabbitmq-server was not found; set FLEETWORKS_RABBITMQ_BIN")?,
        };
        let binary = binary
            .canonicalize()
            .context("resolve host RabbitMQ executable")?;
        ensure!(
            binary.is_file(),
            "host RabbitMQ executable must be a regular file"
        );
        let config = Self { bind, binary };
        config.validate()?;
        Ok(config)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            self.binary.is_absolute()
                && self.bind.port() != 0
                && !self.bind.ip().is_multicast()
                && !self.bind.ip().is_broadcast(),
            "invalid owned host RabbitMQ configuration"
        );
        Ok(())
    }

    pub(crate) fn probe_addr(&self) -> SocketAddrV4 {
        if self.bind.ip().is_unspecified() {
            SocketAddrV4::new(Ipv4Addr::LOCALHOST, self.bind.port())
        } else {
            self.bind
        }
    }

    pub(crate) fn container_url(&self, password: &str) -> String {
        format!(
            "amqp://{BROKER_USER}:{password}@host.docker.internal:{}/{BROKER_VHOST}",
            self.bind.port()
        )
    }

    pub(crate) async fn spawn(&self, workspace: &Workspace, password_file: &Path) -> Result<Child> {
        let workspace = workspace.clone();
        let password_file = password_file.to_owned();
        let bind = self.bind;
        let (data, log) =
            tokio::task::spawn_blocking(move || prepare(&workspace, &password_file, bind))
                .await??;

        let mut command = Command::new(&self.binary);
        command.env_clear();
        for name in ["PATH", "LANG", "LC_ALL"] {
            if let Some(value) = env::var_os(name) {
                command.env(name, value);
            }
        }
        command
            .current_dir(&data)
            .env("HOME", &data)
            .env("RABBITMQ_CONF_ENV_FILE", "/dev/null")
            .env("RABBITMQ_CONFIG_FILE", data.join("rabbitmq.conf"))
            .env(
                "RABBITMQ_ADVANCED_CONFIG_FILE",
                data.join("advanced.config"),
            )
            .env(
                "RABBITMQ_ENABLED_PLUGINS_FILE",
                data.join("enabled_plugins"),
            )
            .env("RABBITMQ_MNESIA_BASE", data.join("mnesia"))
            .env("RABBITMQ_LOG_BASE", &data)
            .env("ERL_CRASH_DUMP_SECONDS", "0")
            // The RabbitMQ launcher execs the Erlang VM in this mode.
            .env("RUNNING_UNDER_SYSTEMD", "true");

        let digest = format!("{:x}", Sha256::digest(data.as_os_str().as_encoded_bytes()));
        let node = format!("fleetworks_{}@localhost", &digest[..16]);
        // Assign each AMQP port a distinct distribution port, wrapping through
        // the valid port range for workspaces with high AMQP ports.
        let dist_port = (1 + (u32::from(bind.port()) + 19_999) % u32::from(u16::MAX)).to_string();
        // Prestarting distribution lets a single local RabbitMQ node boot
        // without spawning a separate Erlang port-mapping daemon.
        command
            .env("RABBITMQ_NODENAME", &node)
            .env("RABBITMQ_DIST_PORT", &dist_port)
            .args([
                "-sname",
                &node,
                "-start_epmd",
                "false",
                "-kernel",
                "erl_epmd_node_listen_port",
                &dist_port,
                "inet_dist_use_interface",
                "{127,0,0,1}",
            ])
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log)
            .kill_on_drop(true)
            .spawn()
            .context("start host RabbitMQ")
    }
}

fn prepare(
    workspace: &Workspace,
    password_file: &Path,
    bind: SocketAddrV4,
) -> Result<(PathBuf, File)> {
    let password = workspace.read_api_token_file(password_file)?;
    ensure!(
        (32..=128).contains(&password.len())
            && password
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-._~".contains(&byte)),
        "invalid Fleetworks broker password file"
    );
    TcpListener::bind(bind).context("host RabbitMQ bind is already in use or unavailable")?;
    let data = workspace.root().join("storage/host-rabbitmq");
    private_dir(&data)?;
    private_dir(&data.join("mnesia"))?;
    let mut config = tempfile::NamedTempFile::new_in(&data)?;
    write!(
        config,
        "listeners.tcp.1 = {bind}\n\
         default_user = {BROKER_USER}\n\
         default_pass = {password}\n\
         default_vhost = {BROKER_VHOST}\n\
         default_user_tags.administrator = false\n\
         log.console = true\n\
         log.file = false\n"
    )?;
    config.as_file().sync_all()?;
    config
        .persist(data.join("rabbitmq.conf"))
        .map_err(|error| error.error)?;
    let log = private_log(&workspace.root().join("logs/host-rabbitmq.log"))?;
    Ok((data, log))
}
