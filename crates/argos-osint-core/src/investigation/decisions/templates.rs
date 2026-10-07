//! Authoritative 12-template registry for Argos adaptive decision roles.

use super::contracts::{DecisionContract, QuestionSpec};

pub const TEMPLATE_VERSION: &str = "2026-10-07";

/// 1. mode: Which deliverable does the request require?
pub fn template_mode() -> DecisionContract {
    DecisionContract::new(
        "classifier",
        "mode",
        TEMPLATE_VERSION,
        ["task"],
        [QuestionSpec::choice(
            "deliverable_mode",
            "Classify which deliverable the request requires. If multiple or unclear, select ambiguous.",
            [
                ("verify", "Fact-check or verify a specific claim or assertion against source evidence."),
                ("explain", "Provide background context, history, and narrative explanation of a topic."),
                ("assess_outlook", "Evaluate future scenarios, forward-looking implications, and trends."),
                ("full_assessment", "Comprehensive multi-source intelligence report covering entity, relationships, and exposure."),
                ("ambiguous", "The prompt does not clearly select a single investigative objective.")
            ],
            "ambiguous"
        )],
    )
}

/// 2. directive_alignment: Does candidate.objective advance task.objective for this subject?
pub fn template_directive_alignment() -> DecisionContract {
    DecisionContract::new(
        "investigation_controller",
        "directive_alignment",
        TEMPLATE_VERSION,
        ["task", "candidate", "subject"],
        [QuestionSpec::choice(
            "directive_alignment",
            "Classify whether the candidate sub-directive advances the stated task objective for this subject.",
            [
                ("aligned", "The directive directly advances information requirements for this subject."),
                ("unrelated", "The directive diverges to an irrelevant topic, secondary entity, or unrequested question."),
                ("insufficient", "Incomplete task or candidate context prevents alignment assessment.")
            ],
            "insufficient"
        )],
    )
}

/// 3. tool_selection: Which eligible candidate best supplies task.required_evidence using confirmed inputs?
pub fn template_tool_selection(eligible_tool_candidates: &[(&str, &str)]) -> DecisionContract {
    let mut criteria: Vec<(&str, &str)> = eligible_tool_candidates.to_vec();
    criteria.push((
        "no_match",
        "None of the offered candidates provide the required evidence with available inputs.",
    ));
    criteria.push((
        "insufficient",
        "Context is ambiguous or required evidence is unspecified.",
    ));

    DecisionContract::new(
        "tool_picker",
        "tool_selection",
        TEMPLATE_VERSION,
        ["task", "subject"],
        [QuestionSpec::choice(
            "next_tool",
            "Select the single most suitable eligible tool from the candidate actions to retrieve required evidence.",
            criteria,
            "no_match"
        )],
    )
}

/// 4. scarce_provider_need: Is distinctive provider evidence needed beyond retained evidence and adequate eligible alternatives?
pub fn template_scarce_provider_need() -> DecisionContract {
    DecisionContract::new(
        "investigation_controller",
        "scarce_provider_need",
        TEMPLATE_VERSION,
        ["task", "candidate", "evidence", "computed_checks"],
        [QuestionSpec::choice(
            "scarce_provider_need",
            "Judge whether invoking a scarce or rate-limited provider is strictly necessary given retained evidence.",
            [
                ("necessary", "Retained evidence and free alternatives leave a material gap that this provider specifically resolves."),
                ("unnecessary", "Existing evidence already satisfies the requirement, or cheaper open tools suffice."),
                ("insufficient", "Current evidence sufficiency cannot be evaluated.")
            ],
            "insufficient"
        )],
    )
}

/// 5. evidence_relevance: Does this passage address the intended subject and directive?
pub fn template_evidence_relevance() -> DecisionContract {
    DecisionContract::new(
        "evidence_curator",
        "evidence_relevance",
        TEMPLATE_VERSION,
        ["subject", "candidate", "evidence"],
        [QuestionSpec::choice(
            "evidence_relevance",
            "Evaluate whether the cited passage directly addresses the intended entity and investigative directive.",
            [
                ("relevant", "The passage directly concerns the intended subject and provides factual information on the directive."),
                ("wrong_subject", "The passage concerns a namesake, different entity, homonym, or unrelated individual."),
                ("unrelated", "The passage mentions the subject only tangentially or discusses an entirely distinct topic."),
                ("insufficient", "Passage brevity or ambiguous context prevents relevance evaluation.")
            ],
            "insufficient"
        )],
    )
}

/// 6. extraction_fidelity: Is the proposed observation faithful to the cited passage?
pub fn template_extraction_fidelity() -> DecisionContract {
    DecisionContract::new(
        "evidence_curator",
        "extraction_fidelity",
        TEMPLATE_VERSION,
        ["candidate", "evidence"],
        [QuestionSpec::choice(
            "extraction_fidelity",
            "Assess whether the extracted claim or data point is strictly supported by the cited original passage.",
            [
                ("faithful", "The extracted facts are directly stated by or strictly entailed by the cited passage."),
                ("unsupported_addition", "The extracted observation introduces dates, amounts, roles, or claims absent from the passage."),
                ("material_omission", "The extraction omits crucial qualifiers, conditional clauses, or contradictory context."),
                ("insufficient", "Passage text is garbled, truncated, or insufficient to evaluate fidelity.")
            ],
            "insufficient"
        )],
    )
}

/// 7. entity_binding: Does supplied evidence establish the intended identity?
pub fn template_entity_binding() -> DecisionContract {
    DecisionContract::new(
        "entity_resolver",
        "entity_binding",
        TEMPLATE_VERSION,
        ["subject", "candidate", "evidence"],
        [QuestionSpec::choice(
            "entity_binding",
            "Determine whether the supplied identifier or account belongs to the same target entity.",
            [
                ("same_entity", "Evidence confirms this identifier, account, or record belongs to the target entity."),
                ("different_entity", "Evidence refutes connection or indicates an unrelated person or organization."),
                ("ambiguous", "Identical names, overlapping profiles, or conflicting data prevent definitive binding.")
            ],
            "ambiguous"
        )],
    )
}

/// 8. claim_relation: How does the exact passage bear on the claim’s qualifiers and time scope?
pub fn template_claim_relation() -> DecisionContract {
    DecisionContract::new(
        "claim_assessor",
        "claim_relation",
        TEMPLATE_VERSION,
        ["candidate", "evidence"],
        [QuestionSpec::choice(
            "claim_relation",
            "Classify how passage bears on claim. An agreement or proposal does not establish completion. Do not infer later events.",
            [
                ("supports", "The passage explicitly establishes the factual truth of the claim within the applicable time scope."),
                ("contradicts", "The passage explicitly refutes or disproves the claim within the applicable time scope."),
                ("mentions_only", "The passage concerns the subject or event without establishing or refuting the consequential claim."),
                ("irrelevant", "The passage concerns a different subject, entity, or unrelated event."),
                ("insufficient", "Ambiguity or missing context prevents relationship classification.")
            ],
            "insufficient"
        )],
    )
}

/// 9. handoff_adequacy: Are material findings, uncertainty and unfinished work preserved?
pub fn template_handoff_adequacy() -> DecisionContract {
    DecisionContract::new(
        "investigation_controller",
        "handoff_adequacy",
        TEMPLATE_VERSION,
        ["task", "candidate", "evidence"],
        [QuestionSpec::choice(
            "handoff_adequacy",
            "Evaluate whether the stage handoff accurately retains all material findings, qualifiers, and remaining uncertainties.",
            [
                ("adequate", "The handoff summary faithfully captures confirmed findings, unresolved questions, and qualifiers."),
                ("omission", "Material discoveries, caveats, or missing tasks were omitted from the handoff."),
                ("unsupported_addition", "The handoff introduces unverified claims not grounded in previous task evidence."),
                ("insufficient", "Context is insufficient to evaluate handoff completeness.")
            ],
            "insufficient"
        )],
    )
}

/// 10. next_action: Which offered task best closes the remaining gap?
pub fn template_next_action(offered_tasks: &[(&str, &str)]) -> DecisionContract {
    let mut criteria: Vec<(&str, &str)> = offered_tasks.to_vec();
    criteria.push((
        "request_replan",
        "Offered tasks cannot close the remaining gap; replanning is necessary.",
    ));
    criteria.push((
        "defer",
        "Remaining gaps cannot be resolved with available sources; defer further action.",
    ));
    criteria.push((
        "insufficient",
        "Information is insufficient to determine the next task.",
    ));

    DecisionContract::new(
        "investigation_controller",
        "next_action",
        TEMPLATE_VERSION,
        ["task", "evidence", "missing_context"],
        [QuestionSpec::choice(
            "next_action",
            "Select which offered task or action best resolves the remaining investigative gap.",
            criteria,
            "insufficient",
        )],
    )
}

/// 11. resolution: Is the required answer established, disputed, incomplete or blocked?
pub fn template_resolution() -> DecisionContract {
    DecisionContract::new(
        "investigation_controller",
        "resolution",
        TEMPLATE_VERSION,
        ["task", "evidence", "computed_checks"],
        [QuestionSpec::choice(
            "resolution",
            "Determine the final resolution status of the investigative objective based on retained evidence.",
            [
                ("resolved_supported", "The central question is established with corroborated evidence passages."),
                ("resolved_disputed", "Substantial contradictory evidence exists, establishing that the proposition is actively disputed."),
                ("needs_evidence", "Evidence collected thus far is insufficient to confirm or refute the question."),
                ("blocked", "Required sources are inaccessible, rate-limited, or permanently unavailable."),
                ("insufficient", "Evaluation criteria or question parameters are undefined.")
            ],
            "insufficient"
        )],
    )
}

/// 12. publication: Is the conclusion faithful to assessed evidence and uncertainty?
pub fn template_publication() -> DecisionContract {
    DecisionContract::new(
        "claim_assessor",
        "publication",
        TEMPLATE_VERSION,
        ["candidate", "evidence", "missing_context"],
        [QuestionSpec::choice(
            "publication",
            "Judge whether the final synthesis conclusion is strictly faithful to assessed evidence without overstatement.",
            [
                ("faithful", "The summary conclusion matches the strength of verified evidence and accurately discloses uncertainties."),
                ("overstated", "The conclusion claims definitive certainty where evidence is only suggestive, single-source, or unverified."),
                ("contradictory", "The conclusion asserts statements contradicted by retained evidence passages."),
                ("insufficient", "Draft report or evidence records are incomplete.")
            ],
            "insufficient"
        )],
    )
}
