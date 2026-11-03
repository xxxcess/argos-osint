//! Tracing subscriber registration and TUI-aware output formatting.

use std::io::{self, IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};

pub static TUI_ACTIVE: AtomicBool = AtomicBool::new(false);

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

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(TuiAwareWriter)
        .with_target(true)
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tui_aware_writer_write_and_flush() {
        let mut writer = TuiAwareWriter;
        // With TUI_ACTIVE false, write should succeed
        TUI_ACTIVE.store(false, Ordering::Relaxed);
        assert!(writer.write(b"").is_ok());
        assert!(writer.flush().is_ok());

        // With TUI_ACTIVE true, write should also succeed
        TUI_ACTIVE.store(true, Ordering::Relaxed);
        let res = writer.write(b"test log message");
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), 16);
        assert!(writer.flush().is_ok());

        // Reset
        TUI_ACTIVE.store(false, Ordering::Relaxed);
    }
}
