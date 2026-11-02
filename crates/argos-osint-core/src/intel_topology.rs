//! Read-only Brain identities for Intel brief navigation. @tag:intel-brief

use anyhow::Result;
use serde_json::json;
use serde_json::Value;

use crate::atlas_metadata;
use crate::brain_topology::TopologyNode;
use crate::brain_topology::TopologyNodeKind;
use crate::brain_topology::TopologySeed;
use crate::storage::repo::Repository;
use crate::store::AtlasArticleRow;

/// Stored claims keep a leading `[claim · supports]` tag. Brain labels do not.
fn evidence_label(text: &str) -> String {
    let trimmed = text.trim();
    let Some(rest) = trimmed
        .strip_prefix('[')
        .and_then(|body| body.split_once(']'))
        .filter(|(tag, _)| {
            let tag = tag.trim();
            tag.starts_with("claim ·") || tag.starts_with("inference ·")
        })
        .map(|(_, rest)| rest.trim())
    else {
        return trimmed.to_owned();
    };
    if rest.is_empty() {
        trimmed.to_owned()
    } else {
        rest.to_owned()
    }
}

fn unique_seed(nodes: &[TopologyNode], kind: TopologyNodeKind, labels: &[String]) -> Value {
    let mut seeds: Vec<_> = nodes
        .iter()
        .filter(|node| {
            node.kind == kind
                && labels.iter().any(|label| {
                    atlas_metadata::comparison_key(label)
                        == atlas_metadata::comparison_key(&node.label)
                })
        })
        .filter_map(|node| node.topology_seed.as_ref())
        .filter(|seed| {
            matches!(
                seed,
                TopologySeed::Actor(_) | TopologySeed::Claim(_) | TopologySeed::Inference(_)
            )
        })
        .collect();
    seeds.sort_by_key(|seed| seed.key());
    seeds.dedup();
    match seeds.as_slice() {
        [seed] => json!(seed),
        _ => Value::Null,
    }
}

/// Reuse the topology reader's current-article provenance and lifecycle fences.
/// Neither this projection nor the subsequent navigation creates Brain memories.
pub(crate) async fn decorate(
    repo: &Repository,
    article: &AtlasArticleRow,
    revision: &str,
    value: &mut Value,
) -> Result<()> {
    let source = TopologySeed::Article {
        run_id: article.run_id.clone(),
        article_id: article.id.clone(),
        revision: revision.into(),
        originating_memory: None,
    };
    let snapshot = crate::storage::topology::brief_binding_snapshot(repo, &source).await?;
    if let Some(items) = value["actors"].as_array_mut() {
        for item in items {
            let mut labels = vec![item["display_name"].as_str().unwrap_or_default().to_owned()];
            labels.extend(
                item["spellings"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|label| label.as_str().map(str::to_owned)),
            );
            let actor_id = item["id"].as_str().unwrap_or_default();
            item["topology_seed"] = if actor_id.starts_with("catalog-actor-") {
                snapshot
                    .nodes
                    .iter()
                    .find_map(|node| {
                        node.topology_seed
                            .as_ref()
                            .filter(
                                |seed| matches!(seed, TopologySeed::Actor(id) if id == actor_id),
                            )
                            .map(|seed| json!(seed))
                    })
                    .unwrap_or(Value::Null)
            } else {
                unique_seed(&snapshot.nodes, TopologyNodeKind::Entity, &labels)
            };
        }
    }
    for field in ["claims", "inferences"] {
        if let Some(items) = value[field].as_array_mut() {
            for item in items {
                let kind = if field == "inferences" || item["classification"] == "inference" {
                    TopologyNodeKind::Inference
                } else {
                    TopologyNodeKind::Claim
                };
                let text = item["text"].as_str().unwrap_or_default();
                let shown = evidence_label(text);
                let mut labels = vec![text.to_owned()];
                if shown != text {
                    labels.push(shown);
                }
                item["topology_seed"] = unique_seed(&snapshot.nodes, kind, &labels);
            }
        }
    }
    value["topology_seed"] = ["claims", "inferences", "actors"]
        .iter()
        .flat_map(|field| value[*field].as_array().into_iter().flatten())
        .find_map(|item| {
            item.get("topology_seed")
                .filter(|seed| !seed.is_null())
                .cloned()
        })
        .unwrap_or(Value::Null);
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn evidence_labels_remove_only_leading_assessment_tags_and_preserve_content() {
        for (stored, shown) in [
            ("[claim · supports] Harbor statement", "Harbor statement"),
            (
                "[inference · tentative] Possible outcome",
                "Possible outcome",
            ),
            (
                "  Ordinary [claim · supports] text  ",
                "Ordinary [claim · supports] text",
            ),
            (
                "[source · wire] Harbor statement",
                "[source · wire] Harbor statement",
            ),
            ("[claim · supports]", "[claim · supports]"),
            ("", ""),
        ] {
            assert_eq!(super::evidence_label(stored), shown, "{stored:?}");
        }
    }

    use super::unique_seed;
    use crate::brain_topology::node;
    use crate::brain_topology::TopologyNodeKind;
    use crate::brain_topology::TopologySeed;
    use serde_json::json;

    #[test]
    fn each_brief_kind_receives_its_own_existing_memory_identity() {
        let mut actor = node(
            "actor-memory".into(),
            TopologyNodeKind::Entity,
            "Recorded item".into(),
        );
        actor.topology_seed = Some(TopologySeed::Actor(actor.id.clone()));
        let mut claim = node(
            "claim-memory".into(),
            TopologyNodeKind::Claim,
            "Recorded item".into(),
        );
        claim.topology_seed = Some(TopologySeed::Claim(claim.id.clone()));
        let mut inference = node(
            "inference-memory".into(),
            TopologyNodeKind::Inference,
            "Recorded item".into(),
        );
        inference.topology_seed = Some(TopologySeed::Inference(inference.id.clone()));
        let nodes = vec![actor, claim, inference];
        for (kind, expected) in [
            (TopologyNodeKind::Entity, json!({"Actor":"actor-memory"})),
            (TopologyNodeKind::Claim, json!({"Claim":"claim-memory"})),
            (
                TopologyNodeKind::Inference,
                json!({"Inference":"inference-memory"}),
            ),
        ] {
            assert_eq!(
                unique_seed(&nodes, kind, &["Recorded item".into()]),
                expected
            );
        }
        assert!(unique_seed(&nodes, TopologyNodeKind::Claim, &["Unindexed item".into()]).is_null());
        assert_eq!(
            unique_seed(
                &nodes,
                TopologyNodeKind::Claim,
                &[
                    "[claim · supports] Recorded item".into(),
                    "Recorded item".into()
                ]
            ),
            json!({"Claim":"claim-memory"})
        );
    }

    #[test]
    fn ambiguous_actor_names_do_not_fabricate_a_canonical_identity() {
        let mut first = node(
            "actor-one".into(),
            TopologyNodeKind::Entity,
            "Same name".into(),
        );
        first.topology_seed = Some(TopologySeed::Actor(first.id.clone()));
        let mut second = first.clone();
        second.id = "actor-two".into();
        second.topology_seed = Some(TopologySeed::Actor(second.id.clone()));
        assert!(unique_seed(
            &[first, second],
            TopologyNodeKind::Entity,
            &["Same name".into()]
        )
        .is_null());
    }
}
