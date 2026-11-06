//! QuickJS Inference and Candidate Worker Engine for Argos Classifier.
//!
//! Evaluates the compiled Natural Bayes bundle and bridge protocol in a
//! dedicated OS worker thread to satisfy QuickJS thread confinement (`!Send`).
//! Memory is tracked and bounded via `crate::js_runtime_budget`.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::artifact::{sha256_hex, ModelId, ValidatedArtifact, WriterLease};
use super::{
    BridgeOperation, ClassificationReceipt, ClassifierError, ClassifierPerformance,
    ClassifierService, ClassifierStatus, ClassifierWorker, ClassifyInput, CompiledIdentity,
    DomainScore, InferenceEngine, InferenceMetrics, ModelSnapshot, RawDomainScore, ReceiptPolicy,
    WorkControl, INFERENCE_BATCH_SIZE, RUNTIME_MEMORY_LIMIT_BYTES, RUNTIME_STACK_LIMIT_BYTES,
};
use crate::atlas_taxonomy::DomainId;
use crate::js_runtime_budget::{classifier_process_budget, ReservationError, RuntimeReservation};

const BUNDLE_SRC: &str = include_str!("../../assets/classifier/natural-classifier.js");
const FEATURES_SRC: &str = include_str!("features.js");
const VOCABULARY_SRC: &str = include_str!("vocabulary.js");
const BRIDGE_SRC: &str = include_str!("bridge.js");
const ADAPT_SRC: &str = include_str!("../../assets/classifier/natural-bayes.adapt.js");
const MANIFEST_SRC: &str = include_str!("../../assets/classifier/source-manifest.json");

/// Total byte reservation required per QuickJS runtime (128 MiB heap + 1 MiB stack).
pub const RUNTIME_RESERVATION_BYTES: usize = RUNTIME_MEMORY_LIMIT_BYTES + RUNTIME_STACK_LIMIT_BYTES;

#[derive(serde::Deserialize)]
struct VocabularyPrepared {
    version: String,
    request_id: String,
    prepared: usize,
    documents: usize,
    terms: usize,
    errors: Vec<String>,
}
#[derive(serde::Deserialize)]
struct VocabularyFrozen {
    version: String,
    request_id: String,
    vocabulary_frozen: bool,
    documents: usize,
    terms: usize,
    errors: Vec<String>,
}

enum WorkerMessage {
    BridgeCall {
        operation: BridgeOperation,
        request: String,
        cancel: Arc<AtomicBool>,
        deadline: Instant,
        enqueued_at: Instant,
        queued_deadline: Instant,
        execution_timeout: Duration,
        reply: SyncSender<Result<String, ClassifierError>>,
    },
    #[cfg(test)]
    RestoreSmoke {
        payload: String,
        reply: SyncSender<Result<(), ClassifierError>>,
    },
}

const MAX_QUEUED_BYTES: usize = 16 * 1024 * 1024;

struct QueueReservation {
    bytes: usize,
    total: Arc<AtomicUsize>,
}
impl Drop for QueueReservation {
    fn drop(&mut self) {
        self.total.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}
struct QueuedMessage {
    message: WorkerMessage,
    reservation: QueueReservation,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueueSendError {
    Full,
    Disconnected,
}

struct BoundedSender {
    sender: SyncSender<QueuedMessage>,
    queued_bytes: Arc<AtomicUsize>,
    observations: Arc<Mutex<Option<WorkerObservations>>>,
}
impl BoundedSender {
    fn try_send(&self, message: WorkerMessage) -> Result<(), QueueSendError> {
        let bytes = match &message {
            WorkerMessage::BridgeCall { request, .. } => request.len(),
            #[cfg(test)]
            WorkerMessage::RestoreSmoke { payload, .. } => payload.len(),
        };
        if self
            .queued_bytes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |total| {
                total
                    .checked_add(bytes)
                    .filter(|sum| *sum <= MAX_QUEUED_BYTES)
            })
            .is_err()
        {
            return Err(QueueSendError::Full);
        }
        let queued = QueuedMessage {
            message,
            reservation: QueueReservation {
                bytes,
                total: self.queued_bytes.clone(),
            },
        };
        self.sender.try_send(queued).map_err(|error| match error {
            TrySendError::Full(_) => QueueSendError::Full,
            TrySendError::Disconnected(_) => QueueSendError::Disconnected,
        })
    }
}

// A handle never releases its runtime reservation before its owning thread exits.
// Shutdown interrupts active JS and wakes an idle receiver within 100 ms.
struct WorkerThread {
    handle: Option<thread::JoinHandle<()>>,
    shutdown: Arc<AtomicBool>,
    _reservation: Option<RuntimeReservation>,
}
impl WorkerThread {
    fn stop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        // Release only after the real worker (and its QuickJS context) has exited.
        self._reservation.take();
    }
}
impl Drop for WorkerThread {
    fn drop(&mut self) {
        self.stop();
    }
}

struct WorkerObservations {
    queue_micros: u64,
    samples: VecDeque<(u64, usize)>,
    heap_current: u64,
    heap_peak: u64,
    reserved: u64,
}
impl WorkerObservations {
    fn heap(runtime: &rquickjs::Runtime) -> Result<u64, ClassifierError> {
        u64::try_from(runtime.memory_usage().memory_used_size)
            .map_err(|_| ClassifierError::WorkerStopped)
    }
    fn snapshot(&self) -> ClassifierPerformance {
        ClassifierPerformance {
            queue_micros: self.queue_micros,
            batch_execution_micros: self.samples.iter().map(|(duration, _)| *duration).collect(),
            batch_sizes: self.samples.iter().map(|(_, size)| *size).collect(),
            js_heap_current_bytes: self.heap_current,
            js_heap_peak_bytes: self.heap_peak,
            runtime_reserved_bytes: self.reserved,
            stack_limit_bytes: RUNTIME_STACK_LIMIT_BYTES as u64,
        }
    }
    fn observe(
        &mut self,
        heap: u64,
        sample: Option<(u128, u128, usize)>,
    ) -> Result<(), ClassifierError> {
        self.heap_current = heap;
        self.heap_peak = self.heap_peak.max(heap);
        if let Some((queue, execution, size)) = sample {
            let queue = u64::try_from(queue).map_err(|_| ClassifierError::WorkerStopped)?;
            let execution = u64::try_from(execution).map_err(|_| ClassifierError::WorkerStopped)?;
            self.queue_micros = self
                .queue_micros
                .checked_add(queue)
                .ok_or(ClassifierError::WorkerStopped)?;
            if self.samples.len() == 4096 {
                self.samples.pop_front();
            }
            self.samples.push_back((execution, size));
        }
        Ok(())
    }
}

/// Computes the hex SHA-256 of the compiled Natural Bayes bundle.
#[must_use]
pub fn compiled_bundle_sha256() -> String {
    sha256_hex(BUNDLE_SRC.as_bytes())
}

/// Returns the trusted compiled identity of the classifier assets embedded
/// in this binary.
pub fn compiled_identity() -> Result<CompiledIdentity, ClassifierError> {
    let manifest_json: serde_json::Value = serde_json::from_str(MANIFEST_SRC).map_err(|e| {
        ClassifierError::InvalidArtifact(format!("source manifest JSON parse: {e}"))
    })?;

    let natural_ver = manifest_json
        .pointer("/upstream/natural/version")
        .and_then(serde_json::Value::as_str)
        .filter(|version| !version.is_empty())
        .ok_or_else(|| ClassifierError::InvalidArtifact("missing upstream Natural version".into()))?
        .to_string();

    let apparatus_ver = manifest_json
        .pointer("/upstream/apparatus/version")
        .and_then(serde_json::Value::as_str)
        .filter(|version| !version.is_empty())
        .ok_or_else(|| {
            ClassifierError::InvalidArtifact("missing upstream apparatus version".into())
        })?
        .to_string();

    for (pointer, bytes) in [
        ("/build/bundle_sha256", BUNDLE_SRC.as_bytes()),
        ("/adaptation/adapter_sha256", ADAPT_SRC.as_bytes()),
        ("/bridge/sha256", BRIDGE_SRC.as_bytes()),
        ("/features/sha256", FEATURES_SRC.as_bytes()),
        ("/vocabulary/sha256", VOCABULARY_SRC.as_bytes()),
    ] {
        let expected = manifest_json
            .pointer(pointer)
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                ClassifierError::InvalidArtifact(format!("missing compiled asset hash {pointer}"))
            })?;
        if expected != sha256_hex(bytes) {
            return Err(ClassifierError::Incompatible(format!(
                "compiled asset integrity mismatch: {pointer}"
            )));
        }
    }
    if manifest_json
        .pointer("/bridge/version")
        .and_then(serde_json::Value::as_str)
        != Some(super::BRIDGE_VERSION)
    {
        return Err(ClassifierError::Incompatible(
            "compiled bridge version mismatch".into(),
        ));
    }
    Ok(CompiledIdentity {
        bundle_sha256: sha256_hex(BUNDLE_SRC.as_bytes()),
        bridge_sha256: sha256_hex(BRIDGE_SRC.as_bytes()),
        adapter_sha256: sha256_hex(ADAPT_SRC.as_bytes()),
        source_manifest_sha256: sha256_hex(MANIFEST_SRC.as_bytes()),
        bridge_version: super::BRIDGE_VERSION.to_string(),
        natural_version: natural_ver,
        apparatus_version: apparatus_ver,
    })
}

pub(super) fn javascript_error(
    ctx: &rquickjs::Ctx<'_>,
    error: rquickjs::Error,
    operation: &str,
) -> ClassifierError {
    if matches!(error, rquickjs::Error::Allocation) {
        return ClassifierError::RuntimeLimitExceeded {
            resource: "heap".into(),
        };
    }
    if error.is_exception() {
        let thrown = ctx.catch();
        let message = thrown
            .as_object()
            .and_then(|object| rquickjs::Exception::from_object(object.clone()))
            .and_then(|exception| exception.message());
        match message.as_deref() {
            Some("out of memory") => {
                return ClassifierError::RuntimeLimitExceeded {
                    resource: "heap".into(),
                }
            }
            Some("stack overflow" | "Maximum call stack size exceeded") => {
                return ClassifierError::RuntimeLimitExceeded {
                    resource: "stack".into(),
                }
            }
            _ => {}
        }
    }
    ClassifierError::BridgeProtocol {
        operation: operation.into(),
        code: "JavaScript execution failed".into(),
    }
}

#[cfg(test)]
static FIXTURE_STARTED: Mutex<Option<SyncSender<()>>> = Mutex::new(None);

#[cfg(test)]
fn fixed_runtime_fixture(
    ctx: &rquickjs::Ctx<'_>,
    request: &str,
) -> Option<Result<String, ClassifierError>> {
    let source = match request {
        "fixture:interrupt-loop" => "while (true) {}",
        "fixture:heap-limit" => {
            "const chunks=[]; for(let i=0;i<140;i++) chunks.push(new Uint8Array(1024*1024)); 'done'"
        }
        "fixture:stack-limit" => "function recurse(){return 1+recurse()} recurse(); 'done'",
        "fixture:tokenize-once" => {
            r#"
            (() => {
                const stemmer=globalThis.argosNaturalBayes.RegexpTokenizer.prototype;
                const original=stemmer.tokenize;
                let count=0;
                stemmer.tokenize=function(...args){ count++; return original.apply(this,args); };
                try {
                    const base={version:'1',model_id:'token-once'};
                    const rows=[{row_id:'row-a',description:'',title:'ballot patrol common',membership:Array(6).fill('Present')},{row_id:'row-b',description:'',title:'ballot policy common',membership:Array(6).fill('Absent')}];
                    const train=JSON.parse(argosClassifier.trainBatch(JSON.stringify({...base,request_id:'a',rows})));
                    if(train.error || count!==2) throw new Error('train token count');
                    const fit=JSON.parse(argosClassifier.finishTraining(JSON.stringify({...base,request_id:'b',max_documents_per_step:16})));
                    if(fit.error || !fit.done || count!==2) throw new Error('fit retokenized');
                    const result=JSON.parse(argosClassifier.classifyBatch(JSON.stringify({...base,request_id:'c',items:[{id:'query',title:'ballot common',description:'policy patrol'}]})));
                    if(result.error || count!==4 || result.items[0].results.length!==6) throw new Error('inference token count');
                    return 'done';
                } finally { stemmer.tokenize=original; }
            })()
        "#
        }
        _ => return None,
    };
    if let Some(started) = FIXTURE_STARTED.lock().expect("fixture signal").take() {
        let _ = started.send(());
    }
    Some(
        ctx.eval::<String, _>(source)
            .map_err(|error| javascript_error(ctx, error, "fixed-runtime-fixture")),
    )
}

fn call_bridge(
    ctx: &rquickjs::Ctx<'_>,
    op_name: &str,
    req_json: &str,
) -> Result<String, ClassifierError> {
    #[cfg(test)]
    if let Some(result) = fixed_runtime_fixture(ctx, req_json) {
        return result;
    }
    let globals = ctx.globals();
    let bridge: rquickjs::Object =
        globals
            .get("argosClassifier")
            .map_err(|e| ClassifierError::BridgeProtocol {
                operation: op_name.to_string(),
                code: format!("missing `argosClassifier` global: {e}"),
            })?;
    let func: rquickjs::Function =
        bridge
            .get(op_name)
            .map_err(|e| ClassifierError::BridgeProtocol {
                operation: op_name.to_string(),
                code: format!("missing method `{op_name}` on `argosClassifier`: {e}"),
            })?;
    let res: String = func
        .call((req_json,))
        .map_err(|error| javascript_error(ctx, error, op_name))?;
    if let Ok(response) = serde_json::from_str::<serde_json::Value>(&res) {
        if response.get("code").and_then(serde_json::Value::as_str)
            == Some("feature_order_mismatch")
        {
            let message = response
                .get("error")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("feature_order_mismatch");
            if let Some(domain) = message.rsplit(": ").next().and_then(DomainId::parse) {
                return Err(ClassifierError::FeatureOrderMismatch { domain });
            }
            return Err(ClassifierError::BridgeProtocol {
                operation: op_name.into(),
                code: "feature_order_mismatch".into(),
            });
        }
    }
    Ok(res)
}

fn startup_control() -> WorkControl {
    let mut control = WorkControl::new();
    control.execution_timeout = Duration::from_secs(10);
    control
}

fn restore_in_context(
    ctx: &rquickjs::Ctx<'_>,
    model: &ModelId,
    payload: &str,
) -> Result<(), ClassifierError> {
    let request_id = "startup-restore";
    let request = serde_json::json!({"version":"1", "request_id":request_id,"model_id":model.as_str(),"artifact":payload}).to_string();
    let output = call_bridge(ctx, "restoreModel", &request)?;
    let parsed: serde_json::Value = serde_json::from_str(&output).map_err(|error| {
        ClassifierError::InvalidArtifact(format!("restore reply JSON: {error}"))
    })?;
    if parsed["version"] != "1"
        || parsed["request_id"] != request_id
        || parsed["restored_model_id"] != model.as_str()
        || parsed["smoke_ok"] != true
        || parsed["errors"]
            .as_array()
            .is_none_or(|errors| !errors.is_empty())
        || parsed.get("error").is_some()
    {
        return Err(ClassifierError::InvalidArtifact(format!(
            "restore smoke rejected: {output}"
        )));
    }
    Ok(())
}

fn spawn_worker_thread(name: &str) -> Result<(BoundedSender, WorkerThread), ClassifierError> {
    spawn_worker_with_initializer(name, || Ok(()))
}

fn spawn_worker_with_initializer(
    name: &str,
    initializer: impl FnOnce() -> Result<(), ClassifierError> + Send + 'static,
) -> Result<(BoundedSender, WorkerThread), ClassifierError> {
    spawn_worker_with_startup(name, initializer, None)
}

fn spawn_worker_with_startup(
    name: &str,
    initializer: impl FnOnce() -> Result<(), ClassifierError> + Send + 'static,
    startup: Option<(ModelId, String, WorkControl)>,
) -> Result<(BoundedSender, WorkerThread), ClassifierError> {
    if startup
        .as_ref()
        .is_some_and(|(_, payload, _)| payload.len() as u64 > super::artifact::MAX_ARTIFACT_BYTES)
    {
        return Err(ClassifierError::ModelTooLarge);
    }
    let reservation = classifier_process_budget()
        .try_reserve(RUNTIME_RESERVATION_BYTES)
        .map_err(|e| match e {
            ReservationError::Exhausted {
                requested,
                reserved,
                limit,
            } => ClassifierError::RuntimeBudgetExceeded {
                requested,
                reserved,
                limit,
            },
            ReservationError::InvalidRequest { requested, limit } => {
                ClassifierError::RuntimeBudgetExceeded {
                    requested,
                    reserved: 0,
                    limit,
                }
            }
        })?;

    let (tx, rx) = sync_channel::<QueuedMessage>(32);
    let observations = Arc::new(Mutex::new(None));
    let worker_observations = observations.clone();
    let reserved_bytes = reservation.bytes() as u64;
    let (initialized_tx, initialized_rx) = sync_channel(1);
    let shutdown = Arc::new(AtomicBool::new(false));
    let worker_shutdown = shutdown.clone();

    let handle = thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            if let Err(error) = initializer() {
                let _ = initialized_tx.send(Err(error));
                return;
            }
            worker_loop(
                rx,
                initialized_tx,
                worker_shutdown,
                worker_observations,
                reserved_bytes,
                startup,
            );
        })
        .map_err(|e| ClassifierError::Io(format!("spawn worker thread `{name}`: {e}")))?;

    let worker_thread = WorkerThread {
        handle: Some(handle),
        shutdown,
        _reservation: Some(reservation),
    };
    initialized_rx
        .recv()
        .map_err(|_| ClassifierError::WorkerStopped)??;
    Ok((
        BoundedSender {
            sender: tx,
            queued_bytes: Arc::new(AtomicUsize::new(0)),
            observations,
        },
        worker_thread,
    ))
}

fn worker_loop(
    rx: Receiver<QueuedMessage>,
    initialized: SyncSender<Result<(), ClassifierError>>,
    shutdown: Arc<AtomicBool>,
    observations: Arc<Mutex<Option<WorkerObservations>>>,
    reserved_bytes: u64,
    startup: Option<(ModelId, String, WorkControl)>,
) {
    if let Err(error) = compiled_identity() {
        let _ = initialized.send(Err(error));
        return;
    }
    // QuickJS runtime creation on dedicated thread
    let runtime = match rquickjs::Runtime::new() {
        Ok(rt) => rt,
        Err(error) => {
            let _ = initialized.send(Err(ClassifierError::InvalidArtifact(format!(
                "QuickJS initialization: {error}"
            ))));
            return;
        }
    };
    super::runtime_proof::configure(&runtime);

    let cancel_flag = Arc::new(Mutex::new(Arc::new(AtomicBool::new(false))));
    let deadline_cell = Arc::new(Mutex::new(Instant::now() + Duration::from_secs(3600)));

    let int_cancel = cancel_flag.clone();
    let int_deadline = deadline_cell.clone();

    let interrupt_shutdown = shutdown.clone();
    runtime.set_interrupt_handler(Some(Box::new(move || {
        if interrupt_shutdown.load(Ordering::Acquire) {
            return true;
        }
        match int_cancel.lock() {
            Ok(cancel) if !cancel.load(Ordering::Acquire) => {}
            _ => return true,
        }
        if let Ok(dl) = int_deadline.lock() {
            Instant::now() >= *dl
        } else {
            true
        }
    })));

    let context = match rquickjs::Context::full(&runtime) {
        Ok(c) => c,
        Err(error) => {
            let _ = initialized.send(Err(ClassifierError::InvalidArtifact(format!(
                "QuickJS initialization: {error}"
            ))));
            return;
        }
    };

    let init_ok = context.with(|ctx| -> Result<(), ClassifierError> {
        ctx.eval::<(), _>("globalThis.console = { log(){}, warn(){}, error(){} };")
            .map_err(|e| ClassifierError::InvalidArtifact(format!("console shim init: {e}")))?;
        ctx.eval::<(), _>(BUNDLE_SRC)
            .map_err(|e| ClassifierError::InvalidArtifact(format!("bundle init: {e}")))?;
        ctx.eval::<(), _>(FEATURES_SRC)
            .map_err(|e| ClassifierError::InvalidArtifact(format!("features init: {e}")))?;
        ctx.eval::<(), _>(VOCABULARY_SRC)
            .map_err(|e| ClassifierError::InvalidArtifact(format!("vocabulary init: {e}")))?;
        ctx.eval::<(), _>(BRIDGE_SRC)
            .map_err(|e| ClassifierError::InvalidArtifact(format!("bridge init: {e}")))?;
        if let Some((model, payload, control)) = startup {
            control.check_active()?;
            *cancel_flag
                .lock()
                .map_err(|_| ClassifierError::WorkerStopped)? = control.cancel.clone();
            let deadline = Instant::now()
                .checked_add(control.execution_timeout)
                .ok_or(ClassifierError::Timeout)?
                .min(control.overall_deadline);
            *deadline_cell
                .lock()
                .map_err(|_| ClassifierError::WorkerStopped)? = deadline;
            let restored = restore_in_context(&ctx, &model, &payload);
            control.check_active()?;
            if Instant::now() >= deadline {
                return Err(ClassifierError::Timeout);
            }
            restored?;
        }
        Ok(())
    });

    let init_ok = init_ok.and_then(|()| {
        let heap = WorkerObservations::heap(&runtime)?;
        let mut observed = observations.lock().map_err(|_| ClassifierError::Busy)?;
        *observed = Some(WorkerObservations {
            queue_micros: 0,
            samples: VecDeque::with_capacity(4096),
            heap_current: heap,
            heap_peak: heap,
            reserved: reserved_bytes,
        });
        Ok(())
    });
    let failed = init_ok.is_err();
    let _ = initialized.send(init_ok);
    if failed {
        return;
    }

    let mut replays: HashMap<(String, String, String), (String, String)> = HashMap::new();
    let mut replay_bytes = 0usize;
    while !shutdown.load(Ordering::Acquire) {
        let queued = match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(queued) => queued,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let QueuedMessage {
            message: msg,
            reservation,
        } = queued;
        drop(reservation);
        match msg {
            WorkerMessage::BridgeCall {
                operation,
                request,
                cancel,
                deadline,
                enqueued_at,
                queued_deadline,
                execution_timeout,
                reply,
            } => {
                let started = Instant::now();
                if started >= queued_deadline || started >= deadline {
                    let _ = reply.send(Err(ClassifierError::Timeout));
                    continue;
                }
                if cancel.load(Ordering::Acquire) {
                    let _ = reply.send(Err(ClassifierError::Cancelled));
                    continue;
                }
                if let Ok(mut current_cancel) = cancel_flag.lock() {
                    *current_cancel = cancel;
                } else {
                    let _ = reply.send(Err(ClassifierError::WorkerStopped));
                    continue;
                }
                let Some(execution_deadline) = started
                    .checked_add(execution_timeout)
                    .map(|value| value.min(deadline))
                else {
                    let _ = reply.send(Err(ClassifierError::Timeout));
                    continue;
                };
                if let Ok(mut dl) = deadline_cell.lock() {
                    *dl = execution_deadline;
                } else {
                    let _ = reply.send(Err(ClassifierError::WorkerStopped));
                    continue;
                }

                let op_name = match operation {
                    BridgeOperation::TrainBatch => "trainBatch",
                    BridgeOperation::FinishTraining => "finishTraining",
                    BridgeOperation::ClassifyBatch => "classifyBatch",
                    BridgeOperation::SerializeModel => "serializeModel",
                    BridgeOperation::RestoreModel => "restoreModel",
                };

                if matches!(
                    operation,
                    BridgeOperation::TrainBatch | BridgeOperation::FinishTraining
                ) && request.len() > super::training::MAX_TRAINING_ENVELOPE_BYTES
                {
                    let _ = reply.send(Err(ClassifierError::ModelTooLarge));
                    continue;
                }
                let replay_key = if matches!(
                    operation,
                    BridgeOperation::TrainBatch | BridgeOperation::FinishTraining
                ) {
                    match serde_json::from_str::<serde_json::Value>(&request) {
                        Ok(envelope) => match (
                            envelope.get("model_id").and_then(serde_json::Value::as_str),
                            envelope
                                .get("request_id")
                                .and_then(serde_json::Value::as_str),
                        ) {
                            (Some(model), Some(id))
                                if !id.is_empty()
                                    && id.len() <= 256
                                    && !id.chars().any(char::is_control)
                                    && ModelId::parse(model).is_some()
                                    && envelope
                                        .get("version")
                                        .and_then(serde_json::Value::as_str)
                                        == Some("1") =>
                            {
                                Some((model.to_owned(), op_name.to_owned(), id.to_owned()))
                            }
                            _ => {
                                let _ = reply.send(Err(ClassifierError::BridgeProtocol {
                                    operation: op_name.into(),
                                    code: "missing mutation request identity".into(),
                                }));
                                continue;
                            }
                        },
                        Err(error) => {
                            let _ = reply.send(Err(ClassifierError::BridgeProtocol {
                                operation: op_name.into(),
                                code: format!("invalid request JSON: {error}"),
                            }));
                            continue;
                        }
                    }
                } else {
                    None
                };
                let digest = sha256_hex(request.as_bytes());
                let mut reserved_identity_bytes = 0usize;
                if let Some(key) = &replay_key {
                    if let Some((prior_digest, response)) = replays.get(key) {
                        let result = if prior_digest == &digest {
                            Ok(response.clone())
                        } else {
                            Err(ClassifierError::BridgeProtocol {
                                operation: op_name.into(),
                                code: "mutation request identity reused with a different payload"
                                    .into(),
                            })
                        };
                        let _ = reply.send(result);
                        continue;
                    }
                    // Reserve a bounded response slot before allowing a mutation. The bridge
                    // mutation replies are small telemetry DTOs; retain IDs for the whole candidate.
                    let identity_bytes =
                        serde_json::to_vec(&(key, &digest)).map_or(usize::MAX, |bytes| bytes.len());
                    reserved_identity_bytes = identity_bytes;
                    if !replay_fits(replays.len(), replay_bytes, identity_bytes) {
                        let _ = reply.send(Err(ClassifierError::Busy));
                        continue;
                    }
                }
                let batch_size = if matches!(operation, BridgeOperation::ClassifyBatch) {
                    serde_json::from_str::<serde_json::Value>(&request)
                        .ok()
                        .and_then(|value| {
                            value
                                .get("items")
                                .and_then(serde_json::Value::as_array)
                                .map(Vec::len)
                        })
                } else {
                    None
                };
                let res = context.with(|ctx| call_bridge(&ctx, op_name, &request));
                let execution_micros = started.elapsed().as_micros();
                let observation = WorkerObservations::heap(&runtime).and_then(|heap| {
                    let mut observed = observations.lock().map_err(|_| ClassifierError::Busy)?;
                    observed
                        .as_mut()
                        .ok_or(ClassifierError::WorkerStopped)?
                        .observe(
                            heap,
                            batch_size.map(|size| {
                                (
                                    started.saturating_duration_since(enqueued_at).as_micros(),
                                    execution_micros,
                                    size,
                                )
                            }),
                        )
                });
                let res = observation.and(res);
                let res = match cancel_flag.lock() {
                    Ok(cancel) if cancel.load(Ordering::Acquire) => Err(ClassifierError::Cancelled),
                    Ok(_) if Instant::now() >= execution_deadline => Err(ClassifierError::Timeout),
                    Ok(_) => res,
                    Err(_) => Err(ClassifierError::WorkerStopped),
                };
                let res = res.and_then(|response| {
                    let valid = match operation {
                        BridgeOperation::TrainBatch => {
                            let request: serde_json::Value =
                                serde_json::from_str(&request).unwrap_or_default();
                            match request["phase"].as_str() {
                                Some("vocabulary") => serde_json::from_str::<VocabularyPrepared>(
                                    &response,
                                )
                                .is_ok_and(|value| {
                                    value.version == "1"
                                        && request["request_id"].as_str()
                                            == Some(value.request_id.as_str())
                                        && value.errors.is_empty()
                                        && value.prepared
                                            == request["rows"].as_array().map_or(0, Vec::len)
                                        && value.documents <= 50_000
                                        && value.terms <= 100_000
                                }),
                                Some("freeze_vocabulary") => {
                                    serde_json::from_str::<VocabularyFrozen>(&response).is_ok_and(
                                        |value| {
                                            value.version == "1"
                                                && request["request_id"].as_str()
                                                    == Some(value.request_id.as_str())
                                                && value.errors.is_empty()
                                                && value.vocabulary_frozen
                                                && (1..=50_000).contains(&value.documents)
                                                && (1..=100_000).contains(&value.terms)
                                        },
                                    )
                                }
                                None => {
                                    serde_json::from_str::<super::training::TrainBatchResponse>(
                                        &response,
                                    )
                                    .is_ok_and(|value| {
                                        value.version == "1" && value.errors.is_empty()
                                    })
                                }
                                _ => false,
                            }
                        }
                        BridgeOperation::FinishTraining => serde_json::from_str::<
                            super::training::FinishTrainingResponse,
                        >(&response)
                        .is_ok_and(|value| value.version == "1" && value.errors.is_empty()),
                        _ => true,
                    };
                    if !valid {
                        let value: serde_json::Value =
                            serde_json::from_str(&response).unwrap_or_default();
                        let machine = value["error"].as_str().filter(|code| {
                            matches!(
                                *code,
                                "no_discriminative_training_vocabulary"
                                    | "vocabulary_document_bound"
                                    | "vocabulary_byte_bound"
                                    | "vocabulary_term_bound"
                                    | "vocabulary_freeze_state"
                                    | "vocabulary_policy_conflict"
                                    | "tfidf_statistics_mismatch"
                                    | "unprepared_training_row"
                            )
                        });
                        return Err(ClassifierError::BridgeProtocol {
                            operation: op_name.into(),
                            code: machine
                                .unwrap_or("invalid mutation reply; candidate discarded")
                                .into(),
                        });
                    }
                    Ok(response)
                });
                if replay_key.is_some() && res.is_err() {
                    // Failed mutations may have made partial progress; discard this candidate worker.
                    shutdown.store(true, Ordering::Release);
                }
                if replay_key.is_some()
                    && res
                        .as_ref()
                        .is_ok_and(|response| response.len() > MAX_MUTATION_REPLY_BYTES)
                {
                    shutdown.store(true, Ordering::Release);
                    let _ = reply.send(Err(ClassifierError::BridgeProtocol {
                        operation: op_name.into(),
                        code: "mutation response exceeds replay bound; candidate discarded".into(),
                    }));
                    continue;
                }
                if let (Some(key), Ok(response)) = (replay_key, &res) {
                    replay_bytes += reserved_identity_bytes + response.len();
                    replays.insert(key, (digest, response.clone()));
                }
                let _ = reply.send(res);
            }
            #[cfg(test)]
            WorkerMessage::RestoreSmoke { payload, reply } => {
                let res = context.with(|ctx| {
                    restore_in_context(
                        &ctx,
                        &ModelId::parse("smoke-model").expect("fixed valid ID"),
                        &payload,
                    )
                });
                let _ = reply.send(res);
            }
        }
    }

    // Explicit drop order: Context -> Runtime
    drop(context);
    drop(runtime);
}

// Host-side replay storage is additional to the QuickJS heap/stack reservation. These
// caps bound serialized identities/responses; they do not claim to cap physical RSS.
const MAX_REPLAY_IDENTITIES: usize = 32_768;
const MAX_REPLAY_SERIALIZED_BYTES: usize = 64 * 1024 * 1024;
const MAX_MUTATION_REPLY_BYTES: usize = 1024;
fn replay_fits(entries: usize, retained_bytes: usize, identity_bytes: usize) -> bool {
    entries < MAX_REPLAY_IDENTITIES
        && identity_bytes
            <= MAX_REPLAY_SERIALIZED_BYTES
                .saturating_sub(retained_bytes.saturating_add(MAX_MUTATION_REPLY_BYTES))
}

/// An inference engine handle communicating with the dedicated worker thread.
pub struct QuickJsEngine {
    sender: BoundedSender,
    _worker_thread: WorkerThread,
    model_id: ModelId,
    policy: Option<Box<dyn ReceiptPolicy>>,
    accepted_work: bool,
}

impl QuickJsEngine {
    /// Creates a new QuickJS engine instance on a dedicated OS worker thread.
    pub fn new(
        model_id: ModelId,
        policy: Option<Box<dyn ReceiptPolicy>>,
    ) -> Result<Self, ClassifierError> {
        let (sender, worker_thread) = spawn_worker_thread("argos-classifier-worker")?;
        Ok(Self {
            sender,
            _worker_thread: worker_thread,
            model_id,
            policy,
            accepted_work: false,
        })
    }
}

impl QuickJsEngine {
    /// Restores one bounded owned payload before accepting ordinary queue work.
    /// The startup payload does not consume the independently bounded normal queue.
    pub fn new_restored(
        model_id: ModelId,
        policy: Option<Box<dyn ReceiptPolicy>>,
        payload: String,
    ) -> Result<Self, ClassifierError> {
        let (sender, worker_thread) = spawn_worker_with_startup(
            "argos-classifier-restored",
            || Ok(()),
            Some((model_id.clone(), payload, startup_control())),
        )?;
        Ok(Self {
            sender,
            _worker_thread: worker_thread,
            model_id,
            policy,
            accepted_work: true,
        })
    }
    fn classify_controlled(
        &mut self,
        inputs: &[ClassifyInput],
        control: &WorkControl,
    ) -> Result<Vec<ClassificationReceipt>, ClassifierError> {
        control.check_active()?;
        if inputs.len() > INFERENCE_BATCH_SIZE {
            return Err(ClassifierError::BatchTooLarge);
        }

        for input in inputs {
            super::validate_inference_input(input)?;
        }

        let items: Vec<serde_json::Value> = inputs
            .iter()
            .map(|input| {
                serde_json::json!({
                    "id": input.occurrence_id,
                    "title": input.title,
                    "description": input.description,
                    "explain": input.explain,
                    "language": input.language,
                })
            })
            .collect();

        let req_json = serde_json::json!({
            "version": "1",
            "request_id": chrono::Utc::now().to_rfc3339(),
            "model_id": self.model_id.as_str(),
            "items": items,
            "max_batch": INFERENCE_BATCH_SIZE,
        })
        .to_string();

        let (reply_tx, reply_rx) = sync_channel(1);
        let cancel = control.cancel.clone();
        let now = Instant::now();
        let deadline = control.overall_deadline;
        let queued_deadline = now
            .checked_add(control.queue_timeout)
            .ok_or(ClassifierError::Timeout)?
            .min(deadline);
        let wait_timeout = control
            .queue_timeout
            .checked_add(control.execution_timeout)
            .ok_or(ClassifierError::Timeout)?
            .min(deadline.saturating_duration_since(now));

        self.sender
            .try_send(WorkerMessage::BridgeCall {
                operation: BridgeOperation::ClassifyBatch,
                request: req_json,
                cancel,
                deadline,
                enqueued_at: Instant::now(),
                queued_deadline,
                execution_timeout: control.execution_timeout,
                reply: reply_tx,
            })
            .map_err(|e| match e {
                QueueSendError::Full => ClassifierError::Busy,
                QueueSendError::Disconnected => ClassifierError::WorkerStopped,
            })?;

        self.accepted_work = true;
        let res_str = reply_rx.recv_timeout(wait_timeout).map_err(|e| match e {
            RecvTimeoutError::Timeout => ClassifierError::Timeout,
            RecvTimeoutError::Disconnected => ClassifierError::WorkerStopped,
        })??;

        let res_json: serde_json::Value =
            serde_json::from_str(&res_str).map_err(|e| ClassifierError::BridgeProtocol {
                operation: "classifyBatch".into(),
                code: format!("invalid JSON response: {e}"),
            })?;

        if let Some(err_val) = res_json.get("error").and_then(serde_json::Value::as_str) {
            return Err(ClassifierError::BridgeProtocol {
                operation: "classifyBatch".into(),
                code: err_val.to_string(),
            });
        }

        let items_res = res_json
            .get("items")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| ClassifierError::BridgeProtocol {
                operation: "classifyBatch".into(),
                code: "missing `items` array".into(),
            })?;

        if items_res.len() != inputs.len() {
            return Err(ClassifierError::BridgeProtocol {
                operation: "classifyBatch".into(),
                code: "response item count mismatch".into(),
            });
        }
        let mut receipts = Vec::with_capacity(inputs.len());

        for (idx, item_res) in items_res.iter().enumerate() {
            let input = &inputs[idx];
            let results = item_res
                .get("results")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| ClassifierError::BridgeProtocol {
                    operation: "classifyBatch".into(),
                    code: "missing `results` array in item".into(),
                })?;

            if results.len() != 6 {
                return Err(ClassifierError::BridgeProtocol {
                    operation: "classifyBatch".into(),
                    code: format!("expected 6 domain results, found {}", results.len()),
                });
            }

            let mut raw_scores: [RawDomainScore; 6] = std::array::from_fn(|i| RawDomainScore {
                domain: DomainId::ALL[i],
                present_log_score: 0.0,
                absent_log_score: 0.0,
                raw_log_odds: 0.0,
                known_feature_count: 0,
                oov_count: 0,
            });

            let mut seen = [false; 6];
            for d_res in results {
                let protocol_error = |field: &str| ClassifierError::BridgeProtocol {
                    operation: "classifyBatch".into(),
                    code: format!("missing or invalid `{field}`"),
                };
                let domain_str = d_res
                    .get("domain")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| protocol_error("domain"))?;
                let domain = DomainId::parse(domain_str).ok_or_else(|| protocol_error("domain"))?;
                let d_idx = DomainId::ALL
                    .iter()
                    .position(|candidate| *candidate == domain)
                    .ok_or_else(|| protocol_error("domain"))?;
                if seen[d_idx] {
                    return Err(protocol_error("duplicate domain"));
                }
                seen[d_idx] = true;
                let number = |field| {
                    d_res
                        .get(field)
                        .and_then(serde_json::Value::as_f64)
                        .filter(|value| value.is_finite())
                        .ok_or_else(|| protocol_error(field))
                };
                let count = |field| {
                    d_res
                        .get(field)
                        .and_then(serde_json::Value::as_u64)
                        .and_then(|value| usize::try_from(value).ok())
                        .ok_or_else(|| protocol_error(field))
                };
                raw_scores[d_idx] = RawDomainScore {
                    domain,
                    present_log_score: number("present_log_score")?,
                    absent_log_score: number("absent_log_score")?,
                    raw_log_odds: number("raw_log_odds")?,
                    known_feature_count: count("known_feature_count")?,
                    oov_count: count("oov_count")?,
                };
            }

            if let Some(ref policy) = self.policy {
                let receipt = policy.receipt(input, &raw_scores, &self.model_id)?;
                receipts.push(receipt);
            } else {
                let mut receipt = ClassificationReceipt::new(
                    &input.occurrence_id,
                    &input.input_hash,
                    self.model_id.as_str(),
                );
                receipt.scores = raw_scores
                    .iter()
                    .map(|r| DomainScore {
                        domain: r.domain,
                        raw_log_odds: r.raw_log_odds,
                        calibrated_score: 0.0,
                        threshold: 0.0,
                        informative_features: r.known_feature_count,
                        oov_features: r.oov_count,
                        passes_gate: false,
                    })
                    .collect();
                receipts.push(receipt);
            }
            if let Some(diagnostics) = receipts
                .last_mut()
                .and_then(|receipt| receipt.raw_diagnostics.as_mut())
            {
                diagnostics.support = results
                    .iter()
                    .map(|result| {
                        let support: super::feature_policy::DomainSupport =
                            serde_json::from_value(result["support"].clone()).map_err(|_| {
                                ClassifierError::BridgeProtocol {
                                    operation: "classifyBatch".into(),
                                    code: "invalid_support_diagnostics".into(),
                                }
                            })?;
                        support.validate()?;
                        Ok(support)
                    })
                    .collect::<Result<Vec<_>, ClassifierError>>()?;
            }
        }

        Ok(receipts)
    }
}

impl InferenceEngine for QuickJsEngine {
    fn classify_batch(
        &mut self,
        inputs: &[ClassifyInput],
    ) -> Result<Vec<ClassificationReceipt>, ClassifierError> {
        self.classify_controlled(inputs, &WorkControl::new())
    }
    fn classify_batch_with_control(
        &mut self,
        inputs: &[ClassifyInput],
        control: &WorkControl,
    ) -> Result<Vec<ClassificationReceipt>, ClassifierError> {
        self.classify_controlled(inputs, control)
    }
    fn performance(&self) -> Result<ClassifierPerformance, ClassifierError> {
        if self
            ._worker_thread
            .handle
            .as_ref()
            .is_none_or(thread::JoinHandle::is_finished)
        {
            return Err(ClassifierError::WorkerStopped);
        }
        let observations = self
            .sender
            .observations
            .lock()
            .map_err(|_| ClassifierError::Busy)?;
        observations
            .as_ref()
            .map(WorkerObservations::snapshot)
            .ok_or(ClassifierError::WorkerStopped)
    }

    fn serialize_model(&mut self) -> Result<String, ClassifierError> {
        let req_json = serde_json::json!({
            "version": "1",
            "request_id": chrono::Utc::now().to_rfc3339(),
            "model_id": self.model_id.as_str(),
            "state": "done",
        })
        .to_string();

        let (reply_tx, reply_rx) = sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let deadline = Instant::now() + Duration::from_secs(10);

        self.sender
            .try_send(WorkerMessage::BridgeCall {
                operation: BridgeOperation::SerializeModel,
                request: req_json,
                cancel,
                deadline,
                enqueued_at: Instant::now(),
                queued_deadline: Instant::now() + Duration::from_secs(5),
                execution_timeout: Duration::from_secs(2),
                reply: reply_tx,
            })
            .map_err(|e| match e {
                QueueSendError::Full => ClassifierError::Busy,
                QueueSendError::Disconnected => ClassifierError::WorkerStopped,
            })?;

        self.accepted_work = true;
        reply_rx
            .recv_timeout(Duration::from_secs(10))
            .map_err(|e| match e {
                RecvTimeoutError::Timeout => ClassifierError::Timeout,
                RecvTimeoutError::Disconnected => ClassifierError::WorkerStopped,
            })?
    }

    fn restore_model(&mut self, payload: &str) -> Result<(), ClassifierError> {
        if self.accepted_work {
            return Err(ClassifierError::InvalidArtifact(
                "restore requires a fresh unused engine".into(),
            ));
        }
        if payload.len() as u64 > super::artifact::MAX_ARTIFACT_BYTES {
            return Err(ClassifierError::ModelTooLarge);
        }
        self._worker_thread.stop();
        self.accepted_work = true;
        let (sender, worker) = spawn_worker_with_startup(
            "argos-classifier-restore",
            || Ok(()),
            Some((self.model_id.clone(), payload.to_owned(), startup_control())),
        )?;
        self.sender = sender;
        self._worker_thread = worker;
        Ok(())
    }
}

/// A candidate worker handle implementing [`ClassifierWorker`].
pub struct CandidateWorkerHandle {
    sender: BoundedSender,
    _worker_thread: WorkerThread,
    _model_id: ModelId,
    _lease: Arc<WriterLease>,
    terminal: bool,
    exported: bool,
    completed_round: u64,
}

impl ClassifierWorker for CandidateWorkerHandle {
    fn performance(&self) -> Result<ClassifierPerformance, ClassifierError> {
        self.sender
            .observations
            .lock()
            .map_err(|_| ClassifierError::Busy)?
            .as_ref()
            .map(WorkerObservations::snapshot)
            .ok_or(ClassifierError::Unavailable)
    }
    fn call(
        &mut self,
        operation: BridgeOperation,
        request: &str,
        control: &WorkControl,
    ) -> Result<String, ClassifierError> {
        if self.terminal {
            return Err(ClassifierError::WorkerStopped);
        }
        if control.cancel.load(Ordering::Acquire) {
            return Err(ClassifierError::Cancelled);
        }
        let now = Instant::now();
        if now >= control.overall_deadline {
            return Err(ClassifierError::Timeout);
        }

        let queued_deadline = now
            .checked_add(control.queue_timeout)
            .ok_or_else(|| ClassifierError::InvalidArtifact("queue deadline overflow".into()))?
            .min(control.overall_deadline);
        let wait_timeout = control
            .queue_timeout
            .checked_add(control.execution_timeout)
            .ok_or_else(|| ClassifierError::InvalidArtifact("worker timeout overflow".into()))?
            .min(control.overall_deadline.saturating_duration_since(now));

        let (reply_tx, reply_rx) = sync_channel(1);

        self.sender
            .try_send(WorkerMessage::BridgeCall {
                operation,
                request: request.to_string(),
                cancel: control.cancel.clone(),
                deadline: control.overall_deadline,
                enqueued_at: Instant::now(),
                queued_deadline,
                execution_timeout: control.execution_timeout,
                reply: reply_tx,
            })
            .map_err(|e| match e {
                QueueSendError::Full => ClassifierError::Busy,
                QueueSendError::Disconnected => ClassifierError::WorkerStopped,
            })?;

        let result = reply_rx.recv_timeout(wait_timeout).map_err(|e| match e {
            RecvTimeoutError::Timeout => ClassifierError::Timeout,
            RecvTimeoutError::Disconnected => ClassifierError::WorkerStopped,
        })?;
        if let Ok(response) = &result {
            if operation == BridgeOperation::TrainBatch
                && serde_json::from_str::<serde_json::Value>(response)
                    .ok()
                    .and_then(|value| value["round"].as_u64())
                    .is_none_or(|round| round > self.completed_round)
            {
                self.exported = false;
            }
            if operation == BridgeOperation::FinishTraining {
                if let Ok(value) =
                    serde_json::from_str::<super::training::FinishTrainingResponse>(response)
                {
                    if value.done {
                        self.completed_round = self.completed_round.max(value.round);
                    }
                }
            }
            if operation == BridgeOperation::SerializeModel {
                self.exported =
                    serde_json::from_str::<serde_json::Value>(response).is_ok_and(|value| {
                        value["domains"]
                            .as_object()
                            .is_some_and(|domains| domains.len() == 6)
                    });
            }
        }
        result
    }
    fn restore_completed_candidate(
        &mut self,
        model_id: &ModelId,
        payload: &str,
        control: &WorkControl,
    ) -> Result<(), ClassifierError> {
        if self.terminal {
            return Err(ClassifierError::WorkerStopped);
        }
        self.terminal = true;
        self._worker_thread.stop();
        control.check_active()?;
        if !self.exported || model_id != &self._model_id {
            return Err(ClassifierError::InvalidArtifact(
                "terminal restore requires this worker's finalized exported candidate".into(),
            ));
        }
        if payload.len() as u64 > super::artifact::MAX_ARTIFACT_BYTES {
            return Err(ClassifierError::ModelTooLarge);
        }
        let (sender, worker) = spawn_worker_with_startup(
            "argos-candidate-final-restore",
            || Ok(()),
            Some((model_id.clone(), payload.to_owned(), control.clone())),
        )?;
        control.check_active()?;
        drop(sender);
        drop(worker);
        Ok(())
    }
}

/// Starts a dedicated candidate classifier worker.
pub fn start_candidate_worker(
    model_id: &ModelId,
    lease: Arc<WriterLease>,
    _control: &WorkControl,
) -> Result<Box<dyn ClassifierWorker>, ClassifierError> {
    let (sender, worker_thread) = spawn_worker_thread("argos-candidate-worker")?;
    Ok(Box::new(CandidateWorkerHandle {
        sender,
        _worker_thread: worker_thread,
        _model_id: model_id.clone(),
        _lease: lease,
        terminal: false,
        exported: false,
        completed_round: 0,
    }))
}

/// Constructs a ready [`ClassifierService`] from a validated snapshot and artifact.
pub fn from_snapshot(
    snapshot: ModelSnapshot,
    artifact: ValidatedArtifact,
    policy: Box<dyn ReceiptPolicy>,
) -> Result<ClassifierService, ClassifierError> {
    if snapshot.model_id != artifact.model_id.as_str() {
        return Err(ClassifierError::InvalidArtifact(format!(
            "snapshot model id `{}` does not match artifact model id `{}`",
            snapshot.model_id,
            artifact.model_id.as_str()
        )));
    }
    if snapshot.artifact_sha256 != artifact.manifest.model_sha256 {
        return Err(ClassifierError::InvalidArtifact(format!(
            "snapshot artifact sha `{}` does not match artifact manifest `{}`",
            snapshot.artifact_sha256, artifact.manifest.model_sha256
        )));
    }

    super::artifact::validate_artifact(
        &artifact.manifest,
        &artifact.payload,
        super::artifact::compiled_bundle_sha256(),
    )?;
    let engine =
        QuickJsEngine::new_restored(artifact.model_id.clone(), Some(policy), artifact.payload)?;

    Ok(ClassifierService {
        status: ClassifierStatus::Ready,
        unavailable_reason: None,
        pinned: Some(snapshot),
        engine: Mutex::new(Some(Box::new(engine))),
        metrics: Mutex::new(InferenceMetrics::default()),
    })
}

/// Verifies fixed-code restore and smoke inference against the candidate payload.
pub fn restore_smoke_check(payload: &str) -> Result<(), ClassifierError> {
    if payload.len() as u64 > super::artifact::MAX_ARTIFACT_BYTES {
        return Err(ClassifierError::ModelTooLarge);
    }
    let (sender, worker) = spawn_worker_with_startup(
        "argos-restore-smoke",
        || Ok(()),
        Some((
            ModelId::parse("smoke-model").expect("fixed valid ID"),
            payload.to_owned(),
            startup_control(),
        )),
    )?;
    drop(sender);
    drop(worker);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime_case() -> tokio::sync::MutexGuard<'static, ()> {
        super::super::TEST_RUNTIME_LOCK.blocking_lock()
    }
    fn candidate() -> (CandidateWorkerHandle, tempfile::TempDir) {
        let home = tempfile::tempdir().unwrap();
        let store = super::super::artifact::ArtifactStore::new(home.path()).unwrap();
        let (sender, worker) = spawn_worker_thread("fixed-worker-fixture").unwrap();
        (
            CandidateWorkerHandle {
                sender,
                _worker_thread: worker,
                _model_id: ModelId::parse("fixed-fixture").unwrap(),
                _lease: Arc::new(store.try_writer().unwrap()),
                terminal: false,
                exported: false,
                completed_round: 0,
            },
            home,
        )
    }

    #[test]
    fn actual_worker_interrupt_heap_stack_and_tokenization_are_enforced() {
        let _serial = runtime_case();
        let (mut worker, _home) = candidate();
        assert_eq!(
            worker
                .call(
                    BridgeOperation::ClassifyBatch,
                    "fixture:tokenize-once",
                    &WorkControl::new()
                )
                .unwrap(),
            "done"
        );
        for (fixture, resource) in [
            ("fixture:heap-limit", "heap"),
            ("fixture:stack-limit", "stack"),
        ] {
            let (mut worker, _home) = candidate();
            let outcome = worker.call(BridgeOperation::ClassifyBatch, fixture, &WorkControl::new());
            let diagnostic = match &outcome {
                Ok(response) => format!("Ok(response_len={})", response.len()),
                Err(error) => format!("Err({error:?})"),
            };
            assert!(
                matches!(&outcome,Err(ClassifierError::RuntimeLimitExceeded{resource:found}) if found==resource),
                "{fixture}: {diagnostic}"
            );
        }
        drop(worker);
        let (mut worker, home) = candidate();
        let (started, waiting) = sync_channel(1);
        *FIXTURE_STARTED.lock().unwrap() = Some(started);
        let control = WorkControl::new();
        let cancellation = control.cancel.clone();
        let caller = thread::spawn(move || {
            let result = worker.call(
                BridgeOperation::ClassifyBatch,
                "fixture:interrupt-loop",
                &control,
            );
            drop(worker);
            drop(home);
            result
        });
        waiting.recv_timeout(Duration::from_secs(5)).unwrap();
        cancellation.store(true, Ordering::Release);
        assert!(matches!(
            caller.join().unwrap(),
            Err(ClassifierError::Cancelled)
        ));
        let (mut worker, _home) = candidate();
        let mut control = WorkControl::new();
        control.execution_timeout = Duration::from_millis(20);
        assert!(matches!(
            worker.call(
                BridgeOperation::ClassifyBatch,
                "fixture:interrupt-loop",
                &control
            ),
            Err(ClassifierError::Timeout)
        ));
    }

    #[test]
    fn owned_startup_restore_above_queue_limit_never_reports_empty_queue_busy() {
        let _serial = runtime_case();
        let baseline = classifier_process_budget().reserved_bytes();
        let (mut candidate, _home) = candidate();
        let recipe: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/classifier/six-model.json"
        ))
        .unwrap();
        let request = serde_json::json!({"version":"1","request_id":"large-train","model_id":"large-model","rows":recipe["rows"]});
        candidate
            .call(
                BridgeOperation::TrainBatch,
                &request.to_string(),
                &WorkControl::new(),
            )
            .unwrap();
        for step in 0..3 {
            let request = serde_json::json!({"version":"1","request_id":format!("large-fit-{step}"),"model_id":"large-model","max_documents_per_step":16});
            let reply = candidate
                .call(
                    BridgeOperation::FinishTraining,
                    &request.to_string(),
                    &WorkControl::new(),
                )
                .unwrap();
            let value: serde_json::Value = serde_json::from_str(&reply).unwrap();
            assert_eq!(value["done"], step == 2);
        }
        let request = serde_json::json!({"version":"1","request_id":"large-serialize","model_id":"large-model"});
        let mut payload = candidate
            .call(
                BridgeOperation::SerializeModel,
                &request.to_string(),
                &WorkControl::new(),
            )
            .unwrap();
        drop(candidate);
        payload.extend(std::iter::repeat_n(
            ' ',
            MAX_QUEUED_BYTES + 1024 - payload.len(),
        ));
        assert_eq!(classifier_process_budget().reserved_bytes(), baseline);
        let first = QuickJsEngine::new(ModelId::parse("held-first").unwrap(), None).unwrap();
        let second = QuickJsEngine::new(ModelId::parse("held-second").unwrap(), None).unwrap();
        let mut fresh = QuickJsEngine::new(ModelId::parse("large-model").unwrap(), None).unwrap();
        assert_eq!(
            classifier_process_budget().reserved_bytes(),
            baseline + 3 * RUNTIME_RESERVATION_BYTES
        );
        match fresh.restore_model(&payload) {
            Ok(()) => {
                let input = ClassifyInput::new(
                    "large-query",
                    "ballot patrol common",
                    "",
                    Some("en".into()),
                );
                let before = fresh.classify_batch(std::slice::from_ref(&input)).unwrap();
                assert!(matches!(
                    fresh.restore_model(&payload),
                    Err(ClassifierError::InvalidArtifact(_))
                ));
                let after = fresh.classify_batch(&[input]).unwrap();
                assert_eq!(before[0].scores, after[0].scores);
            }
            Err(
                ClassifierError::RuntimeLimitExceeded { .. } | ClassifierError::InvalidArtifact(_),
            ) => {
                assert_eq!(
                    classifier_process_budget().reserved_bytes(),
                    baseline + 2 * RUNTIME_RESERVATION_BYTES
                );
                assert!(matches!(
                    fresh.performance(),
                    Err(ClassifierError::WorkerStopped)
                ));
            }
            Err(error) => panic!("owned startup produced wrong failure: {error}"),
        }
        drop(fresh);
        drop(first);
        drop(second);
        assert_eq!(classifier_process_budget().reserved_bytes(), baseline);
        match restore_smoke_check(&payload) {
            Ok(())
            | Err(
                ClassifierError::RuntimeLimitExceeded { .. } | ClassifierError::InvalidArtifact(_),
            ) => {}
            Err(error) => panic!("startup smoke produced wrong failure: {error}"),
        }
        assert_eq!(classifier_process_budget().reserved_bytes(), baseline);
    }

    fn finalized_payload(worker: &mut CandidateWorkerHandle) -> String {
        let recipe: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/classifier/six-model.json"
        ))
        .unwrap();
        let model = worker._model_id.clone();
        let request = serde_json::json!({"version":"1","request_id":"final-train","model_id":model,"rows":recipe["rows"]});
        worker
            .call(
                BridgeOperation::TrainBatch,
                &request.to_string(),
                &WorkControl::new(),
            )
            .unwrap();
        for step in 0..3 {
            let request = serde_json::json!({"version":"1","request_id":format!("final-fit-{step}"),"model_id":model,"max_documents_per_step":16});
            worker
                .call(
                    BridgeOperation::FinishTraining,
                    &request.to_string(),
                    &WorkControl::new(),
                )
                .unwrap();
        }
        let request =
            serde_json::json!({"version":"1","request_id":"final-serialize","model_id":model});
        worker
            .call(
                BridgeOperation::SerializeModel,
                &request.to_string(),
                &WorkControl::new(),
            )
            .unwrap()
    }

    #[test]
    fn terminal_restore_reuses_one_runtime_slot_and_keeps_lease_on_all_outcomes() {
        let _serial = runtime_case();
        let baseline = classifier_process_budget().reserved_bytes();
        let (mut completed, home) = candidate();
        let payload = finalized_payload(&mut completed);
        let first = QuickJsEngine::new_restored(
            ModelId::parse("live-first").unwrap(),
            None,
            payload.clone(),
        )
        .unwrap();
        let second = QuickJsEngine::new_restored(
            ModelId::parse("live-second").unwrap(),
            None,
            payload.clone(),
        )
        .unwrap();
        assert_eq!(
            classifier_process_budget().reserved_bytes(),
            baseline + 3 * RUNTIME_RESERVATION_BYTES
        );
        let mut large = payload.clone();
        large.extend(std::iter::repeat_n(
            ' ',
            MAX_QUEUED_BYTES + 1024 - large.len(),
        ));
        let model = completed._model_id.clone();
        match completed.restore_completed_candidate(&model, &large, &WorkControl::new()) {
            Ok(())
            | Err(
                ClassifierError::RuntimeLimitExceeded { .. } | ClassifierError::InvalidArtifact(_),
            ) => {}
            Err(error) => panic!("terminal restore produced wrong outcome: {error}"),
        }
        assert_eq!(
            classifier_process_budget().reserved_bytes(),
            baseline + 2 * RUNTIME_RESERVATION_BYTES
        );
        let store = super::super::artifact::ArtifactStore::new(home.path()).unwrap();
        assert!(store.read_active_integrity().unwrap().is_none());
        assert!(matches!(store.try_writer(), Err(ClassifierError::Busy)));
        assert!(matches!(
            completed.call(BridgeOperation::TrainBatch, "{}", &WorkControl::new()),
            Err(ClassifierError::WorkerStopped)
        ));
        drop(completed);
        drop(store.try_writer().unwrap());
        drop(first);
        drop(second);
        assert_eq!(classifier_process_budget().reserved_bytes(), baseline);

        for cancelled in [false, true] {
            let (mut completed, home) = candidate();
            let payload = finalized_payload(&mut completed);
            let control = WorkControl::new();
            control.cancel.store(cancelled, Ordering::Release);
            let model = completed._model_id.clone();
            let result = completed.restore_completed_candidate(
                &model,
                if cancelled { &payload } else { "{}" },
                &control,
            );
            if cancelled {
                assert!(matches!(result, Err(ClassifierError::Cancelled)));
            } else {
                assert!(matches!(result, Err(ClassifierError::InvalidArtifact(_))));
            }
            assert_eq!(classifier_process_budget().reserved_bytes(), baseline);
            assert!(matches!(
                completed.call(BridgeOperation::ClassifyBatch, "{}", &WorkControl::new()),
                Err(ClassifierError::WorkerStopped)
            ));
            let store = super::super::artifact::ArtifactStore::new(home.path()).unwrap();
            assert!(store.read_active_integrity().unwrap().is_none());
            assert!(matches!(store.try_writer(), Err(ClassifierError::Busy)));
            drop(completed);
            drop(store.try_writer().unwrap());
        }
    }

    #[test]
    fn initialization_failure_releases_only_its_owned_reservation() {
        let _serial = runtime_case();
        let baseline = classifier_process_budget().reserved_bytes();
        let (sender, live) = spawn_worker_thread("live-reservation").unwrap();
        let reserved = classifier_process_budget().reserved_bytes();
        assert_eq!(reserved, baseline + RUNTIME_RESERVATION_BYTES);
        assert!(matches!(
            spawn_worker_with_initializer("init-fault", || Err(ClassifierError::InvalidArtifact(
                "injected_init_failure".into()
            ))),
            Err(ClassifierError::InvalidArtifact(_))
        ));
        assert_eq!(classifier_process_budget().reserved_bytes(), reserved);
        drop(sender);
        drop(live);
        assert_eq!(classifier_process_budget().reserved_bytes(), baseline);
    }

    #[test]
    fn bounded_paired_performance_samples_preserve_observed_units() {
        let mut observations = WorkerObservations {
            queue_micros: 0,
            samples: VecDeque::new(),
            heap_current: 17,
            heap_peak: 17,
            reserved: RUNTIME_RESERVATION_BYTES as u64,
        };
        for index in 0..4100 {
            observations
                .observe(20, Some((1, index as u128, (index % 64 + 1) as usize)))
                .unwrap();
        }
        let snapshot = observations.snapshot();
        assert_eq!(snapshot.batch_execution_micros.len(), 4096);
        assert_eq!(snapshot.batch_sizes.len(), 4096);
        assert_eq!(snapshot.batch_execution_micros[0], 4);
        assert_eq!(snapshot.batch_sizes[0], 5);
        assert_eq!(snapshot.queue_micros, 4100);
        assert_eq!(snapshot.js_heap_current_bytes, 20);
        assert_eq!(snapshot.js_heap_peak_bytes, 20);
    }

    #[test]
    fn synchronized_queue_count_bytes_and_closed_channel_fail_operationally() {
        let _serial = runtime_case();
        let home = tempfile::tempdir().unwrap();
        let store = super::super::artifact::ArtifactStore::new(home.path()).unwrap();
        let (tx, receiver) = sync_channel(32);
        let sender = BoundedSender {
            sender: tx,
            queued_bytes: Arc::new(AtomicUsize::new(0)),
            observations: Arc::new(Mutex::new(None)),
        };
        let mut worker = CandidateWorkerHandle {
            sender,
            _worker_thread: WorkerThread {
                handle: None,
                shutdown: Arc::new(AtomicBool::new(false)),
                _reservation: None,
            },
            _model_id: ModelId::parse("queue-fixture").unwrap(),
            _lease: Arc::new(store.try_writer().unwrap()),
            terminal: false,
            exported: false,
            completed_round: 0,
        };
        for _ in 0..32 {
            let (reply, _) = sync_channel(1);
            assert!(worker
                .sender
                .try_send(WorkerMessage::RestoreSmoke {
                    payload: "{}".into(),
                    reply
                })
                .is_ok());
        }
        assert!(matches!(
            worker.call(BridgeOperation::SerializeModel, "{}", &WorkControl::new()),
            Err(ClassifierError::Busy)
        ));
        while receiver.try_recv().is_ok() {}
        let (reply, _) = sync_channel(1);
        assert!(worker
            .sender
            .try_send(WorkerMessage::RestoreSmoke {
                payload: "x".repeat(MAX_QUEUED_BYTES),
                reply
            })
            .is_ok());
        assert!(matches!(
            worker.call(BridgeOperation::SerializeModel, "{}", &WorkControl::new()),
            Err(ClassifierError::Busy)
        ));
        drop(receiver);
        assert_eq!(worker.sender.queued_bytes.load(Ordering::Acquire), 0);
        assert!(matches!(
            worker.call(BridgeOperation::SerializeModel, "{}", &WorkControl::new()),
            Err(ClassifierError::WorkerStopped)
        ));
    }

    #[test]
    fn worker_drop_waits_for_the_runtime_owner_to_exit() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = shutdown.clone();
        let (exited_tx, exited_rx) = sync_channel(1);
        let handle = thread::spawn(move || {
            while !worker_shutdown.load(Ordering::Acquire) {
                thread::yield_now();
            }
            // This represents work that must finish before the reservation can be released.
            exited_tx.send(()).unwrap();
        });
        drop(WorkerThread {
            handle: Some(handle),
            shutdown,
            _reservation: None,
        });
        assert_eq!(
            exited_rx.try_recv(),
            Ok(()),
            "drop must join the owner before releasing its budget"
        );
    }

    #[test]
    fn queue_and_execution_deadlines_are_independent() {
        let _serial = runtime_case();
        let (sender, worker) = spawn_worker_thread("deadline-regression").unwrap();
        for queue_expired in [true, false] {
            let (reply, response) = sync_channel(1);
            let now = Instant::now();
            assert!(sender.try_send(WorkerMessage::BridgeCall {
                operation: BridgeOperation::TrainBatch,
                request: serde_json::json!({"version":"1", "request_id":format!("deadline-{queue_expired}"), "model_id":"deadline", "rows":[]}).to_string(),
                cancel: Arc::new(AtomicBool::new(false)),
                deadline: now + Duration::from_secs(5),
                enqueued_at: now,
                queued_deadline: if queue_expired { now } else { now + Duration::from_secs(5) },
                execution_timeout: if queue_expired { Duration::from_secs(2) } else { Duration::ZERO },
                reply,
            }).is_ok());
            assert!(matches!(
                response.recv_timeout(Duration::from_secs(5)).unwrap(),
                Err(ClassifierError::Timeout)
            ));
        }
        drop(sender);
        drop(worker);
    }

    #[test]
    fn oversized_queued_payload_is_rejected_without_consuming_capacity() {
        let (tx, rx) = sync_channel(32);
        let sender = BoundedSender {
            sender: tx,
            queued_bytes: Arc::new(AtomicUsize::new(0)),
            observations: Arc::new(Mutex::new(None)),
        };
        let (reply, _) = sync_channel(1);
        let oversized = WorkerMessage::RestoreSmoke {
            payload: "x".repeat(MAX_QUEUED_BYTES + 1),
            reply,
        };
        assert!(matches!(
            sender.try_send(oversized),
            Err(QueueSendError::Full)
        ));
        assert_eq!(sender.queued_bytes.load(Ordering::Acquire), 0);
        let (reply, _) = sync_channel(1);
        assert!(sender
            .try_send(WorkerMessage::RestoreSmoke {
                payload: "{}".into(),
                reply
            })
            .is_ok());
        let message = rx.try_recv().unwrap();
        assert_eq!(sender.queued_bytes.load(Ordering::Acquire), 2);
        drop(message);
        assert_eq!(sender.queued_bytes.load(Ordering::Acquire), 0);
    }
}

#[cfg(test)]
mod replay_bound_tests {
    use super::*;
    #[test]
    fn short_envelope_maximum_corpus_with_worst_case_ids_fits_without_eviction() {
        // Each of 50,000 rows can contribute all six memberships. Finishing fits
        // 16 TOTAL memberships per call, interleaved with 32-row collection rounds.
        let train_calls = super::super::training::MAX_TRAINING_ROWS.div_ceil(32);
        let finish_calls = (super::super::training::MAX_TRAINING_ROWS * 6).div_ceil(16);
        let model = "m".repeat(64);
        let identifier = "\"\\".repeat(128);
        let digest = "f".repeat(64);
        let mut bytes = 0;
        for index in 0..train_calls + finish_calls {
            let operation = if index < train_calls {
                "trainBatch"
            } else {
                "finishTraining"
            };
            let identity = serde_json::to_vec(&((&model, operation, &identifier), &digest))
                .unwrap()
                .len();
            assert!(replay_fits(index, bytes, identity), "entry {index}");
            bytes += identity + MAX_MUTATION_REPLY_BYTES;
        }
        assert_eq!(train_calls + finish_calls, 20_313);
        assert!(bytes < MAX_REPLAY_SERIALIZED_BYTES);
        assert!(!replay_fits(MAX_REPLAY_IDENTITIES, 0, 1));
        assert!(!replay_fits(
            0,
            MAX_REPLAY_SERIALIZED_BYTES - MAX_MUTATION_REPLY_BYTES,
            1
        ));
    }
    #[test]
    fn adaptive_envelope_splits_and_rounding_still_fit_the_replay_budget() {
        use super::super::training::{
            LabelMembership, TrainingRow, FIT_DOCUMENTS_PER_STEP, MAX_ROW_TEXT_BYTES,
            MAX_TRAINING_ENVELOPE_BYTES, MAX_TRAINING_ROWS, MAX_TRAINING_TEXT_BYTES,
            TRAINING_BATCH_SIZE,
        };
        // JSON escaping costs at most six bytes per UTF-8 input byte. The
        // closed row DTO's punctuation, keys and six enum labels fit in1KiB;
        // the closed envelope header independently fits in1KiB.
        let frame = 1024usize;
        let maximum_row = 6 * (MAX_ROW_TEXT_BYTES + 256) + frame;
        let worst_row = TrainingRow {
            row_id: "\"\\".repeat(128),
            feature_input_hash: "f".repeat(64),
            title: "\0".repeat(MAX_ROW_TEXT_BYTES),
            description: String::new(),
            membership: [LabelMembership::Present; 6],
        };
        assert!(serde_json::to_vec(&worst_row).unwrap().len() <= maximum_row);
        let total_rows_wire = 6 * MAX_TRAINING_TEXT_BYTES + MAX_TRAINING_ROWS * (6 * 256 + frame);
        // An envelope split occurs only when the next bounded row cannot fit.
        // Full32-row batches are counted separately; unknown rows add no calls.
        let minimum_split_fill = MAX_TRAINING_ENVELOPE_BYTES - frame - maximum_row;
        let collections = MAX_TRAINING_ROWS.div_ceil(TRAINING_BATCH_SIZE)
            + total_rows_wire.div_ceil(minimum_split_fill)
            + 1;
        // Finishing every collection separately adds at most one rounded step
        // per batch beyond the total six-membership/16 bound.
        let finishes = (MAX_TRAINING_ROWS * 6).div_ceil(FIT_DOCUMENTS_PER_STEP) + collections;
        let mutations = 2 * collections + finishes + 1;
        assert!(mutations < 26_000);
        assert!(mutations < MAX_REPLAY_IDENTITIES);
        let identity = serde_json::to_vec(&(
            ("m".repeat(64), "finishTraining", "\"\\".repeat(128)),
            "f".repeat(64),
        ))
        .unwrap()
        .len();
        let mut retained = 0;
        for index in 0..mutations {
            assert!(replay_fits(index, retained, identity));
            retained += identity + MAX_MUTATION_REPLY_BYTES;
        }
        assert!(retained < MAX_REPLAY_SERIALIZED_BYTES);
    }
}
