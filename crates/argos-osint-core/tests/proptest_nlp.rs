//! Property-based tests for NLP tokenization, entity extraction, and span invariants.
//!
//! These tests verify invariants that must hold for arbitrary input text, focusing on
//! the pure logic in `crates/argos-osint-core/src/nlp/` and `src/nlp/spans.rs`.

use argos_osint_core::nlp::cache::hash;
use argos_osint_core::nlp::spans::{
    byte_to_scalar, paragraphs, scalar_to_byte, utf16_to_byte, Span,
};
use argos_osint_core::nlp::{Annotation, ParseContext, Source, Status};
use proptest::prelude::*;
use proptest::test_runner::TestRunner;

fn config() -> ProptestConfig {
    ProptestConfig::with_cases(256)
}

/// Span.slice returns a valid substring when start_byte < end_byte and within bounds.
#[test]
fn span_slice_bounds() {
    let mut runner = TestRunner::new(config());
    runner
        .run(
            &(any::<String>(), 0..100usize, 0..100usize),
            |(text, start, end)| {
                let span = Span {
                    start_byte: start.min(end),
                    end_byte: start.max(end),
                };
                let slice = span.slice(&text);
                // slice returns Some only when byte offsets are valid UTF-8 boundaries
                if span.start_byte <= text.len()
                    && span.end_byte <= text.len()
                    && span.start_byte < span.end_byte
                    && text.is_char_boundary(span.start_byte)
                    && text.is_char_boundary(span.end_byte)
                {
                    prop_assert!(slice.is_some());
                    prop_assert_eq!(slice.unwrap(), &text[span.start_byte..span.end_byte]);
                } else {
                    prop_assert!(slice.is_none());
                }
                Ok(())
            },
        )
        .unwrap();
}

/// Span.slice is None when start_byte >= end_byte
#[test]
fn span_slice_empty_when_start_ge_end() {
    let mut runner = TestRunner::new(config());
    runner
        .run(&(any::<String>(), 0..100usize), |(text, idx)| {
            let span = Span {
                start_byte: idx,
                end_byte: idx,
            };
            prop_assert!(span.slice(&text).is_none());
            Ok(())
        })
        .unwrap();
}

/// utf16_to_byte round-trip: for any valid UTF-16 offset, scalar_to_byte and byte_to_scalar are inverses
#[test]
fn utf16_roundtrip() {
    let mut runner = TestRunner::new(config());
    runner
        .run(&(any::<String>(), 0..200usize), |(text, scalar_offset)| {
            if let Some(byte_offset) = scalar_to_byte(&text, scalar_offset) {
                prop_assert_eq!(
                    byte_to_scalar(&text, byte_offset),
                    Some(scalar_offset.min(text.chars().count()))
                );
            }
            Ok(())
        })
        .unwrap();
}

/// utf16_to_byte returns correct byte offset for valid UTF-16 positions
#[test]
fn utf16_to_byte_correct() {
    let mut runner = TestRunner::new(config());
    runner
        .run(&any::<String>(), |text| {
            let mut utf16_pos = 0;
            for (byte_idx, ch) in text.char_indices() {
                prop_assert_eq!(utf16_to_byte(&text, utf16_pos), Some(byte_idx));
                utf16_pos += ch.len_utf16();
            }
            prop_assert_eq!(utf16_to_byte(&text, utf16_pos), Some(text.len()));
            Ok(())
        })
        .unwrap();
}

/// paragraphs() covers all non-whitespace text exactly once
#[test]
fn paragraphs_cover_text() {
    let mut runner = TestRunner::new(config());
    runner
        .run(&any::<String>(), |text| {
            let paras = paragraphs(&text);
            let reconstructed: String = paras.iter().filter_map(|p| p.slice(&text)).collect();
            let non_ws_original: String = text.chars().filter(|c| !c.is_whitespace()).collect();
            let non_ws_reconstructed: String = reconstructed
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            prop_assert_eq!(non_ws_reconstructed, non_ws_original);
            Ok(())
        })
        .unwrap();
}

/// Paragraph spans are non-overlapping and ordered
#[test]
fn paragraphs_non_overlapping_ordered() {
    let mut runner = TestRunner::new(config());
    runner
        .run(&any::<String>(), |text| {
            let paras = paragraphs(&text);
            for w in paras.windows(2) {
                prop_assert!(w[0].end_byte <= w[1].start_byte);
            }
            Ok(())
        })
        .unwrap();
}

/// Hash is deterministic
#[test]
fn hash_deterministic() {
    let mut runner = TestRunner::new(config());
    runner
        .run(&any::<String>(), |text| {
            let h1 = hash(&text);
            let h2 = hash(&text);
            prop_assert_eq!(h1, h2);
            Ok(())
        })
        .unwrap();
}

/// Different texts produce different hashes (with very high probability)
#[test]
fn hash_different_for_different_inputs() {
    let mut runner = TestRunner::new(config());
    runner
        .run(&(any::<String>(), any::<String>()), |(a, b)| {
            if a != b {
                prop_assert_ne!(hash(&a), hash(&b));
            }
            Ok(())
        })
        .unwrap();
}

/// Source::derived produces a valid Source with correct hash
#[test]
fn source_derived_hash_correct() {
    let mut runner = TestRunner::new(config());
    runner
        .run(
            &(any::<String>(), any::<String>(), any::<String>()),
            |(id, revision, prose)| {
                let source = Source::derived(&id, &revision, &prose);
                let prose_hash = hash(&prose);
                prop_assert_eq!(source.source_id, id);
                prop_assert_eq!(source.revision, revision);
                prop_assert_eq!(source.prose, prose);
                prop_assert_eq!(source.prose_hash, prose_hash);
                Ok(())
            },
        )
        .unwrap();
}

/// ParseContext default has valid reference_date format
#[test]
fn parse_context_default_valid() {
    let mut runner = TestRunner::new(config());
    runner
        .run(&any::<String>(), |_text| {
            let ctx = ParseContext::default();
            prop_assert!(ctx.reference_date.len() >= 10);
            prop_assert!(ctx.reference_date[..4].chars().all(|c| c.is_ascii_digit()));
            prop_assert_eq!(&ctx.reference_date[4..5], "-");
            Ok(())
        })
        .unwrap();
}

/// Create a minimal complete annotation for testing
fn make_complete_annotation(source: &Source, prose: &str) -> Annotation {
    Annotation {
        schema_version: 1,
        feature_key: hash(prose),
        prose_hash: hash(prose),
        status: Status::Complete,
        sentences: vec![],
        mentions: vec![],
        date_candidates: vec![],
        numerical_spans: vec![],
        flags: vec![],
        metrics: Default::default(),
    }
}

/// Annotation.validate returns false for mismatched prose_hash
#[test]
fn annotation_validate_rejects_bad_hash() {
    let mut runner = TestRunner::new(config());
    runner
        .run(&any::<String>(), |prose| {
            let source = Source::derived("test", "1", &prose);
            let mut annotation = make_complete_annotation(&source, &prose);
            annotation.prose_hash = "invalid_hash".to_string();
            prop_assert!(!annotation.validate(&source));
            Ok(())
        })
        .unwrap();
}

/// Annotation.validate returns false when span is out of bounds
#[test]
fn annotation_validate_rejects_oob_spans() {
    let mut runner = TestRunner::new(config());
    runner
        .run(&any::<String>(), |prose| {
            let source = Source::derived("test", "1", &prose);
            let mut annotation = make_complete_annotation(&source, &prose);
            annotation.sentences.push(argos_osint_core::nlp::Sentence {
                id: "test".to_string(),
                span: Span {
                    start_byte: 0,
                    end_byte: prose.len() + 10,
                },
            });
            prop_assert!(!annotation.validate(&source));
            Ok(())
        })
        .unwrap();
}

/// Annotation.validate returns true for valid complete annotation
#[test]
fn annotation_validate_accepts_valid() {
    let mut runner = TestRunner::new(config());
    runner
        .run(&any::<String>(), |prose| {
            let source = Source::derived("test", "1", &prose);
            let annotation = make_complete_annotation(&source, &prose);
            prop_assert!(annotation.validate(&source));
            Ok(())
        })
        .unwrap();
}
