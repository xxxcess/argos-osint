//! Venture's offline actor seeding service, with immutable progress snapshots.
use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use argos_osint_core::storage::actor_catalog::{self, Progress};
use argos_osint_core::storage::home::Layout as Home;
use argos_osint_core::storage::repo::Repository;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::{Frame, Terminal};

use crate::tui::{profile_components, theme};

fn dataset_path(file: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(file) = file.or_else(|| std::env::var_os("ARGOS_ACTORS_CSV").map(PathBuf::from)) {
        return Ok(file);
    }
    let paths = [
        std::env::current_dir()?.join("datasets/argos_actors.csv"),
        argos_osint_core::paths::ensure_home()?.join("datasets/argos_actors.csv"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../datasets/argos_actors.csv"),
    ];
    paths
        .into_iter()
        .find(|p| p.is_file())
        .context("actor dataset not found; provide --file or ARGOS_ACTORS_CSV")
}

pub(crate) async fn run(file: Option<PathBuf>) -> Result<()> {
    let path = dataset_path(file)?;
    let repo = Repository::open_home(&Home::new(argos_osint_core::paths::ensure_home()?)).await?;
    let cancel = Arc::new(AtomicBool::new(false));
    let (send, receive) = std::sync::mpsc::channel();
    let ui_cancel = cancel.clone();
    // Terminal I/O and file parsing never block a Tokio worker.
    let ui = tokio::task::spawn_blocking(move || show_progress(receive, ui_cancel));
    let result = actor_catalog::seed_file(&repo, &path, &cancel, |p| {
        let _ = send.send(p);
    })
    .await;
    drop(send);
    ui.await??;
    let final_state = result?;
    println!(
        "Seed DB complete: {} actors, {} aliases",
        final_state.processed, final_state.aliases
    );
    Ok(())
}

struct Restore;
impl Drop for Restore {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, crossterm::cursor::Show);
    }
}

fn show_progress(
    receive: std::sync::mpsc::Receiver<Progress>,
    cancel: Arc<AtomicBool>,
) -> Result<()> {
    if !io::stdout().is_terminal() || !io::stdin().is_terminal() {
        while let Ok(p) = receive.recv() {
            println!(
                "Seed DB: {}/{} actors · {} aliases{}",
                p.processed,
                p.total,
                p.aliases,
                if p.complete { " · complete" } else { "" }
            );
        }
        return Ok(());
    }
    enable_raw_mode()?;
    let _restore = Restore;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let started = Instant::now();
    let mut progress = Progress::default();
    loop {
        let mut disconnected = false;
        loop {
            match receive.try_recv() {
                Ok(next) => progress = next,
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        terminal.draw(|frame| draw(frame, &progress, started.elapsed()))?;
        if disconnected {
            break;
        }
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.code == KeyCode::Esc
                    || (key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL))
                {
                    cancel.store(true, Ordering::Release);
                }
            }
        }
    }
    Ok(())
}

fn draw(frame: &mut Frame, progress: &Progress, elapsed: Duration) {
    let area = frame.area();
    frame.render_widget(Paragraph::new("").style(theme::text()), area);
    if area.width < 40 || area.height < 14 {
        frame.render_widget(
            Paragraph::new(
                "Seed DB\nResize to at least 40 × 14.\nEsc cancels after the current batch.",
            )
            .style(theme::dim()),
            area,
        );
        return;
    }
    let width = area.width.saturating_sub(4).min(76);
    let rect = Rect::new((area.width - width) / 2, (area.height - 12) / 2, width, 12);
    let inner = theme::panel("Seed DB · Actor catalog").inner(rect);
    frame.render_widget(theme::panel("Seed DB · Actor catalog"), rect);
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(2),
        Constraint::Length(2),
        Constraint::Min(1),
    ])
    .split(inner);
    let ratio = if progress.total == 0 {
        0.0
    } else {
        progress.processed as f64 / progress.total as f64
    };
    let headline = if progress.complete {
        "Complete"
    } else if progress.total == 0 {
        "Validating CSV…"
    } else {
        "Ingesting actors"
    };
    frame.render_widget(
        Paragraph::new(format!("{headline} · {:.0}%", ratio * 100.0)).style(theme::accent()),
        rows[0],
    );
    frame.render_widget(
        Paragraph::new(Line::from(profile_components::gauge(
            ratio,
            true,
            usize::from(rows[1].width),
        ))),
        rows[1],
    );
    let counts = if progress.total == 0 {
        "Actors —  Aliases —".to_owned()
    } else {
        format!(
            "Actors {}/{}  Aliases {}",
            progress.processed, progress.total, progress.aliases
        )
    };
    frame.render_widget(
        Paragraph::new(counts)
            .style(theme::text())
            .wrap(ratatui::widgets::Wrap { trim: true }),
        rows[2],
    );
    frame.render_widget(
        Paragraph::new(format!(
            "Elapsed {}s · Esc cancels after this batch\nCommitted batches are safe to retry.",
            elapsed.as_secs()
        ))
        .style(theme::dim()),
        rows[3],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    #[test]
    fn seed_progress_has_bounded_counts_and_small_screen_fallback() {
        for (width, height) in [(160, 50), (80, 24), (40, 14), (30, 10)] {
            for (name, progress) in [
                ("validating", Progress::default()),
                (
                    "running",
                    Progress {
                        processed: 128,
                        total: 1000,
                        aliases: 384,
                        complete: false,
                    },
                ),
                (
                    "complete",
                    Progress {
                        processed: 1000,
                        total: 1000,
                        aliases: 3000,
                        complete: true,
                    },
                ),
            ] {
                let mut terminal =
                    Terminal::new(TestBackend::new(width, height)).expect("terminal");
                terminal
                    .draw(|frame| draw(frame, &progress, Duration::from_secs(10)))
                    .expect("draw");
                let buffer = terminal.backend().buffer();
                let text = (0..height)
                    .map(|y| {
                        (0..width)
                            .map(|x| buffer[(x, y)].symbol())
                            .collect::<String>()
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(text.contains(if width < 40 {
                    "Resize"
                } else {
                    "Actor catalog"
                }));
                if let Ok(folder) = std::env::var("ARGOS_SCREEN_DIR") {
                    let cells: Vec<Vec<_>> = (0..height).map(|y| (0..width).map(|x| { let c=&buffer[(x,y)]; serde_json::json!({"s":c.symbol(),"fg":format!("{:?}",c.fg),"bg":format!("{:?}",c.bg),"b":false,"u":false}) }).collect()).collect();
                    std::fs::create_dir_all(&folder).expect("screen folder");
                    std::fs::write(
                        PathBuf::from(folder).join(format!("seed-{name}-{width}x{height}.json")),
                        serde_json::to_vec(
                            &serde_json::json!({"width":width,"height":height,"cells":cells}),
                        )
                        .expect("cells"),
                    )
                    .expect("write cells");
                }
            }
        }
    }
}
