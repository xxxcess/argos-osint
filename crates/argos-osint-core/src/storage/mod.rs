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

pub mod error;

pub mod actor_bind;
pub mod atlas;
pub mod atlas_delete;
pub mod atlas_metadata;
pub mod atlas_work;
pub mod bootstrap;
pub mod brief;
pub mod browse;
pub mod browse_assoc;
pub mod codec;
pub mod commands;
pub mod counters;
pub mod credits;
pub mod derivations;
pub mod embeddings;
pub mod entities;
pub mod event_log;
pub mod graph;
pub mod home;
pub mod insight;
pub mod insight_owner;
pub mod insight_views;
pub mod install;
pub mod intel;
pub mod investigation;
pub mod jobs;
pub mod jobs_view;
pub mod lifecycle;
pub mod live_evidence;
pub mod memories;
pub mod memory;
pub mod open;
pub mod outbox;
pub mod pipeline_exec;
pub mod pipeline_inputs;
pub mod processes;
pub mod profile_view;
pub mod provenance;
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
pub mod summaries;
pub mod telemetry;
pub mod topology;
pub mod tx;
pub mod vectors;
