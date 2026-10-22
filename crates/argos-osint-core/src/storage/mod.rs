//! SurrealDB storage foundation (migration phases P0 and P1).
//!
//! `error` normalizes engine errors, `tx` runs bounded-retry transactions,
//! and `codec` encodes compound keys. Existing `Store` callers are not
//! converted yet; see `.planning/2026-10-22-surrealdb/phase1-design.md`.

pub mod error;

pub mod codec;
pub mod tx;
