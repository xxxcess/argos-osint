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
                write_field(output, "Tool", tool, "id", 160)?;
                write_field(output, "Description", tool, "description", 1000)?;
                let inputs = terminal::strings(tool, "inputs", 30);
                if !inputs.is_empty() {
                    writeln!(output, "  Inputs  {inputs}")?;
                }
                write_field(output, "Restrictions", tool, "restrictions", 1200)?;
                write_field(output, "Documentation", tool, "documentation", 1024)?;
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
