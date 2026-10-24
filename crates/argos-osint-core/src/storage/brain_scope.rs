//! The Brain scope-analysis pool on SurrealDB (`brain_scope` on the
//! repository).
//!
//! [`enqueue`] stores the scope payload and queues one `brain_scope` task
//! per payload hash; [`drain`] claims them while elected and runs
//! [`analyze`]: inference generation and assessment, then link generation,
//! over a bounded topology snapshot (`storage::topology`). Inferences and
//! links are typed memories written through the publisher, so their graph
//! projection and index work join the same transaction.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::json;
use surrealdb::types::SurrealValue;

use super::actor_bind::add_memory_actor_ref_in;
use super::jobs::{self, rid, run, ClaimedTask, NewJob, NewTask};
use super::memory::{put_typed_in, write_then_index, MemoryWrite, TypedMemory};
use super::repo::Repository;
use super::tx::{run_tx, RetryPolicy};
use crate::brain_actors::{endpoint_key, hid, LINK_RELATIONS};
use crate::brain_scope::{
    generate_inferences_deterministic, generate_inferences_with_model,
    generate_links_deterministic, generate_links_with_model, resolve_node_to_endpoint,
    resolve_scope_secret, ScopeAnalysisPayload, ScopeAnalysisResult, JOB_KIND_BRAIN_SCOPE,
    TASK_KIND_BRAIN_SCOPE_ANALYZE,
};
use crate::brain_topology::{TopologyNodeKind, TopologySeed};
use crate::memory_metadata::{
    ActorRef, EndpointRef, IdentityState, InferencePayload, LinkPayload, MemoryKind, MemoryPayload,
};
use crate::secrets::ProviderSecret;

/// The scope pool (`tasks::POOL_BRAIN_SCOPE`).
pub const POOL: &str = crate::tasks::POOL_BRAIN_SCOPE;

pub const SCHEMA: &str = "
DEFINE TABLE IF NOT EXISTS brain_scope_request SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS payload_json ON brain_scope_request TYPE string;
DEFINE FIELD IF NOT EXISTS result_json ON brain_scope_request TYPE string DEFAULT '';
";

/// `enqueue_scope_analysis`: one job and task per payload hash. Returns the
/// job id; a repeated payload returns the existing job.
pub async fn enqueue(repo: &Repository, payload: &ScopeAnalysisPayload) -> Result<String> {
    let json = serde_json::to_string(payload)?;
    let hash = crate::nlp::cache::hash(&json);
    let short = &hash[..12.min(hash.len())];
    let (job_id, task_id) = (format!("job-scope-{short}"), format!("task-scope-{short}"));
    repo.db()
        .query("UPSERT $r SET payload_json = $json RETURN NONE")
        .bind(("r", rid("brain_scope_request", &task_id)))
        .bind(("json", json))
        .await?
        .check()?;
    if task_exists(repo, &task_id).await? {
        return Ok(job_id);
    }
    let now = Utc::now();
    let created = jobs::create_job(
        repo.db(),
        &NewJob {
            id: &job_id,
            kind: JOB_KIND_BRAIN_SCOPE,
            operation: TASK_KIND_BRAIN_SCOPE_ANALYZE,
            parent: None,
            created_at: now,
        },
        &[NewTask {
            id: &task_id,
            job: &job_id,
            pool: POOL,
            operation: TASK_KIND_BRAIN_SCOPE_ANALYZE,
            priority: 5,
            deps: Vec::new(),
            input_hash: &hash,
            created_at: now,
        }],
    )
    .await;
    if let Err(e) = created {
        if !task_exists(repo, &task_id).await? {
            anyhow::bail!("scope task: {e}");
        }
    }
    Ok(job_id)
}

async fn task_exists(repo: &Repository, task_id: &str) -> Result<bool> {
    let mut r = repo
        .db()
        .query("RETURN record::exists($t)")
        .bind(("t", rid("task", task_id)))
        .await?
        .check()?;
    Ok(r.take::<Option<bool>>(0usize)?.unwrap_or(false))
}

#[derive(Debug, SurrealValue)]
struct KindRow {
    memory_kind: String,
    lifecycle: String,
}

async fn kind(repo: &Repository, id: &str) -> Result<Option<(String, String)>> {
    let mut r = repo
        .db()
        .query("SELECT memory_kind, lifecycle FROM ONLY $m")
        .bind(("m", rid("memory", id)))
        .await?
        .check()?;
    let row: Option<KindRow> = r.take(0usize)?;
    Ok(row.map(|r| (r.memory_kind, r.lifecycle)))
}

/// `Store::is_browse_claim`: an active claim or atomic claim.
pub(crate) async fn is_claim(repo: &Repository, id: &str) -> Result<bool> {
    Ok(matches!(kind(repo, id).await?, Some((k, l))
        if l == "active" && (k == "claim" || k == "atomic_claim")))
}

async fn is_actor(repo: &Repository, id: &str) -> Result<bool> {
    Ok(matches!(kind(repo, id).await?, Some((k, l)) if l == "active" && k == "actor"))
}

fn actor_id(e: &EndpointRef) -> Option<String> {
    match e {
        EndpointRef::Memory {
            memory_id,
            kind: MemoryKind::Actor,
        } => Some(memory_id.clone()),
        _ => None,
    }
}

/// Write one typed memory with actor references through the publisher.
async fn write_typed(
    repo: &Repository,
    m: TypedMemory,
    source: serde_json::Value,
    actors: Vec<(String, &'static str)>,
) -> Result<()> {
    let source = source.to_string();
    let write: MemoryWrite = Arc::new(move |tx| {
        let (m, source, actors) = (m.clone(), source.clone(), actors.clone());
        Box::pin(async move {
            put_typed_in(&tx, &m).await?;
            run(
                &tx,
                tx.query("UPDATE $m SET category = $c, source_json = $s RETURN NONE")
                    .bind(("m", rid("memory", &m.id)))
                    .bind(("c", m.kind.as_str().to_string()))
                    .bind(("s", source)),
            )
            .await?;
            for (a, role) in &actors {
                add_memory_actor_ref_in(
                    &tx,
                    &m.id,
                    a,
                    role,
                    IdentityState::Provisional,
                    &json!({}),
                )
                .await?;
            }
            Ok(vec![m.id.clone()])
        })
    });
    write_then_index(repo.publisher(), write)
        .await
        .map_err(|e| anyhow::anyhow!("scope write: {e}"))?;
    Ok(())
}

/// `run_scope_analysis` on the repository.
pub async fn analyze(
    repo: &Repository,
    payload: &ScopeAnalysisPayload,
    secret: Option<&ProviderSecret>,
) -> Result<ScopeAnalysisResult> {
    let mut options = payload.options.clone();
    options.depth = payload.depth;
    options.cycle_scope = payload.cycle_scope.clone();
    let snapshot = super::topology::topology_snapshot(repo, &payload.seed, &options).await?;
    let claim_nodes: Vec<_> = snapshot
        .nodes
        .iter()
        .filter(|n| {
            n.kind == TopologyNodeKind::Claim
                && matches!(&n.topology_seed, Some(TopologySeed::Claim(id)) if id == &n.id)
        })
        .collect();
    let entity_nodes: Vec<_> = snapshot
        .nodes
        .iter()
        .filter(|n| {
            n.kind == TopologyNodeKind::Entity
                && matches!(&n.topology_seed, Some(TopologySeed::Actor(id)) if id == &n.id)
        })
        .collect();
    let receipt = format!("scope:{}", payload.seed.key());
    let source = json!({"app":"brain","conversation_id":"scope_analysis","message_id":null,
        "reference":payload.seed.key()});
    let mut result = ScopeAnalysisResult::default();

    let inferences = match secret {
        Some(sec) => match generate_inferences_with_model(sec, &claim_nodes, &entity_nodes).await {
            Ok(v) if !v.is_empty() => v,
            _ => generate_inferences_deterministic(&claim_nodes, &entity_nodes),
        },
        None => generate_inferences_deterministic(&claim_nodes, &entity_nodes),
    };
    for inf in inferences {
        if inf.premise_claims.is_empty()
            || !inf
                .premise_claims
                .iter()
                .all(|c| claim_nodes.iter().any(|n| n.id == *c))
        {
            continue;
        }
        let mut valid = true;
        for p in &inf.premise_claims {
            valid &= is_claim(repo, p).await?;
        }
        for a in &inf.actors {
            valid &= entity_nodes.iter().any(|n| n.id == *a) && is_actor(repo, a).await?;
        }
        let state = inf.support_state.trim().to_lowercase();
        if !valid || (state != "supported" && state != "partially_supported") {
            continue;
        }
        let id = crate::store::new_id();
        let p = InferencePayload {
            premise_claims: inf.premise_claims.clone(),
            actors: inf
                .actors
                .iter()
                .map(|a| ActorRef {
                    actor_id: a.clone(),
                    role: "subject".into(),
                    binding_state: IdentityState::Provisional,
                })
                .collect(),
            reasoning: inf.reasoning.clone(),
            limitations: inf.limitations.clone(),
            support_state: inf.support_state.clone(),
            coverage_receipt: receipt.clone(),
        };
        write_typed(
            repo,
            TypedMemory {
                id: id.clone(),
                text: inf.text.clone(),
                kind: MemoryKind::Inference,
                payload: Some(MemoryPayload::Inference(p)),
                lifecycle: "active".into(),
            },
            source.clone(),
            inf.actors.iter().map(|a| (a.clone(), "subject")).collect(),
        )
        .await?;
        result.inferences_created += 1;
        result.details.push(format!("Created inference {id}"));
    }

    let link_nodes: Vec<_> = snapshot
        .nodes
        .iter()
        .filter(|n| resolve_node_to_endpoint(&n.id, &snapshot.nodes).is_some())
        .cloned()
        .collect();
    let links = match secret {
        Some(sec) => match generate_links_with_model(sec, &link_nodes).await {
            Ok(v) if !v.is_empty() => v,
            _ => generate_links_deterministic(&claim_nodes, &entity_nodes),
        },
        None => generate_links_deterministic(&claim_nodes, &entity_nodes),
    };
    for link in links {
        let rel = link.relation.trim();
        if !LINK_RELATIONS.contains(&rel) {
            continue;
        }
        let id = hid(
            "link",
            &[
                rel,
                &endpoint_key(&link.source),
                &endpoint_key(&link.target),
            ],
        );
        // Deterministic id: an existing link is unchanged.
        if kind(repo, &id).await?.is_some() {
            continue;
        }
        let subjects: Vec<(String, &'static str)> = [&link.source, &link.target]
            .iter()
            .filter_map(|e| actor_id(e))
            .map(|a| (a, "endpoint"))
            .collect();
        let p = LinkPayload {
            source: link.source,
            target: link.target,
            relation: rel.to_string(),
            rationale: link.rationale,
            receipts: vec![receipt.clone()],
            promoted_from: None,
        };
        if write_typed(
            repo,
            TypedMemory {
                id: id.clone(),
                text: String::new(),
                kind: MemoryKind::Link,
                payload: Some(MemoryPayload::Link(p)),
                lifecycle: "active".into(),
            },
            json!({}),
            subjects,
        )
        .await
        .is_ok()
        {
            result.links_created += 1;
            result.details.push(format!("Created link {id}"));
        }
    }
    Ok(result)
}

#[derive(Debug, SurrealValue)]
struct Req {
    payload_json: String,
}

/// End `claimed` under its lease fence (`completed` or `failed`).
async fn end(repo: &Repository, claimed: &ClaimedTask, ok: bool, out: &str) -> Result<bool> {
    let now = jobs::ts(Utc::now());
    let out = out.to_string();
    let done = run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let (now, out) = (now.clone(), out.clone());
        async move {
            jobs::bump(&tx, "job_guard", &claimed.root_id).await?;
            match jobs::fence(&tx, claimed, &now).await? {
                Some(f) if jobs::holds(&f, claimed) => {}
                _ => return Ok(false),
            }
            if ok {
                let hash = crate::nlp::cache::hash(&out);
                jobs::complete_in(&tx, claimed, &hash, &now).await?;
                run(
                    &tx,
                    tx.query("UPDATE $r SET result_json = $out RETURN NONE")
                        .bind(("r", rid("brain_scope_request", &claimed.id)))
                        .bind(("out", out)),
                )
                .await?;
                return Ok(true);
            }
            run(
                &tx,
                tx.query(
                    "UPDATE $t SET state = IF attempts >= 3 THEN 'failed' ELSE 'queued' END,
                       lease_owner = '', process_owner = '', lease_until = NONE,
                       next_eligible_at = <datetime>$now + 30s,
                       finished_at = IF attempts >= 3 THEN <datetime>$now ELSE NONE END,
                       updated_at = <datetime>$now RETURN NONE;
                     LET $open = SELECT VALUE id FROM attempt WHERE task = $t AND epoch = $e
                       AND finished_at = NONE;
                     UPDATE $open SET finished_at = <datetime>$now, outcome = 'failed' RETURN NONE;",
                )
                .bind(("t", rid("task", &claimed.id)))
                .bind(("now", now))
                .bind(("e", claimed.lease_epoch)),
            )
            .await?;
            Ok(true)
        }
    })
    .await?;
    Ok(done.value)
}

/// `drain_scope_tasks` on the repository: up to four tasks while not
/// stopped (the caller holds the scheduler election). A failure is retried
/// up to three attempts.
pub async fn drain(repo: &Repository, owner: &str, stop: &AtomicBool) -> Result<usize> {
    let mut drained = 0usize;
    while !stop.load(Ordering::Acquire) && drained < 4 {
        let now = Utc::now();
        let op_key = format!(
            "scope-claim-{owner}-{}-{drained}",
            now.timestamp_nanos_opt().unwrap_or(0)
        );
        let Some(claimed) = jobs::claim_next(
            repo.db(),
            POOL,
            owner,
            crate::scheduler::DEFAULT_LEASE_SECS,
            now,
            &op_key,
        )
        .await
        .map_err(|e| anyhow::anyhow!("scope claim: {e}"))?
        .value
        else {
            break;
        };
        let result = async {
            let mut r = repo
                .db()
                .query("SELECT payload_json FROM ONLY $r")
                .bind(("r", rid("brain_scope_request", &claimed.id)))
                .await?
                .check()?;
            let req: Req = r
                .take::<Option<Req>>(0usize)?
                .context("scope payload missing")?;
            let payload: ScopeAnalysisPayload =
                serde_json::from_str(&req.payload_json).context("parse scope payload")?;
            let auth = crate::secrets::AuthFile::load().unwrap_or_default();
            let settings = crate::provider::SettingsFile::load().unwrap_or_default();
            let secret = resolve_scope_secret(&auth, &settings);
            let analysis = analyze(repo, &payload, secret.as_ref()).await?;
            serde_json::to_string(&analysis).context("serialize analysis result")
        }
        .await;
        match result {
            Ok(out) => end(repo, &claimed, true, &out).await?,
            Err(_) => end(repo, &claimed, false, "").await?,
        };
        drained += 1;
    }
    Ok(drained)
}
