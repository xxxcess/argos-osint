//! Line-oriented Jobs & Logs view over the same network commands as the CLI.

use std::io::{BufRead, Write};

use anyhow::{bail, Context, Result};
use fleetworks_contracts::{Client, JobSummary, Jobs, JOBS_PATH};
use tokio::runtime::Runtime;

use crate::terminal::{self, field, rows, Session};
use crate::tui::{Prompt, Step, View};
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
                write_detail(&self.session, id, output)?;
            }
            ["logs"] | ["logs", _] => {
                let job_id = words
                    .get(1)
                    .map(|selector| self.job_id(selector))
                    .transpose()?;
                write_logs(&self.session, job_id, output)?;
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

fn write_detail(session: &Session<'_>, id: String, output: &mut impl Write) -> Result<()> {
    let reply = session.request(JobsCommand::Detail { id })?;
    let job: JobSummary =
        serde_json::from_value(reply["job"].clone()).context("invalid job detail response")?;
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
    Ok(())
}

fn write_logs(
    session: &Session<'_>,
    job_id: Option<String>,
    output: &mut impl Write,
) -> Result<()> {
    let reply = session.request(JobsCommand::Logs { job_id, limit: 50 })?;
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
    Ok(())
}

fn summary_row(job: &JobSummary) -> String {
    format!(
        "[{}] {}  ({})",
        job.state.escape_debug(),
        job.title.escape_debug(),
        job.app.escape_debug()
    )
}

/// Ratatui Jobs & Logs screen.
pub(crate) struct JobsView<'a> {
    session: Session<'a>,
    jobs: Vec<JobSummary>,
    detail: Vec<String>,
    heading: String,
    pending: Option<(&'static str, String)>,
}

impl<'a> JobsView<'a> {
    pub fn new(client: &'a Client, runtime: &'a Runtime) -> Self {
        Self {
            session: Session { client, runtime },
            jobs: Vec::new(),
            detail: Vec::new(),
            heading: "Job".into(),
            pending: None,
        }
    }

    fn show(
        &mut self,
        heading: &str,
        write: impl FnOnce(&mut Vec<u8>) -> Result<()>,
    ) -> Result<()> {
        let mut buffer = Vec::new();
        write(&mut buffer)?;
        self.heading = heading.into();
        self.detail = terminal::lines(&buffer);
        Ok(())
    }

    fn load_detail(&mut self, index: usize) -> Result<Step> {
        let Some(job) = self.jobs.get(index) else {
            return Ok(Step::none());
        };
        let id = job.id.clone();
        let session = Session {
            client: self.session.client,
            runtime: self.session.runtime,
        };
        self.show("Job", |out| write_detail(&session, id, out))?;
        Ok(Step::none())
    }

    fn job_action(&mut self, action: &'static str, index: usize) -> Result<Step> {
        let Some(job) = self.jobs.get(index) else {
            return Ok(Step::status("No job selected."));
        };
        let question = format!("Confirm {action} for {}?", job.id.escape_debug());
        self.pending = Some((action, job.id.clone()));
        Ok(Step::prompt(Prompt::confirm(action, question)))
    }
}

impl View for JobsView<'_> {
    fn title(&self) -> String {
        "Jobs & Logs".into()
    }

    fn hints(&self) -> &'static str {
        "l job logs · L all logs · t retry · c cancel"
    }

    fn list_title(&self) -> &'static str {
        "Jobs"
    }

    fn detail_title(&self) -> String {
        self.heading.clone()
    }

    fn rows(&self) -> Vec<String> {
        self.jobs.iter().map(summary_row).collect()
    }

    fn detail(&self) -> Vec<String> {
        self.detail.clone()
    }

    fn refresh(&mut self) -> Result<Step> {
        let jobs: Jobs = self
            .session
            .runtime
            .block_on(self.session.client.get(JOBS_PATH))?;
        self.jobs = jobs.rows;
        self.detail.clear();
        let count = self.jobs.len();
        if count > 0 {
            self.load_detail(0)?;
        }
        Ok(Step::status(format!("{count} recent job(s)")))
    }

    fn select(&mut self, index: usize) -> Result<Step> {
        self.load_detail(index)
    }

    fn activate(&mut self, index: usize) -> Result<Step> {
        self.load_detail(index)
    }

    fn key(&mut self, key: char, index: usize) -> Result<Step> {
        match key {
            'l' | 'L' => {
                let job_id = if key == 'l' {
                    self.jobs.get(index).map(|job| job.id.clone())
                } else {
                    None
                };
                let session = Session {
                    client: self.session.client,
                    runtime: self.session.runtime,
                };
                self.show("Logs", |out| write_logs(&session, job_id, out))?;
                Ok(Step::none())
            }
            't' => self.job_action("retry", index),
            'c' => self.job_action("cancel", index),
            _ => Ok(Step::none()),
        }
    }

    fn submit(&mut self, tag: &'static str, _value: &str) -> Result<Step> {
        let Some((action, id)) = self.pending.take().filter(|(action, _)| *action == tag) else {
            return Ok(Step::none());
        };
        if action == "retry" {
            let reply = self.session.request(JobsCommand::Retry { id })?;
            let count = reply["requeued"]
                .as_u64()
                .context("invalid retry response")?;
            self.refresh()?;
            Ok(Step::status(format!("Requeued {count} task(s).")))
        } else {
            let reply = self.session.request(JobsCommand::Cancel { id })?;
            if reply["done"].as_bool() != Some(true) {
                bail!("invalid cancellation response");
            }
            self.refresh()?;
            Ok(Step::status("Cancellation requested."))
        }
    }
}

#[cfg(test)]
mod view_tests {
    use super::*;
    use crate::tui::testing::{capture, offline};
    use crate::tui::Shell;

    fn job(id: &str, state: &str, title: &str) -> JobSummary {
        serde_json::from_value(serde_json::json!({
            "id": id, "app": "atlas", "title": title, "operation": "collect", "state": state,
            "phase": "fetching", "progress_done": 3, "progress_total": 10, "error_summary": ""
        }))
        .unwrap()
    }

    #[test]
    fn jobs_screen_renders_rows_and_detail() {
        let (client, runtime) = offline();
        let mut view = JobsView::new(&client, &runtime);
        view.jobs = vec![
            job("j1", "running", "Atlas cycle 1"),
            job("j2", "failed", "Brief\u{1b}[2J"),
        ];
        view.detail = vec![
            "Atlas cycle 1 [running]  atlas / collect".into(),
            "  Progress 3/10".into(),
        ];
        assert_eq!(view.rows()[1], "[failed] Brief\\u{1b}[2J  (atlas)");
        capture("jobs", &mut Shell::new(), &view);
    }

    #[test]
    fn retry_and_cancel_raise_confirmations_without_a_request() {
        let (client, runtime) = offline();
        let mut view = JobsView::new(&client, &runtime);
        view.jobs = vec![job("j1", "failed", "x")];
        let step = view.key('t', 0).unwrap();
        assert_eq!(step.prompt.unwrap().tag, "retry");
        let step = view.key('c', 0).unwrap();
        assert_eq!(step.prompt.unwrap().tag, "cancel");
        assert_eq!(
            view.key('t', 5).unwrap().status.as_deref(),
            Some("No job selected.")
        );
    }
}
