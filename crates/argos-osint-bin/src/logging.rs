//! Tracing subscriber registration and TUI-aware output formatting.

#[cfg(feature = "surreal")]
use std::cell::Cell;
use std::fmt;
use std::io::{self, IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};

use argos_osint_core::events::{NewEvent, Severity};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;

pub static TUI_ACTIVE: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "surreal")]
tokio::task_local! {
    // A failed diagnostic write must not generate another diagnostic write.
    static WRITING_DIAGNOSTIC: ();
}

#[cfg(feature = "surreal")]
thread_local! {
    // Queue overflow can log synchronously inside spawn_ordered, before the
    // async write's task-local scope starts.
    static CAPTURING_DIAGNOSTIC: Cell<bool> = const { Cell::new(false) };
}

#[cfg(feature = "surreal")]
fn record_diagnostic(event: NewEvent) {
    if !TUI_ACTIVE.load(Ordering::Relaxed) || WRITING_DIAGNOSTIC.try_with(|_| ()).is_ok() {
        return;
    }
    CAPTURING_DIAGNOSTIC.with(|capturing| {
        if capturing.replace(true) {
            return;
        }
        struct Reset<'a>(&'a Cell<bool>);
        impl Drop for Reset<'_> {
            fn drop(&mut self) {
                self.0.set(false);
            }
        }
        let _reset = Reset(capturing);
        // Reuse the bounded ordered queue and its process-shutdown drain.
        // No storage calls or waits occur on the emitting worker/UI thread.
        argos_osint_core::storage::runtime::spawn_ordered(WRITING_DIAGNOSTIC.scope(
            (),
            async move {
                if let Ok(repo) = argos_osint_core::storage::runtime::repository().await {
                    let _ =
                        argos_osint_core::storage::event_log::record_event(repo.db(), &event).await;
                }
            },
        ));
    });
}

#[cfg(not(feature = "surreal"))]
fn record_diagnostic(_event: NewEvent) {}

struct DiagnosticLayer<F> {
    record: F,
}

impl<S, F> Layer<S> for DiagnosticLayer<F>
where
    S: Subscriber,
    F: Fn(NewEvent) + Send + Sync + 'static,
{
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let metadata = event.metadata();
        if *metadata.level() > Level::WARN || !metadata.target().starts_with("argos_") {
            return;
        }
        let mut fields = DiagnosticFields(NewEvent {
            severity: Some(if *metadata.level() == Level::ERROR {
                Severity::Error
            } else {
                Severity::Warn
            }),
            app: "system".into(),
            event_type: "runtime.diagnostic".into(),
            details: format!("target={}", metadata.target()),
            ..Default::default()
        });
        event.record(&mut fields);
        (self.record)(fields.0);
    }
}

// Bound formatting itself: a large debug field must not allocate an unbounded
// intermediate string before reaching the durable sink's redaction/limits.
struct BoundedText<'a> {
    text: &'a mut String,
    remaining: usize,
}

impl fmt::Write for BoundedText<'_> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        for character in text.chars().take(self.remaining) {
            self.text.push(character);
            self.remaining -= 1;
        }
        Ok(())
    }
}

struct DiagnosticFields(NewEvent);

impl Visit for DiagnosticFields {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        use std::fmt::Write as _;

        let mut text = String::new();
        let _ = write!(
            BoundedText {
                text: &mut text,
                remaining: 1_000,
            },
            "{value:?}"
        );
        self.record_str(field, &text);
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        use std::fmt::Write as _;

        let slot = match field.name() {
            "message" => &mut self.0.message,
            "app" => &mut self.0.app,
            "job_id" => &mut self.0.job_id,
            "task_id" => &mut self.0.task_id,
            "attempt_id" => &mut self.0.attempt_id,
            "run_id" => &mut self.0.run_id,
            "correlation_id" => &mut self.0.correlation_id,
            name => {
                let remaining = argos_osint_core::events::MAX_DETAIL_CHARS
                    .saturating_sub(self.0.details.chars().count());
                let _ = write!(
                    BoundedText {
                        text: &mut self.0.details,
                        remaining,
                    },
                    "\n{name}={value}"
                );
                return;
            }
        };
        *slot = value.chars().take(1_000).collect();
    }
}

/// Writer that formats tracing events to stderr when running in CLI mode or
/// when stderr is redirected to a file, while suppressing terminal output
/// when the interactive TUI occupies the alternate screen.
pub struct TuiAwareWriter;

impl Write for TuiAwareWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if TUI_ACTIVE.load(Ordering::Relaxed) && io::stderr().is_terminal() {
            Ok(buf.len())
        } else {
            io::stderr().write(buf)
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        if TUI_ACTIVE.load(Ordering::Relaxed) && io::stderr().is_terminal() {
            Ok(())
        } else {
            io::stderr().flush()
        }
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for TuiAwareWriter {
    type Writer = TuiAwareWriter;

    fn make_writer(&'a self) -> Self::Writer {
        TuiAwareWriter
    }
}

/// Initialize the global tracing subscriber for `argos-osint-bin`.
pub fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        tracing_subscriber::EnvFilter::new("warn,argos_osint_bin=warn,argos_osint_core=warn")
    });

    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(TuiAwareWriter)
                .with_target(true),
        )
        .with(DiagnosticLayer {
            record: record_diagnostic,
        })
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warning_events_preserve_severity_context_and_bound_unicode_fields() {
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = events.clone();
        let subscriber = tracing_subscriber::registry().with(DiagnosticLayer {
            record: move |event| captured.lock().expect("events lock").push(event),
        });
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: "argos_osint_core::atlas", "ordinary progress");
            tracing::warn!(target: "third_party", "third-party noise");
            tracing::warn!(target: "argos_osint_core::storage::publisher",
                job_id = "job-test", run_id = "run-test", batch_size = 1,
                "memory publish batch failed: actor_derivative_stale");
            let long = "界".repeat(20_000);
            tracing::error!(target: "argos_osint_core::atlas", detail = %long, "{long}");
        });
        let events = events.lock().expect("events lock");
        assert_eq!(events.len(), 2);
        let warning = &events[0];
        assert_eq!(warning.severity, Some(Severity::Warn));
        assert_eq!(warning.job_id, "job-test");
        assert_eq!(warning.run_id, "run-test");
        assert!(warning.message.contains("actor_derivative_stale"));
        assert!(warning.details.contains("storage::publisher"));
        assert!(warning.details.contains("batch_size=1"));
        assert_eq!(events[1].severity, Some(Severity::Error));
        assert_eq!(events[1].message.chars().count(), 1_000);
        assert!(events[1].details.chars().count() <= argos_osint_core::events::MAX_DETAIL_CHARS);
    }

    #[cfg(feature = "surreal")]
    #[test]
    fn tui_diagnostics_reach_logs_redacted_and_do_not_recurse() {
        crate::test_storage::ensure();
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                TUI_ACTIVE.store(false, Ordering::Relaxed);
            }
        }
        let _reset = Reset;
        let subscriber = tracing_subscriber::registry().with(DiagnosticLayer {
            record: record_diagnostic,
        });
        tracing::subscriber::with_default(subscriber, || {
            TUI_ACTIVE.store(false, Ordering::Relaxed);
            tracing::warn!(target: "argos_osint_bin", "CLI stays on stderr");
            TUI_ACTIVE.store(true, Ordering::Relaxed);
            tracing::warn!(target: "argos_osint_core::storage::publisher",
                job_id = "job-warning", "memory publish batch failed: actor_derivative_stale");
            tracing::error!(target: "argos_osint_core::atlas",
                "request failed api_key=sk-test-diagnostic-secret");
            WRITING_DIAGNOSTIC.sync_scope((), || {
                tracing::warn!(target: "argos_osint_core::storage::event_log", "write failed");
            });
            CAPTURING_DIAGNOSTIC.with(|capturing| {
                capturing.set(true);
                tracing::warn!(target: "argos_osint_core::storage::runtime", "queue full");
                capturing.set(false);
            });
        });
        crate::test_storage::drain();
        let rows = argos_osint_core::storage::runtime::blocking(|repo| async move {
            argos_osint_core::storage::event_log::list_events(repo.db(), &Default::default(), 100)
                .await
        })
        .expect("stored diagnostics");
        let rows: Vec<_> = rows
            .iter()
            .filter(|row| row.event_type == "runtime.diagnostic")
            .collect();
        assert_eq!(
            rows.len(),
            2,
            "CLI and recursive writes must not create log rows"
        );
        assert!(rows.iter().any(|row| row.severity == "warn"
            && row.job_id == "job-warning"
            && row.message.contains("actor_derivative_stale")));
        let error = rows
            .iter()
            .find(|row| row.severity == "error")
            .expect("error row");
        assert!(!error.message.contains("sk-test-diagnostic-secret"));
        assert!(error.message.contains("[redacted]"));
    }
}
