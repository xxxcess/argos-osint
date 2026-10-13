use super::*;
use crate::intel_recon::body_filter::IrrelevantRange;
use crate::intel_recon::clean_body::apply_removals;
use crate::nlp::{Mention, Sentence as NlpSentence};

const BASE: &str = "Zoë Müller said the talks would continue. Unions denied it.\n\nAdvertisement — buy now.\n\n“We have not agreed,” the envoy said on 3 May. Shares fell 12%.";

fn annotation(text: &str, sentences: &[&str]) -> Annotation {
    let mut a = Annotation {
        schema_version: 0,
        feature_key: "fk".into(),
        prose_hash: crate::nlp::cache::hash(text),
        status: Status::Complete,
        sentences: vec![],
        mentions: vec![],
        date_candidates: vec![],
        numerical_spans: vec![],
        flags: vec![],
        metrics: Default::default(),
    };
    for (n, s) in sentences.iter().enumerate() {
        let start = text.find(s).unwrap();
        a.sentences.push(NlpSentence {
            id: format!("s{n}"),
            span: Span {
                start_byte: start,
                end_byte: start + s.len(),
            },
        });
    }
    let m = text.find("Zoë Müller").unwrap();
    a.mentions.push(Mention {
        span: Span {
            start_byte: m,
            end_byte: m + "Zoë Müller".len(),
        },
        kind: "person".into(),
    });
    a
}

fn ad_range() -> IrrelevantRange {
    let start_b = BASE.find("Advertisement").unwrap();
    let end_b = BASE.find("“We").unwrap();
    IrrelevantRange {
        start: BASE[..start_b].chars().count(),
        end: BASE[..end_b].chars().count(),
        reason: "chrome".into(),
    }
}

#[test]
fn baseline_spans_remap_verbatim_to_clean_coordinates() {
    let a = annotation(
        BASE,
        &[
            "Zoë Müller said the talks would continue.",
            "Unions denied it.",
            "Advertisement — buy now.",
            "“We have not agreed,” the envoy said on 3 May.",
            "Shares fell 12%.",
        ],
    );
    let (clean, maps) = apply_removals(BASE, &[ad_range()]);
    let index = SentenceIndex::from_baseline(&clean, BASE, Some(&a), &maps);
    assert!(index.validates(&clean));
    assert_eq!(index.parser, "argos-nlp");
    assert_eq!(index.coverage, "nlp-remapped");
    let wire = index.wire_sentences(&clean);
    let texts: Vec<&str> = wire.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(
        texts,
        [
            "Zoë Müller said the talks would continue.",
            "Unions denied it.",
            "“We have not agreed,” the envoy said on 3 May.",
            "Shares fell 12%."
        ]
    );
    assert_eq!(wire[0].id, "b1");
    assert_eq!(index.paragraphs.len(), 2);
    assert_eq!(index.paragraph_of("b3"), Some("p2"));
    let m = &index.mentions[0];
    assert_eq!(&clean[m.start_byte..m.end_byte], "Zoë Müller");
    assert_eq!(m.kind, "mention:person");
}

#[test]
fn uncertifiable_paragraphs_are_resplit_locally_and_reported() {
    // Parser merged two sentences across the paragraph break.
    let a = annotation(
        BASE,
        &["Zoë Müller said the talks would continue. Unions denied it.\n\nAdvertisement — buy now."],
    );
    let (clean, maps) = apply_removals(BASE, &[ad_range()]);
    let index = SentenceIndex::from_baseline(&clean, BASE, Some(&a), &maps);
    assert!(
        index.coverage.contains("re-split locally"),
        "{}",
        index.coverage
    );
    let texts: Vec<String> = index
        .wire_sentences(&clean)
        .into_iter()
        .map(|s| s.text)
        .collect();
    assert!(texts.contains(&"Unions denied it.".to_string()));
    assert!(texts.contains(&"Shares fell 12%.".to_string()));
    assert!(index.validates(&clean));
}

#[test]
fn missing_or_stale_annotation_uses_explicit_local_fallback() {
    let index = SentenceIndex::from_baseline(BASE, BASE, None, &[]);
    assert_eq!(index.parser, LOCAL_PARSER);
    assert!(index.coverage.starts_with("local-fallback"));
    let stale = annotation("Zoë Müller spoke. Here.", &["Here."]);
    let index = SentenceIndex::from_baseline(BASE, BASE, Some(&stale), &[]);
    assert!(index.coverage.contains("does not match"));
    // Index for other text never validates.
    assert!(!index.validates("other"));
}

#[test]
fn body_sentences_prefers_valid_index_and_falls_back_for_old_artifacts() {
    let index = SentenceIndex::local(BASE, "test");
    let body = serde_json::json!({"markdown": BASE, "sentence_index": index});
    let (s, i) = body_sentences(&body);
    assert_eq!(i, index);
    assert_eq!(s.len(), index.sentences.len());
    let old = serde_json::json!({"markdown": BASE});
    let (s2, i2) = body_sentences(&old);
    assert!(i2.coverage.contains("predates"));
    assert_eq!(
        s2.iter().map(|s| &s.text).collect::<Vec<_>>(),
        s.iter().map(|s| &s.text).collect::<Vec<_>>()
    );
    let tampered =
        serde_json::json!({"markdown": format!("{BASE} extra"), "sentence_index": index});
    assert!(body_sentences(&tampered).1.coverage.contains("predates"));
}

#[test]
fn wire_registry_prefixes_are_single_sourced() {
    let meta = metadata_wire("Title one. Title two.", "Desc.");
    let ids: Vec<&str> = meta.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["t1", "t2", "d1"]);
    assert_eq!(WireScope::of("b12"), Some(WireScope::Body));
    assert_eq!(WireScope::of("d1"), Some(WireScope::Description));
    assert_eq!(WireScope::of("x1"), None);
}
