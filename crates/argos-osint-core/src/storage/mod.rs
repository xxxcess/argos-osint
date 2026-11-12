//! @tag:surreal-storage
//! SurrealDB storage foundation (migration phases P0 and P1).
//!
//! `error` normalizes engine errors, `tx` runs bounded-retry transactions,
//! `codec` encodes compound keys, `jobs` is the job-system repository, and
//! `embeddings` writes DISKANN-indexed vectors through the retry runner,
//! `publisher` group-commits memory publications, `memory` holds the P2
//! memory write sites on top of it, `insight` the P3 insight
//! publication sites, and `graph` projects memory payloads
//! into native graph edges in the same transaction. `recall` serves fenced
//! DISKANN recall with a lexical fallback and the recall caps.
//! Existing `Store` callers are not converted yet; see
//! `.planning/2026-10-22-surrealdb/phase1-design.md`.

pub mod brain_scope;
pub mod error;

pub mod actor_bind;
pub mod actor_catalog;
pub mod actor_dedup;
pub mod actor_dossier;
pub mod actor_search_outbox;
pub mod app_jobs;
pub mod article_body;
pub mod article_discovery;
pub mod atlas;
pub mod atlas_admission_maintenance;
pub mod atlas_delete;
pub(crate) mod atlas_derivative_invalidation;
pub mod atlas_derivative_status;
pub mod atlas_memory;
pub mod atlas_metadata;
pub mod atlas_work;
pub mod bootstrap;
pub mod brain_backfill;
pub mod brain_store;
pub mod brief;
pub mod brief_publish;
pub mod brief_stages;
pub mod browse;
pub mod browse_assoc;
pub mod classifier_activation;
pub mod classifier_admission;
pub mod classifier_calibration;
pub mod classifier_learning;
mod classifier_learning_fit;
pub mod classifier_review;
pub mod classifier_review_diagnostics;
pub mod codec;
pub mod commands;
pub mod configuration_reset;
pub mod connect_registry;
pub mod counters;
pub mod credits;
pub mod derivations;
pub mod embeddings;
pub mod entities;
pub mod event_log;
pub mod graph;
pub mod home;
pub mod index_drain;
pub mod insight;
pub mod insight_owner;
pub mod insight_retry;
pub mod insight_views;
pub mod install;
pub mod intel;
pub mod intel_brief_view;
pub mod investigation;
pub mod job_actions;
pub mod job_context;
pub mod job_dispatch;
pub mod jobs;
pub mod jobs_client;
pub mod jobs_store;
pub mod jobs_view;
pub mod lifecycle;
pub mod live_evidence;
pub mod memories;
pub mod memory;
pub(crate) mod native_visibility;
pub mod neighborhood_brief;
pub mod nlp_cache;
pub mod open;
pub mod outbox;
pub mod pipeline_exec;
pub mod pipeline_inputs;
pub mod processes;
pub mod profile_view;
pub mod provenance;
pub mod provider_account;
pub mod provider_pacer;
pub mod publisher;
pub mod recall;
pub mod recon;
pub mod recon_exec;
pub mod recon_extract;
pub mod recon_jobs;
pub mod recon_runs;
pub mod recon_threads;
pub mod refresh;
pub mod refresh_driver;
pub mod refresh_exec;
pub mod repo;
pub mod runtime;
pub mod server;
pub mod setup_jobs;
pub mod store_bridge;
pub mod summaries;
pub mod summary_flush;
pub mod telemetry;
pub mod thread_history;
pub mod tool_state;
pub mod topology;
pub mod tx;
pub mod vectors;
