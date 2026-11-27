//! Decision scoring boundary for authorized, bounded activity candidates.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use argos_osint_core::nlp::conversation::ClarificationReason;
use argos_osint_core::storage::inbox_qa::FrozenEvidence;
use argos_osint_core::storage::inbox_qa::PlanContext;
use argos_osint_core::storage::inbox_qa::ResponseMode;
use argos_osint_core::storage::inbox_qa::ResponsePlan;
use argos_osint_core::storage::inbox_qa::ResponseTask;
use fleetworks_contracts::chat::QaEvidenceProvenance;

use fleetworks_contracts::chat::QaDecisionAvailabilityReason;
use fleetworks_contracts::chat::QaDecisionPreference;
use fleetworks_contracts::chat::QaDecisionSettings;
use fleetworks_contracts::chat::QaPhase;
use fleetworks_contracts::qa_worker::QaClarification;
use fleetworks_contracts::qa_worker::QaDialogueKind;
use fleetworks_contracts::qa_worker::QaWorkerPlan;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecisionTask {
    RelatedActivity,
    ResponsePlan,
}

/// Only fixed domain descriptions cross the decision worker boundary. The
/// caller keeps the mapping from menu positions to authorized job references.
pub struct DecisionMenu {
    pub task: DecisionTask,
    pub state: String,
    pub candidates: Vec<String>,
}

/// A verified selection or the scorer's explicit abstention.
pub enum DecisionChoice {
    Candidate(usize),
    Abstain,
    Unavailable,
}

pub type DecisionFuture<'a> = Pin<Box<dyn Future<Output = DecisionChoice> + Send + 'a>>;

/// The Embedded host controls qualification, worker lifetime and resource admission.
pub trait DecisionRuntime: Send + Sync {
    fn status(&self, preference: QaDecisionPreference, generation: i64) -> QaDecisionSettings;

    fn score(
        &self,
        preference: QaDecisionPreference,
        generation: i64,
        menu: DecisionMenu,
    ) -> DecisionFuture<'_>;
}

/// Rules stay in effect until an eligible worker reports readiness.
#[must_use]
pub fn rules_status(
    preference: QaDecisionPreference,
    reason: Option<QaDecisionAvailabilityReason>,
) -> QaDecisionSettings {
    let phase = if preference == QaDecisionPreference::RulesOnly {
        QaPhase::Ready
    } else {
        QaPhase::Unavailable
    };
    QaDecisionSettings::new(preference, QaDecisionPreference::RulesOnly, phase, reason)
}

/// Select from the frozen server-owned facts. The rules plan covers every mode.
#[must_use]
pub fn response_plan(evidence: &FrozenEvidence, context: &PlanContext) -> ResponsePlan {
    let dialogue_kind = evidence
        .request
        .dialogue
        .as_ref()
        .map(|dialogue| dialogue.kind);
    let operational = dialogue_kind == Some(QaDialogueKind::Operational);
    let contextual = dialogue_kind == Some(QaDialogueKind::Contextual);
    let clarify = dialogue_kind == Some(QaDialogueKind::Clarification);
    let historical = context.history_requested;
    let recheck = evidence
        .sources
        .iter()
        .any(|source| source.provenance == Some(QaEvidenceProvenance::PreviousObservation));
    let mode = if clarify {
        ResponseMode::Clarification
    } else if historical {
        ResponseMode::Historical
    } else if recheck {
        ResponseMode::Recheck
    } else if operational || contextual {
        ResponseMode::Current
    } else {
        ResponseMode::Conversation
    };
    let guidance = context.guidance.filter(|guidance| {
        let words = evidence
            .request
            .passages
            .iter()
            .take(3)
            .map(|p| p.text.split_whitespace().count())
            .sum::<usize>();
        operational
            && words + guidance.text().split_whitespace().count()
                <= fleetworks_contracts::chat::QA_MAX_WORDS
            && context
                .focus
                .as_ref()
                .is_none_or(|focus| focus.allows_suggestions())
    });
    let note = match (mode, guidance.is_some()) {
        _ if contextual => "Explain the general part of the question.",
        (ResponseMode::Historical, true) => {
            "Explain the recorded observations and offer the approved check."
        }
        (ResponseMode::Historical, false) => {
            "Explain the recorded observations within their stated coverage."
        }
        (ResponseMode::Recheck, true) => {
            "Compare the earlier and current observations, then offer the approved check."
        }
        (ResponseMode::Recheck, false) => "Compare the earlier and current observations in order.",
        (ResponseMode::Current, true) => {
            "Explain the current observation and offer the approved check."
        }
        (ResponseMode::Current, false) => {
            "Answer using the most relevant verified current observations."
        }
        (ResponseMode::Clarification, _) if historical => {
            "Explain that the requested local history is unavailable."
        }
        (ResponseMode::Clarification, _) => "Ask for the missing focus before answering.",
        (ResponseMode::Conversation, _) => match dialogue_kind {
            Some(QaDialogueKind::Greeting) => "Greet the operator briefly.",
            Some(QaDialogueKind::Acknowledgement) => "Acknowledge the operator briefly.",
            Some(QaDialogueKind::Help) => "Describe general questions and the workstation observations you can help interpret.",
            Some(QaDialogueKind::Recall) => "Answer from the authorized earlier exchanges.",
            Some(QaDialogueKind::General) if context.inbox_notifications.is_some() => {
                "Use the client-reported summary for this Inbox. Its notification counts describe the client view."
            }
            Some(QaDialogueKind::General) => "Answer the question using your knowledge.",
            _ => "Respond briefly to the current conversational request.",
        },
    }
    .to_owned();
    let ordered_passages = evidence
        .request
        .passages
        .iter()
        .take(3)
        .map(|passage| passage.source_id.clone())
        .collect::<Vec<_>>();
    let required = if contextual
        || recheck
        || historical
        || evidence.sources.iter().any(|source| !source.whole_passage)
    {
        ordered_passages.len()
    } else {
        usize::from(operational)
    };
    let focus = context.worker_focus();
    let constrained = context
        .focus
        .as_ref()
        .is_some_and(|focus| !focus.constraints.is_empty());
    let needs_time_clarification = clarify
        && context.pending.as_ref().is_some_and(|pending| {
            pending.reason == ClarificationReason::Time || pending.temporal.is_some()
        });
    let clarification = clarify.then(|| {
        if historical {
            QaClarification::HistoryUnavailable
        } else {
            match context.pending.as_ref().map(|pending| pending.reason) {
                Some(ClarificationReason::Time) => QaClarification::Time,
                _ => QaClarification::Subject,
            }
        }
    });
    let worker = dialogue_kind.map(|_| {
        let mut worker = QaWorkerPlan::new(
            focus,
            note.clone(),
            ordered_passages[..required].to_vec(),
            guidance,
        );
        worker.clarification = clarification;
        worker
    });
    ResponsePlan {
        task: ResponseTask::ResponsePlan,
        version: 1,
        snapshot: evidence.binding().unwrap_or_default(),
        decision_generation: context.decision_generation,
        mode,
        focus: context.focus.clone(),
        clarification: context.pending.clone(),
        ordered_passages,
        worker,
        requires_worker_support: historical
            || needs_time_clarification
            || operational && constrained
            || matches!(
                dialogue_kind,
                Some(QaDialogueKind::General | QaDialogueKind::Contextual)
            ),
        negotiated: false,
        note,
    }
}

/// Give a qualified scorer a bounded choice among prevalidated server plans.
pub async fn select_response_plan(
    runtime: Option<&dyn DecisionRuntime>,
    evidence: &FrozenEvidence,
    context: &PlanContext,
) -> ResponsePlan {
    let baseline = response_plan(evidence, context);
    // Both choices preserve the required facts. The second option only offers
    // the approved follow-up if the request permits one.
    let Some((runtime, with_guidance)) = runtime
        .zip(baseline.worker.as_ref())
        .filter(|(_, worker)| worker.guidance.is_some())
    else {
        return baseline;
    };
    if context.decision_preference == QaDecisionPreference::RulesOnly {
        return baseline;
    }
    let menu = DecisionMenu {
        task: DecisionTask::ResponsePlan,
        state: "Choose how to present the required observations.".to_owned(),
        candidates: vec![
            "Explain the recorded facts and offer the approved check.".to_owned(),
            "Explain the recorded facts only.".to_owned(),
        ],
    };
    let Ok(DecisionChoice::Candidate(1)) = tokio::time::timeout(
        Duration::from_millis(75),
        runtime.score(
            context.decision_preference,
            context.decision_generation,
            menu,
        ),
    )
    .await
    else {
        return baseline;
    };
    let mut plan = baseline.clone();
    plan.note = match plan.mode {
        ResponseMode::Recheck => "Compare the earlier and current observations in order.",
        ResponseMode::Historical => {
            "Explain the recorded observations within their stated coverage."
        }
        _ => "Answer using the most relevant verified current observations.",
    }
    .to_owned();
    plan.worker = Some(QaWorkerPlan::new(
        with_guidance.focus.clone(),
        plan.note.clone(),
        with_guidance.required_passages.clone(),
        None,
    ));
    plan
}

#[cfg(test)]
mod tests {
    use argos_osint_core::nlp::conversation::PendingClarification;
    use fleetworks_contracts::qa_worker::QaWorkerDialogue;
    use fleetworks_contracts::qa_worker::QaWorkerRequest;

    use super::*;

    #[test]
    fn ambiguous_time_requires_a_worker_that_understands_time_clarification() {
        let mut request = QaWorkerRequest::new(
            "turn".into(),
            "snapshot".into(),
            "What happened recently?".into(),
            Vec::new(),
        );
        request.dialogue = Some(QaWorkerDialogue {
            kind: QaDialogueKind::Clarification,
            history: Vec::new(),
        });
        let evidence = FrozenEvidence {
            request,
            sources: Vec::new(),
            sampled_at_ms: 1,
        };
        let mut context = PlanContext {
            focus: None,
            pending: Some(PendingClarification {
                reason: ClarificationReason::Time,
                choices: Vec::new(),
                temporal: None,
            }),
            guidance: None,
            history_requested: false,
            inbox_notifications: None,
            decision_preference: QaDecisionPreference::RulesOnly,
            decision_generation: 1,
        };
        let plan = response_plan(&evidence, &context);
        assert!(plan.requires_worker_support);
        assert_eq!(
            plan.worker.unwrap().clarification,
            Some(QaClarification::Time)
        );

        context.pending.as_mut().unwrap().reason = ClarificationReason::Subject;
        let plan = response_plan(&evidence, &context);
        assert!(!plan.requires_worker_support);
        assert_eq!(
            plan.worker.unwrap().clarification,
            Some(QaClarification::Subject)
        );
    }
}
