//! Typed presentation roles for a Recon turn. The durable run, message, and
//! call records remain authoritative; `ChatBlock` is the current layout adapter.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvestigationPart {
    UserQuery,
    Plan,
    PlanDiagnostics,
    ToolActivity,
    Status,
    StreamingSynthesis,
    Synthesis,
    DirectiveAssessment,
    TurnSummary,
}
