//! Article-centered Intel Recon: body retrieval, report jobs, and section synthesis.

mod body;
mod body_filter;
mod brain;
mod jobs;
mod ledger;
mod modes;
mod persist;
mod synthesize;
#[cfg(test)]
mod tests_acceptance;
mod validate;
mod worker;

pub use body::{
    body_fetch_routes, enqueue_article_body, fetch_article_body, BodyFetchEvent, EnqueueOutcome,
    FAILURE_COOLDOWN_SECS,
};
pub use body_filter::{
    chunk_article_body, merge_ranges, remove_char_ranges, strip_irrelevant_ranges, BodyChunk,
    IrrelevantRange,
};
pub use brain::{upsert_recon_insights, ReconInsightUpdate};
pub use jobs::{
    active_job_for_mode, cancel_job, create_report_job, pause_job, resume_job, retry_failed_tasks,
    scoped_section_plan, start_report_worker, IntelReportEvent, ReportScope,
};
pub use ledger::{body_assertion_candidates, coverage_complete, seed_element_ledger, ElementStatus};
pub use modes::{section_plan, ReportMode, SectionPlan};
pub use persist::{
    ArticleBodyRow, IntelAssessmentRow, IntelElementRow, IntelEvidenceRow, IntelInvestigationRow,
    IntelReportJobRow, IntelReportSectionRow, IntelReportTaskRow, RetrievalAttemptRow,
};
pub use synthesize::{
    refine_retrieved_article_body, save_section, synthesize_section, RefinedArticleBody,
    SectionJudgment, SectionSynthInput, SectionSynthOutput,
};
pub use validate::{validate_article_body, BodyQuality, BodyValidation};
pub use worker::{run_job_slice, run_job_to_completion, JobRuntime};
