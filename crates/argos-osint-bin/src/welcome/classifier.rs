//! Classifier operation presentation. All fitting and authorization belong to core.
use std::io::{self, IsTerminal};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use argos_osint_core::classifier::readiness::{
    ClassifierHome, Diagnostic, ManualTrainRequest, Readiness, TrainingJob, TrainingJobState,
    TrainingOutcomeView,
};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use tokio::sync::watch;

use super::seed::{progress_cancel_requested, CancelOnDrop, ProgressTerminal};
use crate::tui::{profile_components, theme};

/// Immutable UI input; its scroll offset is presentation-only.
#[derive(Clone)]
pub(crate) struct ClassifierPanelSnapshot {
    pub readiness: Readiness,
    pub job: Option<TrainingJob>,
    pub elapsed: Duration,
    pub notice: Option<Diagnostic>,
    pub cancelling: bool,
    pub scroll: u16,
}

/// Runs an owned manual operation and restores presentation before returning.
pub(crate) async fn run(request: ManualTrainRequest) -> Result<TrainingOutcomeView> {
    let home = ClassifierHome::new(request.dataset.home.clone());
    let readiness = home.refresh_integrity(&request.dataset).await?;
    let accepted = home.start_training(request).await?;
    let cancel = Arc::new(AtomicBool::new(false));
    let snapshot = ClassifierPanelSnapshot {
        readiness,
        job: None,
        elapsed: Duration::ZERO,
        notice: None,
        cancelling: false,
        scroll: 0,
    };
    let (send, receive) = watch::channel(snapshot.clone());
    let ui_cancel = cancel.clone();
    let mut ui = tokio::task::spawn_blocking(move || show_progress(receive, ui_cancel));
    let mut ui_done = false;
    let mut ui_error = None;
    let mut snapshot = snapshot;
    let started = Instant::now();
    let mut interval = tokio::time::interval(Duration::from_millis(100));
    #[cfg(unix)]
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let result = loop {
        tokio::select! {
            result = &mut ui, if !ui_done => {
                ui_done = true;
                if let Err(error) = result.map_err(|_| anyhow!("classifier progress worker stopped")).and_then(|r| r) {
                    ui_error = Some(error);
                    cancel.store(true, Ordering::Release);
                }
            }
            signal = tokio::signal::ctrl_c() => {
                if let Err(error) = signal { ui_error = Some(error.into()); }
                cancel.store(true, Ordering::Release);
            }
            _ = async {
                #[cfg(unix)] { sigterm.recv().await; }
                #[cfg(not(unix))] std::future::pending::<()>().await;
            } => { cancel.store(true, Ordering::Release); }
            _ = interval.tick() => {
                if cancel.load(Ordering::Acquire) && !snapshot.cancelling {
                    match home.cancel_training(&accepted.job_id).await {
                        Ok(job) => { snapshot.job = Some(job); snapshot.cancelling = true; }
                        Err(error) => break Err(error.into()),
                    }
                }
                match home.job_detail(&accepted.job_id).await {
                    Ok(job) => {
                        let terminal = matches!(job.state, TrainingJobState::Completed | TrainingJobState::Failed | TrainingJobState::Cancelled | TrainingJobState::Interrupted);
                        snapshot.elapsed = started.elapsed();
                        snapshot.notice = job.diagnostic.clone();
                        snapshot.job = Some(job.clone());
                        let _ = send.send(snapshot.clone());
                        if terminal {
                            break match job.outcome {
                                Some(outcome) if job.state == TrainingJobState::Completed => Ok(outcome),
                                _ => Err(anyhow!("{}", job.diagnostic.map(|d| d.message).unwrap_or_else(|| format!("Classifier training {:?}", job.state)))),
                            };
                        }
                    }
                    Err(error) => break Err(error.into()),
                }
            }
        }
    };
    // Closing the latest-value stream lets the blocking renderer restore modes.
    drop(send);
    if !ui_done {
        match ui.await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => ui_error = Some(error),
            Err(_) => ui_error = Some(anyhow!("classifier progress worker stopped")),
        }
    }
    let shutdown = home.shutdown().await;
    if let Some(error) = ui_error {
        shutdown?;
        return Err(error);
    }
    shutdown?;
    result
}

fn show_progress(
    mut receive: watch::Receiver<ClassifierPanelSnapshot>,
    cancel: Arc<AtomicBool>,
) -> Result<()> {
    let _cancel_on_drop = CancelOnDrop(cancel.clone());
    if !io::stderr().is_terminal() || !io::stdin().is_terminal() {
        let mut shown_revision = None;
        loop {
            let snapshot = receive.borrow_and_update().clone();
            if let Some(job) = snapshot.job {
                if shown_revision != Some(job.revision) {
                    shown_revision = Some(job.revision);
                    if let Some(progress) = job.progress {
                        eprintln!(
                            "Classifier: {} · {} representatives fitted · {} memberships",
                            progress.stage,
                            progress.rows_fitted,
                            progress.fitted_memberships.iter().sum::<u64>()
                        );
                    } else {
                        eprintln!("Classifier: {:?}", job.state);
                    }
                }
            }
            if receive.has_changed().is_err() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    let _restore = ProgressTerminal::enter(true)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stderr()))?;
    let mut scroll: u16 = 0;
    loop {
        let mut snapshot = receive.borrow_and_update().clone();
        snapshot.scroll = scroll;
        terminal.draw(|frame| draw_classifier_progress(frame, &snapshot))?;
        if receive.has_changed().is_err() {
            return Ok(());
        }
        if event::poll(Duration::from_millis(100))? {
            let event = event::read()?;
            if progress_cancel_requested(&event) {
                cancel.store(true, Ordering::Release);
            }
            if let Event::Key(key) = event {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Down => scroll = scroll.saturating_add(1),
                        KeyCode::Up => scroll = scroll.saturating_sub(1),
                        KeyCode::PageDown => scroll = scroll.saturating_add(10),
                        KeyCode::PageUp => scroll = scroll.saturating_sub(10),
                        _ => {}
                    }
                }
            }
        }
    }
}

/// Operational service-launcher panel; no I/O or model work occurs in drawing.
pub(crate) fn draw_classifier_progress(frame: &mut Frame, snapshot: &ClassifierPanelSnapshot) {
    let area = frame.area();
    frame.render_widget(Paragraph::new("").style(theme::text()), area);
    if area.width < 40 || area.height < 14 {
        frame.render_widget(
            Paragraph::new(
                "Classifier training\nResize to at least 40 × 14.\nEsc or Ctrl-C cancels safely.",
            )
            .style(theme::accent())
            .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }
    let width = area.width.min(100);
    let height = area.height.min(34);
    let panel = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    let block = theme::panel("Train and save classifier");
    let inner = block.inner(panel);
    frame.render_widget(block, panel);
    let rows = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(2),
    ])
    .split(inner);
    let readiness = Line::from(format!(
        "Classifier: {} · integrity {}",
        snapshot.readiness.state.as_str(),
        snapshot.readiness.integrity
    ))
    .style(theme::accent());
    let mut summary = if readiness.width() > usize::from(rows[0].width) {
        vec![
            Line::from(format!("Classifier: {}", snapshot.readiness.state.as_str()))
                .style(theme::accent()),
            Line::from(format!("Integrity: {}", snapshot.readiness.integrity))
                .style(theme::accent()),
        ]
    } else {
        vec![readiness]
    };
    if let Some(job) = &snapshot.job {
        summary.push(Line::from(format!("Job: {:?}", job.state)));
        if let Some(progress) = &job.progress {
            summary.push(Line::from(format!(
                "Stage: {} · round {}",
                progress.stage, progress.round
            )));
        }
    }
    if snapshot.cancelling {
        summary.push(Line::from("Cancelling safely").style(theme::dim()));
    }
    if let Some(notice) = &snapshot.notice {
        summary.push(Line::from(format!("Error: {}", notice.code)).style(theme::error()));
    }
    summary
        .push(Line::from(format!("Elapsed {}s", snapshot.elapsed.as_secs())).style(theme::dim()));
    // Current operation state must survive long paths and detail scrolling.
    let summary_height = summary
        .len()
        .min(usize::from(rows[0].height.saturating_sub(1))) as u16;
    let status = Rect::new(rows[0].x, rows[0].y, rows[0].width, summary_height);
    let details = Rect::new(
        rows[0].x,
        rows[0].y + summary_height,
        rows[0].width,
        rows[0].height.saturating_sub(summary_height),
    );
    frame.render_widget(Paragraph::new(summary).style(theme::text()), status);
    let mut lines = Vec::new();
    if let Some(notice) = &snapshot.notice {
        lines
            .push(Line::from(format!("{}: {}", notice.code, notice.message)).style(theme::error()));
    }
    if let Some(path) = &snapshot.readiness.dataset.path {
        lines.push(Line::from(format!("Dataset: {path}")));
    }
    if let Some(source) = snapshot.readiness.dataset.source {
        lines.push(Line::from(format!("Source: {source:?}")));
    }
    if let Some(prior) = &snapshot.readiness.active_model {
        lines.push(Line::from(format!(
            "Prior model: {} · {}",
            prior.model_id, prior.provenance.label
        )));
        lines.push(Line::from(format!(
            "{} · {}",
            prior.taxonomy_version, prior.preprocessing_version
        )));
        lines.push(Line::from(format!(
            "{} · {}",
            prior.policy_version, prior.calibration_version
        )));
        if !prior.quality.production_approved {
            lines.push(
                Line::from("Insufficient independent reviewed evidence").style(theme::warn()),
            );
        }
    } else {
        lines.push(Line::from("Prior model: none"));
    }
    let mut gauge = None;
    if let Some(job) = &snapshot.job {
        if let Some(candidate) = &job.candidate_model_id {
            lines.push(Line::from(format!("Candidate: {candidate}")));
        }
        if let Some(provenance) = &job.provenance {
            lines
                .push(Line::from(format!("Provenance: {}", provenance.label)).style(theme::warn()));
        }
        if let Some(progress) = &job.progress {
            lines.push(Line::from(format!(
                "Representatives fitted: {} · seen {}",
                progress.rows_fitted, progress.rows_seen
            )));
            lines.push(Line::from(format!(
                "Per-domain fitted memberships: {:?}",
                progress.fitted_memberships
            )));
            lines.push(Line::from(format!(
                "Vocabulary sizes: {:?}",
                progress.vocabulary_sizes
            )));
            // rows_total=0 means the core has not established a denominator.
            if progress.rows_total > 0 && progress.rows_fitted <= progress.rows_total {
                gauge = Some(progress.rows_fitted as f64 / progress.rows_total as f64);
            }
        }
        if let Some(outcome) = &job.outcome {
            lines.push(Line::from(format!("Outcome: {}", outcome.state)).style(theme::accent()));
        }
    }
    lines.push(
        Line::from("Synthetic seed counts are not production accuracy.").style(theme::warn()),
    );
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .scroll((snapshot.scroll, 0))
            .style(theme::text()),
        details,
    );
    if let Some(ratio) = gauge {
        frame.render_widget(
            Paragraph::new(Line::from(profile_components::gauge(
                ratio,
                true,
                usize::from(rows[1].width),
            ))),
            rows[1],
        );
    }
    frame.render_widget(
        Paragraph::new("↑/↓ scroll · Esc cancels safely\nCtrl-C cancels and restores terminal")
            .style(theme::dim()),
        rows[2],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    fn snapshot(state: &str) -> ClassifierPanelSnapshot {
        let prior = serde_json::json!({
            "model_id":"prior-verified-model", "generation":3, "activated_at":"2026-01-01T00:00:00Z",
            "artifact_sha256":"a".repeat(64), "manifest_sha256":"b".repeat(64),
            "taxonomy_version":"strategic-domains-v1", "domains":["governance","security","economic","human","environmental","scientific"],
            "algorithm_version":"natural-bayes-binary-ovr-v1", "preprocessing_version":"natural-en-keepstops-v1",
            "policy_version":"exact-two-independent-gates-v1", "receipt_schema_version":"classifier-receipt-v1",
            "calibration_version":"domain-isotonic-family-v1",
            "provenance":{"kind":"synthetic_baseline","label":"Synthetic baseline","dataset_version":"synthetic-fixture-v1","dataset_sha256":"c".repeat(64),"review_reference":null},
            "quality":{"production_approved":false,"reason_codes":["insufficient_quality_evidence"],"domain_acceptance":null}
        });
        let readiness = serde_json::from_value(serde_json::json!({
            "schema_version":"classifier-readiness-v1", "state":if state == "pending" { "unavailable" } else if state == "missing" { "missing" } else { "ready" },
            "active_model_state":if matches!(state,"missing"|"pending") { "missing" } else { "ready" },
            "feature_enabled":true,"can_infer":!matches!(state,"missing"|"pending"),"can_train":true,
            "integrity":if state == "pending" { "pending" } else { "verified" },"retraining_available":false,
            "dataset":{"path":"/fictional/very-long-資料/classifier/dataset/argos_atlas_strategic_domains.csv","source":"repo_plural","availability":"present","change":"unknown","hash_check":"verified","sha256":"d".repeat(64),"diagnostic":null},
            "active_model":if matches!(state,"missing"|"pending") { serde_json::Value::Null } else { prior },
            "training":null,"diagnostic":null
        })).expect("readiness fixture contract");
        let job = if matches!(state, "training" | "cancelling" | "error" | "completed") {
            Some(serde_json::from_value(serde_json::json!({
                "schema_version":"classifier-training-job-v1","job_id":format!("classifier-{}","e".repeat(64)),"request_id":"visual-fixture",
                "state":match state { "cancelling"=>"cancelling","error"=>"failed","completed"=>"completed",_=>"running" },"revision":5,
                "dataset_path":null,"progress":{"request_id":"visual-fixture","stage":"fitting","round":2,"rows_seen":64,"rows_fitted":64,"rows_total":142,"fitted_memberships":[64,64,64,64,64,64],"token_bytes":4096,"vocabulary_sizes":[120,124,118,150,113,141]},
                "active_model":null,"candidate_model_id":"separate-candidate-model","provenance":null,
                "cancel_requested":state=="cancelling","outcome":null,"diagnostic":null
            })).expect("job fixture contract"))
        } else {
            None
        };
        ClassifierPanelSnapshot {
            readiness,
            job,
            elapsed: Duration::from_secs(12),
            scroll: 0,
            cancelling: state == "cancelling",
            notice: if state == "error" {
                Some(serde_json::from_value(serde_json::json!({"code":"training_failed","message":"Synthetic fixture error; prior model remains usable","retryable":true})).expect("diagnostic"))
            } else {
                None
            },
        }
    }

    fn screen_text(buffer: &ratatui::buffer::Buffer) -> String {
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn classifier_progress_viewports_and_states() {
        for (width, height) in [(160, 50), (120, 40), (80, 24), (60, 18), (40, 14), (30, 10)] {
            for state in [
                "pending",
                "missing",
                "ready",
                "training",
                "cancelling",
                "error",
                "completed",
            ] {
                let mut terminal =
                    Terminal::new(TestBackend::new(width, height)).expect("test terminal");
                let snapshot = snapshot(state);
                terminal
                    .draw(|frame| draw_classifier_progress(frame, &snapshot))
                    .expect("draw pure progress fixture");
                let buffer = terminal.backend().buffer();
                let text = screen_text(buffer);
                let assert_status = |text: &str| {
                    if width < 40 {
                        return;
                    }
                    let readiness = match state {
                        "pending" => "Classifier: unavailable",
                        "missing" => "Classifier: missing",
                        _ => "Classifier: ready",
                    };
                    assert!(
                        text.contains(readiness),
                        "{state} at {width}x{height}: {text}"
                    );
                    if let Some(job) = match state {
                        "training" => Some("Job: Running"),
                        "cancelling" => Some("Job: Cancelling"),
                        "error" => Some("Job: Failed"),
                        "completed" => Some("Job: Completed"),
                        _ => None,
                    } {
                        assert!(
                            text.contains(job),
                            "operation state stays visible at {width}x{height}: {text}"
                        );
                        assert!(
                            text.contains("Stage: fitting"),
                            "stage stays visible at {width}x{height}: {text}"
                        );
                    }
                    if state == "cancelling" {
                        assert!(text.contains("Cancelling safely"), "{text}");
                    }
                    if state == "error" {
                        assert!(text.contains("Error: training_failed"), "{text}");
                    }
                };
                assert_status(&text);
                assert!(text.contains(if width < 40 {
                    "Resize"
                } else {
                    "Train and save classifier"
                }));
                assert!(
                    text.contains("Ctrl-C"),
                    "cancellation stays reachable at {width}x{height}"
                );
                if let Ok(folder) = std::env::var("ARGOS_SCREEN_DIR") {
                    let cells:Vec<Vec<_>>=(0..height).map(|y|(0..width).map(|x|{let cell=&buffer[(x,y)];serde_json::json!({"s":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg),"b":false,"u":false})}).collect()).collect();
                    std::fs::create_dir_all(&folder).expect("fixture output directory");
                    std::fs::write(
                        std::path::PathBuf::from(folder)
                            .join(format!("classifier-progress-{state}-{width}x{height}.json")),
                        serde_json::to_vec(
                            &serde_json::json!({"width":width,"height":height,"cells":cells}),
                        )
                        .expect("cell JSON"),
                    )
                    .expect("fixture export");
                }
                if width >= 40 {
                    let mut scrolled = snapshot.clone();
                    scrolled.scroll = 8;
                    terminal
                        .draw(|frame| draw_classifier_progress(frame, &scrolled))
                        .expect("draw scrolled progress fixture");
                    let scrolled_text = screen_text(terminal.backend().buffer());
                    assert_status(&scrolled_text);
                    if (width, height) == (40, 14) {
                        assert_ne!(text, scrolled_text, "metadata details remain scrollable");
                    }
                }
            }
        }
    }
}
