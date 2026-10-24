//! Turn recall and the shared-refresh retry on SurrealDB: vector recall
//! (DISKANN) for `orchestrate::recall_for_turn` with the full-text
//! fallback, the text admission policy, and `refresh_driver`'s run and
//! retry of an owner's refresh. Both modes via `scripts/test_surreal.py`.
#![cfg(feature = "surreal")]

mod support;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use argos_osint_core::brain::MemorySource;
use argos_osint_core::brain_query::{admit_text, AdmissionPolicy, BrainQuery};
use argos_osint_core::investigation::insights::{
    snapshot, EvidenceLocation, EvidenceRef, PipelineInput, PipelineOutput, RetainedEvidence,
};
use argos_osint_core::provider::SettingsFile;
use argos_osint_core::storage::insight::SnapshotStep;
use argos_osint_core::storage::recall::RecallSource;
use argos_osint_core::storage::refresh::schedule_article_refresh;
use argos_osint_core::storage::refresh_driver::{retry_owner, run_one, Retried};
use argos_osint_core::storage::refresh_exec::Execute;
use argos_osint_core::storage::vectors::{self, Embedder};
use argos_osint_core::storage::{memories, recon_extract};
use support::*;

fn source() -> MemorySource {
    MemorySource {
        app: "cli".into(),
        conversation_id: "c1".into(),
        message_id: None,
        reference: None,
    }
}

/// One-hot vectors by first letter (as in `storage_vectors`).
fn fake(model: &str) -> Embedder {
    Embedder {
        model: model.into(),
        embed: Arc::new(|texts: Vec<String>| {
            Ok(texts
                .iter()
                .map(|t| {
                    let w = t
                        .trim()
                        .chars()
                        .next()
                        .unwrap_or(' ')
                        .to_ascii_lowercase()
                        .to_string();
                    let slot = w.bytes().fold(7usize, |h, b| h * 31 + b as usize) % 384;
                    let mut v = vec![0.0f32; 384];
                    v[slot] = 1.0;
                    v
                })
                .collect())
        }),
    }
}

#[tokio::test]
async fn turn_recall_is_vector_first_with_a_full_text_fallback() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "recall_turn").await;
    let e = fake("fake-a");
    let zephyr = memories::add_memory(
        &repo,
        "zephyr winds at the pier",
        "fact",
        false,
        source(),
        Some(&e),
    )
    .await
    .unwrap();
    memories::add_memory(
        &repo,
        "quartz mine output rose",
        "fact",
        false,
        source(),
        Some(&e),
    )
    .await
    .unwrap();

    // No serving generation: the full-text path, which shares no token.
    let hits = recon_extract::recall_for_turn_with(&repo, "no-thread", "Zebra?", Some(&e))
        .await
        .unwrap();
    assert!(hits.is_empty(), "{hits:?}");

    vectors::reindex(&repo, &e).await.unwrap();
    let hits = recon_extract::recall_for_turn_with(&repo, "no-thread", "Zebra?", Some(&e))
        .await
        .unwrap();
    assert_eq!(hits[0].id, zephyr.id);
    assert_eq!(hits[0].source, RecallSource::Vector);
    assert!(hits.len() <= recon_extract::RECALL_LIMIT);

    // Another model's generation is not served: full text again.
    let lexical =
        recon_extract::recall_for_turn_with(&repo, "no-thread", "quartz mine", Some(&fake("b")))
            .await
            .unwrap();
    assert!(lexical
        .iter()
        .all(|hit| hit.source == RecallSource::Lexical));
    assert!(!lexical.is_empty());
}

#[test]
fn text_admission_requires_a_confirmed_subject_in_strict_mode() {
    let query = BrainQuery {
        confirmed_subjects: vec!["Northwind Ferries".into()],
        ..Default::default()
    };
    let strict = AdmissionPolicy {
        strict_mode: true,
        allow_background: false,
    };
    assert!(admit_text(&query, "northwind ferries cut routes", &strict));
    assert!(!admit_text(&query, "northwind cut routes", &strict));
    let loose = AdmissionPolicy {
        strict_mode: false,
        allow_background: false,
    };
    assert!(admit_text(&query, "unrelated", &loose));
}

fn input(article: &str) -> PipelineInput {
    let evidence = vec![RetainedEvidence {
        reference: EvidenceRef {
            source_id: format!("body:{article}"),
            revision: "r1".into(),
            artifact_hash: "hash".into(),
            acquisition_id: "acq".into(),
            origin_root: String::new(),
            url: "https://example.test/a".into(),
            published_at: None,
            retrieved_at: "2026-10-01".into(),
            location: EvidenceLocation::JsonPointer(String::new()),
        },
        text: "Body.".into(),
        native: None,
        substantive: true,
        relevant: true,
        complete_story: true,
        known_truncation: false,
    }];
    PipelineInput {
        snapshot: snapshot("article", article, 0, &evidence),
        evidence,
        objective: "Extract attributed assertions.".into(),
        surface: "article".into(),
        required_tasks_complete: true,
        save_insights: false,
        recall_enabled: false,
    }
}

fn ok_validate() -> Option<SnapshotStep> {
    Some(Arc::new(|_, _| Box::pin(async { Ok(()) })))
}

fn eligible() -> Execute {
    Box::new(|input| {
        Box::pin(async move {
            let mut out = PipelineOutput::default();
            out.readiness.publication_eligible = true;
            Ok((input, out))
        })
    })
}

fn failing() -> Execute {
    Box::new(|_| Box::pin(async { Err(anyhow::anyhow!("provider exploded")) }))
}

#[tokio::test]
async fn retry_reenqueues_a_failed_shared_refresh_and_runs_it() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "refresh_driver").await;
    let s = SettingsFile::default();
    let cancel = Arc::new(AtomicBool::new(false));
    assert_eq!(
        retry_owner(
            &repo,
            "article",
            "a1",
            &s,
            &cancel,
            ok_validate(),
            eligible()
        )
        .await
        .unwrap(),
        Retried::Nothing
    );
    let task = schedule_article_refresh(repo.db(), "a1", Some(&input("a1")), &s, true)
        .await
        .unwrap()
        .expect("task");
    assert!(run_one(&repo, &task, &s, &cancel, ok_validate(), failing())
        .await
        .is_err());

    let Retried::Ran { task_id, published } = retry_owner(
        &repo,
        "article",
        "a1",
        &s,
        &cancel,
        ok_validate(),
        eligible(),
    )
    .await
    .unwrap() else {
        panic!("nothing retried");
    };
    assert!(published);
    assert_ne!(task_id, task, "a new generation's task");
    assert_eq!(
        retry_owner(
            &repo,
            "article",
            "a1",
            &s,
            &cancel,
            ok_validate(),
            eligible()
        )
        .await
        .unwrap(),
        Retried::Completed {
            task_id: task_id.clone()
        },
        "a completed refresh is not retried (and the caller must not mine)"
    );
}

/// Admission runs over the candidate pool, not the capped five: the
/// candidate recall returns more than `RECALL_LIMIT` hits, the turn recall
/// never does.
#[tokio::test]
async fn turn_recall_candidates_exceed_the_cap() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "recall_candidates").await;
    for i in 0..8 {
        memories::add_memory(
            &repo,
            &format!("harbor crane {i} moved cargo"),
            "fact",
            false,
            source(),
            None,
        )
        .await
        .unwrap();
    }
    let capped = recon_extract::recall_for_turn(&repo, "no-thread", "harbor crane")
        .await
        .unwrap();
    assert_eq!(capped.len(), recon_extract::RECALL_LIMIT);
    let pool = recon_extract::recall_candidates_for_turn(
        &repo,
        "no-thread",
        "harbor crane",
        None,
        20,
        usize::MAX,
    )
    .await
    .unwrap();
    assert_eq!(pool.len(), 8);
}
