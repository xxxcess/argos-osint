//! One reusable sentence index for the final clean article (spec §2.1.3–4).
//!
//! The validated baseline is parsed once (local NLP when available). Its
//! sentence/mention/date/number spans are remapped verbatim to clean
//! coordinates through the removal span maps; any paragraph the remap cannot
//! certify (a span crossing a removal or paragraph boundary, or prose the
//! parser left uncovered) is re-split locally and reported in `coverage`.
//! NLP-disabled/unsupported paths use the explicit local splitter. Every brief
//! stage and canonical publication consume this index; wire IDs (`t`/`d`/`b`)
//! come from one registry.

use serde::{Deserialize, Serialize};

use super::brief_stages::Sentence;
use super::clean_body::SpanMap;
use crate::nlp::{Annotation, Span, Status};

/// Index contract; part of cache identity for consumers.
pub const INDEX_VERSION: &str = "sentence-index-v1";
/// Local splitter identity.
pub const LOCAL_PARSER: &str = "argos-local-punct-v1";

/// Wire-ID registry: the one place prefixes are defined.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireScope {
    Title,
    Description,
    Body,
}

impl WireScope {
    pub fn prefix(self) -> &'static str {
        match self {
            WireScope::Title => "t",
            WireScope::Description => "d",
            WireScope::Body => "b",
        }
    }

    pub fn of(id: &str) -> Option<Self> {
        match id.chars().next()? {
            't' => Some(WireScope::Title),
            'd' => Some(WireScope::Description),
            'b' => Some(WireScope::Body),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexedParagraph {
    pub id: String,
    pub start_byte: usize,
    pub end_byte: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexedSentence {
    pub id: String,
    pub paragraph: String,
    pub start_byte: usize,
    pub end_byte: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexedSpan {
    pub kind: String,
    pub start_byte: usize,
    pub end_byte: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SentenceIndex {
    pub version: String,
    /// Hash of the exact clean text these ranges index.
    pub content_hash: String,
    pub parser: String,
    /// NLP feature key/context when the parser was used.
    pub context: String,
    pub paragraphs: Vec<IndexedParagraph>,
    pub sentences: Vec<IndexedSentence>,
    pub mentions: Vec<IndexedSpan>,
    pub dates: Vec<IndexedSpan>,
    pub numbers: Vec<IndexedSpan>,
    /// `nlp-remapped`, `nlp-remapped; N paragraph(s) re-split locally`, or
    /// `local-fallback: <reason>`.
    pub coverage: String,
}

fn hash(text: &str) -> String {
    crate::nlp::cache::hash(text)
}

/// Content paragraphs (trimmed of surrounding whitespace).
fn paragraph_spans(text: &str) -> Vec<Span> {
    crate::nlp::spans::paragraphs(text)
        .into_iter()
        .filter_map(|p| {
            let slice = &text[p.start_byte..p.end_byte];
            let lead = slice.len() - slice.trim_start().len();
            let trimmed = slice.trim();
            (!trimmed.is_empty()).then(|| Span {
                start_byte: p.start_byte + lead,
                end_byte: p.start_byte + lead + trimmed.len(),
            })
        })
        .collect()
}

/// Local punctuation splitter over one range (never crosses it).
fn split_local(text: &str, range: Span) -> Vec<Span> {
    let slice = &text[range.start_byte..range.end_byte];
    let mut out = Vec::new();
    let mut start = 0usize;
    let chars: Vec<(usize, char)> = slice.char_indices().collect();
    let mut push = |s: usize, e: usize| {
        let piece = &slice[s..e];
        let lead = piece.len() - piece.trim_start().len();
        let trimmed = piece.trim();
        if !trimmed.is_empty() {
            let s = range.start_byte + s + lead;
            out.push(Span {
                start_byte: s,
                end_byte: s + trimmed.len(),
            });
        }
    };
    for (i, (b, ch)) in chars.iter().enumerate() {
        let next = chars.get(i + 1).map(|(_, c)| *c);
        let boundary = (matches!(ch, '.' | '!' | '?' | '。')
            && next.is_none_or(char::is_whitespace))
            || (*ch == '\n' && next == Some('\n'));
        if boundary {
            let end = b + ch.len_utf8();
            push(start, end);
            start = end;
        }
    }
    push(start, slice.len());
    out
}

/// Remap a baseline span to clean coordinates when it lies wholly inside one
/// retained span.
fn remap(span: Span, maps: &[SpanMap]) -> Option<Span> {
    maps.iter().find_map(|m| {
        (span.start_byte >= m.baseline_start && span.end_byte <= m.baseline_end).then(|| Span {
            start_byte: m.clean_start + (span.start_byte - m.baseline_start),
            end_byte: m.clean_start + (span.end_byte - m.baseline_start),
        })
    })
}

fn trim_span(text: &str, span: Span) -> Option<Span> {
    let slice = text.get(span.start_byte..span.end_byte)?;
    let lead = slice.len() - slice.trim_start().len();
    let trimmed = slice.trim();
    (!trimmed.is_empty()).then(|| Span {
        start_byte: span.start_byte + lead,
        end_byte: span.start_byte + lead + trimmed.len(),
    })
}

impl SentenceIndex {
    /// Local fallback index (no NLP).
    pub fn local(clean: &str, reason: &str) -> Self {
        Self::assemble(
            clean,
            LOCAL_PARSER,
            "",
            paragraph_spans(clean)
                .into_iter()
                .map(|p| (p, split_local(clean, p)))
                .collect(),
            vec![],
            format!("local-fallback: {reason}"),
        )
    }

    /// Build from the baseline annotation, remapped through `maps`.
    pub fn from_baseline(
        clean: &str,
        baseline: &str,
        annotation: Option<&Annotation>,
        maps: &[SpanMap],
    ) -> Self {
        let Some(a) = annotation else {
            return Self::local(clean, "nlp unavailable");
        };
        if a.status != Status::Complete {
            return Self::local(clean, "nlp incomplete");
        }
        if a.prose_hash != hash(baseline) {
            return Self::local(clean, "annotation does not match the baseline");
        }
        let remapped: Vec<Span> = a
            .sentences
            .iter()
            .filter_map(|s| remap(s.span, maps))
            .filter_map(|s| trim_span(clean, s))
            .collect();
        let crossing = a.sentences.len() - remapped.len();
        let mut resplit = 0;
        let mut per_paragraph = Vec::new();
        for p in paragraph_spans(clean) {
            let inside: Vec<Span> = remapped
                .iter()
                .copied()
                .filter(|s| s.start_byte >= p.start_byte && s.end_byte <= p.end_byte)
                .collect();
            // Certified when the parser's sentences exactly tile the
            // paragraph's non-whitespace text.
            let certified = !inside.is_empty() && {
                let mut cursor = p.start_byte;
                inside.iter().all(|s| {
                    let gap_ok = clean[cursor..s.start_byte].trim().is_empty();
                    cursor = s.end_byte;
                    gap_ok
                }) && clean[cursor..p.end_byte].trim().is_empty()
            };
            if certified {
                per_paragraph.push((p, inside));
            } else {
                resplit += 1;
                per_paragraph.push((p, split_local(clean, p)));
            }
        }
        let spans = |kind: &str, list: Vec<Span>| -> Vec<IndexedSpan> {
            list.into_iter()
                .filter_map(|s| remap(s, maps))
                .filter(|s| clean.get(s.start_byte..s.end_byte).is_some())
                .map(|s| IndexedSpan {
                    kind: kind.into(),
                    start_byte: s.start_byte,
                    end_byte: s.end_byte,
                })
                .collect()
        };
        let mut extra: Vec<IndexedSpan> = a
            .mentions
            .iter()
            .filter_map(|m| {
                let s = remap(m.span, maps)?;
                clean.get(s.start_byte..s.end_byte)?;
                Some(IndexedSpan {
                    kind: format!("mention:{}", m.kind),
                    start_byte: s.start_byte,
                    end_byte: s.end_byte,
                })
            })
            .collect();
        extra.extend(spans(
            "date",
            a.date_candidates.iter().map(|d| d.span).collect(),
        ));
        extra.extend(spans("number", a.numerical_spans.clone()));
        let coverage = if resplit == 0 {
            "nlp-remapped".to_string()
        } else {
            format!(
                "nlp-remapped; {resplit} paragraph(s) re-split locally ({crossing} span(s) crossed a removal)"
            )
        };
        Self::assemble(
            clean,
            "argos-nlp",
            &a.feature_key,
            per_paragraph,
            extra,
            coverage,
        )
    }

    fn assemble(
        clean: &str,
        parser: &str,
        context: &str,
        per_paragraph: Vec<(Span, Vec<Span>)>,
        extra: Vec<IndexedSpan>,
        coverage: String,
    ) -> Self {
        let mut paragraphs = Vec::new();
        let mut sentences = Vec::new();
        for (n, (p, list)) in per_paragraph.into_iter().enumerate() {
            let pid = format!("p{}", n + 1);
            paragraphs.push(IndexedParagraph {
                id: pid.clone(),
                start_byte: p.start_byte,
                end_byte: p.end_byte,
            });
            for s in list {
                sentences.push(IndexedSentence {
                    id: format!("{}{}", WireScope::Body.prefix(), sentences.len() + 1),
                    paragraph: pid.clone(),
                    start_byte: s.start_byte,
                    end_byte: s.end_byte,
                });
            }
        }
        let (mut mentions, mut dates, mut numbers) = (Vec::new(), Vec::new(), Vec::new());
        for s in extra {
            if s.kind.starts_with("mention") {
                mentions.push(s);
            } else if s.kind == "date" {
                dates.push(s);
            } else {
                numbers.push(s);
            }
        }
        Self {
            version: INDEX_VERSION.into(),
            content_hash: hash(clean),
            parser: parser.into(),
            context: context.into(),
            paragraphs,
            sentences,
            mentions,
            dates,
            numbers,
            coverage,
        }
    }

    /// True when this index describes exactly `clean`.
    pub fn validates(&self, clean: &str) -> bool {
        self.version == INDEX_VERSION
            && self.content_hash == hash(clean)
            && self
                .sentences
                .iter()
                .all(|s| s.start_byte < s.end_byte && clean.get(s.start_byte..s.end_byte).is_some())
    }

    /// Wire sentences (`b1…`) with text, in document order.
    pub fn wire_sentences(&self, clean: &str) -> Vec<Sentence> {
        self.sentences
            .iter()
            .map(|s| Sentence {
                id: s.id.clone(),
                text: clean[s.start_byte..s.end_byte].to_string(),
                start_byte: s.start_byte,
                end_byte: s.end_byte,
            })
            .collect()
    }

    /// Paragraph of a sentence id.
    pub fn paragraph_of(&self, sentence_id: &str) -> Option<&str> {
        self.sentences
            .iter()
            .find(|s| s.id == sentence_id)
            .map(|s| s.paragraph.as_str())
    }
}

/// Body sentences from a committed Stage-1 output: its persisted index when
/// it validates against the markdown, else the local fallback (pre-index
/// artifacts). The single entry point for stages and canonical publication.
pub fn body_sentences(body: &serde_json::Value) -> (Vec<Sentence>, SentenceIndex) {
    let markdown = body["markdown"].as_str().unwrap_or("");
    let index = serde_json::from_value::<SentenceIndex>(body["sentence_index"].clone())
        .ok()
        .filter(|i| i.validates(markdown))
        .unwrap_or_else(|| SentenceIndex::local(markdown, "artifact predates the sentence index"));
    (index.wire_sentences(markdown), index)
}

/// Metadata sentences (`t…` title, `d…` description) through the registry.
pub fn metadata_wire(title: &str, description: &str) -> Vec<Sentence> {
    let mut out = Vec::new();
    for (scope, text) in [
        (WireScope::Title, title),
        (WireScope::Description, description),
    ] {
        let full = Span {
            start_byte: 0,
            end_byte: text.len(),
        };
        for (n, s) in split_local(text, full).into_iter().enumerate() {
            out.push(Sentence {
                id: format!("{}{}", scope.prefix(), n + 1),
                text: text[s.start_byte..s.end_byte].to_string(),
                start_byte: s.start_byte,
                end_byte: s.end_byte,
            });
        }
    }
    out
}

#[cfg(test)]
#[path = "sentence_index_tests.rs"]
mod tests;
