//! Bounded service traffic ingestion into the existing Jobs → Logs contract.
use super::OwnedChild;
use anyhow::Result;
use argos_osint_core::{
    events::{NewEvent, Severity},
    job_registry::{Finish, JobHandle, JobSpec},
};
use std::{
    fs::File,
    io::Write,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::AsyncReadExt,
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

const CAPACITY: usize = 256;
const MAX_LINE: usize = 4096;

#[derive(Clone)]
pub(super) struct Sink {
    tx: mpsc::Sender<NewEvent>,
    job_id: String,
    dropped: Arc<AtomicU64>,
}

impl Sink {
    pub fn event(&self, kind: &str, message: String, severity: Severity) {
        let omitted = self.dropped.swap(0, Ordering::Relaxed);
        if omitted > 0
            && self
                .tx
                .try_send(self.row(
                    "service.overflow",
                    format!("{omitted} traffic entries omitted while the log queue was full"),
                    Severity::Warn,
                ))
                .is_err()
        {
            self.dropped.fetch_add(omitted, Ordering::Relaxed);
        }
        if self
            .tx
            .try_send(self.row(kind, clean_line(&message), severity))
            .is_err()
        {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn row(&self, kind: &str, message: String, severity: Severity) -> NewEvent {
        NewEvent {
            severity: Some(severity),
            app: "system".into(),
            event_type: kind.into(),
            message,
            job_id: self.job_id.clone(),
            ..Default::default()
        }
    }
}

pub(super) struct Recorder {
    job: Option<JobHandle>,
    sink: Option<Sink>,
    writer: Option<JoinHandle<()>>,
}

impl Recorder {
    pub async fn start(operation: &str, title: &str, mut file: Option<File>) -> Result<Self> {
        let job = JobHandle::begin(
            &argos_osint_core::paths::db_path(),
            JobSpec::new("system", operation, title),
        )?;
        // Registration and classification share the registry's ordered queue.
        // Service jobs are visible in Jobs but do not offer task retry/cancel.
        let id = job.id().to_string();
        let (ready, registered) = oneshot::channel();
        argos_osint_core::storage::runtime::spawn_ordered(async move {
            let result = async {
                let repo = argos_osint_core::storage::runtime::repository().await?;
                repo.db().query("UPDATE type::record('job', $id) SET kind = 'service', cancellable = false RETURN NONE").bind(("id", id)).await?.check()?;
                anyhow::Ok(())
            }.await;
            let _ = ready.send(result);
        });
        registered.await??;
        let repo = argos_osint_core::storage::runtime::repository().await?;
        let (tx, mut rx) = mpsc::channel::<NewEvent>(CAPACITY);
        let sink = Sink {
            tx,
            job_id: job.id().into(),
            dropped: Arc::new(AtomicU64::new(0)),
        };
        let dropped = sink.dropped.clone();
        let job_id = sink.job_id.clone();
        let writer = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let Some(file) = file.as_mut() {
                    let _ = writeln!(file, "{}", event.message);
                }
                let _ = argos_osint_core::storage::event_log::record_event(repo.db(), &event).await;
            }
            let omitted = dropped.swap(0, Ordering::Relaxed);
            if omitted > 0 {
                let event = NewEvent {
                    severity: Some(Severity::Warn),
                    app: "system".into(),
                    event_type: "service.overflow".into(),
                    job_id,
                    message: format!(
                        "{omitted} traffic entries omitted while the log queue was full"
                    ),
                    ..Default::default()
                };
                let _ = argos_osint_core::storage::event_log::record_event(repo.db(), &event).await;
            }
        });
        Ok(Self {
            job: Some(job),
            sink: Some(sink),
            writer: Some(writer),
        })
    }

    pub fn sink(&self) -> Sink {
        self.sink.as_ref().expect("active recorder").clone()
    }

    pub async fn finish(&mut self, failed: bool) {
        self.sink.take();
        if let Some(mut writer) = self.writer.take() {
            if tokio::time::timeout(Duration::from_secs(5), &mut writer)
                .await
                .is_err()
            {
                writer.abort();
                let _ = writer.await;
            }
        }
        if let Some(job) = self.job.take() {
            job.finish(if failed {
                Finish::failed("service_exit", "Server exited unexpectedly")
            } else {
                Finish::completed()
            });
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if let Some(writer) = self.writer.take() {
            writer.abort();
        }
    }
}

pub(super) struct ServiceTraffic {
    recorder: Recorder,
    readers: Vec<JoinHandle<()>>,
}

impl ServiceTraffic {
    pub async fn capture(child: &mut OwnedChild, log: File, dev: bool) -> Result<Self> {
        let recorder = Recorder::start(
            "web_server",
            if dev {
                "Next.js web server"
            } else {
                "Static web server"
            },
            Some(log),
        )
        .await?;
        let mut readers = Vec::new();
        if let Some(stdout) = child.child.stdout.take() {
            readers.push(capture_lines(stdout, recorder.sink()));
        }
        if let Some(stderr) = child.child.stderr.take() {
            readers.push(capture_lines(stderr, recorder.sink()));
        }
        Ok(Self { recorder, readers })
    }

    pub async fn finish(&mut self, failed: bool) {
        for mut reader in self.readers.drain(..) {
            if tokio::time::timeout(Duration::from_secs(2), &mut reader)
                .await
                .is_err()
            {
                reader.abort();
                let _ = reader.await;
            }
        }
        self.recorder.finish(failed).await;
    }
}

impl Drop for ServiceTraffic {
    fn drop(&mut self) {
        for reader in &self.readers {
            reader.abort();
        }
    }
}

fn capture_lines(
    mut stream: impl tokio::io::AsyncRead + Unpin + Send + 'static,
    sink: Sink,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut buffer = [0u8; 1024];
        let mut line = Vec::new();
        let mut truncated = false;
        while let Ok(n) = stream.read(&mut buffer).await {
            if n == 0 {
                break;
            }
            for byte in &buffer[..n] {
                if matches!(*byte, b'\n' | b'\r') {
                    publish_line(&sink, &line, truncated);
                    line.clear();
                    truncated = false;
                } else if line.len() < MAX_LINE {
                    line.push(*byte);
                } else {
                    truncated = true;
                }
            }
        }
        publish_line(&sink, &line, truncated);
    })
}

fn publish_line(sink: &Sink, line: &[u8], truncated: bool) {
    let mut text = clean_line(&String::from_utf8_lossy(line));
    if text.is_empty() {
        return;
    }
    if truncated {
        text.push_str(" …[truncated]");
    }
    let lower = text.to_ascii_lowercase();
    let severity = if lower.contains("error") {
        Severity::Error
    } else if lower.contains("warn") {
        Severity::Warn
    } else {
        Severity::Info
    };
    sink.event("web.output", text, severity);
}

fn clean_line(text: &str) -> String {
    let mut state = 0;
    let mut plain = String::new();
    for c in text.chars().take(MAX_LINE) {
        match state {
            0 if c == '\x1b' => state = 1,
            0 if !c.is_control() => plain.push(c),
            1 => {
                state = match c {
                    '[' => 2,
                    ']' => 3,
                    _ => 0,
                }
            }
            2 if ('@'..='~').contains(&c) => state = 0,
            3 if c == '\x07' => state = 0,
            3 if c == '\x1b' => state = 4,
            4 => state = if c == '\\' { 0 } else { 3 },
            _ => {}
        }
    }
    // Access-log query strings are application data, not operational context.
    let mut words: Vec<_> = plain.split_whitespace().collect();
    if words.first().is_some_and(|method| {
        matches!(
            *method,
            "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS"
        )
    }) && words.len() > 1
    {
        words[1] = words[1].split('?').next().unwrap_or(words[1]);
    }
    argos_osint_core::events::redact(&words.join(" "))
        .chars()
        .take(2000)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn output_is_bounded_plain_and_redacted() {
        assert_eq!(
            clean_line("\x1b[32mGET /?secret=private 200\x1b[0m"),
            "GET / 200"
        );
        assert!(!clean_line("Authorization: Bearer abcd1234567890").contains("abcd1234567890"));
        assert_eq!(clean_line(&"x".repeat(9000)).len(), 2000);
        assert_eq!(
            clean_line("\x1b]8;;https://example.org\x07hello\x1b]8;;\x07"),
            "hello"
        );
    }
    #[test]
    fn overload_does_not_block_the_service() {
        let (tx, mut rx) = mpsc::channel(1);
        let sink = Sink {
            tx,
            job_id: "fixture".into(),
            dropped: Arc::new(AtomicU64::new(0)),
        };
        sink.event(
            "api.request",
            "GET /api/v1/days · 200".into(),
            Severity::Info,
        );
        sink.event(
            "api.request",
            "GET /api/v1/days · 200".into(),
            Severity::Info,
        );
        assert_eq!(sink.dropped.load(Ordering::Relaxed), 1);
        rx.try_recv().unwrap();
        sink.event(
            "api.request",
            "GET /api/v1/days · 200".into(),
            Severity::Info,
        );
        assert_eq!(rx.try_recv().unwrap().event_type, "service.overflow");
    }
}
