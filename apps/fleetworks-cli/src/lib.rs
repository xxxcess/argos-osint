//! Terminal commands over the versioned Fleetworks HTTP API.

mod jobs_ui;

use std::io;
use std::io::Write;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use clap::Args;
use clap::CommandFactory;
use clap::FromArgMatches;
use clap::Parser;
use clap::Subcommand;
use fleetworks_contracts::API_ORIGIN_ENV;
use fleetworks_contracts::ATLAS_CYCLES_PATH;
use fleetworks_contracts::DEFAULT_API_ORIGIN;
use fleetworks_contracts::JOBS_PATH;
use fleetworks_contracts::JOB_CANCEL_PATH;
use fleetworks_contracts::JOB_DETAIL_PATH;
use fleetworks_contracts::JOB_RETRY_PATH;
use fleetworks_contracts::{AtlasCycleRequest, Client};
use fleetworks_workspace::Workspace;
use serde_json::json;
use serde_json::Value;

#[derive(Args)]
struct Connection {
    /// Select the workspace directory.
    #[arg(long, global = true)]
    workspace: Option<PathBuf>,
    /// Override the API origin for this invocation.
    #[arg(long, global = true)]
    api_origin: Option<String>,
    /// Read a bearer token from this file.
    #[arg(long, global = true)]
    api_token_file: Option<PathBuf>,
    /// Emit compact JSON.
    #[arg(long, global = true)]
    json: bool,
}

#[derive(Parser)]
#[command(version)]
struct Cli<C: Subcommand> {
    #[command(flatten)]
    connection: Connection,
    #[command(subcommand)]
    command: C,
}

enum Operation {
    Workspace,
    JobsTui,
    Get(&'static str, Option<Value>),
    Post(&'static str, Value),
}

impl Operation {
    fn read(route: &'static str) -> Self {
        Self::Get(route, None)
    }

    fn query(route: &'static str, query: Value) -> Self {
        Self::Get(route, Some(query))
    }

    fn execute(self, connection: &Connection) -> Result<Option<Value>> {
        let workspace = Workspace::resolve(connection.workspace.as_deref())?;
        if matches!(self, Self::Workspace) {
            return Ok(Some(
                json!({"workspace": workspace.root().display().to_string()}),
            ));
        }
        anyhow::ensure!(
            !(matches!(self, Self::JobsTui) && connection.json),
            "the interactive Jobs & Logs view does not support --json"
        );
        let environment = workspace.environment()?;
        let origin = match &connection.api_origin {
            Some(origin) => origin.clone(),
            None => environment
                .get(API_ORIGIN_ENV)
                .map(|value| value.into_string().ok().context("API origin must be UTF-8"))
                .transpose()?
                .unwrap_or_else(|| DEFAULT_API_ORIGIN.to_owned()),
        };
        let token = match &connection.api_token_file {
            Some(path) => Some(workspace.read_api_token_file(path)?),
            None => workspace.api_token(&environment)?,
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let client = Client::new(&origin, token.as_deref())?;
        if matches!(self, Self::JobsTui) {
            jobs_ui::run(&client, &runtime, io::stdin().lock(), io::stdout().lock())?;
            return Ok(None);
        }
        Ok(Some(runtime.block_on(self.request(&client))?))
    }

    async fn request(self, client: &Client) -> Result<Value> {
        match self {
            Self::Get(route, None) => Ok(client.get(route).await?),
            Self::Get(route, Some(query)) => Ok(client.get_with_query(route, &query).await?),
            Self::Post(route, body) => Ok(client.post(route, &body).await?),
            Self::Workspace | Self::JobsTui => {
                unreachable!("local and interactive commands are handled before an HTTP request")
            }
        }
    }
}

fn run<C: Subcommand + Into<Operation>>(name: &'static str) -> Result<()> {
    let matches = Cli::<C>::command().name(name).get_matches();
    let args = Cli::<C>::from_arg_matches(&matches).unwrap_or_else(|error| error.exit());
    let Some(value) = args.command.into().execute(&args.connection)? else {
        return Ok(());
    };
    let mut output = io::stdout().lock();
    if args.connection.json {
        serde_json::to_writer(&mut output, &value)?;
    } else {
        serde_json::to_writer_pretty(&mut output, &value)?;
    }
    writeln!(output)?;
    Ok(())
}

#[derive(Subcommand)]
enum AtlasCommand {
    /// Queue a collection cycle. Reuse the request ID when retrying submission.
    Start {
        #[arg(long = "request-id", value_parser = parse_cycle_request)]
        request: AtlasCycleRequest,
    },
    /// List retained collection days and available categories.
    Days,
    /// List admitted articles for one day.
    Articles {
        day: String,
        #[arg(long, default_value = "all")]
        category: String,
        #[arg(long, value_parser = clap::value_parser!(u16).range(1..=100), default_value_t = 30)]
        limit: u16,
        #[arg(long)]
        cursor: Option<String>,
    },
}

fn parse_cycle_request(value: &str) -> Result<AtlasCycleRequest, String> {
    AtlasCycleRequest::new(value.to_owned())
        .ok_or_else(|| "expected 1–64 ASCII letters, digits, dashes or underscores".into())
}

impl From<AtlasCommand> for Operation {
    fn from(command: AtlasCommand) -> Self {
        match command {
            AtlasCommand::Start { request } => Self::Post(ATLAS_CYCLES_PATH, json!(request)),
            AtlasCommand::Days => Self::read("/api/v1/days"),
            AtlasCommand::Articles {
                day,
                category,
                limit,
                cursor,
            } => {
                let mut query = json!({"day": day, "category": category, "limit": limit});
                if let Some(cursor) = cursor {
                    query["cursor"] = json!(cursor);
                }
                Self::query("/api/v1/articles", query)
            }
        }
    }
}

#[derive(Subcommand)]
enum BrainCommand {
    /// Read an actor dossier by ID.
    Actor { id: String },
    /// Read the retained Intel brief for an article.
    Brief { run: String, id: String },
    /// Read topology for a Brain memory ID or brief:RUN/ARTICLE seed.
    Topology { seed: String },
}

impl From<BrainCommand> for Operation {
    fn from(command: BrainCommand) -> Self {
        match command {
            BrainCommand::Actor { id } => Self::query("/api/v1/actors/detail", json!({"id": id})),
            BrainCommand::Brief { run, id } => {
                Self::query("/api/v1/articles/detail", json!({"run": run, "id": id}))
            }
            BrainCommand::Topology { seed } => {
                Self::query("/api/v1/topology", json!({"seed": seed}))
            }
        }
    }
}

#[derive(Subcommand)]
enum JobsCommand {
    /// Open an interactive Jobs & Logs session.
    Tui,
    /// List recent jobs.
    List,
    /// Read job details and stage status.
    Detail { id: String },
    /// Requeue retryable failed tasks for a job.
    Retry { id: String },
    /// Cancel an active job that supports cancellation.
    Cancel { id: String },
    /// Read recent operational logs.
    Logs {
        #[arg(long)]
        job_id: Option<String>,
        #[arg(long, value_parser = clap::value_parser!(u16).range(1..=100), default_value_t = 50)]
        limit: u16,
    },
}

impl From<JobsCommand> for Operation {
    fn from(command: JobsCommand) -> Self {
        match command {
            JobsCommand::Tui => Self::JobsTui,
            JobsCommand::List => Self::read(JOBS_PATH),
            JobsCommand::Detail { id } => Self::query(JOB_DETAIL_PATH, json!({"id": id})),
            JobsCommand::Retry { id } => Self::Post(JOB_RETRY_PATH, json!({"id": id})),
            JobsCommand::Cancel { id } => Self::Post(JOB_CANCEL_PATH, json!({"id": id})),
            JobsCommand::Logs { job_id, limit } => {
                let mut query = json!({"limit": limit});
                if let Some(job_id) = job_id {
                    query["job_id"] = json!(job_id);
                }
                Self::query("/api/v1/logs", query)
            }
        }
    }
}

#[derive(Subcommand)]
enum ToolsCommand {
    /// Read the service tool catalog.
    List,
}

impl From<ToolsCommand> for Operation {
    fn from(command: ToolsCommand) -> Self {
        match command {
            ToolsCommand::List => Self::read("/api/v1/tools"),
        }
    }
}

#[derive(Subcommand)]
enum ModelsCommand {
    /// Read configured model defaults.
    Defaults,
}

impl From<ModelsCommand> for Operation {
    fn from(command: ModelsCommand) -> Self {
        match command {
            ModelsCommand::Defaults => Self::read("/api/v1/models/defaults"),
        }
    }
}

#[derive(Subcommand)]
enum ProfileCommand {
    /// Print the resolved local workspace without opening a connection.
    Workspace,
    /// Check API and database readiness.
    Health,
    /// Read service profile information.
    Show,
}

impl From<ProfileCommand> for Operation {
    fn from(command: ProfileCommand) -> Self {
        match command {
            ProfileCommand::Workspace => Self::Workspace,
            ProfileCommand::Health => Self::read("/api/v1/health/ready"),
            ProfileCommand::Show => Self::read("/api/v1/profile"),
        }
    }
}

/// Run the Atlas client.
pub fn atlas() -> Result<()> {
    run::<AtlasCommand>("fleetworks-atlas")
}

/// Run the Brain client.
pub fn brain() -> Result<()> {
    run::<BrainCommand>("fleetworks-brain")
}

/// Run the Jobs client.
pub fn jobs() -> Result<()> {
    run::<JobsCommand>("fleetworks-jobs")
}

/// Run the Tools client.
pub fn tools() -> Result<()> {
    run::<ToolsCommand>("fleetworks-tools")
}

/// Run the Models client.
pub fn models() -> Result<()> {
    run::<ModelsCommand>("fleetworks-models")
}

/// Run the Profile client.
pub fn profile() -> Result<()> {
    run::<ProfileCommand>("fleetworks-profile")
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use fleetworks_contracts::ATLAS_CYCLES_PATH;
    use serde_json::json;

    use super::{AtlasCommand, Cli, Operation};

    #[test]
    fn atlas_start_builds_the_versioned_submission() {
        let args = Cli::<AtlasCommand>::try_parse_from([
            "fleetworks-atlas",
            "start",
            "--request-id",
            "local_cycle-1",
        ])
        .unwrap();
        let Operation::Post(path, payload) = args.command.into() else {
            panic!("Atlas start must use HTTP POST");
        };
        assert_eq!(path, ATLAS_CYCLES_PATH);
        assert_eq!(payload, json!({"request_id":"local_cycle-1"}));
        assert!(Cli::<AtlasCommand>::try_parse_from([
            "fleetworks-atlas",
            "start",
            "--request-id",
            "invalid/id",
        ])
        .is_err());
    }
}
