import re
import os

GRAPH_FILE = "crates/argos-osint-core/src/recon/graph.rs"
with open(GRAPH_FILE, "r") as f:
    graph_rs = f.read()

# Replace graph_for_memory to handle memory kinds
new_graph_for_memory = """    pub fn graph_for_memory(&self, memory_id: &str) -> anyhow::Result<MemoryGraph> {
        let kind: Option<String> = self.conn.query_row("SELECT memory_kind FROM memory_metadata WHERE memory_id=?1", [memory_id], |r| r.get(0)).optional()?;
        
        let Some(kind_str) = kind else {
            return self.legacy_graph_for_memory(memory_id);
        };
        
        match kind_str.as_str() {
            "cycle_brief" => self.build_brief_graph(memory_id),
            "manual_note" | "user_profile" | "investigation_digest" | "source_summary" => Ok(MemoryGraph::default()),
            "atomic_claim" | _ => self.legacy_graph_for_memory(memory_id),
        }
    }

    fn legacy_graph_for_memory(&self, memory_id: &str) -> anyhow::Result<MemoryGraph> {
        let Some(insight) = self.insight_for_memory(memory_id)? else {
            return Ok(MemoryGraph::default());
        };
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT s.run_id FROM insight_claims c \
             JOIN insight_sources s ON s.fingerprint = c.fingerprint \
             WHERE c.memory_id = ?1 AND s.run_id IS NOT NULL",
        )?;
        let run_ids: Vec<String> = stmt
            .query_map([memory_id], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        if insight.sources.iter().any(|source| {
            source.answer_id.starts_with("atlas-")
                || (source.thread_id.is_none() && source.run_id.is_some())
        }) {
            let mut articles = Vec::new();
            for source in &insight.sources {
                let Some(run_id) = source.run_id.as_deref() else {
                    continue;
                };
                if let Some(article) = self.atlas_article(run_id, &source.call_id)? {
                    articles.push(article);
                }
            }
            return Ok(build_claim_graph(&insight, &articles));
        }
        let mut plans = Vec::new();
        for run_id in run_ids {
            let Some(run) = self.get_run(&run_id)? else {
                continue;
            };
            let Some(raw) = run.plan_json.as_deref() else {
                continue;
            };
            if let Ok(plan) = serde_json::from_str::<Plan>(raw) {
                plans.push(plan);
            }
        }
        Ok(build_memory_graph(&insight, &plans))
    }

    fn build_brief_graph(&self, memory_id: &str) -> anyhow::Result<MemoryGraph> {
        let mut stmt = self.conn.prepare("SELECT member_id FROM memory_brief_memberships WHERE brief_id=?1 AND member_type='atomic_claim'")?;
        let members: Vec<String> = stmt.query_map([memory_id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        
        let mut graph = GraphBuilder::default();
        let brief_node = graph.node(GraphNode {
            id: format!("brief:{}", memory_id),
            kind: GraphNodeKind::Finding,
            label: "Cycle Brief".into(),
            detail: "Cycle Brief".into(),
            tags: vec![],
            article_id: String::new(),
            run_id: String::new(),
            published_at: String::new(),
        });
        
        for member_id in members {
            if let Ok(Some(insight)) = self.insight_for_memory(&member_id) {
                let claim_node = graph.node(GraphNode {
                    id: format!("finding:{}", member_id),
                    kind: GraphNodeKind::Finding,
                    label: format!("{} → {}", insight.predicate, insight.object_value),
                    detail: format!("{} · {} → {}", insight.entity, insight.predicate, insight.object_value),
                    tags: vec![],
                    article_id: String::new(),
                    run_id: String::new(),
                    published_at: String::new(),
                });
                graph.edge(GraphEdge {
                    from: claim_node.clone(),
                    to: brief_node.clone(),
                    kind: GraphEdgeKind::Supports,
                    directive: None,
                });
            }
        }
        
        Ok(MemoryGraph {
            nodes: graph.nodes,
            edges: graph.edges,
        })
    }
"""

graph_rs = re.sub(
    r'    pub fn graph_for_memory.*?Ok\(build_memory_graph\(&insight, &plans\)\)\n    \}',
    new_graph_for_memory.replace('\\', '\\\\'),
    graph_rs,
    flags=re.DOTALL
)

with open(GRAPH_FILE, "w") as f:
    f.write(graph_rs)

print("graph.rs patched.")
