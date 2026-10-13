use super::*;

const STORY: &str = "Annual results were published on Monday. Revenue rose 4% across the group.\n\nAnn Lee, the finance chief, joined in 2019. She oversaw the audit.\n\nAnalysts expect further cuts. Weather was mild in the region.\n\n\"We did nothing wrong,\" a spokesperson for Northwind said.\n\nThe regulator later corrected its earlier statement about the fine.";

fn index() -> SentenceIndex {
    SentenceIndex::local(STORY, "test")
}

fn text_of(i: &SentenceIndex, id: &str) -> String {
    let s = i.sentences.iter().find(|s| s.id == id).unwrap();
    STORY[s.start_byte..s.end_byte].to_string()
}

#[test]
fn token_boundary_matching_never_uses_substrings() {
    assert!(contains_ngram(
        &tokens("Ann Lee joined"),
        &tokens("Ann Lee")
    ));
    assert!(!contains_ngram(&tokens("Annual results"), &tokens("Ann")));
    assert!(!contains_ngram(&tokens("Leeds won"), &tokens("Lee")));
    assert!(contains_ngram(&tokens("rose 4% across"), &tokens("4%")));
}

#[test]
fn actor_hit_expands_pronoun_paragraph_and_protects_late_corrections() {
    let i = index();
    let set = build_packets(
        &i,
        STORY,
        &PacketQuery {
            actors: vec!["Ann Lee".into()],
            ..Default::default()
        },
        1,
    );
    let picked: Vec<String> = set.selected.iter().map(|id| text_of(&i, id)).collect();
    assert!(picked.iter().any(|t| t.starts_with("Ann Lee")));
    assert!(
        picked.iter().any(|t| t == "She oversaw the audit."),
        "pronoun context kept"
    );
    assert!(
        picked
            .iter()
            .any(|t| t.contains("corrected its earlier statement")),
        "late correction protected"
    );
    assert!(
        !picked.iter().any(|t| t.starts_with("Annual")),
        "no substring hit"
    );
    assert!(!picked.iter().any(|t| t.starts_with("Weather")));
    assert_eq!(set.selected.len() + set.omitted.len(), i.sentences.len());
    assert!(set.reasons.iter().any(|(_, w)| w == "protected"));
}

#[test]
fn quotes_pull_whole_paragraph_and_render_marks_omissions() {
    let i = index();
    let set = build_packets(
        &i,
        STORY,
        &PacketQuery {
            actors: vec!["Northwind".into()],
            ..Default::default()
        },
        1,
    );
    let rendered = set.render(&i, STORY);
    assert!(
        rendered.first().unwrap().get("omitted").is_some(),
        "leading gap marked"
    );
    let joined = serde_json::to_string(&rendered).unwrap();
    assert!(joined.contains("We did nothing wrong"));
    // Every omitted ID appears in exactly one omission marker.
    let mut marked: Vec<String> = rendered
        .iter()
        .filter_map(|v| v.get("omitted"))
        .flat_map(|v| {
            v.as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_str().unwrap().to_string())
        })
        .collect();
    marked.sort();
    let mut omitted = set.omitted.clone();
    omitted.sort();
    assert_eq!(marked, omitted);
    assert_eq!(set.content_hash, i.content_hash);
}
