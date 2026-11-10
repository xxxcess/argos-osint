//! Interactive Atlas collection and browsing over the network API.

use std::io::BufRead;
use std::io::Write;

use anyhow::anyhow;
use anyhow::bail;
use anyhow::Context;
use anyhow::Result;
use fleetworks_contracts::AtlasCycleReceipt;
use fleetworks_contracts::Client;
use serde_json::Value;
use tokio::runtime::Runtime;

use crate::brain_ui::write_brief;
use crate::terminal;
use crate::terminal::Session;
use crate::{parse_cycle_request, AtlasCommand, BrainCommand};

const HELP: &str = "days                      Refresh retained dates and categories
articles [DAY [CATEGORY]]  Show a page (default: newest retained day, all categories)
next                      Show the next page
open #N                   Read the selected article and its retained brief
start REQUEST_ID          Confirm and submit a collection cycle
help | quit               Show commands or close this session";

pub(crate) fn run(
    client: &Client,
    runtime: &Runtime,
    input: impl BufRead,
    mut output: impl Write,
) -> Result<()> {
    let mut atlas = Atlas {
        session: Session { client, runtime },
        latest_day: None,
        page: None,
    };
    writeln!(output, "Fleetworks Atlas\n{HELP}")?;
    if let Err(error) = atlas.days(&mut output) {
        writeln!(output, "{error}")?;
    }
    terminal::run(input, output, "atlas> ", |words, input, output| {
        atlas.execute(words, input, output)
    })
}

struct Atlas<'a> {
    session: Session<'a>,
    latest_day: Option<String>,
    page: Option<Page>,
}

struct Page {
    day: String,
    category: String,
    next_cursor: Option<String>,
    rows: Vec<Value>,
}

impl Atlas<'_> {
    fn days(&mut self, output: &mut impl Write) -> Result<()> {
        let reply = self.session.request(AtlasCommand::Days)?;
        let days = terminal::rows(&reply, "days")?;
        self.latest_day = days.first().and_then(Value::as_str).map(str::to_owned);
        if days.is_empty() {
            writeln!(output, "No retained collection days.")?;
        } else {
            writeln!(
                output,
                "Recent days  {}",
                terminal::strings(&reply, "days", 12)
            )?;
        }
        let categories = terminal::strings(&reply, "categories", 20);
        if !categories.is_empty() {
            writeln!(output, "Categories  all, {categories}")?;
        }
        Ok(())
    }

    fn articles(
        &mut self,
        day: String,
        category: String,
        cursor: Option<String>,
        output: &mut impl Write,
    ) -> Result<()> {
        let reply = self.session.request(AtlasCommand::Articles {
            day: day.clone(),
            category: category.clone(),
            limit: 30,
            cursor,
        })?;
        let Value::Object(mut reply) = reply else {
            bail!("invalid API article page");
        };
        let Some(Value::Array(rows)) = reply.remove("rows") else {
            bail!("invalid API article rows");
        };
        let next_cursor = match reply.remove("next_cursor") {
            None | Some(Value::Null) => None,
            Some(Value::String(cursor)) => Some(cursor),
            _ => bail!("invalid API article cursor"),
        };
        writeln!(
            output,
            "{}  [{}]",
            terminal::text(&day, 32),
            terminal::text(&category, 64)
        )?;
        if rows.is_empty() {
            writeln!(output, "No admitted articles on this page.")?;
        }
        for (index, article) in rows.iter().enumerate() {
            let title = terminal::field(article, "title");
            let source = terminal::field(article, "source_name");
            writeln!(
                output,
                "#{}  {}  [{}]",
                index + 1,
                terminal::text(title, 200),
                terminal::text(source, 80),
            )?;
        }
        if next_cursor.is_some() {
            writeln!(output, "Type next for more articles.")?;
        }
        self.page = Some(Page {
            day,
            category,
            next_cursor,
            rows,
        });
        Ok(())
    }

    fn execute(
        &mut self,
        words: &[&str],
        input: &mut impl BufRead,
        output: &mut impl Write,
    ) -> Result<()> {
        match words {
            ["help"] => writeln!(output, "{HELP}")?,
            ["days"] => self.days(output)?,
            ["articles"] | ["articles", _] | ["articles", _, _] => {
                let day = words
                    .get(1)
                    .copied()
                    .or(self.latest_day.as_deref())
                    .context("no retained day selected. Use articles DAY.")?;
                let category = words.get(2).copied().unwrap_or("all");
                self.articles(day.to_owned(), category.to_owned(), None, output)?;
            }
            ["next"] => {
                let page = self
                    .page
                    .as_ref()
                    .context("list articles before requesting the next page")?;
                let cursor = page
                    .next_cursor
                    .clone()
                    .context("no more articles on this date and category")?;
                self.articles(
                    page.day.clone(),
                    page.category.clone(),
                    Some(cursor),
                    output,
                )?;
            }
            ["open", selector] => {
                let row = self
                    .page
                    .as_ref()
                    .and_then(|page| {
                        terminal::row_index(selector).and_then(|index| page.rows.get(index))
                    })
                    .context("no such row in the last article page. Use a row such as #1.")?;
                let run = terminal::field(row, "run_id");
                let id = terminal::field(row, "id");
                if run.is_empty() || id.is_empty() {
                    bail!("article occurrence has no retained identity");
                }
                let reply = self.session.request(BrainCommand::Brief {
                    run: run.into(),
                    id: id.into(),
                })?;
                write_brief(&reply, output)?;
            }
            ["start", request_id] => {
                let request =
                    parse_cycle_request(request_id).map_err(|error| anyhow!("{error}"))?;
                let question = format!("Submit Atlas cycle {request_id} [yes/no] ");
                if !terminal::confirmed(input, output, &question)? {
                    writeln!(output, "No action sent.")?;
                    return Ok(());
                }
                let reply = self.session.request(AtlasCommand::Start { request })?;
                let receipt: AtlasCycleReceipt =
                    serde_json::from_value(reply).context("invalid cycle receipt")?;
                writeln!(
                    output,
                    "Accepted cycle {}\n  Job {}\nReuse the same request ID when retrying an uncertain submission.",
                    terminal::text(&receipt.run_id, 512),
                    terminal::text(&receipt.job_id, 512),
                )?;
            }
            _ => bail!("unknown command or wrong number of arguments. Type help for commands."),
        }
        Ok(())
    }
}
