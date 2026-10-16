use super::*;
use crate::recon::model_exec::DecisionsAdapter;

fn actor(id: &str, name: &str, aliases: &[&str]) -> ContextActor {
    ContextActor {
        id: id.into(),
        name: name.into(),
        aliases: aliases.iter().map(|alias| (*alias).into()).collect(),
    }
}

fn context(body: &str, claim: Value, actors: &[ContextActor]) -> ClaimContext {
    let index = SentenceIndex::local(body, "test");
    claim_candidates(&index, body, &claim, actors).unwrap()
}

#[test]
fn matches_actor_token_boundaries_confirmed_spellings_and_original_utf8_spans() {
    let body = "Élan Council suspended the ferries.\n\nEC announced an inquiry.\n\nSpeculators discussed a pecan council.\n\nWeather was clear.";
    let context = context(
        body,
        json!({"id":"c1","text":"Élan Council suspended the ferries.","subject_id":"actor-1"}),
        &[actor("actor-1", "Élan Council", &["EC"])],
    );
    let ids: Vec<_> = context.candidates.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["b1", "b2"]);
    assert_eq!(context.actors[0].name, "Élan Council");
    assert_eq!(context.omitted.len(), 2);
    for sentence in &context.candidates {
        assert_eq!(
            body.get(sentence.start_byte..sentence.end_byte),
            Some(sentence.text.as_str())
        );
    }
}

#[test]
fn each_claim_associates_only_its_own_detected_actor() {
    let body = "North Board closed the road.\n\nSouth Board opened the harbor.\n\nAn unrelated election ended.";
    let actors = [
        actor("north", "North Board", &["NB"]),
        actor("south", "South Board", &["SB"]),
    ];
    let first = context(
        body,
        json!({"id":"c1","text":"The road closed.","subject_id":"north","attributed_to":"NB"}),
        &actors,
    );
    assert_eq!(first.actors, [actors[0].clone()]);
    assert_eq!(first.candidates.len(), 1);
    assert_eq!(first.candidates[0].id, "b1");
    let second = context(
        body,
        json!({"id":"c2","text":"The harbor opened.","attributed_to":"SB"}),
        &actors,
    );
    assert_eq!(second.actors, [actors[1].clone()]);
    assert_eq!(second.candidates[0].id, "b2");
}

#[test]
fn actorless_claim_uses_event_terms_without_picking_unrelated_sentences() {
    let body = "The ferry service closed after heavy storms.\n\nThe orchestra played on Friday.\n\nThe ferry service resumed on Monday.";
    let context = context(
        body,
        json!({"id":"c1","text":"Storms closed the ferry service.","predicate":"closed","object":"ferry service"}),
        &[],
    );
    assert!(context.actors.is_empty());
    assert_eq!(
        context
            .candidates
            .iter()
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>(),
        ["b1", "b3"]
    );
    assert_eq!(context.omitted[0].id, "b2");
}

#[test]
fn event_details_without_repeated_actor_name_reach_classifier() {
    let body = "North Board suspended ferry service.\n\nThe ferry service remains closed pending repairs.\n\nThe orchestra played a concert.";
    let context = context(
        body,
        json!({"id":"c1","text":"North Board suspended ferry service.","subject_id":"north","predicate":"suspended","object":"ferry service"}),
        &[actor("north", "North Board", &[])],
    );
    assert_eq!(context.actors.len(), 1);
    assert_eq!(
        context
            .candidates
            .iter()
            .map(|sentence| sentence.id.as_str())
            .collect::<Vec<_>>(),
        ["b1", "b2"]
    );
    assert!(context.candidates[1]
        .reasons
        .iter()
        .any(|reason| reason == "claim_event"));
    assert_eq!(context.omitted[0].id, "b3");
    let batches = classifier_batches(&context).unwrap();
    assert!(batches
        .iter()
        .any(|batch| batch.candidates.iter().any(|sentence| sentence.id == "b2")));
}

#[test]
fn protects_denials_and_bounded_pronoun_context_without_expanding_paragraphs() {
    let body = "North Board suspended ferries. It cited storm damage. The gardens remain open. Visitor numbers rose.\n\nA correction disputed the closure duration. They said the earlier duration was mistaken. A concert opened.";
    let context = context(
        body,
        json!({"id":"c1","text":"North Board suspended ferries."}),
        &[actor("north", "North Board", &[])],
    );
    let ids: Vec<_> = context.candidates.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["b1", "b2", "b5", "b6"]);
    assert!(context
        .candidates
        .iter()
        .find(|s| s.id == "b5")
        .unwrap()
        .reasons
        .iter()
        .any(|why| why == "protected_qualification"));
    assert_eq!(context.omitted.len(), 3);
}

#[test]
fn first_person_quote_and_immediate_attribution_context_survive_selection() {
    let body = "The spokesperson described the decision. North Board said, \"Ferries will close. We will reassess on Monday\". The gardens remain open. Visitor numbers rose.";
    let context = context(
        body,
        json!({"id":"c1","text":"North Board said ferries will close."}),
        &[actor("north", "North Board", &[])],
    );
    assert_eq!(
        context
            .candidates
            .iter()
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>(),
        ["b1", "b2", "b3"]
    );
    assert_eq!(context.omitted.len(), 2);
}

#[test]
fn selector_requires_matching_body_revision_and_valid_claim_id() {
    let index = SentenceIndex::local("North Board closed ferries.", "test");
    assert!(claim_candidates(
        &index,
        "South Board opened ferries.",
        &json!({"id":"c1","text":"Ferries closed."}),
        &[]
    )
    .is_err());
    assert!(claim_candidates(
        &index,
        "North Board closed ferries.",
        &json!({"text":"Ferries closed."}),
        &[]
    )
    .is_err());
}

#[test]
fn all_later_actor_candidates_reach_bounded_classifier_batches() {
    let body = (0..40)
        .map(|n| format!("North Board recorded ferry closure number {n}."))
        .collect::<Vec<_>>()
        .join("\n\n");
    let context = context(
        &body,
        json!({"id":"c1","text":"North Board recorded ferry closures."}),
        &[actor("north", "North Board", &[])],
    );
    assert_eq!(context.candidates.len(), 40);
    assert!(context.omitted.is_empty());
    let batches = classifier_batches(&context).unwrap();
    assert!(batches.len() >= 5);
    assert_eq!(
        batches.iter().map(|b| b.candidates.len()).sum::<usize>(),
        context.candidates.len()
    );
    for batch in batches {
        assert!(batch.candidates.len() <= CONTEXT_BATCH_QUESTIONS);
        assert!(batch.payload_bytes() <= CONTEXT_BATCH_BYTES);
        ContextDecisions { batch: &batch }.preflight().unwrap();
        for sentence in &batch.candidates {
            assert_eq!(&body[sentence.start_byte..sentence.end_byte], sentence.text);
        }
    }
}

#[test]
fn oversized_atomic_evidence_fails_explicitly_without_slicing() {
    let body = format!("North Board {}.", "enormous ".repeat(2000));
    let index = SentenceIndex::local(&body, "test");
    let error = claim_candidates(
        &index,
        &body,
        &json!({"id":"c1","text":"North Board closed the ferry."}),
        &[actor("north", "North Board", &[])],
    )
    .unwrap_err();
    assert!(error.contains("sentence `b1`"));
    assert!(error.contains("atomic"));
}

fn one_batch() -> ContextBatch {
    let context = context(
        "North Board closed ferries.\n\nOfficials denied the closure was permanent.",
        json!({"id":"c1","text":"North Board closed ferries."}),
        &[actor("north", "North Board", &[])],
    );
    classifier_batches(&context).unwrap().remove(0)
}

#[test]
fn chat_decisions_reject_unknown_missing_duplicate_ids_and_labels() {
    let batch = one_batch();
    let adapter = ContextDecisions { batch: &batch };
    let labels = adapter.parse_chat(r#"{"items":[{"id":"b1","relevance":"support"},{"id":"b2","relevance":"counterevidence"}]}"#).unwrap();
    assert_eq!(batch.selected(&labels).unwrap().len(), 2);
    for raw in [
        r#"{"items":[{"id":"b1","relevance":"support"}]}"#,
        r#"{"items":[{"id":"b1","relevance":"support"},{"id":"unknown","relevance":"context"}]}"#,
        r#"{"items":[{"id":"b1","relevance":"support"},{"id":"b1","relevance":"context"}]}"#,
        r#"{"items":[{"id":"b1","relevance":"support"},{"id":"b2","relevance":"faithful"}]}"#,
    ] {
        assert!(adapter.parse_chat(raw).is_err(), "{raw}");
    }
    let labels = adapter.parse_chat(r#"{"items":[{"id":"b1","relevance":"irrelevant"},{"id":"b2","relevance":"counterevidence"}]}"#).unwrap();
    assert_eq!(batch.selected(&labels).unwrap()[0].id, "b2");
}

#[test]
fn native_decisions_share_the_classifier_contract_and_validate_question_ids() {
    let batch = one_batch();
    let adapter = ContextDecisions { batch: &batch };
    adapter.preflight().unwrap();
    let (state, questions) = adapter.compile_native();
    assert_eq!(state["claim"]["id"], "c1");
    assert_eq!(state["candidates"][0]["id"], "b1");
    assert_eq!(questions["b1"]["type"], "choice");
    assert_eq!(adapter.contract().role_id, "classifier");
    let response = |answers| {
        serde_json::from_value::<crate::provider::DecisionsResponse>(json!({"answers":answers}))
            .unwrap()
    };
    let labels = adapter
        .parse_native(&response(json!({
            "b1":{"type":"choice","choice":"support"},
            "b2":{"type":"choice","choice":"counterevidence"}
        })))
        .unwrap();
    assert_eq!(labels["b2"], "counterevidence");
    assert!(adapter
        .parse_native(&response(json!({
            "b1":{"type":"choice","choice":"support"},
            "extra":{"type":"choice","choice":"irrelevant"}
        })))
        .is_err());
    assert!(adapter
        .parse_native(&response(json!({
            "b1":{"type":"choice","choice":"support","probabilities":{"invented":1.0}},
            "b2":{"type":"choice","choice":"counterevidence"}
        })))
        .is_err());
}

#[test]
fn no_match_keeps_explicit_omissions_and_dispatches_no_classifier_batch() {
    let context = context(
        "The orchestra played a concert.",
        json!({"id":"c1","text":"Ferries closed after storms."}),
        &[],
    );
    assert!(context.candidates.is_empty());
    assert_eq!(context.omitted.len(), 1);
    assert!(classifier_batches(&context).unwrap().is_empty());
}
