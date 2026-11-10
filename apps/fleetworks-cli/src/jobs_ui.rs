//! Line-oriented Jobs & Logs view over the same network commands as the CLI.

use std::io::{BufRead, Write};

use anyhow::{bail, Context, Result};
use fleetworks_contracts::{Client, JobSummary, Jobs, JOBS_PATH};
use tokio::runtime::Runtime;

use crate::terminal::{self, field, rows, Session};
use crate::JobsCommand;

const HELP: &str = "list                  Refresh recent jobs
detail ID             Show a job and its stages
logs [ID]             Show recent events, optionally for one job
retry ID | cancel ID  Confirm and submit a job action
help | quit           Show commands or close this session
Use a job ID or a row number such as #1 from the last list.";

pub(crate) fn run(
    client: &Client,
    runtime: &Runtime,
    input: impl BufRead,
    mut output: impl Write,
) -> Result<()> {
    let mut dashboard = Dashboard {
        session: Session { client, runtime },
        rows: Vec::new(),
    };
    writeln!(output, "Fleetworks Jobs & Logs\n{HELP}")?;
    if let Err(error) = dashboard.refresh(&mut output) {
        writeln!(output, "{error}")?;
    }
    terminal::run(input, output, "jobs> ", |words, input, output| {
        dashboard.execute(words, input, output)
    })
}

struct Dashboard<'a> {
    session: Session<'a>,
    rows: Vec<JobSummary>,
}

impl Dashboard<'_> {
    fn refresh(&mut self, output: &mut impl Write) -> Result<()> {
        let jobs: Jobs = self
            .session
            .runtime
            .block_on(self.session.client.get(JOBS_PATH))?;
        self.rows = jobs.rows;
        if self.rows.is_empty() {
            writeln!(output, "No recent jobs.")?;
        }
        for (index, row) in self.rows.iter().enumerate() {
            writeln!(
                output,
                "#{}  [{}] {}  ({})",
                index + 1,
                row.state.escape_debug(),
                row.title.escape_debug(),
                row.id.escape_debug()
            )?;
        }
        Ok(())
    }

    fn job_id(&self, selector: &str) -> Result<String> {
        if !selector.starts_with('#') {
            return Ok(selector.to_owned());
        }
        let row = terminal::row_index(selector)
            .and_then(|index| self.rows.get(index))
            .context("no such row in the last job list")?;
        Ok(row.id.clone())
    }

    fn execute(
        &mut self,
        words: &[&str],
        input: &mut impl BufRead,
        output: &mut impl Write,
    ) -> Result<()> {
        match words {
            ["help"] => writeln!(output, "{HELP}")?,
            ["list"] => self.refresh(output)?,
            ["detail", selector] => {
                let id = self.job_id(selector)?;
                let reply = self.session.request(JobsCommand::Detail { id })?;
                let job: JobSummary = serde_json::from_value(reply["job"].clone())
                    .context("invalid job detail response")?;
                writeln!(
                    output,
                    "{} [{}]  {} / {}",
                    job.title.escape_debug(),
                    job.state.escape_debug(),
                    job.app.escape_debug(),
                    job.operation.escape_debug()
                )?;
                if !job.phase.is_empty() {
                    writeln!(output, "  {}", job.phase.escape_debug())?;
                }
                if let (Some(done), Some(total)) = (job.progress_done, job.progress_total) {
                    writeln!(output, "  Progress {done}/{total}")?;
                }
                if !job.error_summary.is_empty() {
                    writeln!(output, "  {}", job.error_summary.escape_debug())?;
                }
                for stage in rows(&reply, "stages")? {
                    writeln!(
                        output,
                        "  {}  {}",
                        field(stage, "label").escape_debug(),
                        field(stage, "row").escape_debug()
                    )?;
                    let detail = field(stage, "detail");
                    if !detail.is_empty() {
                        writeln!(output, "    {}", detail.escape_debug())?;
                    }
                }
            }
            ["logs"] | ["logs", _] => {
                let job_id = words
                    .get(1)
                    .map(|selector| self.job_id(selector))
                    .transpose()?;
                let reply = self
                    .session
                    .request(JobsCommand::Logs { job_id, limit: 50 })?;
                let rows = rows(&reply, "rows")?;
                if rows.is_empty() {
                    writeln!(output, "No recent log events.")?;
                }
                for event in rows {
                    writeln!(
                        output,
                        "{}  [{}] {}  {}",
                        field(event, "ts").escape_debug(),
                        field(event, "severity").escape_debug(),
                        field(event, "app").escape_debug(),
                        field(event, "message").escape_debug()
                    )?;
                }
            }
            [action @ ("retry" | "cancel"), selector] => {
                let id = self.job_id(selector)?;
                let question = format!("Confirm {action} for {} [yes/no] ", id.escape_debug());
                if !terminal::confirmed(input, output, &question)? {
                    writeln!(output, "No action sent.")?;
                    return Ok(());
                }
                if *action == "retry" {
                    let reply = self.session.request(JobsCommand::Retry { id })?;
                    let count = reply["requeued"]
                        .as_u64()
                        .context("invalid retry response")?;
                    writeln!(output, "Requeued {count} task(s).")?;
                } else {
                    let reply = self.session.request(JobsCommand::Cancel { id })?;
                    if reply["done"].as_bool() != Some(true) {
                        bail!("invalid cancellation response");
                    }
                    writeln!(output, "Cancellation requested.")?;
                }
            }
            _ => bail!("unknown command or wrong number of arguments. Type help for commands."),
        }
        Ok(())
    }
}
