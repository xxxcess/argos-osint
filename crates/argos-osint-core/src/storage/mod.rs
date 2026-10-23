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

pub mod atlas;
pub mod atlas_delete;
pub mod atlas_metadata;
pub mod bootstrap;
pub mod codec;
pub mod commands;
pub mod counters;
pub mod embeddings;
pub mod graph;
pub mod home;
pub mod insight;
pub mod install;
pub mod intel;
pub mod jobs;
pub mod lifecycle;
pub mod memory;
pub mod open;
pub mod outbox;
pub mod processes;
pub mod provenance;
pub mod publisher;
pub mod recall;
pub mod refresh;
pub mod server;
pub mod tx;
