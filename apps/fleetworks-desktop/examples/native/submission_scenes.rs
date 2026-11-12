//! Synthetic responses delivered through the production submission handlers.

use fleetworks_contracts::context::JobOrigin;
use fleetworks_contracts::context::JobRequestAssociation;
use fleetworks_contracts::context::OriginSource;
use fleetworks_desktop::app::Intent;
use fleetworks_desktop::backend::ActivityJob;
use fleetworks_desktop::backend::JobDetail;
use fleetworks_desktop::submission::AcceptedReceipt;
use fleetworks_desktop::submission::SubmissionAttempt;
use fleetworks_desktop::submission::SubmissionFailure;
use fleetworks_desktop::submission::SubmissionKind;
use fleetworks_desktop::submission::ToolReceipt;
use fleetworks_desktop::submission::ToolResultView;
use fleetworks_desktop::submission::ToolStatusView;
use fleetworks_desktop::Page;
use fleetworks_desktop::State;

fn setup(state: &mut State, kind: SubmissionKind) {
    super::registered(state);
    super::profile_ready(state);
    if kind == SubmissionKind::Tool {
        super::tools_ready(state);
    }
    if let Some(capabilities) = &mut state.connection.capabilities {
        capabilities
            .features
            .extend(["atlas-cycles".into(), "tool-execution".into()]);
    }
    state.page = match kind {
        SubmissionKind::Atlas => Page::Atlas,
        SubmissionKind::Tool => Page::Tools,
    };
    state.backend.tool_inputs = rune::TextEdit::new("{\"email\":\"fixture@example.test\"}");
}

pub fn atlas_preview(state: &mut State) {
    setup(state, SubmissionKind::Atlas);
    state.preview_atlas_cycle();
}
pub fn atlas_accepted(state: &mut State) {
    atlas_preview(state);
    state.confirm_submission(SubmissionKind::Atlas);
}
pub fn tools_preview(state: &mut State) {
    setup(state, SubmissionKind::Tool);
    state.preview_tool_run();
}
pub fn tools_result(state: &mut State) {
    tools_preview(state);
    state.confirm_submission(SubmissionKind::Tool);
    state.refresh_tool_result();
}

pub fn atlas_uncertain(state: &mut State) {
    atlas_preview(state);
    state.fixture_effect = Some(uncertain_effect);
    state.confirm_submission(SubmissionKind::Atlas);
}

pub fn atlas_resolved(state: &mut State) {
    atlas_uncertain(state);
    state.resolve_submission(SubmissionKind::Atlas);
}

pub fn tools_shared(state: &mut State) {
    tools_preview(state);
    state.confirm_submission(SubmissionKind::Tool);
    let lane = state.submissions.lane(SubmissionKind::Tool);
    let revision = lane.revision();
    let id = lane
        .job_candidate()
        .expect("reported or resolved fixture job")
        .to_owned();
    state.complete_submission_verification(
        SubmissionKind::Tool,
        revision,
        Ok(detail(&id, SubmissionKind::Tool, false)),
    );
}

pub fn tools_result_error(state: &mut State) {
    tools_preview(state);
    state.confirm_submission(SubmissionKind::Tool);
    state.fixture_effect = Some(result_error_effect);
    state.refresh_tool_result();
}

fn uncertain_effect(state: &mut State, intent: &Intent) {
    if let Intent::PostInteractive { kind } = intent {
        let revision = state.submissions.lane(*kind).revision();
        state.complete_submission_post(
            *kind,
            revision,
            Err(SubmissionFailure::Uncertain(
                "Synthetic response lost after delivery".into(),
            )),
        );
    } else {
        super::reply(state, intent);
    }
}

fn result_error_effect(state: &mut State, intent: &Intent) {
    if matches!(intent, Intent::InteractiveToolResult) {
        let revision = state.submissions.tools.revision();
        state.complete_interactive_tool_result(
            revision,
            Err("Synthetic result read unavailable".into()),
        );
    } else {
        super::reply(state, intent);
    }
}

fn detail(id: &str, kind: SubmissionKind, private: bool) -> JobDetail {
    JobDetail {
        job: ActivityJob {
            id: id.into(),
            app: "synthetic".into(),
            title: "Synthetic reported backend job".into(),
            operation: kind.operation().into(),
            state: "queued".into(),
            phase: "queued".into(),
            progress_done: None,
            progress_total: None,
            error_summary: String::new(),
            private_details_available: private,
            activity_actions_available: false,
            status_only: !private,
            can_cancel: false,
            can_retry: false,
            action_reason_code: "not_observed".into(),
        },
        maintenance_result: None,
        stages: Vec::new(),
        private_details_available: private,
        activity_actions_available: false,
    }
}

pub fn reply(state: &mut State, intent: &Intent) -> bool {
    match intent {
        Intent::SubmitInteractive { kind } => {
            let lane = state.submissions.lane(*kind);
            let revision = lane.revision();
            let snapshot = lane
                .confirmation
                .clone()
                .expect("shared confirmation handler snapshot");
            let attempt = SubmissionAttempt::new("synthetic-client-request".into(), snapshot)
                .expect("synthetic request identity");
            state.complete_submission_preparation(*kind, revision, Ok(attempt));
        }
        Intent::PostInteractive { kind } => {
            let revision = state.submissions.lane(*kind).revision();
            let receipt = match kind {
                SubmissionKind::Atlas => AcceptedReceipt::Atlas(serde_json::from_value(serde_json::json!({"job_id":"opaque-server-atlas-job","run_id":"opaque-server-run"})).expect("synthetic server receipt")),
                SubmissionKind::Tool => AcceptedReceipt::Tool(ToolReceipt { call_id: "opaque-server-tool-job".into(), job_id: Some("opaque-server-tool-job".into()) }),
            };
            state.complete_submission_post(*kind, revision, Ok(receipt));
        }
        Intent::VerifyInteractive { kind } => {
            let lane = state.submissions.lane(*kind);
            let revision = lane.revision();
            let id = lane
                .job_candidate()
                .expect("reported or resolved fixture job")
                .to_owned();
            state.complete_submission_verification(*kind, revision, Ok(detail(&id, *kind, true)));
        }
        Intent::InteractiveToolResult => {
            let revision = state.submissions.tools.revision();
            state.complete_interactive_tool_result(
                revision,
                Ok(ToolStatusView {
                    status: "succeeded".into(),
                    result: Some(ToolResultView {
                        tool_id: "synthetic-tool".into(),
                        status: "ok".into(),
                        observed_at: "2026-01-01T00:00:00Z".into(),
                        observations:
                            "{\"synthetic_observation\":\"Controlled native fixture only\"}".into(),
                        error: None,
                        cached: false,
                        truncated: false,
                        source: Some("https://example.test/synthetic-evidence".into()),
                    }),
                }),
            );
        }
        Intent::ResolveInteractive { kind } => {
            let lane = state.submissions.lane(*kind);
            let revision = lane.revision();
            let attempt = lane.attempt.as_ref().expect("original synthetic request");
            let current = state
                .registration
                .context
                .as_ref()
                .expect("current synthetic proof");
            let result = if current.workspace_id == attempt.snapshot.context.workspace_id
                && current.workstation_id == attempt.snapshot.context.workstation_id
            {
                let origin = JobOrigin::new(
                    &attempt.snapshot.context,
                    OriginSource::Native,
                    attempt.request_id.clone(),
                    "2026-01-01T00:00:00Z".into(),
                    attempt.snapshot.configuration_revision,
                    Vec::new(),
                );
                let id = match kind {
                    SubmissionKind::Atlas => "opaque-server-atlas-job",
                    SubmissionKind::Tool => "opaque-server-tool-job",
                };
                Ok(JobRequestAssociation::new(
                    id.into(),
                    origin,
                    "2026-01-01T00:00:00Z".into(),
                ))
            } else {
                Err("Synthetic lookup requires the original selected workspace".into())
            };
            state.complete_submission_resolution(*kind, revision, result);
        }
        _ => return false,
    }
    true
}
