//! Process-shared, bounded local NLP. Tags are candidates, never evidence validators.
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, OnceLock,
};
use tokio::sync::{mpsc, oneshot};
pub mod cache;
mod engine;
mod select;
pub mod spans;
pub use select::{select, Selection, SelectionError};
pub use spans::Span;

pub const SCHEMA: u32 = 1;
pub const MAX_SOURCE: usize = 2 * 1024 * 1024;
pub const MAX_OUTPUT: usize = 4 * 1024 * 1024;
const QUEUE_BYTES: usize = 16 * 1024 * 1024;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Off,
    #[default]
    Annotate,
    Select,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub mode: Mode,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Source {
    pub source_id: String,
    pub revision: String,
    pub attempt_id: String,
    pub raw_hash: Option<String>,
    pub prose_hash: String,
    pub prose: String,
    pub language_hint: Option<String>,
    pub published_at: Option<String>,
    pub retrieved_at: String,
}
impl Source {
    pub fn derived(id: &str, revision: &str, prose: &str) -> Self {
        Self {
            source_id: id.into(),
            revision: revision.into(),
            attempt_id: "historical-derived".into(),
            raw_hash: None,
            prose_hash: cache::hash(prose),
            prose: prose.into(),
            language_hint: None,
            published_at: None,
            retrieved_at: chrono::Utc::now().to_rfc3339(),
        }
    }
    pub fn context(&self) -> ParseContext {
        ParseContext {
            reference_date: self
                .published_at
                .as_ref()
                .unwrap_or(&self.retrieved_at)
                .chars()
                .take(10)
                .collect(),
            ..Default::default()
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ParseContext {
    pub reference_date: String,
    pub timezone: String,
    pub date_order: String,
}
impl Default for ParseContext {
    fn default() -> Self {
        Self {
            reference_date: chrono::Utc::now().format("%Y-%m-%d").to_string(),
            timezone: "UTC".into(),
            date_order: "unknown".into(),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Complete,
    Disabled,
    Unsupported,
    Busy,
    TooLarge,
    Cancelled,
    Timeout,
    Failed,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sentence {
    pub id: String,
    pub span: Span,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Mention {
    pub span: Span,
    pub kind: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DateCandidate {
    pub span: Span,
    pub normalized: Option<String>,
    pub flags: Vec<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ParseMetrics {
    pub queue_wait_ms: u64,
    pub parser_ms: u64,
    pub cache_hit: bool,
    pub input_bytes: usize,
    pub result_bytes: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Annotation {
    pub schema_version: u32,
    pub feature_key: String,
    pub prose_hash: String,
    pub status: Status,
    pub sentences: Vec<Sentence>,
    pub mentions: Vec<Mention>,
    pub date_candidates: Vec<DateCandidate>,
    pub numerical_spans: Vec<Span>,
    pub flags: Vec<String>,
    #[serde(default)]
    pub metrics: ParseMetrics,
}
impl Annotation {
    fn terminal(source: &Source, context: &ParseContext, status: Status) -> Self {
        Self {
            schema_version: SCHEMA,
            feature_key: cache::feature_key(source, context),
            prose_hash: source.prose_hash.clone(),
            status,
            sentences: vec![],
            mentions: vec![],
            date_candidates: vec![],
            numerical_spans: vec![],
            flags: vec![],
            metrics: ParseMetrics::default(),
        }
    }
    pub fn validate(&self, source: &Source) -> bool {
        self.schema_version == SCHEMA
            && self.prose_hash == cache::hash(&source.prose)
            && self
                .sentences
                .iter()
                .all(|s| s.span.slice(&source.prose).is_some())
            && self
                .mentions
                .iter()
                .all(|s| s.span.slice(&source.prose).is_some())
            && self
                .date_candidates
                .iter()
                .all(|s| s.span.slice(&source.prose).is_some())
            && self
                .numerical_spans
                .iter()
                .all(|s| s.slice(&source.prose).is_some())
    }
}
struct Job {
    source: Source,
    context: ParseContext,
    cancel: Arc<AtomicBool>,
    response: oneshot::Sender<Annotation>,
    _bytes: BytePermit,
    enqueued: std::time::Instant,
}
struct BytePermit {
    bytes: usize,
    used: Arc<AtomicUsize>,
}
impl Drop for BytePermit {
    fn drop(&mut self) {
        self.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}
pub struct Service {
    sender: mpsc::Sender<Job>,
    bytes: Arc<AtomicUsize>,
    pub readiness: Result<(), String>,
}
impl Service {
    pub fn shared() -> &'static Self {
        static SERVICE: OnceLock<Service> = OnceLock::new();
        SERVICE.get_or_init(Self::start)
    }
    fn start() -> Self {
        let (sender, mut receiver) = mpsc::channel::<Job>(32);
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("argos-nlp".into())
            .spawn(move || {
                let mut engine = engine::Engine::new();
                let _ = ready_tx.send(engine.as_ref().map(|_| ()).map_err(|e| e.clone()));
                let mut memory = cache::MemoryCache::default();
                while let Some(job) = receiver.blocking_recv() {
                    let started = std::time::Instant::now();
                    let queue_wait_ms = job.enqueued.elapsed().as_millis() as u64;
                    let mut cache_hit = false;
                    let mut result =
                        Annotation::terminal(&job.source, &job.context, Status::Failed);
                    if job.cancel.load(Ordering::Acquire) {
                        result.status = Status::Cancelled;
                    } else if job
                        .source
                        .language_hint
                        .as_deref()
                        .is_some_and(|s| !s.starts_with("en"))
                    {
                        result.status = Status::Unsupported;
                    } else if let Some(hit) = memory.get(&result.feature_key) {
                        cache_hit = true;
                        result = hit;
                    } else {
                        if engine.is_err() {
                            engine = engine::Engine::new();
                        }
                        match engine.as_mut() {
                            Ok(e) => {
                                result =
                                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                        e.annotate(&job.source, &job.context, job.cancel.clone())
                                    }))
                                    .unwrap_or_else(|_| {
                                        while let Ok(pending) = receiver.try_recv() {
                                            let _ = pending.response.send(Annotation::terminal(
                                                &pending.source,
                                                &pending.context,
                                                Status::Failed,
                                            ));
                                        }
                                        let mut failed = Annotation::terminal(
                                            &job.source,
                                            &job.context,
                                            Status::Failed,
                                        );
                                        failed.flags.push("worker recovered from panic".into());
                                        failed
                                    });
                                if result.status == Status::Complete {
                                    memory.put(result.clone());
                                } else if matches!(result.status, Status::Failed | Status::Timeout)
                                {
                                    engine = engine::Engine::new();
                                }
                            }
                            Err(e) => result.flags.push(e.clone()),
                        }
                    }
                    result.metrics = ParseMetrics {
                        queue_wait_ms,
                        parser_ms: if cache_hit {
                            0
                        } else {
                            started.elapsed().as_millis() as u64
                        },
                        cache_hit,
                        input_bytes: job.source.prose.len(),
                        result_bytes: serde_json::to_vec(&result).map_or(0, |v| v.len()),
                    };
                    let _ = job.response.send(result);
                }
            })
            .expect("start NLP worker");
        Self {
            sender,
            bytes: Arc::new(AtomicUsize::new(0)),
            readiness: ready_rx.recv().unwrap_or_else(|e| Err(e.to_string())),
        }
    }
    fn permit(&self, source: &Source) -> Result<BytePermit, Status> {
        if cache::hash(&source.prose) != source.prose_hash {
            return Err(Status::Failed);
        }
        if source.prose.len() > MAX_SOURCE {
            return Err(Status::TooLarge);
        }
        self.bytes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                n.checked_add(source.prose.len())
                    .filter(|n| *n <= QUEUE_BYTES)
            })
            .map_err(|_| Status::Busy)?;
        Ok(BytePermit {
            bytes: source.prose.len(),
            used: self.bytes.clone(),
        })
    }
    pub fn try_submit(
        &self,
        source: Source,
        context: ParseContext,
        cancel: Arc<AtomicBool>,
    ) -> Result<oneshot::Receiver<Annotation>, Status> {
        if !cfg!(feature = "nlp") {
            return Err(Status::Disabled);
        }
        let permit = self.permit(&source)?;
        let (response, rx) = oneshot::channel();
        self.sender
            .try_send(Job {
                source,
                context,
                cancel,
                response,
                _bytes: permit,
                enqueued: std::time::Instant::now(),
            })
            .map_err(|e| {
                if matches!(e, mpsc::error::TrySendError::Closed(_)) {
                    Status::Failed
                } else {
                    Status::Busy
                }
            })?;
        Ok(rx)
    }
    pub async fn annotate(
        &self,
        source: Source,
        context: ParseContext,
        cancel: Arc<AtomicBool>,
    ) -> Annotation {
        if !cfg!(feature = "nlp") {
            return Annotation::terminal(&source, &context, Status::Disabled);
        }
        let terminal = Annotation::terminal(&source, &context, Status::Failed);
        let permit = match self.permit(&source) {
            Ok(p) => p,
            Err(s) => return Annotation::terminal(&source, &context, s),
        };
        let (response, rx) = oneshot::channel();
        if self
            .sender
            .send(Job {
                source,
                context,
                cancel,
                response,
                _bytes: permit,
                enqueued: std::time::Instant::now(),
            })
            .await
            .is_err()
        {
            return terminal;
        }
        rx.await.unwrap_or(terminal)
    }
}

pub fn configured_mode() -> Mode {
    crate::provider::SettingsFile::load()
        .map(|s| s.nlp.mode)
        .unwrap_or_default()
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Prepared {
    pub text: String,
    pub selection: Option<Selection>,
    pub diagnostic: Option<String>,
}
/// Prepare before legacy cuts; incomplete work always returns the caller's bounded fallback.
pub async fn prepare(
    source: Source,
    purpose: &str,
    focus: &str,
    budget: usize,
    fallback_chars: usize,
    cancel: Arc<AtomicBool>,
) -> Prepared {
    prepare_mode(
        source,
        purpose,
        focus,
        budget,
        fallback_chars,
        cancel,
        configured_mode(),
    )
    .await
}
pub async fn prepare_mode(
    source: Source,
    purpose: &str,
    focus: &str,
    budget: usize,
    fallback_chars: usize,
    cancel: Arc<AtomicBool>,
    mode: Mode,
) -> Prepared {
    let fallback = || Prepared {
        text: source.prose.chars().take(fallback_chars).collect(),
        selection: None,
        diagnostic: None,
    };
    if mode == Mode::Off {
        return fallback();
    }
    if !cfg!(feature = "nlp") {
        let mut out = fallback();
        out.diagnostic = Some("NLP Disabled".into());
        return out;
    }
    if cancel.load(Ordering::Acquire) {
        let mut out = fallback();
        out.diagnostic = Some("NLP Cancelled".into());
        return out;
    }
    let context = source.context();
    let cached = {
        crate::store::Store::open(&crate::paths::db_path())
            .ok()
            .and_then(|s| cache::get(&s.conn, &source, &context).ok().flatten())
    };
    let a = match cached {
        Some(a) => a,
        None => {
            Service::shared()
                .annotate(source.clone(), context.clone(), cancel.clone())
                .await
        }
    };
    if a.status == Status::Complete {
        if let Ok(store) = crate::store::Store::open(&crate::paths::db_path()) {
            let _ = cache::put(&store.conn, &source, &context, &a);
        }
    }
    if mode != Mode::Select {
        return fallback();
    }
    if cancel.load(Ordering::Acquire) {
        let mut out = fallback();
        out.diagnostic = Some("NLP Cancelled".into());
        return out;
    }
    match select(&a, &source, purpose, focus, budget) {
        Ok(selection) => match selection.text(&source) {
            Ok(text) => Prepared {
                text,
                selection: Some(selection),
                diagnostic: None,
            },
            Err(e) => {
                let mut out = fallback();
                out.diagnostic = Some(format!("NLP {e:?}"));
                out
            }
        },
        Err(e) => {
            let mut out = fallback();
            out.diagnostic = Some(format!("NLP {:?}: {e:?}", a.status));
            out
        }
    }
}

/// UTF-8 wire accounting (escaping, message fields and supplied schema included).
pub fn message_bytes(
    messages: &[crate::provider::ChatMessage],
    schema: &serde_json::Value,
) -> usize {
    let messages:Vec<_>=messages.iter().map(|m|serde_json::json!({"role":m.role,"content":m.content,"tool_call_id":m.tool_call_id,"tool_calls":m.tool_calls.iter().map(|t|serde_json::json!({"id":t.id,"type":"function","function":{"name":t.name,"arguments":t.arguments}})).collect::<Vec<_>>()})).collect();
    serde_json::to_vec(&serde_json::json!({"messages":messages,"schema":schema}))
        .map_or(usize::MAX, |v| v.len())
}

/// Persist coverage beside its exact derived artifact. Acquisition provenance is never inferred.
pub fn record_view(owner_kind: &str, owner_id: &str, source: &Source, prepared: &Prepared) {
    if configured_mode() == Mode::Off {
        return;
    }
    if let Ok(store) = crate::store::Store::open(&crate::paths::db_path()) {
        if cache::register(
            &store.conn,
            owner_kind,
            owner_id,
            source,
            None,
            "historical-prose-v1",
        )
        .is_ok()
        {
            if let Ok(json) = serde_json::to_string(prepared) {
                let _=store.conn.execute("UPDATE nlp_source_refs SET metadata_json=json_set(metadata_json,'$.selection',json(?4)) WHERE owner_kind=?1 AND owner_id=?2 AND prose_hash=?3 AND transform_version='historical-prose-v1'",rusqlite::params![owner_kind,owner_id,source.prose_hash,json]);
            }
        }
    }
}

/// Acquisition reference placed in bounded observations; the exact body stays in artifacts.
pub fn capture_page(url: &str, text: &str) -> serde_json::Value {
    let hash = cache::hash(text);
    if configured_mode() != Mode::Off {
        if let Ok(store) = crate::store::Store::open(&crate::paths::db_path()) {
            let _ = cache::artifact(&store.conn, text);
        }
    }
    serde_json::json!({"source_id":url,"artifact_hash":hash,"byte_len":text.len()})
}

/// Restore only known artifact-backed page fields before native extraction and model gates.
pub fn expanded_observations(value: &serde_json::Value) -> serde_json::Value {
    fn restore(page: &mut serde_json::Value, conn: &rusqlite::Connection) {
        let Some(hash) = page
            .pointer("/nlp_source/artifact_hash")
            .and_then(serde_json::Value::as_str)
        else {
            return;
        };
        let text = conn.query_row(
            "SELECT text FROM nlp_artifacts WHERE hash=?1 AND byte_len<=?2",
            rusqlite::params![hash, MAX_SOURCE as i64],
            |r| r.get::<_, String>(0),
        );
        if let Ok(text) = text {
            if cache::hash(&text) == hash {
                page["markdown"] = serde_json::json!(text);
            }
        }
    }
    let mut out = value.clone();
    if let Ok(store) = crate::store::Store::open(&crate::paths::db_path()) {
        if let Some(pages) = out
            .get_mut("pages")
            .and_then(serde_json::Value::as_array_mut)
        {
            for page in pages {
                restore(page, &store.conn);
            }
        } else {
            restore(&mut out, &store.conn);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn worker_contract() {
        let s=Source::derived("fixture","1","😀 Alice Smith said tomorrow in Geneva. Alice Smith denied $1.2 billion, not $2 billion.");
        let ctx = ParseContext {
            reference_date: "2026-10-11".into(),
            ..Default::default()
        };
        let a = Service::shared()
            .annotate(s.clone(), ctx, Arc::new(AtomicBool::new(false)))
            .await;
        #[cfg(feature = "nlp")]
        {
            assert!(
                Service::shared().readiness.is_ok(),
                "{:?}",
                Service::shared().readiness
            );
            assert_eq!(a.status, Status::Complete, "{:?}", a.flags);
            assert!(a.validate(&s));
            assert_eq!(
                a.mentions
                    .iter()
                    .filter(|m| m.span.slice(&s.prose) == Some("Alice Smith"))
                    .count(),
                2,
                "{:?}",
                a.mentions
            );
            assert!(
                a.numerical_spans
                    .iter()
                    .any(|r| r.slice(&s.prose) == Some("$1.2 billion")),
                "{:?}",
                a.numerical_spans
            );
            assert!(
                a.date_candidates.iter().any(|d| d
                    .normalized
                    .as_ref()
                    .is_some_and(|v| v.starts_with("2026-10-12"))),
                "{:?}",
                a.date_candidates
            );
        }
        #[cfg(not(feature = "nlp"))]
        assert_eq!(a.status, Status::Disabled);
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod contract_tests;
