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
use crate::tui::{Prompt, Step, View};
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

/// Ratatui Models screen: one row per canonical role with its saved routes beside it.
pub(crate) struct ModelsView<'a> {
    session: Session<'a>,
    roles: Vec<String>,
    defaults: Value,
    shown: usize,
    pending: Option<ModelsCommand>,
}

impl<'a> ModelsView<'a> {
    pub fn new(client: &'a Client, runtime: &'a Runtime) -> Self {
        Self {
            session: Session { client, runtime },
            roles: Vec::new(),
            defaults: Value::Null,
            shown: 0,
            pending: None,
        }
    }

    fn route_summary(&self, role: &str) -> String {
        let route = &self.defaults[role];
        let model = field(route, "model");
        let provider = field(route, "provider");
        if model.is_empty() && provider.is_empty() {
            "No explicit route".into()
        } else {
            format!("{} / {}", text(provider, 80), text(model, 240))
        }
    }

    fn role(&self, index: usize) -> Option<&String> {
        self.roles.get(index)
    }
}

impl View for ModelsView<'_> {
    fn title(&self) -> String {
        "Models".into()
    }

    fn hints(&self) -> &'static str {
        "e set primary route · x clear role"
    }

    fn list_title(&self) -> &'static str {
        "Roles"
    }

    fn detail_title(&self) -> String {
        "Routes".into()
    }

    fn rows(&self) -> Vec<String> {
        self.roles
            .iter()
            .map(|role| format!("{}  {}", text(role, 80), self.route_summary(role)))
            .collect()
    }

    fn detail(&self) -> Vec<String> {
        let Some(role) = self.roles.get(self.shown) else {
            return Vec::new();
        };
        let route = &self.defaults[role.as_str()];
        let mut buffer = Vec::new();
        let _ = writeln!(buffer, "{}", text(role, 80));
        let _ = write_route(&mut buffer, "Primary", route);
        if let Some(fallbacks) = route["fallbacks"].as_array() {
            for (index, fallback) in fallbacks.iter().take(20).enumerate() {
                let _ = write_route(&mut buffer, &format!("Fallback {}", index + 1), fallback);
            }
            if fallbacks.len() > 20 {
                let _ = writeln!(
                    buffer,
                    "  {} more in the one-shot JSON response.",
                    fallbacks.len() - 20
                );
            }
        }
        terminal::lines(&buffer)
    }

    fn refresh(&mut self) -> Result<Step> {
        let reply = self.session.request(ModelsCommand::Defaults)?;
        self.roles = serde_json::from_value(reply["roles"].clone())
            .context("invalid API model role catalog")?;
        self.defaults = reply["defaults"].clone();
        if !self.defaults.is_object() {
            bail!("invalid API model defaults");
        }
        Ok(Step::status(format!("{} role(s)", self.roles.len())))
    }

    fn select(&mut self, index: usize) -> Result<Step> {
        self.shown = index;
        Ok(Step::none())
    }

    fn activate(&mut self, index: usize) -> Result<Step> {
        self.shown = index;
        Ok(Step::none())
    }

    fn key(&mut self, key: char, index: usize) -> Result<Step> {
        let Some(role) = self.role(index).cloned() else {
            return Ok(Step::none());
        };
        Ok(match key {
            'e' => {
                let route = &self.defaults[role.as_str()];
                let initial = format!("{} {}", field(route, "provider"), field(route, "model"));
                self.pending = Some(ModelsCommand::Clear { role: role.clone() });
                Step::prompt(Prompt::input(
                    "set",
                    format!(
                        "Primary route for {}: PROVIDER MODEL [ACCOUNT]",
                        text(&role, 80)
                    ),
                    initial.trim(),
                ))
            }
            'x' => {
                self.pending = Some(ModelsCommand::Clear { role: role.clone() });
                Step::prompt(Prompt::confirm(
                    "clear",
                    format!("Clear all saved routes for {}?", text(&role, 80)),
                ))
            }
            _ => Step::none(),
        })
    }

    fn submit(&mut self, tag: &'static str, value: &str) -> Result<Step> {
        let change = match tag {
            "set" => {
                let Some(ModelsCommand::Clear { role }) = self.pending.take() else {
                    return Ok(Step::none());
                };
                let words: Vec<&str> = value.split_whitespace().collect();
                let (provider, model, account) = match words[..] {
                    [provider, model] => (provider, model, None),
                    [provider, model, account] => (provider, model, Some(account)),
                    _ => bail!("expected PROVIDER MODEL [ACCOUNT]"),
                };
                let question = format!(
                    "Set primary route for {} to {} / {}?",
                    text(&role, 80),
                    text(provider, 80),
                    text(model, 240)
                );
                self.pending = Some(ModelsCommand::Set {
                    role,
                    provider: provider.into(),
                    model: model.into(),
                    account: account.map(Into::into),
                    fallbacks: None,
                });
                return Ok(Step::prompt(Prompt::confirm("set-confirm", question)));
            }
            "set-confirm" | "clear" => self.pending.take(),
            _ => None,
        };
        let Some(change) = change else {
            return Ok(Step::none());
        };
        let receipt = self.session.request(change)?;
        if receipt["saved"].as_bool() != Some(true) {
            bail!("invalid model settings receipt");
        }
        self.refresh()?;
        Ok(Step::status("Saved model defaults."))
    }
}

#[cfg(test)]
mod view_tests {
    use super::*;
    use crate::tui::testing::{capture, offline};
    use crate::tui::Shell;

    fn seeded<'a>(client: &'a Client, runtime: &'a Runtime) -> ModelsView<'a> {
        let mut view = ModelsView::new(client, runtime);
        view.roles = vec!["recon_agent".into(), "atlas_analyst".into()];
        view.defaults = serde_json::json!({
            "recon_agent": {"provider": "openai", "model": "gpt-x", "fallbacks": [{"provider": "anthropic", "model": "claude-y"}]},
            "atlas_analyst": null
        });
        view
    }

    #[test]
    fn models_screen_lists_roles_and_routes() {
        let (client, runtime) = offline();
        let view = seeded(&client, &runtime);
        assert_eq!(view.rows()[0], "recon_agent  openai / gpt-x");
        assert_eq!(view.rows()[1], "atlas_analyst  No explicit route");
        assert!(view.detail().iter().any(|line| line.contains("Fallback 1")));
        capture("models", &mut Shell::new(), &view);
    }

    #[test]
    fn set_goes_input_then_confirm_and_clear_confirms() {
        let (client, runtime) = offline();
        let mut view = seeded(&client, &runtime);
        let step = view.key('e', 0).unwrap();
        assert_eq!(step.prompt.unwrap().initial, "openai gpt-x");
        let step = view.submit("set", "openai gpt-z").unwrap();
        let prompt = step.prompt.unwrap();
        assert_eq!(prompt.tag, "set-confirm");
        assert!(prompt.label.contains("gpt-z"));
        view.key('e', 0).unwrap();
        assert!(view.submit("set", "only-one").is_err());
        assert_eq!(view.key('x', 1).unwrap().prompt.unwrap().tag, "clear");
    }
}
