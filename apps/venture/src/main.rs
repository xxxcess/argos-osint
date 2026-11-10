mod classifier;
mod compose;
mod host_storage;

use std::env;
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, ensure, Context, Result};
use clap::{Args as ClapArgs, Parser, Subcommand, ValueEnum};
use fleetworks_contracts::{
    Client, API_ORIGIN_ENV, API_TOKEN_ENV, API_TOKEN_FILE_ENV, DEFAULT_API_ORIGIN,
};
use fleetworks_workspace::Workspace;
use tokio::process::Command;

use crate::compose::{Owner, StorageMode};

#[derive(Parser)]
#[command(about = "Manage a local Fleetworks stack and launch network clients")]
struct Args {
    /// Workspace path. Set this before the command when forwarding client arguments.
    #[arg(long, global = true, value_name = "PATH")]
    workspace: Option<PathBuf>,
    #[command(subcommand)]
    action: Action,
}

#[derive(Subcommand)]
enum Action {
    /// Create the workspace directories and its local API token.
    Init,
    /// Import supported API keys and model defaults from a legacy Argos home.
    Import {
        #[arg(long, value_name = "ARGOS_HOME")]
        from: PathBuf,
    },
    /// Start or reconcile this workspace's local Compose stack.
    Up(StackOptions),
    /// Start the web stack and attach this terminal to Jobs or service logs.
    Web {
        #[command(flatten)]
        stack: StackOptions,
        #[arg(long, value_enum, default_value = "jobs")]
        terminal: WebTerminal,
    },
    /// Check readiness against the configured Fleetworks HTTP API.
    Status {
        #[arg(long)]
        api_origin: Option<String>,
        #[arg(long, value_name = "PATH")]
        api_token_file: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Read service logs for the stack this workspace owns.
    Logs {
        #[arg(long, short)]
        follow: bool,
    },
    /// Stop this workspace's stack and keep its persistent data.
    Stop,
    /// Train the legacy Natural classifier on the current workspace CSV.
    TrainLegacy {
        /// Override the workspace article dataset.
        #[arg(long, value_name = "PATH")]
        file: Option<PathBuf>,
        /// Explicitly activate the trained synthetic baseline for legacy use.
        #[arg(long, visible_alias = "activate-synthetic-baseline")]
        activate: bool,
    },
    /// Launch a sibling client. Use `venture help client` for launcher help.
    #[command(disable_help_flag = true)]
    Client {
        #[arg(value_enum)]
        kind: ClientKind,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        arguments: Vec<OsString>,
    },
    #[command(name = "__host-storage-supervisor", hide = true)]
    HostStorageSupervisor { generation: String },
}

#[derive(ClapArgs)]
struct StackOptions {
    #[arg(long, value_enum)]
    storage: Option<StorageMode>,
    #[arg(long, value_name = "PATH")]
    compose_file: Option<PathBuf>,
    /// Start the private Atlas, classifier, NLP and broker services too.
    #[arg(long)]
    atlas: bool,
}

impl StackOptions {
    async fn up(self, workspace: &Workspace) -> Result<()> {
        compose::up(
            workspace,
            self.storage,
            self.compose_file.as_deref(),
            self.atlas,
        )
        .await
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum WebTerminal {
    Jobs,
    Logs,
}

#[derive(Clone, Copy, ValueEnum)]
enum ClientKind {
    Atlas,
    Brain,
    Jobs,
    Tools,
    Models,
    Profile,
}

impl ClientKind {
    fn binary(self) -> &'static str {
        match self {
            Self::Atlas => "fleetworks-atlas",
            Self::Brain => "fleetworks-brain",
            Self::Jobs => "fleetworks-jobs",
            Self::Tools => "fleetworks-tools",
            Self::Models => "fleetworks-models",
            Self::Profile => "fleetworks-profile",
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let workspace = Workspace::resolve(args.workspace.as_deref())?;
    eprintln!("Workspace {}", workspace.root().display());
    match args.action {
        Action::Init => {
            workspace.initialize_api_token()?;
            println!("Workspace initialized");
            Ok(())
        }
        Action::Import { from } => {
            let summary = workspace.import_legacy(&from)?;
            println!(
                "Imported {} API keys and {} model defaults",
                summary.api_keys, summary.model_defaults
            );
            Ok(())
        }
        Action::Up(stack) => stack.up(&workspace).await,
        Action::Web { stack, terminal } => {
            stack.up(&workspace).await?;
            match terminal {
                WebTerminal::Jobs => launch(&workspace, ClientKind::Jobs, vec!["tui".into()]).await,
                WebTerminal::Logs => compose::logs(&workspace, true).await,
            }
        }
        Action::Status {
            api_origin,
            api_token_file,
            json,
        } => status(&workspace, api_origin, api_token_file.as_deref(), json).await,
        Action::Logs { follow } => compose::logs(&workspace, follow).await,
        Action::Stop => compose::stop(&workspace).await,
        Action::TrainLegacy { file, activate } => {
            classifier::train(&workspace, file, activate).await
        }
        Action::Client { kind, arguments } => launch(&workspace, kind, arguments).await,
        Action::HostStorageSupervisor { generation } => {
            let owner = Owner::load(&workspace)
                .await?
                .context("missing Venture stack ownership")?;
            owner.supervise_host_storage(&workspace, generation).await
        }
    }
}

async fn status(
    workspace: &Workspace,
    api_origin: Option<String>,
    api_token_file: Option<&Path>,
    json: bool,
) -> Result<()> {
    let environment = workspace.environment()?;
    let owner = Owner::load(workspace).await?;
    let origin = match api_origin {
        Some(origin) => origin,
        None => environment
            .get(API_ORIGIN_ENV)
            .map(|origin| {
                origin
                    .into_string()
                    .map_err(|_| anyhow!("Fleetworks API origin must be UTF-8"))
            })
            .transpose()?
            .or_else(|| owner.as_ref().map(Owner::api_origin))
            .unwrap_or_else(|| DEFAULT_API_ORIGIN.into()),
    };
    let token = match api_token_file {
        Some(path) => Some(workspace.read_api_token_file(path)?),
        None if environment.get(API_TOKEN_ENV).is_none()
            && environment.get(API_TOKEN_FILE_ENV).is_none()
            && owner.is_some() =>
        {
            owner
                .as_ref()
                .map(|owner| workspace.read_api_token_file(owner.token_file()))
                .transpose()?
        }
        None => workspace.api_token(&environment)?,
    };
    if !json {
        if let Some(owner) = &owner {
            owner.print_status(workspace).await?;
        }
    }
    let ready = Client::new(&origin, token.as_deref())?.ready().await?;
    if json {
        let mut output = io::stdout().lock();
        serde_json::to_writer(&mut output, &ready)?;
        writeln!(output)?;
    }
    ensure!(ready.ready && ready.storage, "Fleetworks API is not ready");
    if !json {
        println!("Fleetworks API v{} is ready at {origin}", ready.api_version);
    }
    Ok(())
}

async fn launch(workspace: &Workspace, kind: ClientKind, arguments: Vec<OsString>) -> Result<()> {
    let name = kind.binary();
    let executable =
        env::current_exe()?.with_file_name(format!("{name}{}", env::consts::EXE_SUFFIX));
    let mut command = Command::new(&executable);
    if let Some(owner) = Owner::load(workspace).await? {
        let environment = workspace.environment()?;
        if environment.get(API_ORIGIN_ENV).is_none() {
            command.env(API_ORIGIN_ENV, owner.api_origin());
        }
        if environment.get(API_TOKEN_ENV).is_none() && environment.get(API_TOKEN_FILE_ENV).is_none()
        {
            command.env(API_TOKEN_FILE_ENV, owner.token_file());
        }
    }
    let status = command
        .env("FLEETWORKS_WORKSPACE", workspace.root())
        .arg("--workspace")
        .arg(workspace.root())
        .args(arguments)
        .kill_on_drop(true)
        .status()
        .await
        .with_context(|| format!("launch sibling client {}", executable.display()))?;
    ensure!(status.success(), "{name} exited with {status}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::Path;

    use clap::Parser;

    use super::{Action, Args, ClientKind};

    #[test]
    fn launcher_keeps_client_options_in_order() {
        let args = Args::try_parse_from([
            "venture",
            "--workspace",
            "case",
            "client",
            "profile",
            "--json",
            "workspace",
        ])
        .unwrap();
        assert_eq!(args.workspace.as_deref(), Some(Path::new("case")));
        let Action::Client {
            kind: ClientKind::Profile,
            arguments,
        } = args.action
        else {
            panic!("expected Profile client");
        };
        assert_eq!(
            arguments,
            [OsString::from("--json"), OsString::from("workspace")]
        );
        let args = Args::try_parse_from(["venture", "client", "atlas", "--help"]).unwrap();
        let Action::Client {
            kind: ClientKind::Atlas,
            arguments,
        } = args.action
        else {
            panic!("expected Atlas client");
        };
        assert_eq!(arguments, [OsString::from("--help")]);
    }
}
