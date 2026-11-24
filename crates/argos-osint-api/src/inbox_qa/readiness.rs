use argos_osint_core::storage::inbox_qa::HealthFact;
use fleetworks_contracts::chat::QA_MAX_FINDINGS;
use fleetworks_contracts::chat::QA_MAX_WORDS;
use fleetworks_contracts::qa_worker::exact_span;
use fleetworks_contracts::readiness::ReadinessCheckId;

use super::question::StatusChecks;

pub(super) fn label(fact: &HealthFact) -> &str {
    crate::workstation::qa_readiness_label(&fact.id).unwrap_or(&fact.label)
}

pub(super) fn answer(fact: &HealthFact) -> String {
    let label = label(fact);
    let status = match fact.status.as_str() {
        "ready" => "is ready",
        "failed" | "not ready" | "not_ready" => "is not ready",
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

/// Summarize the same fresh status groups used by the Workstation view.
pub(super) fn status_check_summary<'a>(
    facts: impl Iterator<Item = &'a HealthFact>,
    selection: StatusChecks,
) -> Option<(String, Vec<&'a HealthFact>)> {
    let mut facts = facts.peekable();
    facts.peek()?;
    let (mut passed, mut failed, mut unresolved) = (0, 0, 0);
    let mut relevant = Vec::new();
    for fact in facts.filter(|fact| {
        fact.id == ReadinessCheckId::Registration.as_str()
            || (fact.id != ReadinessCheckId::ClassifierTraining.as_str()
                && crate::workstation::qa_readiness_label(&fact.id).is_some())
    }) {
        let status = match fact.status.as_str() {
            "not applicable" | "not_applicable" => continue,
            "ready" => {
                passed += 1;
                StatusChecks::Passed
            }
            "failed" => {
                failed += 1;
                StatusChecks::Failed
            }
            _ => {
                unresolved += 1;
                StatusChecks::Unresolved
            }
        };
        if selection == status || (selection == StatusChecks::All && status != StatusChecks::Passed)
        {
            relevant.push(fact);
        }
    }
    relevant.sort_by_key(|fact| priority(fact));
    let summary = match selection {
        StatusChecks::All if passed + failed + unresolved == 0 => {
            "No status checks apply in the latest observation.".into()
        }
        StatusChecks::All => format!(
            "The current status checks show {passed} passed, {failed} failed, and {unresolved} unresolved."
        ),
        selection => {
            let group = match selection {
                StatusChecks::Passed => "passed",
                StatusChecks::Failed => "failed",
                StatusChecks::Unresolved => "unresolved",
                StatusChecks::All => unreachable!(),
            };
            match relevant.as_slice() {
                [] => format!("No {group} status checks were observed."),
                [fact] => format!("One {group} status check covers {}.", label(fact)),
                _ => {
                    let names = relevant.iter().map(|fact| label(fact)).collect::<Vec<_>>();
                    format!("{} {group} status checks cover {}.", names.len(), names.join(", "))
                }
            }
        }
    };
    Some((summary, relevant))
}

/// Show current blockers and in-progress work before passed checks in a broad summary.
pub(super) fn priority(fact: &HealthFact) -> u8 {
    match fact.status.as_str() {
        "failed" => 0,
        "not ready" | "not_ready" | "degraded" => 1,
        "unavailable" => 2,
        "running" => 3,
        "ready" => 5,
        _ => 4,
    }
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
    use super::status_check_summary;
    use super::validated_summary_finding;
    use super::StatusChecks;
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

    #[test]
    fn unresolved_checks_exclude_passed_failed_and_retired_checks() {
        let facts = [
            fact("gateway", "ready", None),
            fact("jobs", "failed", None),
            fact(
                "actor_search",
                "running",
                Some("Index visibility is incomplete."),
            ),
            fact("actor_catalog", "not ready", None),
            fact("classifier_inference", "not applicable", None),
            fact("classifier_training", "not ready", None),
        ];
        let (summary, details) =
            status_check_summary(facts.iter(), StatusChecks::Unresolved).unwrap();
        assert_eq!(
            summary,
            "2 unresolved status checks cover Actor catalog, Actor search."
        );
        assert_eq!(
            details
                .iter()
                .map(|fact| fact.id.as_str())
                .collect::<Vec<_>>(),
            ["actor_catalog", "actor_search"]
        );
        let (summary, details) = status_check_summary(facts.iter(), StatusChecks::Failed).unwrap();
        assert_eq!(summary, "One failed status check covers Jobs service.");
        assert_eq!(details.len(), 1);
        let (summary, _) =
            status_check_summary(facts[..2].iter(), StatusChecks::Unresolved).unwrap();
        assert_eq!(summary, "No unresolved status checks were observed.");
        let registration = HealthFact {
            label: "Registration".into(),
            ..fact("registration", "ready", None)
        };
        let (summary, details) =
            status_check_summary(facts.iter().chain([&registration]), StatusChecks::Passed)
                .unwrap();
        assert_eq!(
            summary,
            "2 passed status checks cover Gateway, Registration."
        );
        assert_eq!(details.len(), 2);
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
