//! P0-lite perf-time scaffolding (plan
//! `.planning/2026-10-24-rust-performance-and-scalability-audit`): one
//! topology snapshot over an N-row memory corpus, against a generous wall
//! budget, reporting the `storage::tx` round-trip debug counter. Both modes
//! via `scripts/test_surreal.py`.
#![cfg(feature = "surreal")]

mod support;

use std::time::Instant;

use argos_osint_core::brain::MemorySource;
use argos_osint_core::brain_topology::{CycleScope, TopologyOptions, TopologySeed};
use argos_osint_core::storage::tx::{query_round_trips, reset_query_round_trips};
use argos_osint_core::storage::{memories, topology};
use support::*;

/// Memories seeded before the measured section.
const CORPUS: usize = 400;
/// Wall-clock budget for one snapshot build; the real cost is far lower.
const BUDGET_MS: u128 = 30_000;

fn source() -> MemorySource {
    MemorySource {
        app: "cli".into(),
        conversation_id: "c1".into(),
        message_id: None,
        reference: None,
    }
}

#[tokio::test]
async fn topology_snapshot_round_trip_budget() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "perf_snapshot").await;
    let mut seed_id = String::new();
    for i in 0..CORPUS {
        let memory = memories::add_memory(
            &repo,
            &format!("Perf corpus fact number {i}"),
            "fact",
            false,
            source(),
            None,
        )
        .await
        .unwrap();
        if seed_id.is_empty() {
            seed_id = memory.id;
        }
    }

    let seed = TopologySeed::Claim(seed_id);
    let options = TopologyOptions {
        depth: 2,
        cycle_scope: CycleScope::AllRetained,
        ..Default::default()
    };

    reset_query_round_trips();
    let before = query_round_trips();
    let started = Instant::now();
    let snap = topology::topology_snapshot(&repo, &seed, &options)
        .await
        .unwrap();
    let elapsed_ms = started.elapsed().as_millis();
    let round_trips = query_round_trips() - before;
    println!("topology snapshot: {elapsed_ms} ms, query round trips: {round_trips}");

    assert!(
        !snap.root_id.is_empty(),
        "the seeded corpus memory resolves to a root"
    );
    assert!(
        elapsed_ms < BUDGET_MS,
        "snapshot took {elapsed_ms} ms over a {BUDGET_MS} ms budget"
    );
    assert!(
        round_trips >= 2,
        "round-trip counter saw only attempt granularity ({round_trips}); \
         statement batches through jobs::run/topology::stmt are uncounted"
    );
}
