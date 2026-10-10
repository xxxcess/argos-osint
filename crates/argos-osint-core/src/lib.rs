//! Shared services for Argos apps: memory recall, providers, and hardware.

#![allow(
    dead_code,
    clippy::nonminimal_bool,
    clippy::too_many_arguments,
    clippy::single_match,
    clippy::if_same_then_else,
    clippy::type_complexity,
    clippy::redundant_closure,
    clippy::bool_comparison,
    clippy::cloned_ref_to_slice_refs,
    clippy::needless_option_as_deref,
    clippy::empty_line_after_doc_comments
)]

pub mod atlas;
pub mod atlas_insights;
pub mod atlas_memory;
pub mod atlas_recall_tests;
pub mod atlas_work;
pub mod brain;
#[cfg_attr(not(feature = "lancedb"), path = "brain_lance_off.rs")]
pub mod brain_lance;
pub mod brain_query;
pub mod config_commit;
pub mod config_transfer;
#[cfg(test)]
mod config_transfer_tests;
pub mod embed;
pub mod events;
pub mod evidence;
pub mod explore;
pub mod graph_explanation;
pub mod grok_oauth;
pub mod hardware;
pub mod intel_recon;
pub mod investigation;
pub mod iso3166;
pub mod job_registry;
pub mod jobs_view;
pub mod memory_metadata;
pub mod osint;
pub mod paths;
pub mod pipeline;
pub mod provider;
pub mod provider_attempt;
pub mod provider_chain;
pub mod provider_diag;
pub mod provider_request;
pub mod recon;
pub mod related_memories;
pub mod reliability_faults;
pub mod scheduler;
pub mod secrets;
pub mod store;
pub mod subscription;
pub mod summarization;
pub mod tasks;
