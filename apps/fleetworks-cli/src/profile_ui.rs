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
use crate::tui::{Prompt, Step, View};
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

const SECTIONS: [&str; 4] = [
    "Local workspace",
    "Service health",
    "Service profile",
    "Export configuration",
];

/// Ratatui Profile screen: workspace identity, readiness and configuration export.
pub(crate) struct ProfileView<'a> {
    session: Session<'a>,
    workspace: &'a Path,
    shown: usize,
    detail: Vec<String>,
    export: Option<String>,
}

impl<'a> ProfileView<'a> {
    pub fn new(client: &'a Client, runtime: &'a Runtime, workspace: &'a Path) -> Self {
        Self {
            session: Session { client, runtime },
            workspace,
            shown: 0,
            detail: Vec::new(),
            export: None,
        }
    }

    fn load(&mut self, index: usize) -> Result<Step> {
        self.shown = index;
        let mut buffer = Vec::new();
        match index {
            0 => writeln!(
                buffer,
                "Local workspace  {}",
                text(&self.workspace.to_string_lossy(), 2048)
            )?,
            1 => {
                let reply = self.session.request(ProfileCommand::Health)?;
                write_field(&mut buffer, "Service", &reply, "service", 120)?;
                if let Some(version) = reply["api_version"].as_u64() {
                    writeln!(buffer, "  API version  {version}")?;
                }
                write_health(&reply, &mut buffer)?;
            }
            2 => {
                let reply = self.session.request(ProfileCommand::Show)?;
                write_field(&mut buffer, "Service workspace", &reply, "workspace", 2048)?;
                write_field(&mut buffer, "Access", &reply, "access_mode", 80)?;
                write_health(&reply, &mut buffer)?;
            }
            _ => {
                writeln!(
                    buffer,
                    "Export saved configuration and credentials to a private file in the service workspace."
                )?;
                writeln!(buffer, "Press e to choose a name.")?;
            }
        }
        self.detail = terminal::lines(&buffer);
        Ok(Step::none())
    }
}

impl View for ProfileView<'_> {
    fn title(&self) -> String {
        "Profile".into()
    }

    fn hints(&self) -> &'static str {
        "e export configuration"
    }

    fn list_title(&self) -> &'static str {
        "Profile"
    }

    fn detail_title(&self) -> String {
        SECTIONS[self.shown].into()
    }

    fn rows(&self) -> Vec<String> {
        SECTIONS
            .iter()
            .map(|section| (*section).to_owned())
            .collect()
    }

    fn detail(&self) -> Vec<String> {
        self.detail.clone()
    }

    fn refresh(&mut self) -> Result<Step> {
        self.load(self.shown)
    }

    fn select(&mut self, index: usize) -> Result<Step> {
        self.load(index)
    }

    fn activate(&mut self, index: usize) -> Result<Step> {
        if index == 3 {
            return self.key('e', index);
        }
        self.load(index)
    }

    fn key(&mut self, key: char, _index: usize) -> Result<Step> {
        Ok(match key {
            'e' => Step::prompt(Prompt::input("export", "Export name", "")),
            _ => Step::none(),
        })
    }

    fn submit(&mut self, tag: &'static str, value: &str) -> Result<Step> {
        match tag {
            "export" => {
                self.export = Some(value.to_owned());
                Ok(Step::prompt(Prompt::confirm(
                    "export-confirm",
                    format!(
                        "Export saved configuration and credentials as {}?",
                        text(value, 64)
                    ),
                )))
            }
            "export-confirm" => {
                let name = self.export.take().context("no export pending")?;
                let receipt = self.session.request(ProfileCommand::Export { name })?;
                let path = receipt["path"]
                    .as_str()
                    .context("invalid profile export receipt")?;
                self.detail = vec![format!(
                    "Saved {} in the service workspace.",
                    text(path, 1024)
                )];
                Ok(Step::status("Configuration exported."))
            }
            _ => Ok(Step::none()),
        }
    }
}

#[cfg(test)]
mod view_tests {
    use super::*;
    use crate::tui::testing::{capture, offline};
    use crate::tui::Shell;

    #[test]
    fn profile_screen_shows_the_local_workspace_without_a_request() {
        let (client, runtime) = offline();
        let path = Path::new("/tmp/fleetworks-workspace");
        let mut view = ProfileView::new(&client, &runtime, path);
        view.refresh().unwrap();
        assert!(view.detail()[0].contains("/tmp/fleetworks-workspace"));
        assert_eq!(view.rows().len(), 4);
        capture("profile", &mut Shell::new(), &view);
    }

    #[test]
    fn export_requires_a_name_then_a_confirmation() {
        let (client, runtime) = offline();
        let mut view = ProfileView::new(&client, &runtime, Path::new("/w"));
        assert_eq!(view.activate(3).unwrap().prompt.unwrap().tag, "export");
        let prompt = view.submit("export", "backup").unwrap().prompt.unwrap();
        assert_eq!(prompt.tag, "export-confirm");
        assert!(prompt.label.contains("backup"));
    }
}
