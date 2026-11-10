use std::env;
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, ensure, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use fleetworks_contracts::{Client, API_ORIGIN_ENV, DEFAULT_API_ORIGIN};
use fleetworks_workspace::Workspace;
use tokio::process::Command;

#[derive(Parser)]
#[command(about = "Initialize Fleetworks workspaces and launch network clients")]
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
    /// Check readiness against the configured Fleetworks HTTP API.
    Status {
        #[arg(long)]
        api_origin: Option<String>,
        #[arg(long, value_name = "PATH")]
        api_token_file: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Launch a sibling client. Use `venture help client` for launcher help.
    #[command(disable_help_flag = true)]
    Client {
        #[arg(value_enum)]
        kind: ClientKind,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        arguments: Vec<OsString>,
    },
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
        Action::Status {
            api_origin,
            api_token_file,
            json,
        } => status(&workspace, api_origin, api_token_file.as_deref(), json).await,
        Action::Client { kind, arguments } => launch(&workspace, kind, arguments).await,
    }
}

async fn status(
    workspace: &Workspace,
    api_origin: Option<String>,
    api_token_file: Option<&Path>,
    json: bool,
) -> Result<()> {
    let environment = workspace.environment()?;
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
            .unwrap_or_else(|| DEFAULT_API_ORIGIN.into()),
    };
    let token = match api_token_file {
        Some(path) => Some(workspace.read_api_token_file(path)?),
        None => workspace.api_token(&environment)?,
    };
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
    let status = Command::new(&executable)
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
