pub(crate) mod frame;

#[path = "dialogue.rs"]
mod dialogue;

// Share the private frame contract without pulling the application into the model process.
#[allow(dead_code)]
#[path = "../../../crates/fleetworks-contracts/src/qa_worker.rs"]
pub(crate) mod protocol;

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::io;
use std::io::BufReader;
use std::io::BufWriter;
use std::io::Read;
use std::panic;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::Arc;
use std::sync::OnceLock;
use std::thread;

use anyhow::anyhow;
use anyhow::ensure;
use anyhow::Result;
use rust_bert::pipelines::common::ModelResource;
use rust_bert::pipelines::common::ModelType;
use rust_bert::pipelines::question_answering::Answer;
use rust_bert::pipelines::question_answering::QaInput;
use rust_bert::pipelines::question_answering::QuestionAnsweringConfig;
use rust_bert::pipelines::question_answering::QuestionAnsweringModel;
use rust_bert::resources::LocalResource;
use rust_bert::t5::T5Generator;
use sha2::Digest;
use sha2::Sha256;
use tch::Device;

use protocol::valid_draft_delta;
use protocol::verified_operational_sources;
use protocol::QaContextUsage;
use protocol::QaDialogueKind;
use protocol::QaModelTarget;
use protocol::QaPassage;
use protocol::QaWorkerCommand;
use protocol::QaWorkerEvent;
use protocol::QaWorkerOutcome;
use protocol::QaWorkerPanicCause;
use protocol::QaWorkerPanicSource;
use protocol::QaWorkerReply;
use protocol::QaWorkerRequest;
use protocol::QaWorkerSequence;
use protocol::QA_CONTEXT_USAGE_ENV;
use protocol::QA_CONTEXT_WINDOW_TOKENS;
use protocol::QA_DIALOGUE_ENV;
use protocol::QA_MAX_FINDINGS;
use protocol::QA_MAX_FRAME_BYTES;
use protocol::QA_MAX_VISIBLE_WORDS;
use protocol::QA_MODEL_FILES;
use protocol::QA_MODEL_REVISION;
use protocol::QA_MODEL_TARGET_ENV;
use protocol::QA_STREAMING_ENV;
use protocol::QA_WORKER_PROTOCOL;

const MAX_QUERY_TOKENS: usize = 96;
const MAX_PASSAGE_TOKENS: usize = 384;
const MAX_SEQUENCE_TOKENS: usize = QA_CONTEXT_WINDOW_TOKENS as usize;
const BERT_PAIR_SPECIAL_TOKENS: usize = 3;
const WINDOW_OVERLAP: usize = 32;
const MAX_ANSWER_TOKENS: usize = 36;
const PASSAGES_PER_BATCH: usize = 2;
const MAX_PHYSICAL_BATCH: usize = 2;
const CANDIDATES_PER_PASSAGE: i64 = 4;
const MIN_SCORE: f64 = 0.12;
const COMPETITOR_RATIO: f64 = 0.72;

static STARTUP_PANIC: OnceLock<(QaWorkerPanicSource, u32, QaWorkerPanicCause)> = OnceLock::new();

fn startup_panic(info: &panic::PanicHookInfo<'_>) {
    let (source, line) = match info.location() {
        Some(location) => {
            let path = location.file();
            let source = if path.contains("tch-0.14.0/") {
                match path.rsplit('/').next() {
                    Some("utils.rs") => QaWorkerPanicSource::TorchUtils,
                    Some("tensor.rs" | "tensor_generated.rs") => QaWorkerPanicSource::TorchTensor,
                    _ => QaWorkerPanicSource::TorchOther,
                }
            } else if path.contains("rust-bert-0.22.0/") {
                QaWorkerPanicSource::RustBert
            } else if path.ends_with("src/main.rs") {
                QaWorkerPanicSource::Worker
            } else {
                QaWorkerPanicSource::Other
            };
            (source, location.line())
        }
        None => (QaWorkerPanicSource::Other, 0),
    };
    let message = info
        .payload()
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| info.payload().downcast_ref::<&'static str>().copied())
        .unwrap_or_default();
    let cause = if [
        "Cannot allocate memory",
        "can't allocate memory",
        "DefaultCPUAllocator",
        "std::bad_alloc",
    ]
    .iter()
    .any(|&needle| message.contains(needle))
    {
        QaWorkerPanicCause::Memory
    } else if ["Permission denied", "Operation not permitted"]
        .iter()
        .any(|&needle| message.contains(needle))
    {
        QaWorkerPanicCause::Permission
    } else if [
        "set_num_interop_threads",
        "cannot set number of interop threads",
        "Resource temporarily unavailable",
    ]
    .iter()
    .any(|&needle| message.contains(needle))
    {
        QaWorkerPanicCause::Threads
    } else {
        QaWorkerPanicCause::Other
    };
    let _ = STARTUP_PANIC.set((source, line, cause));
}

fn verify_model_files(dir: &Path, files: &[(&str, &str)], max_file_bytes: u64) -> Result<()> {
    ensure!(dir.is_absolute(), "QA model directory must be absolute");
    for &(name, expected) in files {
        let path = dir.join(name);
        let metadata = fs::symlink_metadata(&path)?;
        // Validate the exact contents before the parser or LibTorch sees the file.
        ensure!(
            metadata.is_file() && (1..=max_file_bytes).contains(&metadata.len()),
            "invalid QA model resource"
        );
        let mut file = fs::File::open(path)?.take(max_file_bytes + 1);
        let mut digest = Sha256::new();
        io::copy(&mut file, &mut digest)?;
        ensure!(
            format!("{:x}", digest.finalize()) == expected,
            "QA model digest mismatch"
        );
    }
    Ok(())
}

fn configure_torch() -> Result<()> {
    // The launcher's OpenMP and MKL limits apply before LibTorch initializes. The
    // explicit intra-op setter also initializes an unrelated native thread pool.
    let threads = env::var("OMP_NUM_THREADS")
        .ok()
        .and_then(|value| value.parse::<i32>().ok())
        .filter(|count| (1..=4).contains(count))
        .unwrap_or(1);
    tch::set_num_interop_threads(1);
    ensure!(
        tch::get_num_threads() == threads && tch::get_num_interop_threads() == 1,
        "QA model did not honor its Torch thread limits"
    );
    Ok(())
}

fn load_model(dir: &Path) -> Result<QuestionAnsweringModel> {
    verify_model_files(dir, &QA_MODEL_FILES, 261_000_000)?;

    let resource = |name: &str| -> Result<LocalResource> {
        let local_path = dir.join(name);
        ensure!(
            local_path.to_str().is_some(),
            "invalid QA model path encoding"
        );
        Ok(LocalResource { local_path })
    };
    QuestionAnsweringModel::new(QuestionAnsweringConfig {
        model_resource: ModelResource::Torch(Box::new(resource("rust_model.ot")?)),
        config_resource: Box::new(resource("config.json")?),
        vocab_resource: Box::new(resource("vocab.txt")?),
        merges_resource: None,
        device: Device::Cpu,
        model_type: ModelType::DistilBert,
        lower_case: false,
        strip_accents: Some(false),
        add_prefix_space: None,
        max_seq_length: MAX_SEQUENCE_TOKENS,
        doc_stride: WINDOW_OVERLAP,
        max_query_length: MAX_QUERY_TOKENS,
        max_answer_length: MAX_ANSWER_TOKENS,
        kind: None,
    })
    .map_err(|_| anyhow!("could not load the pinned QA model"))
}

fn initial_model(
    dir: &Path,
    target: QaModelTarget,
    enable_dialogue: bool,
) -> Result<(ResidentModel, Option<PathBuf>, &'static str)> {
    let nested = dir.join("generator");
    let generator_dir = if nested.try_exists()? {
        nested
    } else {
        dir.to_path_buf()
    };
    match fs::symlink_metadata(dir.join("vocab.txt")) {
        Ok(_) => {
            let model = ResidentModel::Extractive(Box::new(load_model(dir)?));
            let generator_dir = (enable_dialogue
                && dialogue::verify(&generator_dir, target).is_ok())
            .then_some(generator_dir);
            Ok((model, generator_dir, QA_MODEL_REVISION))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            ensure!(enable_dialogue, "QA generator was not enabled");
            let model = ResidentModel::Dialogue(dialogue::load(&generator_dir, target)?);
            Ok((model, Some(generator_dir), target.revision()))
        }
        Err(error) => Err(error.into()),
    }
}

enum ResidentModel {
    Unloaded,
    Extractive(Box<QuestionAnsweringModel>),
    Dialogue(Box<T5Generator>),
}

impl ResidentModel {
    fn extractive(&mut self, dir: &Path) -> Result<&QuestionAnsweringModel> {
        if !matches!(self, Self::Extractive(_)) {
            *self = Self::Unloaded;
            *self = Self::Extractive(Box::new(load_model(dir)?));
        }
        let Self::Extractive(model) = self else {
            unreachable!("extractive model was loaded");
        };
        Ok(model)
    }

    fn dialogue(
        &mut self,
        dir: Option<&Path>,
        request: dialogue::DecodeRequest<'_>,
        stopping: &AtomicBool,
        draft: &mut impl FnMut(&str) -> Result<()>,
    ) -> (Option<String>, Option<QaContextUsage>) {
        let skipped = (
            None,
            Some(QaContextUsage::new(
                dialogue::input_limit(request.target, request.streaming),
                0,
                0,
            )),
        );
        let Some(dir) = dir else {
            return skipped;
        };
        if !matches!(self, Self::Dialogue(_)) {
            // Drop the extractor before allocating the generator's weights.
            *self = Self::Unloaded;
            let Ok(Ok(generator)) = panic::catch_unwind(|| dialogue::load(dir, request.target))
            else {
                return skipped;
            };
            *self = Self::Dialogue(generator);
        }
        if stopping.load(Ordering::Acquire) {
            return skipped;
        }
        let Self::Dialogue(generator) = self else {
            unreachable!("dialogue model was loaded");
        };
        let generated = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            dialogue::generate(generator, request, stopping, draft)
        }));
        match generated {
            Ok(Ok(Some(generation))) => (
                generation.reply,
                Some(QaContextUsage::new(
                    generation.window_tokens,
                    generation.input_tokens,
                    1,
                )),
            ),
            Ok(Ok(None)) => skipped,
            Ok(Err(_)) | Err(_) => {
                // A failed native call cannot attest how much of the input was processed.
                *self = Self::Unloaded;
                (None, None)
            }
        }
    }
}

/// Adjacent nested variants of the same extraction aren't competing answers. Disjoint spans
/// with similar confidence are ambiguous and should not produce a finding.
fn best_span<'a>(passage: &QaPassage, candidates: &'a [Answer]) -> Option<&'a Answer> {
    let valid = |answer: &&Answer| {
        let (Ok(start), Ok(end)) = (u32::try_from(answer.start), u32::try_from(answer.end)) else {
            return false;
        };
        answer.score.is_finite()
            && answer.score > 0.0
            && protocol::exact_span(&passage.text, start, end, &answer.answer)
    };
    let mut answers = candidates.iter().filter(valid);
    let best = answers.next()?;
    if best.score < MIN_SCORE
        || answers.any(|candidate| {
            (candidate.end <= best.start || best.end <= candidate.start)
                && candidate.score >= best.score * COMPETITOR_RATIO
        })
    {
        None
    } else {
        Some(best)
    }
}

fn passage_context_usage(query_tokens: usize, passage_tokens: usize) -> (usize, usize) {
    // Mirror rust-bert 0.22.0's QA generate_features loop for the pinned BERT pair
    // encoding: [CLS], question, [SEP], passage window, [SEP].
    let fixed_tokens = query_tokens + BERT_PAIR_SPECIAL_TOKENS;
    let capacity = MAX_SEQUENCE_TOKENS - fixed_tokens;
    let mut start = 0;
    let mut peak_input_tokens = 0;
    let mut windows = 0;
    while windows * WINDOW_OVERLAP < passage_tokens {
        let end = (start + capacity).min(passage_tokens);
        peak_input_tokens = peak_input_tokens.max(fixed_tokens + end - start);
        windows += 1;
        if end == passage_tokens {
            break;
        }
        start = end - WINDOW_OVERLAP;
    }
    (peak_input_tokens, windows)
}

/// Stream at most one independently supported extraction per opaque source ID.
fn answer(
    model: &QuestionAnsweringModel,
    request: QaWorkerRequest,
    report_context: bool,
    output: &mut impl FnMut(QaWorkerReply) -> io::Result<()>,
    stopping: &AtomicBool,
) -> Result<()> {
    let tokenizer = model.get_tokenizer();
    let mut sequence = QaWorkerSequence::default();
    let mut found = 0;
    let mut words = 0;
    let mut peak_input_tokens = 0;
    let mut model_windows = 0;
    let mut unique_answers = BTreeSet::new();
    let query_tokens = tokenizer
        .tokenize_with_offsets(&request.question)
        .tokens
        .len();
    if (1..=MAX_QUERY_TOKENS).contains(&query_tokens) {
        for batch in request.passages.chunks(PASSAGES_PER_BATCH) {
            if stopping.load(Ordering::Acquire) {
                return Ok(());
            }
            let passages = batch
                .iter()
                .filter_map(|passage| {
                    let tokens = tokenizer.tokenize_with_offsets(&passage.text).tokens.len();
                    (1..=MAX_PASSAGE_TOKENS)
                        .contains(&tokens)
                        .then_some((passage, tokens))
                })
                .collect::<Vec<_>>();
            let inputs = passages
                .iter()
                .map(|(passage, _)| QaInput {
                    question: request.question.clone(),
                    context: passage.text.clone(),
                })
                .collect::<Vec<_>>();
            if inputs.is_empty() {
                continue;
            }
            let predicted = model.predict(&inputs, CANDIDATES_PER_PASSAGE, MAX_PHYSICAL_BATCH);
            if report_context {
                for &(_, tokens) in &passages {
                    let (peak, windows) = passage_context_usage(query_tokens, tokens);
                    peak_input_tokens = peak_input_tokens.max(peak);
                    model_windows += windows;
                }
            }
            if stopping.load(Ordering::Acquire) {
                return Ok(());
            }
            for ((passage, _), candidates) in passages.into_iter().zip(&predicted) {
                let Some(best) = best_span(passage, candidates) else {
                    continue;
                };
                let total_words = words + best.answer.split_whitespace().count();
                if total_words > QA_MAX_VISIBLE_WORDS
                    || !unique_answers.insert(best.answer.to_lowercase())
                {
                    continue;
                }
                let event = QaWorkerEvent::Finding {
                    turn_id: request.turn_id.clone(),
                    sequence: found + 1,
                    source_id: passage.source_id.clone(),
                    start: u32::try_from(best.start)?,
                    end: u32::try_from(best.end)?,
                    text: best.answer.clone(),
                };
                sequence.accept(&request, &event)?;
                output(QaWorkerReply::Event { event })?;
                words = total_words;
                found += 1;
                if found as usize == QA_MAX_FINDINGS {
                    break;
                }
            }
            if found as usize == QA_MAX_FINDINGS {
                break;
            }
        }
    }
    let event = QaWorkerEvent::Complete {
        turn_id: request.turn_id.clone(),
        sequence: found + 1,
        outcome: if found == 0 {
            QaWorkerOutcome::InsufficientContext
        } else {
            QaWorkerOutcome::Answered
        },
        context: if report_context {
            Some(QaContextUsage::new(
                QA_CONTEXT_WINDOW_TOKENS,
                u16::try_from(peak_input_tokens)?,
                u16::try_from(model_windows)?,
            ))
        } else {
            None
        },
        reply: None,
    };
    sequence.accept(&request, &event)?;
    output(QaWorkerReply::Event { event })?;
    Ok(())
}

fn complete_passage_findings<'a>(
    request: &QaWorkerRequest,
    passages: impl IntoIterator<Item = &'a QaPassage>,
) -> Result<Vec<QaWorkerEvent>> {
    let mut sequence = QaWorkerSequence::default();
    let mut findings = Vec::new();
    for passage in passages {
        let event = QaWorkerEvent::Finding {
            turn_id: request.turn_id.clone(),
            sequence: u32::try_from(findings.len())? + 1,
            source_id: passage.source_id.clone(),
            start: 0,
            end: u32::try_from(passage.text.chars().count())?,
            text: passage.text.clone(),
        };
        // A candidate that would exceed a bound leaves the sequence unchanged.
        if sequence.accept(request, &event).is_ok() {
            findings.push(event);
        }
        if findings.len() == QA_MAX_FINDINGS {
            break;
        }
    }
    Ok(findings)
}

fn operational_answer(
    request: &QaWorkerRequest,
    reply: Option<String>,
) -> Result<(Option<String>, Vec<QaWorkerEvent>)> {
    if let Some(sources) = reply
        .as_deref()
        .and_then(|text| verified_operational_sources(request, text))
    {
        let source_count = sources.len();
        let findings = complete_passage_findings(request, sources)?;
        if source_count > 0 && findings.len() == source_count {
            return Ok((reply, findings));
        }
    }
    Ok((None, complete_passage_findings(request, &request.passages)?))
}

fn draft_prefix<'a>(
    request: &QaWorkerRequest,
    text: &'a str,
    operational: bool,
) -> Option<&'a str> {
    let text = text.trim();
    // Byte-level tokenizers can temporarily end on an incomplete UTF-8 character.
    if text.contains('\u{fffd}') {
        return None;
    }
    if operational {
        text.rmatch_indices('.').find_map(|(index, _)| {
            let prefix = &text[..=index];
            verified_operational_sources(request, prefix)
                .is_some()
                .then_some(prefix)
        })
    } else {
        Some(text)
    }
}

fn answer_dialogue(
    model: &mut ResidentModel,
    model_dir: Option<&Path>,
    request: &QaWorkerRequest,
    decode: dialogue::DecodeRequest<'_>,
    report_context: bool,
    output: &mut impl FnMut(QaWorkerReply) -> io::Result<()>,
    stopping: &AtomicBool,
) -> Result<()> {
    let operational = decode.dialogue.kind == QaDialogueKind::Operational;
    let mut sequence = QaWorkerSequence::default();
    let mut emitted = String::new();
    let (reply, context) = model.dialogue(model_dir, decode, stopping, &mut |text| {
        let Some(prefix) = draft_prefix(request, text, operational) else {
            return Ok(());
        };
        let Some(delta) = prefix
            .strip_prefix(emitted.as_str())
            .filter(|delta| valid_draft_delta(delta))
        else {
            return Ok(());
        };
        let event = QaWorkerEvent::Delta {
            turn_id: request.turn_id.clone(),
            sequence: sequence.next_sequence(),
            text: delta.to_owned(),
        };
        // The shared sequence also owns cumulative draft bytes and event caps.
        if sequence.accept(request, &event).is_ok() {
            output(QaWorkerReply::Event { event })?;
            emitted.push_str(delta);
        }
        Ok(())
    });
    if stopping.load(Ordering::Acquire) {
        return Ok(());
    }
    let (reply, findings) = if operational {
        operational_answer(request, reply)?
    } else {
        (reply, Vec::new())
    };
    let answered = reply.is_some() || !findings.is_empty();
    for mut event in findings {
        if stopping.load(Ordering::Acquire) {
            return Ok(());
        }
        if let QaWorkerEvent::Finding {
            sequence: number, ..
        } = &mut event
        {
            *number = sequence.next_sequence();
        }
        sequence.accept(request, &event)?;
        output(QaWorkerReply::Event { event })?;
    }
    if stopping.load(Ordering::Acquire) {
        return Ok(());
    }
    let event = QaWorkerEvent::Complete {
        turn_id: request.turn_id.clone(),
        sequence: sequence.next_sequence(),
        outcome: if answered {
            QaWorkerOutcome::Answered
        } else {
            QaWorkerOutcome::InsufficientContext
        },
        context: if report_context { context } else { None },
        reply,
    };
    sequence.accept(request, &event)?;
    output(QaWorkerReply::Event { event })?;
    Ok(())
}

fn receive_commands(stopping: Arc<AtomicBool>) -> io::Result<mpsc::Receiver<QaWorkerCommand>> {
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name("qa-input".into())
        .spawn(move || {
            let mut input = BufReader::new(io::stdin().lock());
            loop {
                match frame::read::<QaWorkerCommand>(&mut input, QA_MAX_FRAME_BYTES) {
                    Ok(Some(QaWorkerCommand::Shutdown)) => break,
                    Ok(Some(command)) => {
                        if sender.try_send(command).is_err() {
                            break;
                        }
                    }
                    // A disconnected supervisor cannot reap a wedged native
                    // prediction. Exit the process independently of that thread.
                    _ => {
                        // SAFETY: This private worker has no durable writes to flush.
                        unsafe { libc::_exit(1) }
                    }
                }
            }
            stopping.store(true, Ordering::Release);
        })?;
    Ok(receiver)
}

fn ready(dialogue: bool, streaming: bool, model_revision: &str) -> QaWorkerReply {
    let model_revision = model_revision.into();
    if dialogue && streaming {
        QaWorkerReply::ReadyWithStreaming {
            protocol: QA_WORKER_PROTOCOL,
            model_revision,
        }
    } else if dialogue {
        QaWorkerReply::ReadyWithDialogue {
            protocol: QA_WORKER_PROTOCOL,
            model_revision,
        }
    } else {
        QaWorkerReply::Ready {
            protocol: QA_WORKER_PROTOCOL,
            model_revision,
        }
    }
}

pub(crate) fn run(
    mut send: impl FnMut(QaWorkerReply, Option<QaDialogueKind>) -> io::Result<()>,
) -> Result<()> {
    // Even an unexpected parser or model failure must not write request data to stderr.
    panic::set_hook(Box::new(startup_panic));
    let mut args = env::args_os().skip(1);
    ensure!(
        args.next().as_deref() == Some("--model-dir".as_ref()),
        "missing QA model directory"
    );
    let dir = args
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("missing QA model directory"))?;
    ensure!(args.next().is_none(), "unexpected QA worker arguments");

    // Older launchers reject unknown fields in a terminal event.
    let report_context = env::var_os(QA_CONTEXT_USAGE_ENV).as_deref() == Some("1".as_ref());
    let enable_dialogue = env::var_os(QA_DIALOGUE_ENV).as_deref() == Some("1".as_ref());
    let streaming = env::var_os(QA_STREAMING_ENV).as_deref() == Some("1".as_ref());
    let target = match env::var_os(QA_MODEL_TARGET_ENV) {
        None => QaModelTarget::default(),
        Some(value) => value
            .to_str()
            .and_then(QaModelTarget::from_slug)
            .ok_or_else(|| anyhow!("invalid QA model target"))?,
    };
    let stopping = Arc::new(AtomicBool::new(false));
    let commands = receive_commands(stopping.clone())?;
    let (mut model, generator_dir, model_revision) = match panic::catch_unwind(|| {
        configure_torch()?;
        initial_model(&dir, target, enable_dialogue)
    }) {
        Ok(model) => model?,
        Err(_) => {
            let (source, line, cause) = STARTUP_PANIC.get().copied().unwrap_or((
                QaWorkerPanicSource::Other,
                0,
                QaWorkerPanicCause::Other,
            ));
            send(
                QaWorkerReply::StartupFailed {
                    source,
                    line,
                    cause,
                },
                None,
            )?;
            return Err(anyhow!("QA model process panicked during startup"));
        }
    };
    send(
        ready(generator_dir.is_some(), streaming, model_revision),
        None,
    )?;
    for command in commands {
        if stopping.load(Ordering::Acquire) {
            break;
        }
        match command {
            QaWorkerCommand::Ping => send(
                ready(generator_dir.is_some(), streaming, model_revision),
                None,
            )?,
            QaWorkerCommand::Ask { request } => {
                request.validate()?;
                let kind = request.dialogue.as_ref().map(|dialogue| dialogue.kind);
                let mut output = |reply| send(reply, kind);
                if let Some(dialogue) = &request.dialogue {
                    answer_dialogue(
                        &mut model,
                        generator_dir.as_deref(),
                        &request,
                        dialogue::DecodeRequest {
                            target,
                            streaming,
                            question: &request.question,
                            dialogue,
                            passages: &request.passages,
                        },
                        report_context,
                        &mut output,
                        &stopping,
                    )?;
                } else {
                    ensure!(
                        model_revision == QA_MODEL_REVISION,
                        "QA chat model requires a typed inference request"
                    );
                    let extractive = model.extractive(&dir)?;
                    if !stopping.load(Ordering::Acquire) {
                        answer(extractive, request, report_context, &mut output, &stopping)?;
                    }
                }
            }
            QaWorkerCommand::Shutdown => return Ok(()),
            QaWorkerCommand::Open
            | QaWorkerCommand::OpenWithContext
            | QaWorkerCommand::OpenWithDialogue
            | QaWorkerCommand::OpenWithStreaming => {
                return Err(anyhow!("unexpected QA worker handshake"));
            }
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let mut output = BufWriter::new(io::stdout().lock());
    if run(|reply, _kind| frame::write(&mut output, &reply)).is_ok() {
        ExitCode::SUCCESS
    } else {
        eprintln!("QA model process unavailable");
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use protocol::QaWorkerDialogue;

    use super::*;

    fn candidate(score: f64, start: usize, end: usize, answer: &str) -> Answer {
        Answer {
            score,
            start,
            end,
            answer: answer.into(),
        }
    }

    #[test]
    fn measures_bert_sequence_limit_and_overlapping_windows() {
        assert_eq!(passage_context_usage(8, 0), (0, 0));
        assert_eq!(passage_context_usage(8, 12), (23, 1));
        assert_eq!(passage_context_usage(96, 157), (256, 1));
        assert_eq!(passage_context_usage(96, 158), (256, 2));
        assert_eq!(passage_context_usage(96, 282), (256, 2));
        assert_eq!(passage_context_usage(96, 283), (256, 3));
    }

    #[test]
    fn admits_exact_unicode_extraction_and_abstains_over_competing_spans() {
        let source = QaPassage::new("probe".into(), "État du café: prêt, ou arrêté.".into());
        let exact = candidate(0.81, 14, 18, "prêt");
        let ambiguous = candidate(0.72, 23, 29, "arrêté");
        assert!(best_span(&source, std::slice::from_ref(&exact)).is_some());
        assert!(best_span(&source, &[exact, ambiguous]).is_none());
        assert!(best_span(&source, &[candidate(0.07, 14, 18, "prêt")]).is_none());
        assert!(best_span(&source, &[candidate(0.99, 14, 18, "faux")]).is_none());
    }

    #[test]
    fn grounds_operational_reply_or_publishes_complete_fallback_findings() {
        let mut request = QaWorkerRequest::new(
            "turn".into(),
            "snapshot".into(),
            "What is the actor search status?".into(),
            vec![
                QaPassage::new("actor-search".into(), "Actor search is ready.".into()),
                QaPassage::new("gateway".into(), "Gateway is ready.".into()),
            ],
        );
        request.dialogue = Some(QaWorkerDialogue {
            kind: QaDialogueKind::Operational,
            history: Vec::new(),
        });

        assert_eq!(
            draft_prefix(&request, "Actor search is ready. Gateway stopped.", true),
            Some("Actor search is ready.")
        );
        assert_eq!(draft_prefix(&request, "Actor search is", true), None);
        assert_eq!(draft_prefix(&request, "caf\u{fffd}", false), None);

        let (reply, findings) =
            operational_answer(&request, Some("Gateway is ready.".into())).unwrap();
        assert_eq!(reply.as_deref(), Some("Gateway is ready."));
        assert!(matches!(
            findings.as_slice(),
            [QaWorkerEvent::Finding { source_id, sequence: 1, start: 0, end: 17, text, .. }]
                if source_id == "gateway" && text == "Gateway is ready."
        ));

        let (reply, findings) =
            operational_answer(&request, Some("Actor search is stopped.".into())).unwrap();
        assert!(reply.is_none());
        assert_eq!(findings.len(), 2);
        assert!(matches!(
            &findings[0],
            QaWorkerEvent::Finding { source_id, sequence: 1, start: 0, end: 22, text, .. }
                if source_id == "actor-search" && text == "Actor search is ready."
        ));
    }
}
