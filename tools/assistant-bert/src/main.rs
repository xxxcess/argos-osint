pub(crate) mod frame;

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
use sha2::Digest;
use sha2::Sha256;
use tch::Device;

use protocol::QaPassage;
use protocol::QaWorkerCommand;
use protocol::QaWorkerEvent;
use protocol::QaWorkerOutcome;
use protocol::QaWorkerPanicCause;
use protocol::QaWorkerPanicSource;
use protocol::QaWorkerReply;
use protocol::QaWorkerRequest;
use protocol::QaWorkerSequence;
use protocol::QA_MAX_FINDINGS;
use protocol::QA_MAX_FRAME_BYTES;
use protocol::QA_MAX_VISIBLE_WORDS;
use protocol::QA_MODEL_FILES;
use protocol::QA_MODEL_REVISION;
use protocol::QA_WORKER_PROTOCOL;

const MAX_QUERY_TOKENS: usize = 96;
const MAX_PASSAGE_TOKENS: usize = 384;
const MAX_SEQUENCE_TOKENS: usize = 256;
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

fn load_model(dir: &Path) -> Result<QuestionAnsweringModel> {
    ensure!(dir.is_absolute(), "QA model directory must be absolute");
    for (name, expected) in QA_MODEL_FILES {
        let path = dir.join(name);
        let metadata = fs::symlink_metadata(&path)?;
        // Validate the exact contents before the parser or LibTorch sees the file.
        ensure!(
            metadata.is_file() && (1..=261_000_000).contains(&metadata.len()),
            "invalid QA model resource"
        );
        let mut file = fs::File::open(path)?.take(261_000_001);
        let mut digest = Sha256::new();
        io::copy(&mut file, &mut digest)?;
        ensure!(
            format!("{:x}", digest.finalize()) == expected,
            "QA model digest mismatch"
        );
    }

    let resource = |name: &str| -> Result<LocalResource> {
        let local_path = dir.join(name);
        ensure!(
            local_path.to_str().is_some(),
            "invalid QA model path encoding"
        );
        Ok(LocalResource { local_path })
    };
    // The launcher's OpenMP and MKL limits apply before LibTorch initializes. The
    // explicit intra-op setter also initializes an unrelated native thread pool.
    tch::set_num_interop_threads(1);
    ensure!(
        tch::get_num_threads() == 1 && tch::get_num_interop_threads() == 1,
        "QA model did not honor its Torch thread limits"
    );
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

/// Stream at most one independently supported extraction per opaque source ID.
fn answer(
    model: &QuestionAnsweringModel,
    request: QaWorkerRequest,
    output: &mut impl FnMut(QaWorkerReply) -> io::Result<()>,
    stopping: &AtomicBool,
) -> Result<()> {
    request.validate()?;
    let tokenizer = model.get_tokenizer();
    let mut sequence = QaWorkerSequence::default();
    let mut found = 0;
    let mut words = 0;
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
                .filter(|passage| {
                    let tokens = tokenizer.tokenize_with_offsets(&passage.text).tokens.len();
                    (1..=MAX_PASSAGE_TOKENS).contains(&tokens)
                })
                .collect::<Vec<_>>();
            let inputs = passages
                .iter()
                .map(|passage| QaInput {
                    question: request.question.clone(),
                    context: passage.text.clone(),
                })
                .collect::<Vec<_>>();
            if inputs.is_empty() {
                continue;
            }
            let predicted = model.predict(&inputs, CANDIDATES_PER_PASSAGE, MAX_PHYSICAL_BATCH);
            if stopping.load(Ordering::Acquire) {
                return Ok(());
            }
            for (passage, candidates) in passages.into_iter().zip(&predicted) {
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
    };
    sequence.accept(&request, &event)?;
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

fn ready() -> QaWorkerReply {
    QaWorkerReply::Ready {
        protocol: QA_WORKER_PROTOCOL,
        model_revision: QA_MODEL_REVISION.into(),
    }
}

pub(crate) fn run(mut output: impl FnMut(QaWorkerReply) -> io::Result<()>) -> Result<()> {
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

    let stopping = Arc::new(AtomicBool::new(false));
    let commands = receive_commands(stopping.clone())?;
    let model = match panic::catch_unwind(|| load_model(&dir)) {
        Ok(model) => model?,
        Err(_) => {
            let (source, line, cause) = STARTUP_PANIC.get().copied().unwrap_or((
                QaWorkerPanicSource::Other,
                0,
                QaWorkerPanicCause::Other,
            ));
            output(QaWorkerReply::StartupFailed {
                source,
                line,
                cause,
            })?;
            return Err(anyhow!("QA model process panicked during startup"));
        }
    };
    output(ready())?;
    for command in commands {
        if stopping.load(Ordering::Acquire) {
            break;
        }
        match command {
            QaWorkerCommand::Ping => output(ready())?,
            QaWorkerCommand::Ask { request } => answer(&model, request, &mut output, &stopping)?,
            QaWorkerCommand::Shutdown => return Ok(()),
            QaWorkerCommand::Open => return Err(anyhow!("unexpected QA worker handshake")),
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let mut output = BufWriter::new(io::stdout().lock());
    if run(|reply| frame::write(&mut output, &reply)).is_ok() {
        ExitCode::SUCCESS
    } else {
        eprintln!("QA model process unavailable");
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
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
    fn admits_exact_unicode_extraction_and_abstains_over_competing_spans() {
        let source = QaPassage::new("probe".into(), "État du café: prêt, ou arrêté.".into());
        let exact = candidate(0.81, 14, 18, "prêt");
        let ambiguous = candidate(0.72, 23, 29, "arrêté");
        assert!(best_span(&source, std::slice::from_ref(&exact)).is_some());
        assert!(best_span(&source, &[exact, ambiguous]).is_none());
        assert!(best_span(&source, &[candidate(0.07, 14, 18, "prêt")]).is_none());
        assert!(best_span(&source, &[candidate(0.99, 14, 18, "faux")]).is_none());
    }
}
