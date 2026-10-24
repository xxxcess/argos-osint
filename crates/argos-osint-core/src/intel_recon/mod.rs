//! Article-centered Intel Recon: body retrieval, cleanup and briefs.

mod actor_review;
mod body;
pub mod body_db;
pub(crate) mod body_filter;
pub mod brief_canonical;
pub mod brief_context;
pub mod brief_contracts;
pub mod brief_fidelity;
pub mod brief_pipeline;
pub mod brief_stages;
pub mod brief_timing;
mod briefing_view;
mod classify_mode;
pub mod clean_body;
mod confidence;
pub(crate) mod modes;
pub mod passage_packets;
mod persist;
mod replace_insights;
pub mod sentence_index;
mod validate;
mod worker;

pub use actor_review::{
    apply_reviewed_actors, is_meaningful_actor, resolve_actor_reviewer_secret,
    review_article_actors, ActorReviewItem, ActorReviewResult,
};
pub use body::{
    body_fetch_routes, enqueue_article_body, fetch_article_body, BodyFetchEvent, EnqueueOutcome,
    FAILURE_COOLDOWN_SECS,
};
pub use body_filter::{
    chunk_article_body, merge_ranges, remove_char_ranges, strip_irrelevant_ranges, BodyChunk,
    IrrelevantRange,
};
pub use briefing_view::{
    bucket_extracted, bucket_extracted_with_explanations, explain_intel_links_with_model,
    explain_relation_link, ExtractedBuckets, ExtractedLine,
};
pub use classify_mode::{
    classifiable_modes, classify_prompt_mode, classify_prompt_mode_for_role, classify_recon_mode,
    classify_recon_mode_for_role, default_recon_mode, heuristic_prompt_mode, parse_mode_choice,
    parse_mode_from_chat, ModeClassifyInput, PromptModeClassifyInput,
};
pub(crate) use confidence::persist_article_rating;
#[cfg(feature = "surreal")]
pub(crate) use confidence::persist_article_rating_surreal;
pub use confidence::{article_confidence, initial_rating, EvidenceConfidence};
pub use modes::{
    chat_response_spec, investigation_mode_spec, section_guideline, section_plan, ReportMode,
    SectionPlan,
};
pub use persist::{
    ArticleBodyRow, IntelAssessmentRow, IntelElementRow, IntelEvidenceRow, IntelInvestigationRow,
    IntelReportJobRow, IntelReportSectionRow, IntelReportTaskRow, RetrievalAttemptRow,
};
pub use replace_insights::{replace_article_insights_from_body, ReplaceOutcome};
pub use validate::{validate_article_body, BodyQuality, BodyValidation};
pub use worker::JobRuntime;
