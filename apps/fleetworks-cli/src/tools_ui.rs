//! Catalog browsing and explicit manual tool calls through the service.

use std::io::BufRead;
use std::io::Write;

use anyhow::anyhow;
use anyhow::bail;
use anyhow::Context;
use anyhow::Result;
use fleetworks_contracts::Client;
use serde_json::Value;
use tokio::runtime::Runtime;

use crate::parse_tool_input;
use crate::terminal::{self, field, text, write_field, Session};
use crate::tui::{Prompt, Step, View};
use crate::ToolsCommand;

const HELP: &str = "list [FILTER]      Refresh and filter the catalog by ID, name, or category
detail ID | #N     Show inputs and restrictions
run ID REQUEST_ID  Prompt for a JSON object and confirm submission
result CALL_ID     Show status and a bounded result preview
help | quit        Show commands or close this session";
const VISIBLE: usize = 30;

pub(crate) fn run(
    client: &Client,
    runtime: &Runtime,
    input: impl BufRead,
    mut output: impl Write,
) -> Result<()> {
    let mut tools = Tools {
        session: Session { client, runtime },
        catalog: Vec::new(),
        visible: Vec::new(),
    };
    writeln!(output, "Fleetworks Tools\n{HELP}")?;
    if let Err(error) = tools.list("", &mut output) {
        writeln!(output, "{error}")?;
    }
    terminal::run(input, output, "tools> ", |words, input, output| {
        tools.execute(words, input, output)
    })
}

struct Tools<'a> {
    session: Session<'a>,
    catalog: Vec<Value>,
    visible: Vec<usize>,
}

impl Tools<'_> {
    fn list(&mut self, filter: &str, output: &mut impl Write) -> Result<()> {
        let reply = self.session.request(ToolsCommand::List)?;
        let Value::Object(mut reply) = reply else {
            bail!("invalid API tool catalog");
        };
        let Some(Value::Array(catalog)) = reply.remove("tools") else {
            bail!("invalid API tool catalog");
        };
        let filter = filter.to_lowercase();
        let matches: Vec<_> = catalog
            .iter()
            .enumerate()
            .filter_map(|(index, tool)| {
                ["id", "name", "category"]
                    .iter()
                    .any(|key| field(tool, key).to_lowercase().contains(&filter))
                    .then_some(index)
            })
            .collect();
        if matches.is_empty() {
            writeln!(output, "No matching tools.")?;
        }
        for (row, index) in matches.iter().copied().take(VISIBLE).enumerate() {
            let tool = &catalog[index];
            writeln!(
                output,
                "#{}  {}  [{}]  ({})",
                row + 1,
                text(field(tool, "name"), 120),
                text(field(tool, "category"), 80),
                text(field(tool, "id"), 160),
            )?;
        }
        if matches.len() > VISIBLE {
            writeln!(
                output,
                "{} more tools. Filter the catalog with list FILTER.",
                matches.len() - VISIBLE
            )?;
        }
        self.catalog = catalog;
        self.visible = matches.into_iter().take(VISIBLE).collect();
        Ok(())
    }

    fn tool(&self, selector: &str) -> Result<&Value> {
        if selector.starts_with('#') {
            let row = terminal::row_index(selector)
                .and_then(|index| self.visible.get(index))
                .and_then(|index| self.catalog.get(*index));
            return row.context("no such row in the last tool list");
        }
        self.catalog
            .iter()
            .find(|tool| field(tool, "id") == selector)
            .context("unknown tool ID. Refresh the catalog with list.")
    }

    fn execute(
        &mut self,
        words: &[&str],
        input: &mut impl BufRead,
        output: &mut impl Write,
    ) -> Result<()> {
        match words {
            ["help"] => writeln!(output, "{HELP}")?,
            ["list", filter @ ..] => self.list(&filter.join(" "), output)?,
            ["detail", selector] => {
                let tool = self.tool(selector)?;
                write_tool_detail(tool, output)?;
            }
            ["run", selector, request_id] => {
                let tool_id = if selector.starts_with('#') {
                    field(self.tool(selector)?, "id")
                } else {
                    selector
                };
                let Some(line) = terminal::prompt(input, output, "inputs JSON> ")? else {
                    return Ok(());
                };
                let body = parse_tool_input(&line).map_err(|error| anyhow!("{error}"))?;
                let question = format!(
                    "Submit {} with request ID {} [yes/no] ",
                    text(tool_id, 160),
                    text(request_id, 64),
                );
                if !terminal::confirmed(input, output, &question)? {
                    writeln!(output, "No action sent.")?;
                    return Ok(());
                }
                let reply = self.session.request(ToolsCommand::Run {
                    tool_id: tool_id.into(),
                    request_id: (*request_id).into(),
                    input: body,
                })?;
                let id = reply["call_id"].as_str().context("invalid tool receipt")?;
                writeln!(
                    output,
                    "Accepted call {}\nUse result CALL_ID to check status.",
                    text(id, 512)
                )?;
            }
            ["result", id] => {
                let reply = self
                    .session
                    .request(ToolsCommand::Result { id: (*id).into() })?;
                write_result(&reply, output)?;
            }
            _ => bail!("unknown command or wrong number of arguments. Type help for commands."),
        }
        Ok(())
    }
}

fn write_result(reply: &Value, output: &mut impl Write) -> Result<()> {
    let status = reply["status"].as_str().context("invalid tool result")?;
    writeln!(
        output,
        "Call {}  [{}]",
        text(field(reply, "call_id"), 512),
        text(status, 80)
    )?;
    write_field(output, "Tool", reply, "tool_id", 160)?;
    let result = &reply["result"];
    if result.is_null() {
        return Ok(());
    }
    write_field(output, "Source", result, "source_url", 1024)?;
    write_field(output, "Error", result, "error", 512)?;
    if let Some(credits) = result["credits_charged"].as_u64() {
        writeln!(output, "  Credits charged  {credits}")?;
    }
    if !result["observations"].is_null() {
        let observations = result["observations"].to_string();
        writeln!(output, "  Observations  {}", text(&observations, 1200))?;
        writeln!(
            output,
            "Use the one-shot result command for the complete response."
        )?;
    }
    Ok(())
}

fn write_tool_detail(tool: &Value, output: &mut impl Write) -> Result<()> {
    write_field(output, "Tool", tool, "id", 160)?;
    write_field(output, "Description", tool, "description", 1000)?;
    let inputs = terminal::strings(tool, "inputs", 30);
    if !inputs.is_empty() {
        writeln!(output, "  Inputs  {inputs}")?;
    }
    write_field(output, "Restrictions", tool, "restrictions", 1200)?;
    write_field(output, "Documentation", tool, "documentation", 1024)
}

/// Ratatui Tools screen: filtered catalog, tool detail, manual calls and results.
pub(crate) struct ToolsView<'a> {
    session: Session<'a>,
    catalog: Vec<Value>,
    filter: String,
    detail: Vec<String>,
    heading: String,
    call: Option<(String, String, Option<Value>)>,
}

impl<'a> ToolsView<'a> {
    pub fn new(client: &'a Client, runtime: &'a Runtime) -> Self {
        Self {
            session: Session { client, runtime },
            catalog: Vec::new(),
            filter: String::new(),
            detail: Vec::new(),
            heading: "Tool".into(),
            call: None,
        }
    }

    fn visible(&self) -> Vec<&Value> {
        let filter = self.filter.to_lowercase();
        self.catalog
            .iter()
            .filter(|tool| {
                ["id", "name", "category"]
                    .iter()
                    .any(|key| field(tool, key).to_lowercase().contains(&filter))
            })
            .collect()
    }

    fn show_tool(&mut self, index: usize) -> Result<Step> {
        let Some(tool) = self.visible().get(index).copied() else {
            return Ok(Step::none());
        };
        let mut buffer = Vec::new();
        write_tool_detail(tool, &mut buffer)?;
        self.heading = "Tool".into();
        self.detail = terminal::lines(&buffer);
        Ok(Step::none())
    }
}

impl View for ToolsView<'_> {
    fn title(&self) -> String {
        if self.filter.is_empty() {
            "Tools".into()
        } else {
            format!("Tools · filter {}", text(&self.filter, 64))
        }
    }

    fn hints(&self) -> &'static str {
        "/ filter · x run · R result"
    }

    fn list_title(&self) -> &'static str {
        "Catalog"
    }

    fn detail_title(&self) -> String {
        self.heading.clone()
    }

    fn rows(&self) -> Vec<String> {
        self.visible()
            .into_iter()
            .map(|tool| {
                format!(
                    "{}  [{}]  ({})",
                    text(field(tool, "name"), 120),
                    text(field(tool, "category"), 80),
                    text(field(tool, "id"), 160)
                )
            })
            .collect()
    }

    fn detail(&self) -> Vec<String> {
        self.detail.clone()
    }

    fn refresh(&mut self) -> Result<Step> {
        let reply = self.session.request(ToolsCommand::List)?;
        let Value::Object(mut reply) = reply else {
            bail!("invalid API tool catalog");
        };
        let Some(Value::Array(catalog)) = reply.remove("tools") else {
            bail!("invalid API tool catalog");
        };
        self.catalog = catalog;
        let count = self.visible().len();
        if count > 0 {
            self.show_tool(0)?;
        } else {
            self.detail.clear();
        }
        Ok(Step::status(format!("{count} tool(s)")))
    }

    fn select(&mut self, index: usize) -> Result<Step> {
        self.show_tool(index)
    }

    fn activate(&mut self, index: usize) -> Result<Step> {
        self.show_tool(index)
    }

    fn key(&mut self, key: char, index: usize) -> Result<Step> {
        Ok(match key {
            '/' => Step::prompt(Prompt::input(
                "filter",
                "Filter by ID, name or category",
                self.filter.clone(),
            )),
            'x' => {
                let Some(tool) = self.visible().get(index).copied() else {
                    return Ok(Step::status("No tool selected."));
                };
                let id = field(tool, "id").to_owned();
                let label = format!("Request ID for {}", text(&id, 160));
                self.call = Some((id, String::new(), None));
                Step::prompt(Prompt::input("run-id", label, ""))
            }
            'R' => Step::prompt(Prompt::input("result", "Call ID", "")),
            _ => Step::none(),
        })
    }

    fn submit(&mut self, tag: &'static str, value: &str) -> Result<Step> {
        match tag {
            "filter" => {
                self.filter = value.to_owned();
                self.detail.clear();
                let count = self.visible().len();
                if count > 0 {
                    self.show_tool(0)?;
                }
                Ok(Step::status(format!("{count} tool(s)")))
            }
            "run-id" => {
                if let Some(call) = self.call.as_mut() {
                    call.1 = value.to_owned();
                }
                Ok(Step::prompt(Prompt::input(
                    "run-json",
                    "Inputs as a JSON object",
                    "{}",
                )))
            }
            "run-json" => {
                let body = parse_tool_input(value).map_err(|error| anyhow!("{error}"))?;
                let call = self.call.as_mut().context("no tool call pending")?;
                call.2 = Some(body);
                let question = format!(
                    "Submit {} with request ID {}?",
                    text(&call.0, 160),
                    text(&call.1, 64)
                );
                Ok(Step::prompt(Prompt::confirm("run", question)))
            }
            "run" => {
                let (tool_id, request_id, input) =
                    self.call.take().context("no tool call pending")?;
                let input = input.context("no tool inputs pending")?;
                let reply = self.session.request(ToolsCommand::Run {
                    tool_id,
                    request_id,
                    input,
                })?;
                let id = reply["call_id"].as_str().context("invalid tool receipt")?;
                self.heading = "Call".into();
                self.detail = vec![
                    format!("Accepted call {}", text(id, 512)),
                    "Press R with this call ID to check status.".into(),
                ];
                Ok(Step::status("Call accepted."))
            }
            "result" => {
                let reply = self
                    .session
                    .request(ToolsCommand::Result { id: value.into() })?;
                let mut buffer = Vec::new();
                write_result(&reply, &mut buffer)?;
                self.heading = "Result".into();
                self.detail = terminal::lines(&buffer);
                Ok(Step::none())
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

    fn seeded<'a>(client: &'a Client, runtime: &'a Runtime) -> ToolsView<'a> {
        let mut view = ToolsView::new(client, runtime);
        view.catalog = vec![
            serde_json::json!({"id": "web.fetch", "name": "Fetch", "category": "web", "description": "Fetch a page", "inputs": ["url"]}),
            serde_json::json!({"id": "dns.lookup", "name": "DNS lookup", "category": "network"}),
        ];
        view
    }

    #[test]
    fn tools_screen_filters_the_catalog() {
        let (client, runtime) = offline();
        let mut view = seeded(&client, &runtime);
        assert_eq!(view.rows().len(), 2);
        view.submit("filter", "dns").unwrap();
        assert_eq!(view.rows(), ["DNS lookup  [network]  (dns.lookup)"]);
        assert!(view.title().contains("filter dns"));
        view.submit("filter", "web").unwrap();
        capture("tools", &mut Shell::new(), &view);
    }

    #[test]
    fn run_collects_id_then_json_then_confirmation() {
        let (client, runtime) = offline();
        let mut view = seeded(&client, &runtime);
        assert_eq!(view.key('x', 0).unwrap().prompt.unwrap().tag, "run-id");
        assert_eq!(
            view.submit("run-id", "req-1").unwrap().prompt.unwrap().tag,
            "run-json"
        );
        assert!(view.submit("run-json", "[1]").is_err());
        let prompt = view
            .submit("run-json", "{\"url\":\"x\"}")
            .unwrap()
            .prompt
            .unwrap();
        assert_eq!(prompt.tag, "run");
        assert!(prompt.label.contains("web.fetch") && prompt.label.contains("req-1"));
    }
}
