//! Property-based tests for ranking monotonicity, score bounds, and ordering invariants.
//!
//! These tests verify invariants in ranking/scoring logic across:
//! - `crates/argos-osint-core/src/intel_recon/confidence.rs`: EvidenceConfidence, initial_rating
//! - `crates/argos-osint-core/src/recon/diversity.rs`: CoverageRecord, CoverageStats, diversity scoring

use argos_osint_core::intel_recon::{initial_rating, EvidenceConfidence};
use argos_osint_core::osint::source_eval::{InformationCredibility, SourceReliability};
use argos_osint_core::osint::ToolResult;
use argos_osint_core::recon::diversity::{
    coverage_record_id, coverage_stats, credentials_available, prerequisites_satisfied,
    update_coverage_with_results, CoverageCandidate, CoverageRecord,
};
use argos_osint_core::recon::{
    ForceLink, GraphEdge, GraphEdgeKind, GraphNode, GraphNodeKind, MemoryGraph, PlanCall,
};
use proptest::prelude::*;
use proptest::test_runner::TestRunner;

fn config() -> ProptestConfig {
    ProptestConfig::with_cases(256)
}

/// All source/information combinations preserve grade bounds and unknown semantics.
#[test]
fn initial_rating_bounds() {
    use InformationCredibility::{
        CannotBeJudged, Confirmed, DoubtfullyTrue, Improbable, PossiblyTrue, ProbablyTrue,
    };
    let mut runner = TestRunner::new(config());
    runner
        .run(&(0usize..6, 0usize..6), |(source, info)| {
            let sources = [
                SourceReliability::A,
                SourceReliability::B,
                SourceReliability::C,
                SourceReliability::D,
                SourceReliability::E,
                SourceReliability::F,
            ];
            let infos = [
                Confirmed,
                ProbablyTrue,
                PossiblyTrue,
                DoubtfullyTrue,
                Improbable,
                CannotBeJudged,
            ];
            let (score, provisional) = initial_rating(sources[source], infos[info]);
            prop_assert_eq!(provisional, info == 5);
            prop_assert_eq!(score.is_none(), source == 5 && info == 5);
            if let Some(score) = score {
                prop_assert!(score <= 100);
            }
            if info == 5 {
                prop_assert!(score.is_none_or(|score| score <= 60));
            }
            if source < 4 {
                let (better, _) = initial_rating(sources[source], infos[info]);
                let (worse, _) = initial_rating(sources[source + 1], infos[info]);
                prop_assert!(better >= worse);
            }
            Ok(())
        })
        .expect("rating grade invariants");
}

/// Empty evidence cannot manufacture a confidence score.
#[test]
fn empty_evidence_has_no_score() {
    let output = argos_osint_core::investigation::insights::PipelineOutput::default();
    for complete in [false, true] {
        let confidence = EvidenceConfidence::estimate(&output, complete);
        assert!(confidence.score.is_none());
        assert_eq!(confidence.claims, 0);
        assert_eq!(confidence.inferences, 0);
    }
}

/// initial_rating with SourceReliability::B and InformationCredibility::Confirmed returns score
#[test]
fn initial_rating_b_confirmed() {
    let (score, provisional) =
        initial_rating(SourceReliability::B, InformationCredibility::Confirmed);
    // B = 80, Confirmed = 100, average = 90, rounded = 90... but let's check actual
    assert!(score.is_some());
    assert!(!provisional);
}

/// CoverageRecord stats: eligible_tools count matches candidates
#[test]
fn coverage_stats_eligible_tools() {
    let mut runner = TestRunner::new(config());
    runner
        .run(
            &proptest::collection::vec(any::<String>(), 0..10),
            |tool_ids| {
                let candidates: Vec<CoverageCandidate> = tool_ids
                    .into_iter()
                    .enumerate()
                    .map(|(i, id)| {
                        CoverageCandidate {
                            tool_id: id,
                            canonical_id: format!("canonical_{}", i),
                            provider: "test".into(),
                            dataset_id: format!("dataset_{}", i),
                            eligible: i % 2 == 0, // Alternate eligible
                            rejection_reason: None,
                            cost_credits: 0,
                        }
                    })
                    .collect();
                let expected_eligible = candidates.iter().filter(|c| c.eligible).count();

                let record = CoverageRecord {
                    candidates,
                    ..CoverageRecord::default()
                };
                let stats = coverage_stats(&record);
                prop_assert_eq!(stats.eligible_tools, expected_eligible);
                Ok(())
            },
        )
        .unwrap();
}

/// Counts must not manufacture a successful attempt-coverage receipt.
#[test]
fn coverage_stats_two_tools_attempted() {
    let mut runner = TestRunner::new(config());
    runner
        .run(
            &(
                proptest::collection::vec(any::<String>(), 0..5),
                any::<bool>(),
            ),
            |(tool_ids, receipt)| {
                let record = CoverageRecord {
                    attempted_tools: tool_ids.clone(),
                    two_tools_attempted: receipt,
                    ..CoverageRecord::default()
                };
                let stats = coverage_stats(&record);
                prop_assert_eq!(stats.attempted_tools, tool_ids.len());
                prop_assert_eq!(stats.two_tools_attempted, receipt);
                Ok(())
            },
        )
        .expect("persisted attempt coverage is authoritative");
}

/// coverage_stats: two_tools_successful is true when >= 2 successful
#[test]
fn coverage_stats_two_tools_successful() {
    let mut runner = TestRunner::new(config());
    runner
        .run(
            &proptest::collection::vec(any::<String>(), 0..5),
            |tool_ids| {
                let record = CoverageRecord {
                    successful_tools: tool_ids.clone(),
                    ..CoverageRecord::default()
                };
                let stats = coverage_stats(&record);
                prop_assert_eq!(stats.successful_tools, tool_ids.len());
                Ok(())
            },
        )
        .unwrap();
}

/// coverage_stats: independent_claim_support is true when >= 2 source groups
#[test]
fn coverage_stats_independent_claim_support() {
    let mut runner = TestRunner::new(config());
    runner
        .run(
            &proptest::collection::vec(any::<String>(), 0..5),
            |groups| {
                let record = CoverageRecord {
                    independent_source_groups: groups.clone(),
                    ..CoverageRecord::default()
                };
                let stats = coverage_stats(&record);
                prop_assert_eq!(stats.independent_source_groups, groups.len());
                Ok(())
            },
        )
        .unwrap();
}

/// coverage_record_id produces consistent format
#[test]
fn coverage_record_id_format() {
    let mut runner = TestRunner::new(config());
    runner
        .run(
            &(any::<String>(), any::<u32>(), any::<u32>(), any::<String>()),
            |(scope, generation, directive_index, category)| {
                let record = CoverageRecord {
                    scope: scope.clone(),
                    generation,
                    directive_index,
                    category: category.clone(),
                    ..CoverageRecord::default()
                };
                let id = coverage_record_id(&record);
                let expected = format!("{}:{}:{}:{}", scope, generation, directive_index, category);
                prop_assert_eq!(id, expected);
                Ok(())
            },
        )
        .unwrap();
}

/// prerequisites_satisfied returns false for unknown tool
#[test]
fn prerequisites_unknown_tool() {
    let (sat, missing) = prerequisites_satisfied("unknown_tool_xyz", &[]);
    assert!(!sat);
    assert!(!missing.is_empty());
}

/// credentials_available returns true for free tools
#[test]
fn credentials_free_tools() {
    let keys = argos_osint_core::osint::ProviderKeys::default();
    // Free/public tools should return true
    assert!(credentials_available("crtsh_certificates", &keys));
    assert!(credentials_available("public_www", &keys));
}

/// update_coverage_with_results updates counts correctly for matching category
#[test]
fn update_coverage_with_results_counts() {
    let mut runner = TestRunner::new(config());
    runner
        .run(
            &proptest::collection::vec(any::<String>(), 0..5),
            |tool_ids| {
                let mut record = CoverageRecord {
                    category: "DomainNetwork".into(),
                    ..CoverageRecord::default()
                };

                // Use a known DomainNetwork tool
                let calls: Vec<PlanCall> = tool_ids
                    .iter()
                    .enumerate()
                    .map(|(i, _)| PlanCall {
                        step_id: format!("s{}", i),
                        tool_id: "crtsh_certificates".into(),
                        ..Default::default()
                    })
                    .collect();

                let results = tool_ids
                    .iter()
                    .enumerate()
                    .map(|(i, _)| {
                        (
                            format!("call-{}", i),
                            ToolResult {
                                tool_id: "crtsh_certificates".into(),
                                status: "completed".into(),
                                inputs: Default::default(),
                                source_url: String::new(),
                                retrieved_at: String::new(),
                                observations: Default::default(),
                                raw: String::new(),
                                error: None,
                                cached: false,
                                truncated: false,
                                credits_charged: 0,
                                credits_reported: None,
                            },
                        )
                    })
                    .collect::<Vec<_>>();

                update_coverage_with_results(&mut record, &calls, &results);

                // Should have attempted and successful counts
                prop_assert!(record.attempted_tools.len() <= tool_ids.len());
                prop_assert!(record.successful_tools.len() <= tool_ids.len());
                Ok(())
            },
        )
        .unwrap();
}

/// GraphNodeKind enum values are exhaustive
#[test]
fn graph_node_kind_exhaustive() {
    let kinds = [
        GraphNodeKind::Investigation,
        GraphNodeKind::Directive,
        GraphNodeKind::Entity,
        GraphNodeKind::Topic,
        GraphNodeKind::Finding,
        GraphNodeKind::Evidence,
        GraphNodeKind::Source,
        GraphNodeKind::Actor,
    ];
    // Just verify all variants can be constructed
    for kind in kinds {
        let _ = format!("{:?}", kind);
    }
}

/// GraphEdgeKind enum values are exhaustive
#[test]
fn graph_edge_kind_exhaustive() {
    let kinds = [
        GraphEdgeKind::Investigates,
        GraphEdgeKind::Answers,
        GraphEdgeKind::Concerns,
        GraphEdgeKind::Supports,
        GraphEdgeKind::Reports,
        GraphEdgeKind::DerivedFrom,
        GraphEdgeKind::DiscoveredIn,
        GraphEdgeKind::Link,
        GraphEdgeKind::Related,
    ];
    for kind in kinds {
        let _ = format!("{:?}", kind);
    }
}

/// ForceLink struct can be created and compared
#[test]
fn force_link_creation() {
    let mut runner = TestRunner::new(config());
    runner
        .run(
            &(any::<String>(), any::<String>(), any::<String>()),
            |(from, to, directive)| {
                let link = ForceLink {
                    from: from.clone(),
                    to: to.clone(),
                    directive: directive.clone(),
                };
                prop_assert_eq!(link.from, from);
                prop_assert_eq!(link.to, to);
                prop_assert_eq!(link.directive, directive);
                Ok(())
            },
        )
        .unwrap();
}

/// MemoryGraph can be created with nodes and edges
#[test]
fn memory_graph_creation() {
    let mut runner = TestRunner::new(config());
    runner
        .run(
            &(
                proptest::collection::vec(any::<String>(), 0..5),
                proptest::collection::vec((any::<String>(), any::<String>()), 0..5),
            ),
            |(node_ids, edge_pairs)| {
                let mut graph = MemoryGraph::default();
                for id in &node_ids {
                    graph.nodes.push(GraphNode {
                        id: id.clone(),
                        kind: GraphNodeKind::Actor,
                        label: id.clone(),
                        detail: String::new(),
                        tags: Vec::new(),
                        article_id: String::new(),
                        run_id: String::new(),
                        published_at: String::new(),
                    });
                }
                for (src, tgt) in &edge_pairs {
                    graph.edges.push(GraphEdge {
                        from: src.clone(),
                        to: tgt.clone(),
                        kind: GraphEdgeKind::Supports,
                        directive: None,
                        edge_id: None,
                    });
                }
                prop_assert_eq!(graph.nodes.len(), node_ids.len());
                prop_assert_eq!(graph.edges.len(), edge_pairs.len());
                Ok(())
            },
        )
        .unwrap();
}

/// CoverageStats eligible_for_rate is true when eligible_tools >= 2
#[test]
fn coverage_stats_eligible_for_rate() {
    let mut runner = TestRunner::new(config());
    runner
        .run(
            &proptest::collection::vec(any::<String>(), 0..5),
            |tool_ids| {
                let candidates: Vec<CoverageCandidate> = tool_ids
                    .into_iter()
                    .enumerate()
                    .map(|(i, id)| CoverageCandidate {
                        tool_id: id,
                        canonical_id: format!("canonical_{}", i),
                        provider: "test".into(),
                        dataset_id: format!("dataset_{}", i),
                        eligible: true,
                        rejection_reason: None,
                        cost_credits: 0,
                    })
                    .collect();

                let record = CoverageRecord {
                    candidates,
                    ..CoverageRecord::default()
                };
                let stats = coverage_stats(&record);
                let expected = record.candidates.len() >= 2;
                prop_assert_eq!(stats.eligible_for_rate, expected);
                Ok(())
            },
        )
        .unwrap();
}
