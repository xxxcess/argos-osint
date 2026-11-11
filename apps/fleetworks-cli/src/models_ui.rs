//! Saved role defaults and their ordered fallback routes.

use std::io::BufRead;
use std::io::Write;

use anyhow::bail;
use anyhow::Context;
use anyhow::Result;
use fleetworks_contracts::Client;
use serde_json::Value;
use tokio::runtime::Runtime;

use crate::terminal::{self, field, text, Session};
use crate::ModelsCommand;

// Older servers did not advertise their role catalog. Their current pipeline
// contract has these six roles; new servers supply the authoritative list.
const LEGACY_SERVER_ROLES: [&str; 6] = [
    "recon_agent",
    "recon_decisions",
    "recon_analyst",
    "atlas_evaluator",
    "atlas_analyst",
    "brain_summary",
];

/// Projects only active roles, retaining the server's catalog order.
pub(crate) fn project_defaults(mut reply: Value) -> Result<Value> {
    let roles: Vec<String> = match reply.get("roles") {
        Some(roles) => {
            serde_json::from_value(roles.clone()).context("invalid API model role catalog")?
        }
        None => LEGACY_SERVER_ROLES
            .iter()
            .map(|role| (*role).to_owned())
            .collect(),
    };
    let saved = reply["defaults"]
        .as_object()
        .context("invalid API model defaults")?;
    let defaults = roles
        .iter()
        .map(|role| {
            (
                role.clone(),
                saved.get(role).cloned().unwrap_or(Value::Null),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    reply["defaults"] = Value::Object(defaults);
    reply["roles"] = serde_json::to_value(roles)?;
    Ok(reply)
}

const HELP: &str = "defaults [ROLE]                     List current roles and saved routes
set ROLE PROVIDER MODEL [ACCOUNT]    Confirm a primary route change
clear ROLE                           Confirm clearing all saved routes for a role
help | quit                          Show commands or close this session";

pub(crate) fn run(
    client: &Client,
    runtime: &Runtime,
    input: impl BufRead,
    mut output: impl Write,
) -> Result<()> {
    let session = Session { client, runtime };
    writeln!(output, "Fleetworks Models\n{HELP}")?;
    if let Err(error) = defaults(&session, None, &mut output) {
        writeln!(output, "{error}")?;
    }
    terminal::run(input, output, "models> ", |words, input, output| {
        match words {
            ["help"] => writeln!(output, "{HELP}")?,
            ["defaults"] | ["defaults", _] => defaults(&session, words.get(1).copied(), output)?,
            ["set", role, provider, model] | ["set", role, provider, model, _] => {
                let change = ModelsCommand::Set {
                    role: (*role).into(),
                    provider: (*provider).into(),
                    model: (*model).into(),
                    account: words.get(4).map(|value| (*value).into()),
                    fallbacks: None,
                };
                let question = format!("Set primary route for {} [yes/no] ", text(role, 80));
                save(&session, change, input, output, &question)?;
            }
            ["clear", role] => {
                let change = ModelsCommand::Clear {
                    role: (*role).into(),
                };
                let question = format!("Clear all saved routes for {} [yes/no] ", text(role, 80));
                save(&session, change, input, output, &question)?;
            }
            _ => bail!("unknown command or wrong number of arguments. Type help for commands."),
        }
        Ok(())
    })
}

fn save(
    session: &Session<'_>,
    change: ModelsCommand,
    input: &mut impl BufRead,
    output: &mut impl Write,
    question: &str,
) -> Result<()> {
    if !terminal::confirmed(input, output, question)? {
        writeln!(output, "No action sent.")?;
        return Ok(());
    }
    let receipt = session.request(change)?;
    if receipt["saved"].as_bool() != Some(true) {
        bail!("invalid model settings receipt");
    }
    writeln!(output, "Saved model defaults.")?;
    Ok(())
}

fn defaults(session: &Session<'_>, role: Option<&str>, output: &mut impl Write) -> Result<()> {
    let reply = session.request(ModelsCommand::Defaults)?;
    let values = reply["defaults"]
        .as_object()
        .context("invalid API model defaults")?;
    let roles = reply["roles"]
        .as_array()
        .context("invalid API model role catalog")?;
    let mut found = false;
    for role_name in roles {
        let name = role_name.as_str().context("invalid API model role")?;
        let route = &values[name];
        if role.is_some_and(|wanted| name != wanted) {
            continue;
        }
        found = true;
        writeln!(output, "{}", text(name, 80))?;
        write_route(output, "Primary", route)?;
        if let Some(fallbacks) = route["fallbacks"].as_array() {
            for (index, fallback) in fallbacks.iter().take(20).enumerate() {
                write_route(output, &format!("Fallback {}", index + 1), fallback)?;
            }
            if fallbacks.len() > 20 {
                writeln!(
                    output,
                    "  {} more in the one-shot JSON response.",
                    fallbacks.len() - 20
                )?;
            }
        }
    }
    if !found {
        writeln!(output, "No matching saved role defaults.")?;
    }
    Ok(())
}

fn write_route(output: &mut impl Write, label: &str, route: &Value) -> Result<()> {
    let model = field(route, "model");
    let provider = field(route, "provider");
    let account = field(route, "account");
    if model.is_empty() && provider.is_empty() && account.is_empty() {
        writeln!(output, "  {label}  No explicit route")?;
    } else {
        writeln!(
            output,
            "  {label}  {} / {}",
            text(provider, 80),
            text(model, 240)
        )?;
        if !account.is_empty() {
            writeln!(output, "    Account  {}", text(account, 128))?;
        }
    }
    Ok(())
}
