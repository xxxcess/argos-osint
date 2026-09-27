//! Interactive Text Network Analysis: local co-occurrence graphs.

mod build;
mod clean;
mod corpus;
mod extract;
mod types;

pub use build::{build_snapshot, build_snapshot_blocking};
pub use corpus::{CorpusDoc, TnaCorpus};
pub use extract::{extract_occurrences, ip_is_blocked_label, Occurrence};
pub use types::{
    desk_key, report_key, TnaAnchor, TnaCluster, TnaClusterSummary, TnaDecision, TnaEdge, TnaGap,
    TnaNode, TnaNodeKind, TnaScope, TnaSnapshot, PIPELINE_VERSION,
};

use anyhow::Result;

use crate::store::Store;

/// Rebuild the targeted snapshot for one filed report and upsert it.
pub fn rebuild_for_report(store: &Store, report_id: &str) -> Result<TnaSnapshot> {
    let corpus = TnaCorpus::targeted(store, report_id)?;
    let snap = build_snapshot(&corpus);
    store.upsert_tna_graph(&report_key(report_id), &snap)?;
    Ok(snap)
}

/// Rebuild the desk collection snapshot from every completed report.
pub fn rebuild_collection(store: &Store) -> Result<TnaSnapshot> {
    let corpus = TnaCorpus::collection(store)?;
    let snap = build_snapshot(&corpus);
    store.upsert_tna_graph(desk_key(), &snap)?;
    Ok(snap)
}

/// After a report is filed: refresh targeted + collection.
pub fn rebuild_after_file(store: &Store, report_id: &str) -> Result<()> {
    let corpus = TnaCorpus::collection(store)?;
    let doc = corpus
        .docs
        .iter()
        .find(|d| d.report_id == report_id)
        .ok_or_else(|| anyhow::anyhow!("report {report_id} not found"))?
        .clone();
    let targeted = TnaCorpus {
        scope: TnaScope::Targeted {
            report_id: doc.report_id.clone(),
            title: doc.title.clone(),
        },
        docs: vec![doc],
    };
    store.upsert_tna_graph(&report_key(report_id), &build_snapshot(&targeted))?;
    store.upsert_tna_graph(desk_key(), &build_snapshot(&corpus))?;
    Ok(())
}

/// After a report is deleted: drop targeted key and refresh collection.
pub fn rebuild_after_delete(store: &Store, report_id: &str) -> Result<()> {
    store.delete_tna_graph(&report_key(report_id))?;
    let _ = rebuild_collection(store)?;
    Ok(())
}

#[cfg(test)]
mod document_tests {
    use super::*;
    use crate::report::{render_report, ReportMeta};
    use crate::search::SearchHit;
    use std::collections::BTreeSet;

    const REPORT: &str = include_str!("fixtures/report.md");

    fn ids(snap: &TnaSnapshot) -> BTreeSet<&str> {
        snap.nodes
            .iter()
            .filter(|n| n.kind != TnaNodeKind::Doc)
            .map(|n| n.id.as_str())
            .collect()
    }

    #[test]
    fn realistic_markdown_cleanup_and_scope_agree() {
        let doc = CorpusDoc {
            report_id: "r1".into(),
            title: "Case Desk".into(),
            text: REPORT.into(),
        };
        let col = build_snapshot(&TnaCorpus {
            scope: TnaScope::Collection,
            docs: vec![doc.clone()],
        });
        let tgt = build_snapshot(&TnaCorpus {
            scope: TnaScope::Targeted {
                report_id: "r1".into(),
                title: "Case Desk".into(),
            },
            docs: vec![doc],
        });
        assert_eq!(ids(&col), ids(&tgt));
        let expected = BTreeSet::from([
            "person:ada lovelace",
            "person:alan turing",
            "org:openai inc",
            "domain:example.com",
            "domain:subject.example",
            "ip:8.8.8.8",
            "email:ada@example.com",
            "handle:ada_research",
            "topic:credential theft",
            "topic:supply chain compromise",
        ]);
        assert_eq!(ids(&tgt), expected, "{:?}", tgt.nodes);
        assert!(col
            .anchors
            .iter()
            .all(|a| expected.contains(a.node_id.as_str())));
        assert!(tgt.decisions.iter().any(|d| d.label.is_none()));
        for d in tgt.decisions.iter().filter(|d| d.label.is_some()) {
            assert_eq!(&REPORT[d.start..d.end], d.original);
            assert!(!d.section.is_empty());
        }
        assert_eq!(
            tgt.nodes
                .iter()
                .find(|n| n.id == "email:ada@example.com")
                .unwrap()
                .mentions,
            1
        );
    }

    #[test]
    fn capitalized_theme_is_not_also_a_person() {
        let snap = build_snapshot(&TnaCorpus {
            scope: TnaScope::Collection,
            docs: vec![CorpusDoc {
                report_id: "r1".into(),
                title: "Test".into(),
                text: "## Analyst note\nTheme: Cyber Security\n".into(),
            }],
        });
        assert_eq!(ids(&snap), BTreeSet::from(["topic:cyber security"]));
    }

    #[test]
    fn normalization_keeps_standalone_hosts_after_email_and_canonical_ipv6() {
        let text = "## Evidence\nada@EXAMPLE.COM then EXAMPLE.COM and example.com.\nPublic IPv6 2001:4860:4860:0:0:0:0:8888\nWho is ada lovelace?\nAda Lovelace [2]\nhttps://source.example/path/8.8.8.8/foo@example.com/@fake\n## Analyst note\nThemes: Credential theft; credential theft\n";
        let snap = build_snapshot(&TnaCorpus {
            scope: TnaScope::Collection,
            docs: vec![CorpusDoc {
                report_id: "r1".into(),
                title: "Test".into(),
                text: text.into(),
            }],
        });
        assert!(ids(&snap).contains("domain:example.com"));
        assert!(ids(&snap).contains("ip:2001:4860:4860::8888"));
        assert!(!ids(&snap).contains("ip:8.8.8.8"));
        assert!(!ids(&snap).contains("email:foo@example.com"));
        assert!(!ids(&snap).contains("handle:fake"));
        assert_eq!(
            snap.nodes
                .iter()
                .filter(|n| n.kind == TnaNodeKind::Person)
                .count(),
            1
        );
        assert_eq!(
            snap.nodes
                .iter()
                .find(|n| n.id == "person:ada lovelace")
                .unwrap()
                .label,
            "Ada Lovelace"
        );
        assert_eq!(
            snap.nodes
                .iter()
                .filter(|n| n.kind == TnaNodeKind::Topic)
                .count(),
            1
        );
    }

    #[test]
    fn filed_generated_report_uses_excerpts_not_source_hosts_or_memories() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::memory().unwrap();
        let md = render_report(
            "Case Desk",
            None,
            "Who is Ada Lovelace?",
            "Theme: Credential theft",
            &[SearchHit {
                title: "Public Sources".into(),
                url: "https://citation.example/report.md".into(),
                snippet: "Ada Lovelace operates example.com with OpenAI Inc.".into(),
            }],
        );
        let report = crate::report::write_report(dir.path(), "Case Desk", None, &md).unwrap();
        store.add_report(&report).unwrap();
        store
            .add_report_fact("Invented Person owns memory-only.example", &report.id)
            .unwrap();
        rebuild_after_file(&store, &report.id).unwrap();
        let col = store.get_tna_graph(desk_key()).unwrap().unwrap();
        let tgt = store
            .get_tna_graph(&report_key(&report.id))
            .unwrap()
            .unwrap();
        assert_eq!(ids(&col), ids(&tgt));
        assert!(!ids(&tgt).contains("domain:citation.example"));
        assert!(!ids(&tgt).contains("domain:memory-only.example"));
        assert!(ids(&tgt).contains("person:ada lovelace"));
        assert!(ids(&tgt).contains("topic:credential theft"));
        assert_eq!(
            tgt.nodes
                .iter()
                .find(|n| n.id == "org:openai inc")
                .unwrap()
                .mentions,
            1
        );
        store.delete_report_bundle(&report.id).unwrap();
        rebuild_after_delete(&store, &report.id).unwrap();
        assert!(store
            .get_tna_graph(&report_key(&report.id))
            .unwrap()
            .is_none());
        assert!(store
            .get_tna_graph(desk_key())
            .unwrap()
            .unwrap()
            .nodes
            .is_empty());
    }

    #[test]
    fn missing_file_surfaces_error_and_stale_snapshots_are_cache_misses() {
        let store = Store::memory().unwrap();
        let report = ReportMeta {
            id: "missing".into(),
            title: "Missing".into(),
            case_id: None,
            path: "/nonexistent/argos-tna-report.md".into(),
            created_at: "now".into(),
        };
        store.add_report(&report).unwrap();
        assert!(TnaCorpus::collection(&store)
            .unwrap_err()
            .to_string()
            .contains("missing"));
        assert!(TnaCorpus::targeted(&store, "missing").is_err());
        let mut old = TnaSnapshot::empty(TnaScope::Collection);
        old.pipeline_version = 0;
        store.upsert_tna_graph(desk_key(), &old).unwrap();
        assert!(store.get_tna_graph(desk_key()).unwrap().is_none());
        store.delete_report_bundle("missing").unwrap();
        let current = rebuild_collection(&store).unwrap();
        assert_eq!(store.get_tna_graph(desk_key()).unwrap().unwrap(), current);
    }
}
