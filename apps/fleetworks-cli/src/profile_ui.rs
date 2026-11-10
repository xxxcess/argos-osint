//! Local workspace identity and remote service readiness.

use std::io::BufRead;
use std::io::Write;
use std::path::Path;

use anyhow::bail;
use anyhow::Context;
use anyhow::Result;
use fleetworks_contracts::Client;
use serde_json::Value;
use tokio::runtime::Runtime;

use crate::terminal::{self, text, write_field, Session};
use crate::ProfileCommand;

const HELP: &str = "workspace         Show the resolved local workspace
health            Check API and datastore readiness
show              Show the service profile and its workspace
export NAME       Confirm a private portable configuration export
help | quit       Show commands or close this session";

pub(crate) fn run(
    client: &Client,
    runtime: &Runtime,
    workspace: &Path,
    input: impl BufRead,
    mut output: impl Write,
) -> Result<()> {
    let session = Session { client, runtime };
    writeln!(output, "Fleetworks Profile\n{HELP}")?;
    terminal::run(input, output, "profile> ", |words, input, output| {
        match words {
            ["help"] => writeln!(output, "{HELP}")?,
            ["workspace"] => {
                writeln!(
                    output,
                    "Local workspace  {}",
                    text(&workspace.to_string_lossy(), 2048)
                )?;
            }
            ["health"] => {
                let reply = session.request(ProfileCommand::Health)?;
                write_field(output, "Service", &reply, "service", 120)?;
                if let Some(version) = reply["api_version"].as_u64() {
                    writeln!(output, "  API version  {version}")?;
                }
                write_health(&reply, output)?;
            }
            ["show"] => {
                let reply = session.request(ProfileCommand::Show)?;
                write_field(output, "Service workspace", &reply, "workspace", 2048)?;
                write_field(output, "Access", &reply, "access_mode", 80)?;
                write_health(&reply, output)?;
            }
            ["export", name] => {
                let question = format!(
                    "Export saved configuration and credentials as {} [yes/no] ",
                    text(name, 64),
                );
                if !terminal::confirmed(input, output, &question)? {
                    writeln!(output, "No action sent.")?;
                    return Ok(());
                }
                let receipt = session.request(ProfileCommand::Export {
                    name: (*name).into(),
                })?;
                let path = receipt["path"]
                    .as_str()
                    .context("invalid profile export receipt")?;
                writeln!(
                    output,
                    "Saved {} in the service workspace.",
                    text(path, 1024)
                )?;
            }
            _ => bail!("unknown command or wrong number of arguments. Type help for commands."),
        }
        Ok(())
    })
}

fn write_health(reply: &Value, output: &mut impl Write) -> Result<()> {
    for (key, label) in [("ready", "Readiness"), ("storage", "Datastore")] {
        let status = reply[key]
            .as_bool()
            .context("invalid API readiness response")?;
        writeln!(
            output,
            "  {label}  {}",
            if status { "Ready" } else { "Unavailable" }
        )?;
    }
    write_field(output, "Instance", reply, "instance_id", 160)
}
