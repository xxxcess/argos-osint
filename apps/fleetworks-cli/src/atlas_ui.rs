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
use crate::tui::{Prompt, Step, View};
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

const PAGE_LIMIT: u16 = 30;

/// Ratatui Atlas screen: articles for one day and category, with the brief beside them.
pub(crate) struct AtlasView<'a> {
    session: Session<'a>,
    days: Vec<String>,
    categories: Vec<String>,
    day: usize,
    category: usize,
    page: Option<Page>,
    detail: Vec<String>,
    heading: String,
    request: Option<String>,
}

impl<'a> AtlasView<'a> {
    pub fn new(client: &'a Client, runtime: &'a Runtime) -> Self {
        Self {
            session: Session { client, runtime },
            days: Vec::new(),
            categories: vec!["all".into()],
            day: 0,
            category: 0,
            page: None,
            detail: Vec::new(),
            heading: "Overview".into(),
            request: None,
        }
    }

    fn overview(&mut self) {
        self.heading = "Overview".into();
        self.detail = vec![
            format!(
                "Day  {}",
                self.days.get(self.day).map_or("none", String::as_str)
            ),
            format!("Category  {}", self.categories[self.category]),
            String::new(),
            "[ ] previous/next day · c category · n next page · s start a cycle".into(),
        ];
        if self.days.is_empty() {
            self.detail.insert(0, "No retained collection days.".into());
        }
    }

    fn load(&mut self, cursor: Option<String>) -> Result<Step> {
        let Some(day) = self.days.get(self.day).cloned() else {
            self.page = None;
            self.overview();
            return Ok(Step::status("No retained collection days."));
        };
        let category = self.categories[self.category].clone();
        let reply = self.session.request(AtlasCommand::Articles {
            day: day.clone(),
            category: category.clone(),
            limit: PAGE_LIMIT,
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
        let count = rows.len();
        self.page = Some(Page {
            day,
            category,
            next_cursor,
            rows,
        });
        self.overview();
        Ok(Step::status(format!("{count} article(s)")))
    }

    fn open(&mut self, index: usize) -> Result<Step> {
        let Some(row) = self.page.as_ref().and_then(|page| page.rows.get(index)) else {
            return Ok(Step::none());
        };
        let run = terminal::field(row, "run_id");
        let id = terminal::field(row, "id");
        if run.is_empty() || id.is_empty() {
            bail!("article occurrence has no retained identity");
        }
        let reply = self.session.request(BrainCommand::Brief {
            run: run.into(),
            id: id.into(),
        })?;
        let mut buffer = Vec::new();
        write_brief(&reply, &mut buffer)?;
        self.heading = "Brief".into();
        self.detail = terminal::lines(&buffer);
        Ok(Step::none())
    }
}

impl View for AtlasView<'_> {
    fn title(&self) -> String {
        match (self.days.get(self.day), self.categories.get(self.category)) {
            (Some(day), Some(category)) => format!("Atlas · {day} · {category}"),
            _ => "Atlas".into(),
        }
    }

    fn hints(&self) -> &'static str {
        "[ ] day · c category · n next page · s start cycle"
    }

    fn list_title(&self) -> &'static str {
        "Articles"
    }

    fn detail_title(&self) -> String {
        self.heading.clone()
    }

    fn dense(&self) -> bool {
        true
    }

    fn rows(&self) -> Vec<String> {
        self.page
            .iter()
            .flat_map(|page| &page.rows)
            .map(|article| {
                format!(
                    "{}  [{}]",
                    terminal::text(terminal::field(article, "title"), 200),
                    terminal::text(terminal::field(article, "source_name"), 80)
                )
            })
            .collect()
    }

    fn detail(&self) -> Vec<String> {
        self.detail.clone()
    }

    fn refresh(&mut self) -> Result<Step> {
        let reply = self.session.request(AtlasCommand::Days)?;
        self.days = terminal::rows(&reply, "days")?
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        self.categories = std::iter::once("all".to_owned())
            .chain(
                terminal::rows(&reply, "categories")?
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned),
            )
            .collect();
        self.day = self.day.min(self.days.len().saturating_sub(1));
        self.category = self.category.min(self.categories.len() - 1);
        self.load(None)
    }

    fn select(&mut self, _index: usize) -> Result<Step> {
        Ok(Step::none())
    }

    fn activate(&mut self, index: usize) -> Result<Step> {
        self.open(index)
    }

    fn key(&mut self, key: char, _index: usize) -> Result<Step> {
        match key {
            '[' | ']' if !self.days.is_empty() => {
                self.day = if key == '[' {
                    self.day.saturating_sub(1)
                } else {
                    (self.day + 1).min(self.days.len() - 1)
                };
                self.load(None)
            }
            'c' => {
                self.category = (self.category + 1) % self.categories.len();
                self.load(None)
            }
            'n' => {
                let cursor = self
                    .page
                    .as_ref()
                    .context("list articles before requesting the next page")?
                    .next_cursor
                    .clone();
                match cursor {
                    Some(cursor) => self.load(Some(cursor)),
                    None => Ok(Step::status("No more articles on this date and category.")),
                }
            }
            's' => Ok(Step::prompt(Prompt::input(
                "request",
                "New collection request ID (1–64 letters, digits, dashes or underscores)",
                "",
            ))),
            _ => Ok(Step::none()),
        }
    }

    fn submit(&mut self, tag: &'static str, value: &str) -> Result<Step> {
        match tag {
            "request" => {
                parse_cycle_request(value).map_err(|error| anyhow!("{error}"))?;
                self.request = Some(value.to_owned());
                Ok(Step::prompt(Prompt::confirm(
                    "start",
                    format!("Submit Atlas cycle {}?", terminal::text(value, 64)),
                )))
            }
            "start" => {
                let id = self.request.take().context("no cycle request pending")?;
                let request = parse_cycle_request(&id).map_err(|error| anyhow!("{error}"))?;
                let reply = self.session.request(AtlasCommand::Start { request })?;
                let receipt: AtlasCycleReceipt =
                    serde_json::from_value(reply).context("invalid cycle receipt")?;
                self.heading = "Cycle".into();
                self.detail = vec![
                    format!("Accepted cycle {}", terminal::text(&receipt.run_id, 512)),
                    format!("  Job {}", terminal::text(&receipt.job_id, 512)),
                    "Reuse the same request ID when retrying an uncertain submission.".into(),
                ];
                Ok(Step::status("Cycle accepted."))
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
    fn atlas_screen_lists_articles_densely_and_validates_request_ids() {
        let (client, runtime) = offline();
        let mut view = AtlasView::new(&client, &runtime);
        view.days = vec!["2026-11-11".into()];
        view.categories = vec!["all".into(), "cyber".into()];
        view.page = Some(Page {
            day: "2026-11-11".into(),
            category: "all".into(),
            next_cursor: None,
            rows: vec![
                serde_json::json!({"title": "Port scan wave", "source_name": "Feed", "run_id": "r", "id": "a"}),
            ],
        });
        view.overview();
        assert!(view.dense());
        assert_eq!(view.rows(), ["Port scan wave  [Feed]"]);
        assert_eq!(view.title(), "Atlas · 2026-11-11 · all");
        assert!(view.submit("request", "bad/id").is_err());
        let prompt = view.submit("request", "review-1").unwrap().prompt.unwrap();
        assert_eq!(prompt.tag, "start");
        capture("atlas", &mut Shell::new(), &view);
    }
}
