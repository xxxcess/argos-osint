use super::*;
#[cfg(feature = "nlp")]
use std::sync::Mutex;
#[cfg(feature = "nlp")]
use std::time::{Duration, Instant};
#[cfg(feature = "nlp")]
const CORE: &str = include_str!("../../assets/nlp/compromise.js");
#[cfg(feature = "nlp")]
const DATES: &str = include_str!("../../assets/nlp/compromise-dates.min.js");
#[cfg(feature = "nlp")]
#[derive(Deserialize)]
struct JsSpan {
    start: usize,
    end: usize,
    #[serde(default)]
    kind: String,
}
#[cfg(feature = "nlp")]
#[derive(Deserialize)]
struct JsAnnotation {
    sentences: Vec<JsSpan>,
    mentions: Vec<JsSpan>,
    dates: Vec<JsSpan>,
    numbers: Vec<JsSpan>,
}
#[cfg(feature = "nlp")]
struct Guard {
    cancel: Arc<AtomicBool>,
    deadline: Instant,
}
pub(crate) struct Engine {
    #[cfg(feature = "nlp")]
    context: rquickjs::Context,
    #[cfg(feature = "nlp")]
    runtime: rquickjs::Runtime,
    #[cfg(feature = "nlp")]
    guard: Arc<Mutex<Guard>>,
}
impl Engine {
    pub(crate) fn actor_names(&mut self, names: &[String]) -> Result<Vec<String>, String> {
        #[cfg(not(feature = "nlp"))]
        {
            Ok(names.to_vec())
        }
        #[cfg(feature = "nlp")]
        {
            {
                let mut guard = self.guard.lock().unwrap();
                guard.cancel = Arc::new(AtomicBool::new(false));
                guard.deadline = Instant::now() + Duration::from_secs(2);
            }
            self.context.with(|ctx| {
                let f: rquickjs::Function = ctx
                    .globals()
                    .get("argosActorNames")
                    .map_err(|e| e.to_string())?;
                let input = serde_json::to_string(names).map_err(|e| e.to_string())?;
                let output: String = f.call((input,)).map_err(|e| e.to_string())?;
                serde_json::from_str(&output).map_err(|e| e.to_string())
            })
        }
    }
    pub fn new() -> Result<Self, String> {
        #[cfg(feature = "nlp")]
        {
            Self::with_assets(CORE, DATES, include_str!("bridge.js"))
        }
        #[cfg(not(feature = "nlp"))]
        {
            Ok(Self {})
        }
    }
    #[cfg(feature = "nlp")]
    fn with_assets(core: &str, dates: &str, bridge: &str) -> Result<Self, String> {
        {
            if cache::hash(core)
                != "61e2541597f27c592947e5059c083548f24aa6fba14cf2b109a00db44de6ca57"
                || cache::hash(dates)
                    != "3bfc87cf5c0fc6d4190c195078aa969675e9de32a70836cb233d87b724d6aaf2"
            {
                return Err("NLP bundle checksum mismatch".into());
            }
            let runtime = rquickjs::Runtime::new().map_err(|e| e.to_string())?;
            runtime.set_memory_limit(128 * 1024 * 1024);
            runtime.set_max_stack_size(1024 * 1024);
            let guard = Arc::new(Mutex::new(Guard {
                cancel: Arc::new(AtomicBool::new(false)),
                deadline: Instant::now() + Duration::from_secs(10),
            }));
            let interrupt = guard.clone();
            runtime.set_interrupt_handler(Some(Box::new(move || {
                let g = interrupt.lock().unwrap();
                g.cancel.load(Ordering::Acquire) || Instant::now() >= g.deadline
            })));
            let context = rquickjs::Context::full(&runtime).map_err(|e| e.to_string())?;
            context
                .with(|ctx| -> rquickjs::Result<()> {
                    ctx.eval::<(), _>("globalThis.console={log(){},warn(){},error(){}};")?;
                    ctx.eval::<(), _>(core)?;
                    ctx.eval::<(), _>(dates)?;
                    ctx.eval::<(), _>(bridge)?;
                    Ok(())
                })
                .map_err(|e| format!("NLP startup: {e}"))?;
            Ok(Self {
                context,
                runtime,
                guard,
            })
        }
    }
    pub fn annotate(
        &mut self,
        source: &Source,
        context: &ParseContext,
        cancel: Arc<AtomicBool>,
    ) -> Annotation {
        #[cfg(not(feature = "nlp"))]
        {
            let _ = cancel;
            Annotation::terminal(source, context, Status::Disabled)
        }
        #[cfg(feature = "nlp")]
        {
            let mut a = Annotation::terminal(source, context, Status::Complete);
            if source.prose.len() > MAX_SOURCE {
                a.status = Status::TooLarge;
                return a;
            }
            if cache::hash(&source.prose) != source.prose_hash {
                a.status = Status::Failed;
                a.flags.push("source hash mismatch".into());
                return a;
            }
            if source
                .language_hint
                .as_deref()
                .is_some_and(|s| !s.starts_with("en"))
            {
                a.status = Status::Unsupported;
                return a;
            }
            let document_deadline = Instant::now() + Duration::from_secs(10);
            let mut parsed_dates: std::collections::HashMap<String, Option<String>> =
                std::collections::HashMap::new();
            for (span, split) in chunks(&source.prose) {
                if cancel.load(Ordering::Acquire) {
                    a.status = Status::Cancelled;
                    break;
                }
                if Instant::now() >= document_deadline {
                    a.status = Status::Timeout;
                    break;
                }
                let chunk = span.slice(&source.prose).unwrap();
                {
                    let mut g = self.guard.lock().unwrap();
                    g.cancel = cancel.clone();
                    g.deadline = document_deadline.min(Instant::now() + Duration::from_secs(1));
                }
                let dto = self.context.with(|ctx| -> Result<JsAnnotation, String> {
                    let f: rquickjs::Function = ctx
                        .globals()
                        .get("argosAnnotate")
                        .map_err(|e| e.to_string())?;
                    let json: String = f.call((chunk,)).map_err(|e| e.to_string())?;
                    if json.len() > MAX_OUTPUT {
                        return Err("NLP result too large".into());
                    }
                    serde_json::from_str(&json).map_err(|e| e.to_string())
                });
                let dto = match dto {
                    Ok(v) => v,
                    Err(e) => {
                        a.status = if e == "NLP result too large" {
                            Status::TooLarge
                        } else if cancel.load(Ordering::Acquire) {
                            Status::Cancelled
                        } else if Instant::now() >= self.guard.lock().unwrap().deadline {
                            Status::Timeout
                        } else {
                            Status::Failed
                        };
                        a.flags.push(e);
                        break;
                    }
                };
                let convert = |s: &JsSpan| -> Option<Span> {
                    let start = spans::utf16_to_byte(chunk, s.start)?;
                    let end = spans::utf16_to_byte(chunk, s.end)?;
                    let range = Span {
                        start_byte: span.start_byte + start,
                        end_byte: span.start_byte + end,
                    };
                    range.slice(&source.prose)?;
                    Some(range)
                };
                let mut valid = true;
                for s in dto.sentences {
                    if let Some(span) = convert(&s) {
                        a.sentences.push(Sentence {
                            id: cache::hash(&format!(
                                "{}:{}:{}",
                                source.prose_hash, span.start_byte, span.end_byte
                            )),
                            span,
                        });
                    } else {
                        valid = false;
                    }
                }
                for s in dto.mentions {
                    if let Some(span) = convert(&s) {
                        a.mentions.push(Mention { span, kind: s.kind });
                    } else {
                        valid = false;
                    }
                }
                for s in dto.numbers {
                    if let Some(span) = convert(&s) {
                        a.numerical_spans.push(span);
                    } else {
                        valid = false;
                    }
                }
                for s in dto.dates {
                    if let Some(span) = convert(&s) {
                        let phrase = span.slice(&source.prose).unwrap();
                        let mut flags = vec![];
                        let normalized = if let Some(value) = parsed_dates.get(phrase) {
                            value.clone()
                        } else if parsed_dates.len() >= 32 || phrase.chars().count() > 256 {
                            flags.push("unparsed".into());
                            None
                        } else {
                            let value = if ambiguous_date(phrase, context) {
                                flags.push("ambiguous".into());
                                None
                            } else if invalid_calendar(phrase) {
                                flags.push("invalid-calendar".into());
                                None
                            } else {
                                self.context
                                    .with(|ctx| -> rquickjs::Result<String> {
                                        let f: rquickjs::Function =
                                            ctx.globals().get("argosDate")?;
                                        f.call((
                                            phrase,
                                            context.reference_date.as_str(),
                                            context.timezone.as_str(),
                                            context.date_order.as_str(),
                                        ))
                                    })
                                    .ok()
                                    .and_then(|j| {
                                        serde_json::from_str::<serde_json::Value>(&j).ok()
                                    })
                                    .and_then(|v| {
                                        v.get(0)?
                                            .get("dates")?
                                            .get("start")?
                                            .as_str()
                                            .map(str::to_owned)
                                    })
                            };
                            parsed_dates.insert(phrase.to_owned(), value.clone());
                            value
                        };
                        if normalized.is_none() && flags.is_empty() {
                            flags.push("unparsed".into());
                        }
                        a.date_candidates.push(DateCandidate {
                            span,
                            normalized,
                            flags,
                        });
                    } else {
                        valid = false;
                    }
                }
                if Instant::now() >= self.guard.lock().unwrap().deadline {
                    a.status = if cancel.load(Ordering::Acquire) {
                        Status::Cancelled
                    } else {
                        Status::Timeout
                    };
                    break;
                }
                if !valid {
                    a.status = Status::Failed;
                    a.flags.push("invalid UTF-16 span".into());
                    break;
                }
                if split {
                    a.flags.push("split-sentence".into());
                }
                if serde_json::to_vec(&a).map_or(true, |v| v.len() > MAX_OUTPUT) {
                    a.status = Status::TooLarge;
                    break;
                }
            }
            if cancel.load(Ordering::Acquire) {
                a.status = Status::Cancelled;
            } else if Instant::now() >= document_deadline {
                a.status = Status::Timeout;
            }
            a.sentences.sort_by_key(|s| s.span);
            a.sentences.dedup_by_key(|s| s.span);
            a.mentions.sort_by_key(|s| s.span);
            a.mentions
                .dedup_by(|a, b| a.span == b.span && a.kind == b.kind);
            a.date_candidates.sort_by_key(|s| s.span);
            a.date_candidates.dedup_by_key(|s| s.span);
            a.numerical_spans.sort();
            a.numerical_spans.dedup();
            if a.status == Status::Complete && !a.validate(source) {
                a.status = Status::Failed;
                a.flags.push("annotation span validation failed".into());
            }
            // Release dead input/cancellation references after the job.
            self.guard.lock().unwrap().cancel = Arc::new(AtomicBool::new(false));
            self.runtime.run_gc();
            a
        }
    }
}
/// Bounded UTF-8 chunks; prefer paragraphs/sentences and overlap one sentence.
#[cfg(feature = "nlp")]
fn chunks(text: &str) -> Vec<(Span, bool)> {
    let mut out = vec![];
    let mut start = 0;
    while start < text.len() {
        let mut max = (start + 8192).min(text.len());
        while !text.is_char_boundary(max) {
            max -= 1;
        }
        let mut target = (start + 4096).min(max);
        while !text.is_char_boundary(target) {
            target -= 1;
        }
        let region = &text[start..max];
        let boundary = region[..target - start]
            .rfind("\n\n")
            .map(|i| start + i + 2)
            .or_else(|| {
                region[..target - start]
                    .rfind(['.', '!', '?', '\n'])
                    .map(|i| start + i + 1)
            });
        let end = if max == text.len() {
            max
        } else {
            boundary.filter(|b| *b > start + 1024).unwrap_or(max)
        };
        let split = boundary.is_none() && end < text.len();
        out.push((
            Span {
                start_byte: start,
                end_byte: end,
            },
            split,
        ));
        if end == text.len() {
            break;
        }
        // Overlap only complete previous sentence, and ensure forward progress.
        let previous = &text[start..end];
        let overlap = previous
            .trim_end_matches(['\n', '\r', ' '])
            .trim_end_matches(['.', '!', '?'])
            .rfind(['.', '!', '?', '\n'])
            .map(|i| start + i + 1)
            .filter(|b| *b > start && end - *b <= 1024);
        start = overlap.unwrap_or(end);
    }
    out
}
#[cfg(feature = "nlp")]
fn ambiguous_date(phrase: &str, ctx: &ParseContext) -> bool {
    ctx.date_order == "unknown"
        && regex::Regex::new(r"\b\d{1,2}[/.-]\d{1,2}[/.-]\d{2,4}\b")
            .unwrap()
            .is_match(phrase)
}
#[cfg(feature = "nlp")]
fn invalid_calendar(phrase: &str) -> bool {
    use chrono::NaiveDate;
    let numeric = regex::Regex::new(r"\b(\d{1,2})[/.-](\d{1,2})[/.-](\d{4})\b").unwrap();
    if let Some(c) = numeric.captures(phrase) {
        let a = c[1].parse::<u32>().unwrap_or(0);
        let b = c[2].parse::<u32>().unwrap_or(0);
        let y = c[3].parse::<i32>().unwrap_or(0);
        if chrono::NaiveDate::from_ymd_opt(y, a, b).is_none()
            && chrono::NaiveDate::from_ymd_opt(y, b, a).is_none()
        {
            return true;
        }
    }
    let re=regex::Regex::new(r"(?i)\b(January|February|March|April|May|June|July|August|September|October|November|December)\s+(\d{1,2})(?:st|nd|rd|th)?[,]?\s+(\d{4})\b").unwrap();
    if let Some(c) = re.captures(phrase) {
        let months = [
            "january",
            "february",
            "march",
            "april",
            "may",
            "june",
            "july",
            "august",
            "september",
            "october",
            "november",
            "december",
        ];
        let month = months
            .iter()
            .position(|m| *m == c[1].to_lowercase())
            .unwrap()
            + 1;
        return NaiveDate::from_ymd_opt(
            c[3].parse().unwrap_or(0),
            month as u32,
            c[2].parse().unwrap_or(0),
        )
        .is_none();
    }
    false
}

#[cfg(all(test, feature = "nlp"))]
mod tests {
    use super::*;
    #[test]
    fn chunk_limits_and_interrupt() {
        let text = "😀中e\u{301}".repeat(3000);
        let chunks = chunks(&text);
        assert!(chunks
            .iter()
            .all(|(r, _)| r.slice(&text).is_some_and(|s| s.len() <= 8192)));
        assert!(chunks.iter().any(|(_, split)| *split));
        let cold = Instant::now();
        let mut engine = Engine::new().unwrap();
        eprintln!("parser startup_ms={}", cold.elapsed().as_millis());
        engine.guard.lock().unwrap().deadline = Instant::now() + Duration::from_millis(10);
        // The interrupt must stop the loop promptly in terms of work done:
        // bound the thread's CPU time, which (unlike wall time) does not grow
        // while a loaded host keeps this thread off-CPU.
        let (interrupted, spent) = crate::test_clock::cpu_timed(|| {
            engine
                .context
                .with(|ctx| ctx.eval::<(), _>("while(true) {}"))
        });
        assert!(interrupted.is_err());
        assert!(spent < Duration::from_secs(1), "spent {spent:?}");
        engine = Engine::new().unwrap();
        let source = Source::derived("recovery", "1", "Alice Smith said tomorrow.");
        assert_eq!(
            engine
                .annotate(&source, &source.context(), Arc::new(AtomicBool::new(false)))
                .status,
            Status::Complete
        );
        assert!(invalid_calendar("January 34th 2020"));
        assert!(invalid_calendar("February 29th 2021"));
        assert!(!invalid_calendar("February 29th 2020"));
    }
    #[test]
    fn missing_assets_bad_dto_output_limit_and_cancellation() {
        assert!(Engine::with_assets(CORE, "", include_str!("bridge.js")).is_err());
        assert!(Engine::with_assets("invalid", DATES, include_str!("bridge.js")).is_err());
        assert!(
            Engine::with_assets(CORE, DATES, "throw new Error('fixture startup failure')").is_err()
        );
        let mut engine = Engine::new().unwrap();
        engine.context.with(|ctx|ctx.eval::<(),_>("argosAnnotate = () => JSON.stringify({sentences:[{start:1,end:2}],mentions:[],dates:[],numbers:[]})")).unwrap();
        let s = Source::derived("invalid", "1", "😀 Alice Smith.");
        assert_eq!(
            engine
                .annotate(&s, &s.context(), Arc::new(AtomicBool::new(false)))
                .status,
            Status::Failed
        );
        engine
            .context
            .with(|ctx| ctx.eval::<(), _>("argosAnnotate = () => 'x'.repeat(4194305)"))
            .unwrap();
        assert_eq!(
            engine
                .annotate(&s, &s.context(), Arc::new(AtomicBool::new(false)))
                .status,
            Status::TooLarge
        );
        let cancel = Arc::new(AtomicBool::new(false));
        engine.guard.lock().unwrap().cancel = cancel.clone();
        engine.guard.lock().unwrap().deadline = Instant::now() + Duration::from_secs(1);
        let interrupt = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(10));
            cancel.store(true, Ordering::Release);
        });
        assert!(engine
            .context
            .with(|ctx| ctx.eval::<(), _>("while(true){}"))
            .is_err());
        interrupt.join().unwrap();
    }
}
