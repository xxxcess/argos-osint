use argos_osint_core::nlp::conversation::CheckSelection;
use argos_osint_core::nlp::conversation::ClarificationReason;
use argos_osint_core::nlp::conversation::Constraint;
use argos_osint_core::nlp::conversation::Features;
use argos_osint_core::nlp::conversation::Focus;
use argos_osint_core::nlp::conversation::Goal;
use argos_osint_core::nlp::conversation::Intent;
use argos_osint_core::nlp::conversation::Interval;
use argos_osint_core::nlp::conversation::Metric as FocusMetric;
use argos_osint_core::nlp::conversation::PendingClarification;
use argos_osint_core::nlp::conversation::TemporalScope;
use argos_osint_core::nlp::Span;
use argos_osint_core::nlp::Status;
use fleetworks_contracts::chat::ChatSourceReference;
use fleetworks_contracts::readiness::ReadinessCheckId;

use super::question::{self, Metric, Question, StatusChecks};

const MAX_CHOICES: usize = 3;

pub(super) struct Resolution {
    pub intent: Question,
    pub source: Option<ChatSourceReference>,
    pub focus: Option<Focus>,
    pub pending: Option<PendingClarification>,
    pub used_previous_exchange: bool,
}

impl Resolution {
    fn original(intent: Question, source: Option<&ChatSourceReference>) -> Self {
        Self {
            intent,
            source: source.cloned(),
            focus: None,
            pending: None,
            used_previous_exchange: false,
        }
    }

    fn focused(focus: Focus, used_previous_exchange: bool) -> Self {
        Self {
            intent: question_for(focus.intent),
            source: focus.source.clone(),
            focus: Some(focus),
            pending: None,
            used_previous_exchange,
        }
    }

    fn clarify(
        reason: ClarificationReason,
        mut choices: Vec<Focus>,
        time: &TimeRequest,
        reused: bool,
    ) -> Self {
        choices.truncate(MAX_CHOICES);
        Self {
            intent: Question::ChooseSubject,
            source: None,
            focus: None,
            pending: Some(PendingClarification {
                reason,
                choices,
                temporal: match time {
                    TimeRequest::Historical(interval) => Some(interval.clone()),
                    _ => None,
                },
            }),
            used_previous_exchange: reused,
        }
    }
}

/// Resolve only known routing forms and standalone selections. Parser hints never
/// grant access, identify a resource, or override the established routing allowlist.
pub(super) fn resolve(
    question: &str,
    selected: Option<&ChatSourceReference>,
    original: Question,
    features: &Features,
    previous: Option<&Focus>,
    pending: Option<&PendingClarification>,
) -> Resolution {
    if features.status != Status::Complete {
        return Resolution::original(original, selected);
    }
    let (mut ignored, constraints) = advice_constraints(question, features);
    let (temporal, time) = extract_time(question, features);
    ignored.extend(temporal);
    let has_reference = features.references.iter().any(|reference| {
        !ignored.iter().any(|span| {
            span.start_byte <= reference.start_byte && reference.end_byte <= span.end_byte
        })
    });
    let normalized = normalize(&remove_phrases(question, ignored));
    let corrected = !features.corrections.is_empty();
    let base = if corrected {
        correct_subject(&normalized)
    } else {
        &normalized
    };
    let routed = question::classify(base);
    let routed =
        if routed == Question::Unsupported && original == Question::Metric(Metric::FailedJobs) {
            original
        } else {
            routed
        };
    let standalone = if features.subjects.is_empty() {
        None
    } else {
        subject_question(base)
    };

    if selected.is_some() && (original == Question::FollowUp || reference_request(base)) {
        let focus = make_focus(Question::ExplainSelected, selected, constraints)
            .expect("a selected explanation has an operational focus");
        return apply_time(focus, &time, false);
    }
    if routed == Question::ChooseSubject {
        return Resolution::clarify(
            ClarificationReason::Subject,
            split_choices(base, &constraints),
            &time,
            false,
        );
    }
    if routed != Question::ExplainSelected || selected.is_some() || previous.is_none() {
        if let Some(focus) = make_focus(routed, selected, constraints.clone()) {
            return apply_time(focus, &time, false);
        }
    }
    if selected.is_none() {
        if let Some(pending) = pending {
            if let Some(mut focus) = answer_clarification(base, &time, &constraints, pending) {
                if let Some(interval) = &pending.temporal {
                    focus.temporal = Some(interval.clone());
                }
                return apply_time(focus, &time, true);
            }
        }
    }
    if let Some(focus) =
        standalone.and_then(|intent| make_focus(intent, selected, constraints.clone()))
    {
        return apply_time(focus, &time, false);
    }

    let safe_follow_up = original == Question::FollowUp
        || routed == Question::FollowUp
        || base == "what about" && time != TimeRequest::Unspecified
        || base == "what changed"
        || base.is_empty() && matches!(time, TimeRequest::Historical(_));
    let anaphora = has_reference
        && (reference_request(base)
            || routed == Question::FollowUp
            || original == Question::FollowUp
            || matches!(routed, Question::StatusChecksFollowUp(..)));
    if let Some(previous) = previous {
        if anaphora {
            if matches!(previous.intent, Intent::StatusChecks(_))
                && matches!(routed, Question::StatusChecksFollowUp(..))
            {
                // Existing readiness-tally follow-up carries its own prior counts.
                return Resolution::original(routed, selected);
            }
            if let Some(choices) = previous_choices(previous) {
                return Resolution::clarify(ClarificationReason::Subject, choices, &time, true);
            }
            if matches!(base, "what about them" | "what about those") {
                return Resolution::clarify(ClarificationReason::Subject, vec![], &time, false);
            }
        }
        if safe_follow_up || anaphora && reference_request(base) {
            let focus = with_constraints(previous.clone(), &constraints);
            return apply_time(focus, &time, true);
        }
    }
    if time == TimeRequest::Ambiguous {
        return Resolution::clarify(ClarificationReason::Time, vec![], &time, false);
    }
    if matches!(time, TimeRequest::Historical(_)) {
        return Resolution::clarify(ClarificationReason::Subject, vec![], &time, false);
    }
    if previous.is_none() && anaphora && original != Question::ExplainSelected {
        return Resolution::clarify(ClarificationReason::Subject, vec![], &time, false);
    }
    Resolution::original(original, selected)
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TimeRequest {
    Unspecified,
    Current,
    Historical(Interval),
    Ambiguous,
}

fn normalize(question: &str) -> String {
    question
        .to_ascii_lowercase()
        .replace('’', "'")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(['?', '!', '.', ',', ';', ':', ' '])
        .to_owned()
}

fn correction_prefix(text: &str) -> Option<&str> {
    ["actually", "rather", "i meant", "i mean", "no"]
        .into_iter()
        .find_map(|prefix| {
            let rest = text.strip_prefix(prefix)?;
            rest.starts_with([' ', ',', ';', ':'])
                .then(|| rest.trim_start_matches([' ', ',', ';', ':']))
        })
}

fn correct_subject(text: &str) -> &str {
    let corrected = correction_prefix(text).unwrap_or(text);
    let corrected = correction_prefix(corrected).unwrap_or(corrected);
    if let Some((discarded, replacement)) = corrected
        .strip_prefix("not ")
        .and_then(|rest| rest.split_once(',').or_else(|| rest.split_once(" but ")))
    {
        if subject_question(discarded.trim()).is_some() {
            return replacement.trim();
        }
    }
    corrected
}

fn advice_constraints(question: &str, features: &Features) -> (Vec<Span>, Vec<Constraint>) {
    let mut spans = Vec::new();
    let mut constraints = Vec::new();
    for span in &features.exclusions {
        let Some(exclusion) = span.slice(question) else {
            continue;
        };
        let exclusion = normalize(exclusion);
        let directive = exclusion.strip_prefix("please ").unwrap_or(&exclusion);
        let action = ["do not ", "don't ", "never "]
            .into_iter()
            .find_map(|prefix| directive.strip_prefix(prefix))
            .and_then(|text| text.split_whitespace().next());
        let constraint = if matches!(
            action,
            Some("restart" | "stop" | "start" | "cancel" | "retry" | "change")
        ) {
            Constraint::NoActions
        } else {
            Constraint::NoSuggestions
        };
        if !constraints.contains(&constraint) {
            constraints.push(constraint);
        }
        spans.push(*span);
    }
    (spans, constraints)
}

fn remove_phrases(question: &str, mut spans: Vec<Span>) -> String {
    spans.sort_unstable_by_key(|span| span.start_byte);
    let mut parts = Vec::new();
    let mut offset = 0;
    for span in spans {
        if let Some(text) = question.get(offset..span.start_byte) {
            parts.push(text);
        }
        offset = offset.max(span.end_byte);
    }
    parts.push(&question[offset..]);
    parts
        .into_iter()
        .map(|part| part.trim_matches(|ch: char| ch.is_whitespace() || ",;:.!?".contains(ch)))
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn extract_time(question: &str, features: &Features) -> (Option<Span>, TimeRequest) {
    let mut candidates = features
        .temporal
        .iter()
        .filter(|candidate| candidate.span.slice(question).is_some());
    let Some(candidate) = candidates.next() else {
        return (None, TimeRequest::Unspecified);
    };
    if candidates.next().is_some() {
        return (None, TimeRequest::Ambiguous);
    }
    let scope = match &candidate.scope {
        TemporalScope::Current => TimeRequest::Current,
        TemporalScope::Historical(interval) => TimeRequest::Historical(interval.clone()),
        TemporalScope::Ambiguous => TimeRequest::Ambiguous,
    };
    (Some(candidate.span), scope)
}

fn apply_time(mut focus: Focus, time: &TimeRequest, reused: bool) -> Resolution {
    match time {
        TimeRequest::Unspecified => {}
        TimeRequest::Current => focus.temporal = None,
        TimeRequest::Historical(interval) => focus.temporal = Some(interval.clone()),
        TimeRequest::Ambiguous => {
            return Resolution::clarify(ClarificationReason::Time, vec![focus], time, reused)
        }
    }
    Resolution::focused(focus, reused)
}

fn with_constraints(mut focus: Focus, constraints: &[Constraint]) -> Focus {
    for &constraint in constraints {
        if !focus.constraints.contains(&constraint) {
            focus.constraints.push(constraint);
        }
    }
    focus
}

fn answer_clarification(
    base: &str,
    time: &TimeRequest,
    constraints: &[Constraint],
    pending: &PendingClarification,
) -> Option<Focus> {
    let choice = match pending.reason {
        ClarificationReason::Time
            if matches!(base, "" | "what about" | "i mean" | "i meant")
                && !matches!(time, TimeRequest::Unspecified | TimeRequest::Ambiguous)
                && pending.choices.len() == 1 =>
        {
            pending.choices.first()
        }
        ClarificationReason::Subject if pending.choices.is_empty() => {
            return subject_question(base)
                .and_then(|intent| make_focus(intent, None, constraints.to_vec()));
        }
        ClarificationReason::Subject => {
            let index = match base {
                "first" | "the first" | "first one" | "the first one" | "former" | "the former" => {
                    0
                }
                "second" | "the second" | "second one" | "the second one" => 1,
                "third" | "the third" | "third one" | "the third one" => 2,
                "latter" | "the latter" if pending.choices.len() == 2 => 1,
                _ => {
                    let intent = subject_question(base)?;
                    let mut matching = pending.choices.iter().filter(|choice| {
                        question_for(choice.intent) == intent
                            || matches!(
                                (question_for(choice.intent), intent),
                                (Question::ReadinessExplanation(left), Question::Readiness(right))
                                    if left == right
                            )
                    });
                    let choice = matching.next()?;
                    return matching
                        .next()
                        .is_none()
                        .then(|| with_constraints(choice.clone(), constraints));
                }
            };
            pending.choices.get(index)
        }
        _ => None,
    }?;
    Some(with_constraints(choice.clone(), constraints))
}

fn split_choices(base: &str, constraints: &[Constraint]) -> Vec<Focus> {
    let mut choices = Vec::new();
    for part in base.split(" and ").take(MAX_CHOICES + 1) {
        let part = part.trim_matches([' ', ',']);
        let intent = question::classify(part);
        let intent = if intent == Question::Unsupported {
            subject_question(part)
        } else {
            Some(intent)
        };
        let Some(focus) = intent.and_then(|intent| make_focus(intent, None, constraints.to_vec()))
        else {
            return Vec::new();
        };
        if !choices
            .iter()
            .any(|choice: &Focus| choice.intent == focus.intent)
        {
            choices.push(focus);
        }
    }
    if choices.len() > MAX_CHOICES {
        choices.clear();
    }
    choices
}

fn previous_choices(previous: &Focus) -> Option<Vec<Focus>> {
    let choices: &[Question] = match previous.intent {
        Intent::HostResources => &[
            Question::Metric(Metric::Cpu),
            Question::Metric(Metric::Memory),
            Question::Metric(Metric::Disk),
        ],
        Intent::JobsSummary => &[
            Question::Metric(Metric::QueuedJobs),
            Question::Metric(Metric::RunningJobs),
            Question::Metric(Metric::RetryingJobs),
        ],
        Intent::Summary | Intent::StatusChecks(_) | Intent::RunningJobList => &[],
        _ => return None,
    };
    Some(
        choices
            .iter()
            .filter_map(|&intent| make_focus(intent, None, previous.constraints.clone()))
            .map(|mut choice| {
                choice.temporal.clone_from(&previous.temporal);
                choice
            })
            .collect(),
    )
}

fn reference_request(text: &str) -> bool {
    matches!(
        text,
        "it" | "that"
            | "what about it"
            | "what about that"
            | "what about them"
            | "what about those"
            | "check it"
            | "check that"
            | "what changed"
            | "what changed with it"
            | "how is it doing"
            | "how is it"
            | "how's it"
            | "is it still paused"
            | "is it still running"
            | "is it ready"
            | "what does this mean"
            | "what does this status mean"
            | "summarize this"
            | "explain this"
            | "tell me about this"
            | "what happened to it"
            | "what happened with it"
            | "why did it pause"
            | "why is it paused"
            | "could you explain it"
            | "explain it"
            | "explain that"
    )
}

fn subject_question(text: &str) -> Option<Question> {
    let text = text.strip_prefix("what about ").unwrap_or(text);
    let explanation = [
        "what happened to ",
        "what happened with ",
        "what changed with ",
        "what has changed with ",
        "why did ",
    ]
    .into_iter()
    .find_map(|prefix| text.strip_prefix(prefix))
    .map(|subject| subject.strip_suffix(" pause").unwrap_or(subject))
    .or_else(|| text.strip_prefix("why is ")?.strip_suffix(" paused"));
    let status_subject = text.strip_prefix("is ").and_then(|subject| {
        [
            " paused",
            " still paused",
            " ready",
            " running",
            " still running",
        ]
        .into_iter()
        .rev()
        .find_map(|suffix| subject.strip_suffix(suffix))
    });
    let subject = explanation.or(status_subject).unwrap_or(text);
    let subject = subject
        .strip_prefix("the ")
        .or_else(|| subject.strip_prefix("our "))
        .unwrap_or(subject);
    if explanation.is_none() && status_subject.is_none() {
        let metric = match subject {
            "cpu" | "processor" => Some(Metric::Cpu),
            "ram" | "memory" => Some(Metric::Memory),
            "disk" | "disk space" => Some(Metric::Disk),
            "queued jobs" => Some(Metric::QueuedJobs),
            "retrying jobs" => Some(Metric::RetryingJobs),
            "failed jobs" => Some(Metric::FailedJobs),
            _ => None,
        };
        if let Some(metric) = metric {
            return Some(Question::Metric(metric));
        }
        match subject {
            "jobs" | "job activity" => return Some(Question::JobsSummary),
            "running jobs" => return Some(Question::RunningJobList),
            "workstation" | "backend" | "readiness" => return Some(Question::Summary),
            "status checks" => return Some(Question::StatusChecks(StatusChecks::All)),
            _ => {}
        }
    }
    let check = match subject {
        "index" | "actor index" | "search index" | "actor search" | "actor search index" => {
            ReadinessCheckId::ActorSearch
        }
        "catalog" | "actor catalog" => ReadinessCheckId::ActorCatalog,
        "gateway" | "api gateway" => ReadinessCheckId::Gateway,
        "datastore" | "database" => ReadinessCheckId::Datastore,
        "jobs service" => ReadinessCheckId::Jobs,
        "brain service" => ReadinessCheckId::Brain,
        "control service" => ReadinessCheckId::Control,
        "nlp service" => ReadinessCheckId::Nlp,
        "classifier" | "classifier service" => ReadinessCheckId::Classifier,
        "classifier inference" | "atlas inference" => ReadinessCheckId::ClassifierInference,
        "classifier training" | "natural training" => ReadinessCheckId::ClassifierTraining,
        _ => return None,
    };
    Some(if explanation.is_some() {
        Question::ReadinessExplanation(check)
    } else {
        Question::Readiness(check)
    })
}

fn make_focus(
    question: Question,
    source: Option<&ChatSourceReference>,
    constraints: Vec<Constraint>,
) -> Option<Focus> {
    let (intent, goal) = match question {
        Question::ExplainSelected => (Intent::ExplainSelected, Goal::Explanation),
        Question::Readiness(check) => (Intent::Readiness(check), Goal::Status),
        Question::ReadinessExplanation(check) => {
            (Intent::ReadinessExplanation(check), Goal::Explanation)
        }
        Question::StatusChecks(selection) => {
            (Intent::StatusChecks(focus_check(selection)), Goal::Listing)
        }
        Question::Metric(metric) => (Intent::Metric(focus_metric(metric)), Goal::Measurement),
        Question::HostResources => (Intent::HostResources, Goal::Measurement),
        Question::JobsSummary => (Intent::JobsSummary, Goal::Summary),
        Question::RunningJobList => (Intent::RunningJobList, Goal::Listing),
        Question::ContextStatus(kind) => (Intent::ContextStatus(kind), Goal::Status),
        Question::ContextLogs(kind) => (Intent::ContextLogs(kind), Goal::Summary),
        Question::Configuration => (Intent::Configuration, Goal::Status),
        Question::Summary => (Intent::Summary, Goal::Summary),
        _ => return None,
    };
    Some(Focus {
        intent,
        source: source.cloned(),
        goal,
        temporal: None,
        constraints,
    })
}

pub(super) fn question_for(intent: Intent) -> Question {
    match intent {
        Intent::ExplainSelected => Question::ExplainSelected,
        Intent::Readiness(check) => Question::Readiness(check),
        Intent::ReadinessExplanation(check) => Question::ReadinessExplanation(check),
        Intent::StatusChecks(selection) => Question::StatusChecks(match selection {
            CheckSelection::All => StatusChecks::All,
            CheckSelection::Passed => StatusChecks::Passed,
            CheckSelection::NotPassed => StatusChecks::NotPassed,
            CheckSelection::Unresolved => StatusChecks::Unresolved,
            CheckSelection::Failed => StatusChecks::Failed,
        }),
        Intent::Metric(metric) => Question::Metric(match metric {
            FocusMetric::Cpu => Metric::Cpu,
            FocusMetric::Memory => Metric::Memory,
            FocusMetric::Disk => Metric::Disk,
            FocusMetric::QueuedJobs => Metric::QueuedJobs,
            FocusMetric::RunningJobs => Metric::RunningJobs,
            FocusMetric::RetryingJobs => Metric::RetryingJobs,
            FocusMetric::FailedJobs => Metric::FailedJobs,
        }),
        Intent::HostResources => Question::HostResources,
        Intent::JobsSummary => Question::JobsSummary,
        Intent::RunningJobList => Question::RunningJobList,
        Intent::ContextStatus(kind) => Question::ContextStatus(kind),
        Intent::ContextLogs(kind) => Question::ContextLogs(kind),
        Intent::Configuration => Question::Configuration,
        Intent::Summary => Question::Summary,
    }
}

fn focus_check(selection: StatusChecks) -> CheckSelection {
    match selection {
        StatusChecks::All => CheckSelection::All,
        StatusChecks::Passed => CheckSelection::Passed,
        StatusChecks::NotPassed => CheckSelection::NotPassed,
        StatusChecks::Unresolved => CheckSelection::Unresolved,
        StatusChecks::Failed => CheckSelection::Failed,
    }
}

fn focus_metric(metric: Metric) -> FocusMetric {
    match metric {
        Metric::Cpu => FocusMetric::Cpu,
        Metric::Memory => FocusMetric::Memory,
        Metric::Disk => FocusMetric::Disk,
        Metric::QueuedJobs => FocusMetric::QueuedJobs,
        Metric::RunningJobs => FocusMetric::RunningJobs,
        Metric::RetryingJobs => FocusMetric::RetryingJobs,
        Metric::FailedJobs => FocusMetric::FailedJobs,
    }
}

#[cfg(test)]
mod tests {
    use argos_osint_core::nlp::conversation::TemporalCandidate;
    use argos_osint_core::nlp::Span;
    use fleetworks_contracts::chat::{ChatSourceKind, ChatSourceReference};

    use super::{question, resolve, ClarificationReason, Constraint, Features, Interval};
    use super::{Metric, Question, ReadinessCheckId, Status, TemporalScope};

    fn span(input: &str, part: &str) -> Span {
        let start_byte = input
            .find(part)
            .expect("test span must occur in the prompt");
        Span {
            start_byte,
            end_byte: start_byte + part.len(),
        }
    }

    fn complete() -> Features {
        Features::terminal(Status::Complete)
    }

    fn interval() -> Interval {
        Interval {
            start_ms: 1,
            end_ms: 10,
            reference_ms: 9,
            timezone: "UTC".to_owned(),
        }
    }

    #[test]
    fn current_refresh_and_correction_replace_the_relevant_parts_of_focus() {
        let prompt = "What happened to the index since yesterday?";
        let mut features = complete();
        features.subjects.push(span(prompt, "index"));
        features.temporal.push(TemporalCandidate {
            span: span(prompt, "since yesterday"),
            scope: TemporalScope::Historical(interval()),
        });
        let first = resolve(
            prompt,
            None,
            question::classify(prompt),
            &features,
            None,
            None,
        );
        assert_eq!(
            first.intent,
            Question::ReadinessExplanation(ReadinessCheckId::ActorSearch)
        );
        assert_eq!(
            first.focus.as_ref().unwrap().temporal.as_ref(),
            Some(&interval())
        );
        let recheck = resolve(
            "Check again",
            None,
            Question::FollowUp,
            &complete(),
            first.focus.as_ref(),
            None,
        );
        assert!(recheck.used_previous_exchange);
        assert_eq!(recheck.focus.as_ref().unwrap().temporal, Some(interval()));

        let mut features = complete();
        features.temporal.push(TemporalCandidate {
            span: span("What about now?", "now"),
            scope: TemporalScope::Current,
        });
        let recheck = resolve(
            "What about now?",
            None,
            Question::FollowUp,
            &features,
            first.focus.as_ref(),
            None,
        );
        assert_eq!(recheck.intent, first.intent);
        assert!(recheck.used_previous_exchange);
        assert_eq!(recheck.focus.as_ref().unwrap().temporal, None);

        let prompt = "Actually, not CPU, memory";
        let mut features = complete();
        features.subjects = vec![span(prompt, "CPU"), span(prompt, "memory")];
        features.corrections.push(span(prompt, "Actually"));
        let correction = resolve(
            prompt,
            None,
            Question::Unsupported,
            &features,
            recheck.focus.as_ref(),
            None,
        );
        assert_eq!(correction.intent, Question::Metric(Metric::Memory));
        assert!(!correction.used_previous_exchange);
    }

    #[test]
    fn ambiguous_reference_requires_an_explicit_clarification_selection() {
        let initial = resolve(
            "Show CPU, memory and disk usage",
            None,
            Question::HostResources,
            &complete(),
            None,
            None,
        );
        for (prompt, reference) in [
            ("What about it?", "it"),
            ("Has that changed?", "that"),
            ("Is it still the same?", "it"),
        ] {
            let mut features = complete();
            features.references.push(span(prompt, reference));
            let ambiguous = resolve(
                prompt,
                None,
                question::classify(prompt),
                &features,
                initial.focus.as_ref(),
                None,
            );
            assert_eq!(ambiguous.intent, Question::ChooseSubject, "{prompt}");
            assert_eq!(
                ambiguous.pending.as_ref().unwrap().reason,
                ClarificationReason::Subject
            );

            let selected = resolve(
                "the second one",
                None,
                Question::Unsupported,
                &complete(),
                None,
                ambiguous.pending.as_ref(),
            );
            assert_eq!(selected.intent, Question::Metric(Metric::Memory));
            assert!(selected.used_previous_exchange);
        }
        let expired = resolve(
            "the second one",
            None,
            Question::Unsupported,
            &complete(),
            None,
            None,
        );
        assert_eq!(expired.intent, Question::Unsupported);
        assert!(expired.focus.is_none());
    }

    #[test]
    fn unknown_time_is_not_routed_to_a_current_observation() {
        let prompt = "What happened to the index recently?";
        let mut features = complete();
        features.subjects.push(span(prompt, "index"));
        features.temporal.push(TemporalCandidate {
            span: span(prompt, "recently"),
            scope: TemporalScope::Ambiguous,
        });
        let clarification = resolve(prompt, None, Question::Unsupported, &features, None, None);
        assert_eq!(clarification.intent, Question::ChooseSubject);
        assert_eq!(
            clarification.pending.as_ref().unwrap().reason,
            ClarificationReason::Time
        );

        let prompt = "Since yesterday";
        let mut features = complete();
        features.temporal.push(TemporalCandidate {
            span: span(prompt, prompt),
            scope: TemporalScope::Historical(interval()),
        });
        let selected = resolve(
            prompt,
            None,
            Question::Unsupported,
            &features,
            None,
            clarification.pending.as_ref(),
        );
        assert_eq!(
            selected.intent,
            Question::ReadinessExplanation(ReadinessCheckId::ActorSearch)
        );
        assert_eq!(selected.focus.unwrap().temporal, Some(interval()));
        assert!(selected.used_previous_exchange);
    }

    #[test]
    fn clarified_subject_keeps_the_requested_interval_until_explicitly_replaced() {
        let prompt = "What changed since yesterday?";
        let mut features = complete();
        features.temporal.push(TemporalCandidate {
            span: span(prompt, "since yesterday"),
            scope: TemporalScope::Historical(interval()),
        });
        let initial = resolve(prompt, None, Question::Unsupported, &features, None, None);
        let pending = initial.pending.as_ref().expect("subject clarification");
        assert_eq!(pending.reason, ClarificationReason::Subject);
        assert_eq!(pending.temporal, Some(interval()));

        let resolved = resolve(
            "actor search",
            None,
            Question::Unsupported,
            &complete(),
            None,
            Some(pending),
        );
        assert_eq!(
            resolved.intent,
            Question::Readiness(ReadinessCheckId::ActorSearch)
        );
        assert_eq!(resolved.focus.unwrap().temporal, Some(interval()));
        assert!(resolved.used_previous_exchange);

        let prompt = "actor search now";
        let mut features = complete();
        features.temporal.push(TemporalCandidate {
            span: span(prompt, "now"),
            scope: TemporalScope::Current,
        });
        let current = resolve(
            prompt,
            None,
            Question::Unsupported,
            &features,
            None,
            Some(pending),
        );
        assert_eq!(
            current.intent,
            Question::Readiness(ReadinessCheckId::ActorSearch)
        );
        assert_eq!(current.focus.unwrap().temporal, None);
    }

    #[test]
    fn ordinal_answer_retains_the_interval_from_multiple_subjects() {
        let prompt = "Summarize backend health and CPU usage since yesterday";
        let mut features = complete();
        features.temporal.push(TemporalCandidate {
            span: span(prompt, "since yesterday"),
            scope: TemporalScope::Historical(interval()),
        });
        let initial = resolve(prompt, None, Question::Unsupported, &features, None, None);
        let pending = initial.pending.as_ref().expect("subject clarification");
        assert_eq!(pending.choices.len(), 2);

        let resolved = resolve(
            "the second",
            None,
            Question::Unsupported,
            &complete(),
            None,
            Some(pending),
        );
        assert_eq!(resolved.intent, Question::Metric(Metric::Cpu));
        assert_eq!(resolved.focus.unwrap().temporal, Some(interval()));
        assert!(resolved.used_previous_exchange);
    }

    #[test]
    fn explicit_selection_wins_and_negated_actions_remain_constraints() {
        let prompt = "What is the current memory usage? Do not restart the backend.";
        let mut features = complete();
        features
            .exclusions
            .push(span(prompt, " Do not restart the backend"));
        let answer = resolve(prompt, None, Question::Unsupported, &features, None, None);
        let focus = answer.focus.as_ref().unwrap();
        assert_eq!(focus.constraints, vec![Constraint::NoActions]);
        assert!(!focus.allows_suggestions());

        let selected = ChatSourceReference::new(ChatSourceKind::Job, "scoped-job".into()).unwrap();
        let recheck = resolve(
            "What about now?",
            Some(&selected),
            Question::FollowUp,
            &complete(),
            answer.focus.as_ref(),
            None,
        );
        assert_eq!(recheck.intent, Question::ExplainSelected);
        assert_eq!(recheck.source, Some(selected));
        assert!(!recheck.used_previous_exchange);
    }

    #[test]
    fn leading_and_embedded_advice_constraints_keep_the_historical_request() {
        for (prompt, exclusion, constraint) in [
            (
                "Do not suggest a restart; what happened to the index since yesterday?",
                "Do not suggest a restart",
                Constraint::NoSuggestions,
            ),
            (
                "What happened to the index, never restart it, since yesterday?",
                ", never restart it",
                Constraint::NoActions,
            ),
        ] {
            let mut features = complete();
            features.subjects.push(span(prompt, "index"));
            features.exclusions.push(span(prompt, exclusion));
            features.temporal.push(TemporalCandidate {
                span: span(prompt, "since yesterday"),
                scope: TemporalScope::Historical(interval()),
            });
            let answer = resolve(
                prompt,
                None,
                question::classify(prompt),
                &features,
                None,
                None,
            );
            assert_eq!(
                answer.intent,
                Question::ReadinessExplanation(ReadinessCheckId::ActorSearch),
                "{prompt}"
            );
            let focus = answer.focus.unwrap();
            assert_eq!(focus.constraints, vec![constraint]);
            assert_eq!(focus.temporal, Some(interval()));
            assert!(!focus.allows_suggestions());
        }
    }

    #[test]
    fn unavailable_parser_preserves_the_established_route() {
        let original = Question::Metric(Metric::Cpu);
        for status in [
            Status::Disabled,
            Status::Busy,
            Status::Timeout,
            Status::Unsupported,
        ] {
            let result = resolve(
                "What is current CPU usage?",
                None,
                original,
                &Features::terminal(status),
                None,
                None,
            );
            assert_eq!(result.intent, original);
            assert!(result.focus.is_none());
            assert!(!result.used_previous_exchange);
        }
    }
}
