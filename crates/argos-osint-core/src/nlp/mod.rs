//! Process-shared, bounded local NLP. Tags are candidates, never evidence validators.
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, OnceLock,
};
use tokio::sync::{mpsc, oneshot};
pub mod cache;
mod engine;
pub(crate) mod select;
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
    #[default]
    Auto,
    Off,
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
enum Work {
    Annotation(Box<Job>),
    ActorNames {
        names: Vec<String>,
        response: std::sync::mpsc::SyncSender<Option<Vec<String>>>,
        enqueued: std::time::Instant,
        _bytes: BytePermit,
    },
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
    sender: mpsc::Sender<Work>,
    bytes: Arc<AtomicUsize>,
    pub readiness: Result<(), String>,
}
impl Service {
    pub fn shared() -> &'static Self {
        static SERVICE: OnceLock<Service> = OnceLock::new();
        SERVICE.get_or_init(Self::start)
    }
    fn start() -> Self {
        let (sender, mut receiver) = mpsc::channel::<Work>(32);
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("argos-nlp".into())
            .spawn(move || {
                let mut engine = engine::Engine::new();
                let _ = ready_tx.send(engine.as_ref().map(|_| ()).map_err(|e| e.clone()));
                let mut memory = cache::MemoryCache::default();
                while let Some(work) = receiver.blocking_recv() {
                    let job = match work {
                        Work::Annotation(job) => *job,
                        Work::ActorNames {
                            names,
                            response,
                            enqueued,
                            _bytes,
                        } => {
                            if enqueued.elapsed() >= std::time::Duration::from_secs(10) {
                                let _ = response.send(None);
                                continue;
                            }
                            if engine.is_err() {
                                engine = engine::Engine::new();
                            }
                            let cleaned =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    engine
                                        .as_mut()
                                        .ok()
                                        .and_then(|e| e.actor_names(&names).ok())
                                }))
                                .unwrap_or_default();
                            if cleaned.is_none() {
                                engine = Err("actor parser needs restart".into());
                            }
                            let _ = response.send(cleaned);
                            continue;
                        }
                    };
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
                                            match pending {
                                                Work::Annotation(pending) => {
                                                    let _ = pending.response.send(
                                                        Annotation::terminal(
                                                            &pending.source,
                                                            &pending.context,
                                                            Status::Failed,
                                                        ),
                                                    );
                                                }
                                                Work::ActorNames { response, .. } => {
                                                    let _ = response.send(None);
                                                }
                                            }
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
        self.reserve_bytes(source.prose.len())
    }
    fn reserve_bytes(&self, bytes: usize) -> Result<BytePermit, Status> {
        self.bytes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                n.checked_add(bytes).filter(|n| *n <= QUEUE_BYTES)
            })
            .map_err(|_| Status::Busy)?;
        Ok(BytePermit {
            bytes,
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
            .try_send(Work::Annotation(Box::new(Job {
                source,
                context,
                cancel,
                response,
                _bytes: permit,
                enqueued: std::time::Instant::now(),
            })))
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
            .send(Work::Annotation(Box::new(Job {
                source,
                context,
                cancel,
                response,
                _bytes: permit,
                enqueued: std::time::Instant::now(),
            })))
            .await
            .is_err()
        {
            return terminal;
        }
        rx.await.unwrap_or(terminal)
    }

    /// Bounded local maintenance, called outside the SQLite transaction. Busy
    /// or failed parsing is retried later rather than caching a false success.
    pub(crate) fn actor_names(&self, names: Vec<String>) -> Option<Vec<String>> {
        if !cfg!(feature = "nlp") {
            return Some(names);
        }
        if names.len() > 100 || names.iter().any(|name| name.len() > 1024) {
            return None;
        }
        let permit = self
            .reserve_bytes(names.iter().map(String::len).sum())
            .ok()?;
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        self.sender
            .try_send(Work::ActorNames {
                names,
                response: tx,
                enqueued: std::time::Instant::now(),
                _bytes: permit,
            })
            .ok()?;
        rx.recv_timeout(std::time::Duration::from_secs(12))
            .ok()
            .flatten()
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
    pub entity_packet: Vec<EntityCandidate>,
    pub selection: Option<Selection>,
    pub coverage: PreparationCoverage,
    pub effective_action: EffectiveAction,
    pub diagnostic: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EntityCandidate {
    pub id: String,
    pub type_hint: String,
    pub comparison_key: String,
    pub original_forms: Vec<String>,
    pub mention_refs: Vec<Span>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CoverageRequirement {
    Optional,
    Complete,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PreparePolicy {
    pub purpose: String,
    pub coverage_requirement: CoverageRequirement,
    pub need_entities: bool,
    /// Budget for serialized prose, after reserving mandatory request fields.
    pub byte_budget: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectiveAction {
    Verbatim,
    Selected,
    LegacyFallback,
    RequiresWindows,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PreparationCoverage {
    pub prose_hash: String,
    pub retained_ranges: Vec<Span>,
    pub complete: bool,
}

fn entity_candidates(source: &Source, annotation: &Annotation) -> Vec<EntityCandidate> {
    if annotation.status != Status::Complete || !annotation.validate(source) {
        return Vec::new();
    }
    let mut candidates: Vec<EntityCandidate> = Vec::new();
    for mention in &annotation.mentions {
        let Some(form) = mention.span.slice(&source.prose) else {
            continue;
        };
        if mention.kind == "person"
            && calendar_month_person(
                &source.prose,
                mention.span.start_byte,
                mention.span.end_byte,
                form,
                annotation
                    .date_candidates
                    .iter()
                    .map(|d| (d.span.start_byte, d.span.end_byte)),
            )
        {
            continue;
        }
        let key = form
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        if key.is_empty() {
            continue;
        }
        if let Some(candidate) = candidates
            .iter_mut()
            .find(|c| c.comparison_key == key && c.type_hint == mention.kind)
        {
            if !candidate.original_forms.iter().any(|s| s == form) {
                candidate.original_forms.push(form.into());
            }
            candidate.mention_refs.push(mention.span);
        } else {
            candidates.push(EntityCandidate {
                id: format!(
                    "{}:{}:entity:{}",
                    source.source_id,
                    source.prose_hash,
                    candidates.len()
                ),
                type_hint: mention.kind.clone(),
                comparison_key: key,
                original_forms: vec![form.into()],
                mention_refs: vec![mention.span],
            });
        }
    }
    candidates
}

/// Reject a standalone month in a temporal context, while preserving names.
pub(crate) fn calendar_month_person(
    text: &str,
    start: usize,
    end: usize,
    name: &str,
    dates: impl Iterator<Item = (usize, usize)>,
) -> bool {
    let name = name.trim().to_ascii_lowercase();
    if !matches!(
        name.as_str(),
        "january"
            | "february"
            | "march"
            | "april"
            | "may"
            | "june"
            | "july"
            | "august"
            | "september"
            | "october"
            | "november"
            | "december"
    ) {
        return false;
    }
    if dates.into_iter().any(|(a, b)| a <= start && end <= b) {
        return true;
    }
    let Some(prefix) = text.get(..start) else {
        return false;
    };
    let previous = prefix
        .split_whitespace()
        .next_back()
        .unwrap_or("")
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_ascii_lowercase();
    matches!(
        previous.as_str(),
        "in" | "since"
            | "during"
            | "until"
            | "through"
            | "late"
            | "early"
            | "mid"
            | "last"
            | "next"
            | "this"
    )
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
    let policy = PreparePolicy {
        purpose: purpose.into(),
        coverage_requirement: CoverageRequirement::Optional,
        need_entities: true,
        byte_budget: budget,
    };
    prepare_with_policy(source, &policy, focus, fallback_chars, cancel, mode).await
}

/// Complete local annotations using an explicit cache database. The policy
/// keeps entity preparation typed and does not permit pressure-driven selection.
/// None means process cache only, which is useful for isolated callers/tests.
pub async fn annotate_with_policy(
    source: &Source,
    policy: &PreparePolicy,
    cancel: Arc<AtomicBool>,
    mode: Mode,
    db_path: Option<&std::path::Path>,
) -> Annotation {
    let context = source.context();
    if mode == Mode::Off || !cfg!(feature = "nlp") {
        return Annotation::terminal(source, &context, Status::Disabled);
    }
    if cancel.load(Ordering::Acquire) {
        return Annotation::terminal(source, &context, Status::Cancelled);
    }
    let cached = db_path
        .and_then(|db| crate::store::Store::open_initialized_light(db).ok())
        .and_then(|s| cache::get(&s.conn, source, &context).ok().flatten());
    let annotation = match cached {
        Some(a) => a,
        None => {
            Service::shared()
                .annotate(source.clone(), context.clone(), cancel)
                .await
        }
    };
    if policy.coverage_requirement == CoverageRequirement::Complete
        && annotation.status == Status::Complete
        && !annotation.validate(source)
    {
        return Annotation::terminal(source, &context, Status::Failed);
    }
    if annotation.status == Status::Complete {
        if let Some(store) =
            db_path.and_then(|db| crate::store::Store::open_initialized_light(db).ok())
        {
            let _ = cache::put(&store.conn, source, &context, &annotation);
        }
    }
    annotation
}

pub async fn prepare_with_policy(
    source: Source,
    policy: &PreparePolicy,
    focus: &str,
    fallback_chars: usize,
    cancel: Arc<AtomicBool>,
    mode: Mode,
) -> Prepared {
    let serialized_bytes = serde_json::to_vec(&source.prose).map_or(usize::MAX, |s| s.len());
    let whole = policy.coverage_requirement == CoverageRequirement::Complete
        || (mode == Mode::Auto && serialized_bytes <= policy.byte_budget);
    let fallback = || Prepared {
        text: if whole {
            source.prose.clone()
        } else {
            source.prose.chars().take(fallback_chars).collect()
        },
        entity_packet: Vec::new(),
        selection: None,
        coverage: PreparationCoverage {
            prose_hash: source.prose_hash.clone(),
            retained_ranges: if source.prose.is_empty() {
                vec![]
            } else {
                vec![Span {
                    start_byte: 0,
                    end_byte: if whole {
                        source.prose.len()
                    } else {
                        source
                            .prose
                            .char_indices()
                            .nth(fallback_chars)
                            .map_or(source.prose.len(), |(i, _)| i)
                    },
                }]
            },
            complete: whole || source.prose.chars().count() <= fallback_chars,
        },
        effective_action: if whole && serialized_bytes > policy.byte_budget {
            EffectiveAction::RequiresWindows
        } else if whole {
            EffectiveAction::Verbatim
        } else {
            EffectiveAction::LegacyFallback
        },
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
    let db_path = crate::paths::db_path();
    let a = annotate_with_policy(&source, policy, cancel.clone(), mode, Some(&db_path)).await;
    let candidates = entity_candidates(&source, &a);
    if (mode == Mode::Auto && whole)
        || mode == Mode::Annotate
        || policy.coverage_requirement == CoverageRequirement::Complete
    {
        let mut out = fallback();
        if policy.need_entities {
            out.entity_packet = candidates;
        }
        if policy.need_entities && out.entity_packet.is_empty() {
            out.diagnostic =
                Some("NLP yielded no entities; native/model discovery required".into());
        }
        if a.status != Status::Complete {
            out.diagnostic = Some(format!(
                "NLP {:?}; native/model discovery required",
                a.status
            ));
        }
        return out;
    }
    if cancel.load(Ordering::Acquire) {
        let mut out = fallback();
        out.diagnostic = Some("NLP Cancelled".into());
        return out;
    }
    match select(&a, &source, &policy.purpose, focus, policy.byte_budget) {
        Ok(selection) => match selection.text(&source) {
            Ok(text) => Prepared {
                text,
                entity_packet: if policy.need_entities {
                    candidates
                } else {
                    Vec::new()
                },
                coverage: PreparationCoverage {
                    prose_hash: source.prose_hash.clone(),
                    retained_ranges: selection.ranges.clone(),
                    complete: selection.omitted_sentence_ids.is_empty(),
                },
                effective_action: EffectiveAction::Selected,
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
    if let Ok(store) = crate::store::Store::open(&crate::paths::db_path()) {
        let _ = cache::artifact(&store.conn, text);
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
