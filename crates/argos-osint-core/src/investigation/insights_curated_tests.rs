//! Offline host-curator and exact semantic proof contracts; no provider calls.
use super::*;
use std::collections::BTreeMap;

#[derive(Clone)]
struct Case {
    input: PipelineInput,
    claims: Vec<SourceClaim>,
    link: AssessedLink,
}
fn passed(probability: f64, runner_up: f64) -> Assessment {
    use crate::investigation::decisions::{DecisionPolicy, NormalizedAnswer};
    let answer = NormalizedAnswer {
        label: "faithful".into(),
        score: None,
        score_value: None,
        noul: None,
        probability: Some(probability),
        confidence: None,
        probabilities: BTreeMap::from([
            ("faithful".into(), probability),
            ("unfaithful".into(), runner_up),
            ("insufficient".into(), 0.0),
        ]),
    };
    Assessment {
        fidelity: answer.label.clone(),
        threshold_passed: DecisionPolicy::default()
            .evaluate("curated_link", &answer)
            .is_approved(),
        relation: "insufficient".into(),
        rationale: "Controlled offline semantic decision".into(),
        gaps: Vec::new(),
    }
}
fn case(kind: &str) -> Case {
    let mut evidence: Vec<_> = [
        (
            "left",
            "acquisition-root-a",
            "Port closure redirects coastal shipping traffic toward the northern harbor.",
        ),
        (
            "right",
            "acquisition-root-b",
            "Port closure redirects coastal shipping traffic toward an inland harbor.",
        ),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (id, root, text))| RetainedEvidence {
        reference: EvidenceRef {
            source_id: format!("call:{id}"),
            revision: format!("revision-{id}"),
            artifact_hash: hash(text),
            acquisition_id: id.into(),
            origin_root: root.into(),
            url: format!("https://{id}.test/story"),
            published_at: Some(format!(
                "2026-10-{}T00:00:00Z",
                if kind == "updates" {
                    23 - index
                } else {
                    22 + index
                }
            )),
            retrieved_at: "2026-10-24T00:00:00Z".into(),
            location: EvidenceLocation::Range(Span {
                start_byte: 0,
                end_byte: text.len(),
            }),
        },
        text: text.into(),
        native: None,
        substantive: true,
        relevant: true,
        complete_story: true,
        known_truncation: false,
    })
    .collect();
    if kind == "contradicts" {
        evidence[1].text="Port closure does not redirect coastal shipping traffic; ships continue on their normal route.".into();
    } else if kind == "updates" {
        evidence[0].text="Updated report: port closure now redirects shipping traffic to the northern harbor, correcting the earlier inland routing.".into();
    }
    for source in &mut evidence {
        source.reference.artifact_hash = hash(&source.text);
        source.reference.location = EvidenceLocation::Range(Span {
            start_byte: 0,
            end_byte: source.text.len(),
        });
    }
    let mut snapshot = snapshot("article", "source", 1, &evidence);
    snapshot.key = "full-story-current-snapshot".into();
    snapshot.entities.push(EntityCandidate {
        id: "port-authority".into(),
        type_hint: "org".into(),
        comparison_key: "port authority".into(),
        original_forms: vec!["Port Authority".into()],
        mention_refs: Vec::new(),
    });
    let claims: Vec<_> = evidence
        .iter()
        .enumerate()
        .map(|(index, evidence)| SourceClaim {
            id: format!("claim-{index}"),
            subject_ref: "port-authority".into(),
            predicate: "redirects".into(),
            object: "shipping traffic".into(),
            text: evidence.text.clone(),
            polarity: if kind == "contradicts" && index == 1 {
                "negative"
            } else {
                "positive"
            }
            .into(),
            modality: "asserted".into(),
            attributed_to: format!("Source{index}"),
            temporal_scope: "Reported closure period".into(),
            evidence_refs: vec![evidence.reference.clone()],
            extraction_scope: "Complete retained story".into(),
            assessment: passed(0.9, 0.1),
            support_receipts: Vec::new(),
        })
        .collect();
    let mut link = AssessedLink {
        id: "link:closure-fixture".into(),
        typed_endpoints: claims
            .iter()
            .map(|claim| TypedEndpoint {
                kind: "claim".into(),
                id: claim.id.clone(),
            })
            .collect(),
        relation: kind.into(),
        evidence_refs: evidence
            .iter()
            .map(|evidence| evidence.reference.clone())
            .collect(),
        rationale:
            "Both cited assertions describe the specific port closure redirecting shipping traffic."
                .into(),
        assessment: passed(0.9, 0.1),
        snapshot_key: snapshot.key.clone(),
        chronology_basis: Some(crate::intelligence_contracts::ChronologyBasis::Publication),
        conditions: vec!["If the reported port closure continues".into()],
        limitations: vec!["The effect is limited to the reported closure period".into()],
        curation: None,
    };
    link.rationale=match kind {
        "contradicts"=>"The two independently cited assertions disagree about whether the same port closure redirects shipping traffic.",
        "updates"=>"The later source explicitly corrects the earlier reported inland routing of shipping traffic.",
        "precedes"=>"The source's publication predates the target publication; this does not establish event causation.",
        "potential_impact"=>"If the reported port closure continues, the retained redirection premise could affect shipping traffic at the inland harbor; the extent remains uncertain.",
        _=>"Both cited assertions describe the specific port closure redirecting shipping traffic.",
    }.into();
    Case {
        input: PipelineInput {
            snapshot,
            evidence,
            objective: "Assess the shipping closure".into(),
            surface: "article".into(),
            required_tasks_complete: true,
            save_insights: true,
            recall_enabled: false,
        },
        claims,
        link,
    }
}
fn rebuild(case: &mut Case) {
    for (claim, evidence) in case.claims.iter_mut().zip(&case.input.evidence) {
        claim.evidence_refs = vec![evidence.reference.clone()];
    }
    case.link.evidence_refs = case
        .input
        .evidence
        .iter()
        .map(|evidence| evidence.reference.clone())
        .collect();
    case.input.snapshot.source_refs = case.link.evidence_refs.clone();
}
fn semantic_proof(case: &mut Case, probability: f64, runner_up: f64) -> (String, String) {
    let mut proof = prepare_full_story_link(&case.input, &case.claims, &[], &case.link).unwrap();
    let item = full_story_link_item(&case.link, &proof);
    let assessment = passed(probability, runner_up);
    let (key, raw) = super::super::insight_assessment::checkpoint_proof(
        &case.input,
        &[item],
        &crate::provider::SettingsFile::default(),
        &BTreeMap::from([(case.link.id.clone(), assessment.clone())]),
    )
    .unwrap();
    proof.decision_receipt = key.clone();
    case.link.assessment = assessment;
    case.link.curation = Some(proof);
    (key, raw)
}
#[test]
fn full_story_curated_six_kinds_preserve_exact_side_assertions_and_qualifiers() {
    for kind in [
        "supports",
        "contradicts",
        "updates",
        "precedes",
        "contextualizes",
        "potential_impact",
    ] {
        let mut case = case(kind);
        let (key, raw) = semantic_proof(&mut case, 0.9, 0.1);
        let proof = case.link.curation.as_ref().unwrap();
        assert_eq!(proof.kind.as_str(), kind);
        assert!(!proof.source.lineage.is_empty());
        assert!(!proof.target.lineage.is_empty());
        assert!(
            full_story_receipt_assessment(&case.input, &key, &raw, &case.link, proof)
                .unwrap()
                .faithful()
        );
        assert_eq!(proof.conditions, case.link.conditions);
        assert_eq!(proof.limitations, case.link.limitations);
        let mut borrowed = case.link.clone();
        borrowed.rationale = "A different relationship proposition".into();
        assert!(
            full_story_receipt_assessment(&case.input, &key, &raw, &borrowed, proof).is_err(),
            "passed decision cannot be borrowed for changed meaning"
        );
    }
}
#[test]
fn full_story_curated_unknown_roots_syndication_shared_acquisition_and_unknown_chronology_abstain()
{
    for kind in ["supports", "contradicts"] {
        for root in ["", "publisher:one.test", "acquisition-root-a"] {
            let mut case = case(kind);
            case.input.evidence[1].reference.origin_root = root.into();
            rebuild(&mut case);
            assert!(
                prepare_full_story_link(&case.input, &case.claims, &[], &case.link).is_err(),
                "{kind}:{root}"
            );
        }
        let mut syndicated = case(kind);
        let original = syndicated.input.evidence[0].clone();
        syndicated.input.evidence[1].text = original.text;
        syndicated.input.evidence[1].reference.artifact_hash = original.reference.artifact_hash;
        syndicated.input.evidence[1].reference.location = original.reference.location;
        rebuild(&mut syndicated);
        assert!(prepare_full_story_link(
            &syndicated.input,
            &syndicated.claims,
            &[],
            &syndicated.link
        )
        .is_err());
        let mut shared = case(kind);
        shared.input.evidence[1].reference.acquisition_id =
            shared.input.evidence[0].reference.acquisition_id.clone();
        rebuild(&mut shared);
        assert!(prepare_full_story_link(&shared.input, &shared.claims, &[], &shared.link).is_err());
    }
    for time in [
        None,
        Some("malformed"),
        Some("2026-10-22T00:00:00Z"),
        Some("2026-10-21T00:00:00Z"),
    ] {
        let mut case = case("precedes");
        case.input.evidence[1].reference.published_at = time.map(str::to_string);
        rebuild(&mut case);
        assert!(
            prepare_full_story_link(&case.input, &case.claims, &[], &case.link).is_err(),
            "{time:?}"
        );
    }
    let mut conditional = case("potential_impact");
    conditional.link.conditions.clear();
    assert!(prepare_full_story_link(
        &conditional.input,
        &conditional.claims,
        &[],
        &conditional.link
    )
    .is_err());
}
#[test]
fn full_story_curated_full_context_and_thresholds_cannot_be_replaced_by_a_hash_or_fragment() {
    let mut case = case("supports");
    let (key, raw) = semantic_proof(&mut case, 0.51, 0.49);
    assert!(!full_story_receipt_assessment(
        &case.input,
        &key,
        &raw,
        &case.link,
        case.link.curation.as_ref().unwrap()
    )
    .unwrap()
    .faithful());
    let mut forged: Value = serde_json::from_str(&raw).unwrap();
    forged["items"][0]["text"] = "A different assertion".into();
    assert!(full_story_receipt_assessment(
        &case.input,
        &key,
        &forged.to_string(),
        &case.link,
        case.link.curation.as_ref().unwrap()
    )
    .is_err());
    let mut changed = case.input.clone();
    changed.evidence[0].reference.revision = "refreshed-source".into();
    assert!(full_story_receipt_assessment(
        &changed,
        &key,
        &raw,
        &case.link,
        case.link.curation.as_ref().unwrap()
    )
    .is_err());
    let mut oversized = case.clone();
    oversized.claims[0].text = "Shipping port closure ".repeat(2000);
    assert!(
        prepare_full_story_link(&oversized.input, &oversized.claims, &[], &oversized.link).is_err(),
        "complete proof is withheld, not silently shortened"
    );
}

#[cfg(feature = "embedded")]
async fn seed_full_story_sources(repo: &crate::storage::repo::Repository, case: &mut Case) {
    use crate::storage::{atlas, pipeline_inputs};
    atlas::upsert_article(
        repo.db(),
        &crate::store::AtlasArticleRow {
            run_id: "fixture-run".into(),
            id: "source".into(),
            title: "Port closure coverage".into(),
            description: "Independent full-story fixture".into(),
            url: "https://source.test/fixture".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    for evidence in &mut case.input.evidence {
        let result=serde_json::json!({"tool_id":"fixture-full-story","observations":{"markdown":evidence.text,"origin_root":evidence.reference.origin_root},"status":"completed"}).to_string();
        evidence.reference.revision = hash(&result);
        repo.db().query("CREATE $call SET tool_id='fixture-full-story',origin='fixture',inputs_json='{}',status='completed',result_json=$result,started_at='2026-10-24T00:00:00Z',completed_at='2026-10-24T00:00:00Z' RETURN NONE")
            .bind(("call",surrealdb::types::RecordId::new("osint_call",evidence.reference.acquisition_id.clone())))
            .bind(("result",result)).await.unwrap().check().unwrap();
        let text = evidence.text.clone();
        let artifact = crate::storage::tx::run_tx(
            repo.db(),
            crate::storage::tx::RetryPolicy::default(),
            move |tx| {
                let text = text.clone();
                async move { pipeline_inputs::artifact_in(&tx, &text).await }
            },
        )
        .await
        .unwrap()
        .value;
        assert_eq!(artifact, evidence.reference.artifact_hash);
    }
    rebuild(case);
}

#[cfg(feature = "embedded")]
async fn publish_full_story_case(
    repo: &crate::storage::repo::Repository,
    case: &mut Case,
    probability: f64,
    runner_up: f64,
    corrupt_proof: bool,
) -> String {
    publish_full_story_case_inner(repo, case, probability, runner_up, corrupt_proof, true).await
}

#[cfg(feature = "embedded")]
async fn publish_full_story_case_inner(
    repo: &crate::storage::repo::Repository,
    case: &mut Case,
    probability: f64,
    runner_up: f64,
    corrupt_proof: bool,
    advance: bool,
) -> String {
    use crate::storage::tx::{run_tx, RetryPolicy};
    use crate::storage::{derivations, insight, pipeline_exec};
    let generation = if advance {
        run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
            insight::advance_generation(&tx, "article", "source", "2026-10-24T00:00:00Z").await
        })
        .await
        .unwrap()
        .value
    } else {
        case.input.snapshot.generation as i64
    };
    case.input.snapshot.generation = generation as u32;
    case.input.snapshot.key = format!("full-story-fixture-{generation}");
    case.link.snapshot_key = case.input.snapshot.key.clone();
    let (key, raw) = semantic_proof(case, probability, runner_up);
    pipeline_exec::put(
        repo,
        &key,
        &case.input,
        "assessment",
        &case.link.id,
        "completed",
        &raw,
        "{}",
        "",
    )
    .await
    .unwrap();
    if corrupt_proof {
        case.link.curation.as_mut().unwrap().target.revision = "stale-target-proof".into();
    }
    let output = PipelineOutput {
        entities: case.input.snapshot.entities.clone(),
        claims: case.claims.clone(),
        links: vec![case.link.clone()],
        readiness: Readiness {
            input_eligible: true,
            publication_eligible: true,
            gaps: Vec::new(),
        },
        ..Default::default()
    };
    let input = std::sync::Arc::new(case.input.clone());
    let publish = derivations::derivation_step(
        input.clone(),
        std::sync::Arc::new(output),
        None,
        derivations::DerivationContext {
            save_insights: true,
            recon_thread: None,
        },
    );
    let validate = crate::storage::live_evidence::step(input.snapshot.clone());
    run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let input = input.clone();
        let publish = publish.clone();
        let validate = validate.clone();
        async move {
            insight::publish_snapshot_on(
                &tx,
                &input.snapshot,
                &input.snapshot.key,
                "active",
                "2026-10-24T00:00:00Z",
                &validate,
                &publish,
            )
            .await
        }
    })
    .await
    .unwrap()
    .value
    .expect("current canonical snapshot")
}

#[cfg(feature = "embedded")]
async fn persisted_links(
    repo: &crate::storage::repo::Repository,
) -> Vec<(String, String, crate::memory_metadata::MemoryPayload)> {
    let mut result=repo.db().query("SELECT meta::id(id) AS id,lifecycle,payload_json FROM memory WHERE memory_kind='link' ORDER BY id").await.unwrap().check().unwrap();
    let rows: Vec<Value> = result.take(0).unwrap();
    rows.into_iter()
        .map(|row| {
            (
                row["id"].as_str().unwrap().to_string(),
                row["lifecycle"].as_str().unwrap().to_string(),
                serde_json::from_str(row["payload_json"].as_str().unwrap()).unwrap(),
            )
        })
        .collect()
}

#[cfg(feature = "embedded")]
#[tokio::test]
async fn full_story_curated_actual_transaction_publishes_support_and_contradiction_and_preserves_proof_roundtrip(
) {
    for kind in ["supports", "contradicts"] {
        let root = tempfile::tempdir().unwrap();
        let repo = crate::storage::repo::Repository::connect(
            &crate::storage::open::Target::Embedded {
                path: root.path().join("surreal"),
            },
            "argos",
            "full_story_curated",
        )
        .await
        .unwrap();
        let mut case = case(kind);
        seed_full_story_sources(&repo, &mut case).await;
        let snapshot = publish_full_story_case(&repo, &mut case, 0.9, 0.1, false).await;
        let original = persisted_links(&repo).await;
        assert_eq!(original.len(), 1, "{kind}: {original:?}");
        let (id, lifecycle, crate::memory_metadata::MemoryPayload::Link(link)) = &original[0]
        else {
            panic!("typed canonical Link");
        };
        assert_eq!(lifecycle, "active");
        assert_eq!(link.relation, kind);
        let receipt = link.validation.as_ref().unwrap();
        receipt.validate_structure().unwrap();
        assert!(!receipt.source_lineage.is_empty());
        assert!(!receipt.target_lineage.is_empty());
        let mut work = repo
            .db()
            .query("RETURN count(SELECT 1 FROM index_change WHERE record_id=$id)")
            .bind(("id", id.clone()))
            .await
            .unwrap()
            .check()
            .unwrap();
        assert!(
            work.take::<Option<i64>>(0).unwrap().unwrap_or(0) > 0,
            "confirmed Link has real transactional index work"
        );
        let roundtrip = crate::storage::insight_owner::active_output(&repo, "article", "source")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::to_value(&roundtrip.links[0]).unwrap(),
            serde_json::to_value(&case.link).unwrap(),
            "typed proof, chronology and qualifiers survive indexed projection"
        );
        let before_work = repo
            .db()
            .query("RETURN count(SELECT 1 FROM index_change WHERE record_id=$id)")
            .bind(("id", id.clone()))
            .await
            .unwrap()
            .check()
            .unwrap()
            .take::<Option<i64>>(0)
            .unwrap();
        // Retry the exact current publication after losing its prior reply.
        // It must not create a second memory or duplicate index work.
        let mut replay = case.clone();
        publish_full_story_case_inner(&repo, &mut replay, 0.9, 0.1, false, false).await;
        let repeated = persisted_links(&repo).await;
        assert_eq!(
            repeated.len(),
            1,
            "same semantic relationship has one canonical memory"
        );
        assert_eq!(repeated[0].0, *id);
        assert_eq!(repeated[0].1, "active");
        let after_work = repo
            .db()
            .query("RETURN count(SELECT 1 FROM index_change WHERE record_id=$id)")
            .bind(("id", id.clone()))
            .await
            .unwrap()
            .check()
            .unwrap()
            .take::<Option<i64>>(0)
            .unwrap();
        assert_eq!(
            after_work, before_work,
            "same canonical index work key is replayed idempotently"
        );
        let mut invalid = case.clone();
        publish_full_story_case(&repo, &mut invalid, 0.9, 0.1, true).await;
        assert_eq!(
            persisted_links(&repo).await,
            original,
            "stale new proof cannot retire an unrelated still-live confirmed sibling"
        );
        let mut below = case.clone();
        publish_full_story_case(&repo, &mut below, 0.51, 0.49, false).await;
        assert_eq!(
            persisted_links(&repo).await,
            original,
            "uncertain decision cannot become a confirmed Link"
        );
        // Existing empty items remain explicitly legacy; corrupt nonempty
        // current JSON is an error rather than an invented legacy read.
        let mut row=repo.db().query("SELECT VALUE meta::id(id) FROM insight_derivation_link WHERE snapshot_id=$snapshot").bind(("snapshot",snapshot)).await.unwrap().check().unwrap();
        let rows: Vec<String> = row.take(0).unwrap();
        assert_eq!(rows.len(), 1);
        repo.db().query("UPDATE insight_derivation_link SET item_json='' WHERE snapshot_id=(SELECT VALUE active_snapshot_id FROM ONLY type::record('insight_owner_state',['article','source'])) RETURN NONE").await.unwrap().check().unwrap();
        let legacy = crate::storage::insight_owner::active_output(&repo, "article", "source")
            .await
            .unwrap()
            .unwrap();
        assert!(legacy.links[0].curation.is_none());
        repo.db().query("UPDATE insight_derivation_link SET item_json='{broken' WHERE snapshot_id=(SELECT VALUE active_snapshot_id FROM ONLY type::record('insight_owner_state',['article','source'])) RETURN NONE").await.unwrap().check().unwrap();
        assert!(
            crate::storage::insight_owner::active_output(&repo, "article", "source")
                .await
                .is_err()
        );
    }
}
