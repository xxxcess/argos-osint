use argos_osint_core::storage::inbox_qa::HealthFact;
use fleetworks_contracts::chat::QA_MAX_FINDINGS;
use fleetworks_contracts::chat::QA_MAX_WORDS;
use fleetworks_contracts::qa_worker::exact_span;

pub(super) fn label(fact: &HealthFact) -> &str {
    crate::workstation::qa_readiness_label(&fact.id).unwrap_or(&fact.label)
}

pub(super) fn answer(fact: &HealthFact) -> String {
    let label = label(fact);
    let status = match fact.status.as_str() {
        "ready" => "is ready",
        "not ready" | "not_ready" => "is not ready",
        "unavailable" => "readiness could not be verified",
        "not applicable" | "not_applicable" => "readiness is not applicable",
        "degraded" => "is degraded",
        "running" if fact.id == "classifier_training" => "has unfinished resource work",
        "running" => "work is in progress",
        _ => "readiness is unknown",
    };
    let mut answer = format!("{label} {status}.");
    if let Some(reason) = fact.reason.as_deref().filter(|reason| {
        !reason.is_empty()
            && answer.split_whitespace().count() + reason.split_whitespace().count()
                <= QA_MAX_WORDS / QA_MAX_FINDINGS
    }) {
        answer.push(' ');
        answer.push_str(reason);
    }
    answer
}

/// Promote a model-selected readiness subject, state, or reason to its complete
/// authorized observation. A partial name, description fragment, or negated
/// status word alone cannot select an observation.
pub fn validated_summary_finding<'a>(
    passage: &'a str,
    label: &str,
    start: u32,
    end: u32,
    text: &str,
) -> Option<&'a str> {
    if label.is_empty() || !exact_span(passage, start, end, text) {
        return None;
    }
    let (state, reason) = passage
        .split_once(". ")
        .map_or((passage.strip_suffix('.')?, None), |(state, reason)| {
            (state, reason.strip_suffix('.'))
        });
    let status = state.strip_prefix(label)?.strip_prefix(' ')?;
    let status = status
        .strip_prefix("readiness is ")
        .or_else(|| status.strip_prefix("is "))
        .unwrap_or(status);
    let state_end = state.chars().count();
    let covers = |lo, hi| (start as usize) <= lo && (end as usize) >= hi;
    let selected = covers(0, label.chars().count())
        || covers(state_end - status.chars().count(), state_end)
        || reason
            .is_some_and(|reason| covers(state_end + 2, state_end + 2 + reason.chars().count()));
    selected.then_some(passage)
}

#[cfg(test)]
mod tests {
    use super::answer;
    use super::validated_summary_finding;
    use argos_osint_core::storage::inbox_qa::HealthFact;

    fn fact(id: &str, status: &str, reason: Option<&str>) -> HealthFact {
        HealthFact {
            id: id.into(),
            label: "Old cached label".into(),
            status: status.into(),
            explanation: "This old generic statement is irrelevant to the result.".into(),
            reason: reason.map(str::to_owned),
        }
    }

    #[test]
    fn direct_answers_preserve_probe_uncertainty_and_scope() {
        for (status, expected) in [
            ("ready", "Gateway is ready."),
            ("not ready", "Gateway is not ready."),
            ("unavailable", "Gateway readiness could not be verified."),
            ("not_applicable", "Gateway readiness is not applicable."),
            ("unknown", "Gateway readiness is unknown."),
        ] {
            assert_eq!(answer(&fact("gateway", status, None)), expected);
        }
        assert_eq!(
            answer(&fact(
                "actor_catalog",
                "not ready",
                Some("No completed seed was verified, and existing records may remain."),
            )),
            "Actor catalog is not ready. No completed seed was verified, and existing records may remain.",
        );
        assert_eq!(
            answer(&fact("classifier", "ready", None)),
            "Classifier service is ready.",
        );
        assert_eq!(
            answer(&fact("classifier_inference", "unknown", None)),
            "Atlas Rust-BERT inference readiness is unknown.",
        );
        assert_eq!(
            answer(&fact("classifier_training", "running", None)),
            "Legacy Natural training has unfinished resource work.",
        );
    }

    fn selected<'a>(passage: &'a str, label: &str, text: &str) -> Option<&'a str> {
        let index = passage.find(text).expect("test extract occurs in passage");
        let start = passage[..index].chars().count() as u32;
        validated_summary_finding(
            passage,
            label,
            start,
            start + text.chars().count() as u32,
            text,
        )
    }

    #[test]
    fn model_may_select_a_full_subject_state_or_reason_for_a_summary() {
        let passage = "Actor catalog is not ready. No finished seed was verified.";
        for selection in [
            "Actor catalog",
            "not ready",
            "No finished seed was verified",
        ] {
            assert_eq!(selected(passage, "Actor catalog", selection), Some(passage));
        }
        for selection in ["catalog", "ready", "seed was verified"] {
            assert_eq!(selected(passage, "Actor catalog", selection), None);
        }
        assert_eq!(
            validated_summary_finding(passage, "Actor catalog", 1, 14, "Actor catalog"),
            None
        );
        assert_eq!(
            selected(
                "Atlas Rust-BERT inference readiness is unknown.",
                "Atlas Rust-BERT inference",
                "known",
            ),
            None,
        );
    }
}
