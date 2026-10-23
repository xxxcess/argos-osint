//! SurrealDB storage foundation (migration phases P0 and P1).
//!
//! `error` normalizes engine errors, `tx` runs bounded-retry transactions,
//! `codec` encodes compound keys, `jobs` is the job-system repository, and
//! `embeddings` writes DISKANN-indexed vectors through the retry runner,
//! `publisher` group-commits memory publications, `memory` holds the P2
//! memory write sites on top of it and `graph` projects memory payloads
//! into native graph edges in the same transaction. `recall` serves fenced
//! DISKANN recall with a lexical fallback and the recall caps.
//! Existing `Store` callers are not converted yet; see
//! `.planning/2026-10-22-surrealdb/phase1-design.md`.

pub mod error;

pub mod codec;
pub mod counters;
pub mod embeddings;
pub mod graph;
pub mod jobs;
pub mod memory;
pub mod outbox;
pub mod processes;
pub mod publisher;
pub mod recall;
pub mod tx;
