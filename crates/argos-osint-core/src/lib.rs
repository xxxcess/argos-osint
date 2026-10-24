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
pub mod atlas_actor_index;
pub mod atlas_classify;
#[cfg(test)]
mod atlas_classify_tests;
pub mod atlas_insights;
pub mod atlas_memory;
pub mod atlas_metadata;
pub mod atlas_recall_tests;
pub mod atlas_store;
pub mod atlas_work;
pub mod brain;
pub mod brain_actor_dedup;
pub mod brain_actors;
pub mod brain_backfill;
pub mod brain_browse;
pub mod brain_kinds;
#[cfg_attr(not(feature = "lancedb"), path = "brain_lance_off.rs")]
pub mod brain_lance;
pub mod brain_neighborhood_brief;
pub mod brain_neighborhood_brief_exec;
pub mod brain_query;
pub mod brain_schema;
pub mod brain_scope;
mod brain_source_lifecycle;
pub mod brain_topology;
pub mod config_commit;
pub mod config_transfer;
#[cfg(test)]
mod config_transfer_tests;
pub mod embed;
pub mod events;
pub mod evidence;
pub mod explore;
pub mod hardware;
pub mod intel_recon;
pub mod investigation;
pub mod iso3166;
pub mod job_cleanup;
pub mod job_registry;
pub mod jobs_view;
pub mod memory_metadata;
pub mod nlp;
pub mod osint;
pub mod paths;
pub mod pipeline;
pub mod profile_stats;
pub mod provider;
pub mod provider_adapters;
pub mod provider_attempt;
pub mod provider_capabilities;
pub mod provider_chain;
pub mod provider_diag;
pub mod provider_execution_schema;
pub mod provider_metrics;
mod provider_rate_limit;
pub mod provider_request;
pub mod recon;
pub mod related_articles;
pub mod related_global;
pub mod related_memories;
pub mod reliability_faults;
pub mod request_policy;
pub mod scheduler;
pub mod secrets;
#[cfg(feature = "surreal")]
pub mod storage;
pub mod store;
pub mod subscription;
pub mod summarization;
pub mod tasks;
pub mod telemetry;
