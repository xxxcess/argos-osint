//! Immutable derivation history and transaction-scoped publication fencing.
use anyhow::{ensure, Result};
use rusqlite::{params, Connection, OptionalExtension};

use super::insights::{EvidenceSnapshot, CONTRACT, READINESS_POLICY};
use crate::tasks::ClaimedTask;

use super::insights::{PipelineInput, PipelineOutput};

pub fn article_input(
    store: &crate::store::Store,
    article: &crate::store::AtlasArticleRow,
    generation: u32,
    objective: &str,
    include_tools: bool,
) -> Result<PipelineInput> {
    use super::insights::{EvidenceLocation, EvidenceRef, RetainedEvidence};
    let mut evidence = Vec::new();
    if let Some(body) = store.article_body_for_article(&article.id)? {
        // Only the retained acquisition can establish story coverage. A refined
        // display body and a legacy bounded body cannot certify completeness.
        let acquisition:Option<(String,String,String,String)>=store.conn.query_row("SELECT raw_hash,prose_hash,attempt_id,revision FROM nlp_source_refs WHERE owner_kind='article' AND owner_id=?1 AND source_id=?2 AND transform_version='validated-baseline-v1' AND revision=?3 AND raw_hash IS NOT NULL ORDER BY rowid DESC LIMIT 1",params![article.id,body.id,body.body_version.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        if let Some((raw_hash, baseline_hash, attempt, revision)) = acquisition {
            // Display and extraction share the Stage-1 clean artifact derived
            // from this same acquisition; fall back to the validated baseline.
            let clean: Option<String> = store.conn.query_row("SELECT prose_hash FROM nlp_source_refs WHERE owner_kind='article' AND owner_id=?1 AND source_id=?2 AND transform_version=?3 AND revision=?4 AND attempt_id=?5 AND raw_hash=?6 ORDER BY rowid DESC LIMIT 1",params![article.id,body.id,crate::intel_recon::clean_body::TRANSFORM,revision,attempt,raw_hash],|r|r.get(0)).optional()?;
            let prose_hash = clean.unwrap_or(baseline_hash);
            let text: Option<String> = store
                .conn
                .query_row(
                    "SELECT text FROM nlp_artifacts WHERE hash=?1",
                    [&prose_hash],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(text) = text {
                let quality =
                    crate::intel_recon::validate_article_body(&text, &article.title, &article.url);
                let acquisition_coverage = store
                    .app_state_get(&format!("article-acquisition:{attempt}"))?
                    .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok());
                let complete = quality.quality == crate::intel_recon::BodyQuality::Complete
                    && acquisition_coverage.as_ref().is_some_and(|coverage| {
                        coverage
                            .get("artifact_hash")
                            .and_then(serde_json::Value::as_str)
                            == Some(raw_hash.as_str())
                            && coverage
                                .get("complete_story")
                                .and_then(serde_json::Value::as_bool)
                                == Some(true)
                            && coverage
                                .get("known_truncation")
                                .and_then(serde_json::Value::as_bool)
                                == Some(false)
                    });
                let reference = EvidenceRef {
                    source_id: format!("body:{}", body.id),
                    revision,
                    artifact_hash: prose_hash,
                    acquisition_id: attempt,
                    origin_root: format!("acquisition:{raw_hash}"),
                    url: article.url.clone(),
                    published_at: Some(article.published_at.clone()),
                    retrieved_at: body.fetched_at.clone(),
                    location: EvidenceLocation::Range(crate::nlp::Span {
                        start_byte: 0,
                        end_byte: text.len(),
                    }),
                };
                evidence.push(RetainedEvidence {
                    reference,
                    text,
                    native: None,
                    substantive: true,
                    relevant: true,
                    complete_story: complete,
                    known_truncation: !complete,
                });
            }
        }
    }
    // Metadata is an independent retained passage, with its own byte offsets.
    {
        let text = format!("{}\n{}", article.title, article.description);
        let artifact = crate::nlp::cache::artifact(&store.conn, &text)?;
        evidence.push(RetainedEvidence {
            reference: EvidenceRef {
                source_id: format!("article:{}", article.id),
                revision: artifact.clone(),
                artifact_hash: artifact,
                acquisition_id: article.id.clone(),
                origin_root: String::new(),
                url: article.url.clone(),
                published_at: Some(article.published_at.clone()),
                retrieved_at: article.seen_at.clone(),
                location: EvidenceLocation::Range(crate::nlp::Span {
                    start_byte: 0,
                    end_byte: text.len(),
                }),
            },
            text,
            native: None,
            substantive: false,
            relevant: true,
            complete_story: false,
            known_truncation: true,
        });
    }
    if include_tools {
        let mut stmt=store.conn.prepare("SELECT DISTINCT c.id,c.result_json FROM osint_calls c JOIN intel_evidence e ON e.call_id=c.id JOIN intel_investigations i ON i.id=e.investigation_id WHERE i.article_id=?1 AND c.status='completed' AND c.result_json IS NOT NULL ORDER BY c.id")?;
        let results = stmt
            .query_map([&article.id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let results = results
            .into_iter()
            .filter_map(|(id, json)| serde_json::from_str(&json).ok().map(|r| (id, r)))
            .collect::<Vec<_>>();
        evidence.extend(super::insights::evidence_from_tools(&store.conn, &results)?);
    }
    let mut input = PipelineInput {
        snapshot: super::insights::snapshot("article", &article.id, generation, &evidence),
        evidence,
        objective: objective.into(),
        surface: "article".into(),
        required_tasks_complete: true,
        save_insights: true,
        recall_enabled: false,
    };
    if let Some(metadata) = store.atlas_metadata(&article.run_id, &article.id)? {
        for actor in crate::atlas_metadata::article_actors(article, &metadata) {
            input.snapshot.entities.push(crate::nlp::EntityCandidate {
                id: actor.id,
                type_hint: actor.type_hint,
                comparison_key: actor.display_name.to_lowercase(),
                original_forms: actor.spellings,
                mention_refs: actor
                    .mentions
                    .iter()
                    .map(|mention| {
                        let offset = if mention.field == "description" {
                            article.title.len() + 1
                        } else {
                            0
                        };
                        crate::nlp::Span {
                            start_byte: offset + mention.start_byte,
                            end_byte: offset + mention.end_byte,
                        }
                    })
                    .collect(),
            });
        }
    }
    input.required_tasks_complete =
        !article.title.trim().is_empty() && !article.description.trim().is_empty();
    Ok(input)
}

/// Only a retained complete report is supplied to this builder. Article fields,
/// tools, old snapshots and recall are deliberately absent from the payload.
pub fn report_input(
    conn: &Connection,
    article_id: &str,
    generation: u32,
) -> Result<Option<PipelineInput>> {
    use super::insights::{EvidenceLocation, EvidenceRef, RetainedEvidence};
    let result: Option<(String, String, String, i64)> = conn.query_row(
        "SELECT job_id,source_hash,details_json,completion_sequence FROM intel_report_results WHERE article_id=?1 AND completion_sequence>0 ORDER BY completion_sequence DESC LIMIT 1",
        [article_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
    let Some((job, source_hash, details, sequence)) = result else {
        return Ok(None);
    };
    let bundle: serde_json::Value = serde_json::from_str(&details)?;
    ensure!(
        crate::nlp::cache::hash(&details) == source_hash,
        "report bundle hash mismatch"
    );
    let text = bundle["text"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("report text missing"))?
        .to_owned();
    ensure!(!text.trim().is_empty(), "empty report details");
    let mut retained = crate::nlp::Source::derived(
        &format!("report:{job}"),
        &format!("{sequence}:{source_hash}"),
        &text,
    );
    retained.attempt_id = source_hash.clone();
    crate::nlp::cache::register(
        conn,
        "report_result",
        &job,
        &retained,
        None,
        "report-details-v1",
    )?;
    let artifact = crate::nlp::cache::artifact(conn, &text)?;
    let evidence = vec![RetainedEvidence {
        reference: EvidenceRef {
            source_id: format!("report:{job}"),
            revision: format!("{sequence}:{source_hash}"),
            artifact_hash: artifact,
            acquisition_id: source_hash,
            origin_root: String::new(),
            url: String::new(),
            published_at: None,
            retrieved_at: bundle["completed_at"].as_str().unwrap_or_default().into(),
            location: EvidenceLocation::Range(crate::nlp::Span {
                start_byte: 0,
                end_byte: text.len(),
            }),
        },
        text,
        native: None,
        substantive: true,
        relevant: true,
        complete_story: true,
        known_truncation: false,
    }];
    Ok(Some(PipelineInput {
        snapshot: super::insights::snapshot("article", article_id, generation, &evidence), evidence,
        objective: "Extract attributed assertions and justified conclusions exclusively from this completed Recon report. Report prose is not independent corroboration.".into(),
        surface: "recon_report".into(), required_tasks_complete: true, save_insights: true, recall_enabled: false,
    }))
}

pub fn article_details_eligible(input: &PipelineInput) -> bool {
    input.required_tasks_complete
        && input.evidence.iter().any(|e| {
            e.reference.source_id.starts_with("body:")
                && e.substantive
                && e.complete_story
                && !e.known_truncation
        })
}

pub fn article_refresh_status(conn: &Connection, article_id: &str) -> Result<String> {
    let state: Option<(String, String, String)> = conn.query_row("SELECT h.lifecycle,h.last_error,COALESCE((SELECT t.state FROM argos_tasks t WHERE t.job_id=h.refresh_job_id ORDER BY t.created_at DESC LIMIT 1),'') FROM insight_owner_state h WHERE h.owner_kind='article' AND h.owner_id=?1",[article_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    let Some((lifecycle, error, task)) = state else {
        return Ok("Waiting · complete article details required".into());
    };
    if !error.is_empty() {
        return Ok(format!("Previous · {error}"));
    }
    if task == "running" {
        let progress = super::insight_execution::owner_progress(conn, "article", article_id)?;
        return Ok(progress
            .map(|p| {
                super::insight_execution::status_step(
                    &p.state,
                    &p.stage,
                    &p.activity,
                    p.activity_age_secs,
                    p.index_status.starts_with("Indexed"),
                )
            })
            .unwrap_or_else(|| "Waiting for output".into()));
    }
    if matches!(task.as_str(), "queued" | "retry_scheduled") {
        return Ok("Queued".into());
    }
    if lifecycle == "active" {
        let pending: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM argos_index_changes WHERE state<>'completed' AND record_id IN (SELECT d.memory_id FROM insight_derivations d JOIN insight_owner_state h ON h.active_snapshot_id=d.snapshot_id WHERE h.owner_kind='article' AND h.owner_id=?1))",[article_id],|r|r.get(0))?;
        return Ok(if pending { "Published" } else { "Indexed" }.into());
    }
    Ok(format!("Waiting · {lifecycle}"))
}

/// Shared admission for visits, complete body commits, durable report commits
/// and recovery. Deduplicate before changing the desired generation.
pub fn schedule_article_refresh(
    store: &crate::store::Store,
    article: &crate::store::AtlasArticleRow,
    settings: &crate::provider::SettingsFile,
    visit: bool,
) -> Result<Option<String>> {
    let own = store.conn.is_autocommit();
    if own {
        store.conn.execute_batch("BEGIN IMMEDIATE")?;
    }
    let result = schedule_article_refresh_on(store, article, settings, visit);
    if own {
        if result.is_ok() {
            store.conn.execute_batch("COMMIT")?;
        } else {
            store.conn.execute_batch("ROLLBACK")?;
        }
    }
    result
}

fn schedule_article_refresh_on(
    store: &crate::store::Store,
    article: &crate::store::AtlasArticleRow,
    settings: &crate::provider::SettingsFile,
    visit: bool,
) -> Result<Option<String>> {
    let now = chrono::Utc::now().to_rfc3339();
    if visit {
        store.conn.execute(
            "INSERT OR IGNORE INTO article_refresh_intents(article_id,requested_at) VALUES(?1,?2)",
            params![article.id, now],
        )?;
    }
    let report = report_input(&store.conn, &article.id, 0)?;
    if report.is_none()
        && store.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM intel_report_results WHERE article_id=?1)",
            [&article.id],
            |r| r.get::<_, bool>(0),
        )?
    {
        return Ok(None);
    }

    let mut input = if let Some(input) = report {
        input
    } else {
        let intent: bool = store.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM article_refresh_intents WHERE article_id=?1)",
            [&article.id],
            |r| r.get(0),
        )?;
        if !intent {
            return Ok(None);
        }
        let input = article_input(store, article, 0, "Extract attributed assertions from the complete article title, description and retained cleaned story.", false)?;
        if !article_details_eligible(&input) {
            return Ok(None);
        }
        input
    };
    // The generation is excluded only for admission identity, never publication.
    let identity = super::insights::input_key(&input, settings);
    let expired: bool = store.conn.query_row("SELECT EXISTS(SELECT 1 FROM article_refresh_retirements WHERE article_id=?1 AND input_key=?2)",params![article.id,identity],|r|r.get(0))?;
    if expired {
        return Ok(None);
    }
    let prior: Option<(String, String,String,String)> = store.conn.query_row(
        "SELECT id,input_ref,role_snapshot,input_hash FROM argos_tasks WHERE operation='insight_refresh' AND job_id IN (SELECT id FROM argos_jobs WHERE owner_scope=?1) ORDER BY created_at DESC,rowid DESC LIMIT 1",
        [format!("article:{}",article.id)], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
    let mut reason = ("initial refresh".to_string(), serde_json::json!({}));
    if let Some((id, json, captured, stored_hash)) = &prior {
        if let Ok(mut prior) = serde_json::from_str::<PipelineInput>(json) {
            let captured = serde_json::from_str::<crate::provider::SettingsFile>(captured)?;
            let compatible = super::insights::input_key(&prior, &captured) == *stored_hash;
            prior.snapshot.generation = 0;
            if compatible && super::insights::input_key(&prior, &captured) == identity {
                return Ok(Some(id.clone()));
            }
            reason = if compatible {
                supersession_reason(&prior, &captured, &input, settings)
            } else {
                (
                    "preparation contract changed".into(),
                    serde_json::json!({"contract":"stored fingerprint is from an older contract"}),
                )
            };
        } else {
            reason = ("preparation contract changed".into(), serde_json::json!({}));
        }
    }
    input.snapshot.generation = advance_generation(&store.conn, "article", &article.id, &now)?;
    let task = enqueue_refresh(&store.conn, &mut input, settings, &now)?;
    retire_replaced(
        &store.conn,
        "article",
        &article.id,
        &input,
        &task,
        &reason.0,
        &reason.1,
        &now,
    )?;
    Ok(Some(task))
}

/// Why a new generation was needed, comparing the prior task's input with its
/// own captured settings against the new input with current settings.
pub fn supersession_reason(
    prior: &PipelineInput,
    prior_settings: &crate::provider::SettingsFile,
    next: &PipelineInput,
    settings: &crate::provider::SettingsFile,
) -> (String, serde_json::Value) {
    if prior.surface != next.surface && next.surface == "recon_report" {
        return (
            "completed report replaces article details".into(),
            serde_json::json!({"from":prior.surface,"to":next.surface}),
        );
    }
    let mut a = prior.snapshot.clone();
    let mut b = next.snapshot.clone();
    a.key.clear();
    b.key.clear();
    a.generation = 0;
    b.generation = 0;
    let sa = serde_json::to_value(&a).unwrap_or_default();
    let sb = serde_json::to_value(&b).unwrap_or_default();
    let (ea, eb) = (
        serde_json::to_value(&prior.evidence).unwrap_or_default(),
        serde_json::to_value(&next.evidence).unwrap_or_default(),
    );
    if sa != sb || ea != eb {
        let changed: Vec<String> = sb
            .as_object()
            .map(|m| {
                m.iter()
                    .filter(|(k, v)| sa.get(k.as_str()) != Some(v))
                    .map(|(k, _)| k.clone())
                    .collect()
            })
            .unwrap_or_default();
        return (
            "source changed".into(),
            serde_json::json!({"fields":changed,"evidence_changed":ea!=eb}),
        );
    }
    let roles = |s: &crate::provider::SettingsFile| {
        serde_json::to_value(
            super::insights::INSIGHT_ROLES
                .iter()
                .map(|r| (r.to_string(), s.defaults.resolve_role(r).0))
                .collect::<std::collections::BTreeMap<_, _>>(),
        )
        .unwrap_or_default()
    };
    let (ra, rb) = (roles(prior_settings), roles(settings));
    if ra != rb {
        let changed: Vec<String> = rb
            .as_object()
            .map(|m| {
                m.iter()
                    .filter(|(k, v)| ra.get(k.as_str()) != Some(v))
                    .map(|(k, _)| k.clone())
                    .collect()
            })
            .unwrap_or_default();
        return (
            "model routes changed".into(),
            serde_json::json!({"roles":changed}),
        );
    }
    ("preparation contract changed".into(), serde_json::json!({}))
}

/// Retire every older live task for this owner: queued work is superseded
/// immediately; running work is recorded here and retired at its next fence
/// (admission, pre-send dispatch check, heartbeat or publication).
#[allow(clippy::too_many_arguments)]
fn retire_replaced(
    conn: &Connection,
    kind: &str,
    owner: &str,
    next: &PipelineInput,
    next_task: &str,
    reason: &str,
    detail: &serde_json::Value,
    now: &str,
) -> Result<()> {
    let next_job: String = conn.query_row(
        "SELECT job_id FROM argos_tasks WHERE id=?1",
        [next_task],
        |r| r.get(0),
    )?;
    let mut stmt = conn.prepare("SELECT t.id,t.job_id,t.state,t.source_revision FROM argos_tasks t JOIN argos_jobs j ON j.id=t.job_id WHERE j.owner_scope=?1 AND t.operation='insight_refresh' AND t.id<>?2 AND t.state IN ('queued','retry_scheduled','running','paused')")?;
    let rows = stmt
        .query_map(params![format!("{kind}:{owner}"), next_task], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (task, job, state, revision) in rows {
        conn.execute("INSERT OR REPLACE INTO insight_supersessions(task_id,job_id,owner_kind,owner_id,generation,replaced_by_job,replaced_by_task,replaced_by_generation,reason,detail_json,at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![task,job,kind,owner,revision.parse::<i64>().unwrap_or(0),next_job,next_task,next.snapshot.generation,reason,detail.to_string(),now])?;
        if state != "running" {
            conn.execute("UPDATE argos_tasks SET state='superseded',error_message=?2,error_category='',updated_at=?3 WHERE id=?1",
                params![task, format!("Superseded · replaced by {next_job} · {reason}"), now])?;
            crate::tasks::refresh_job_state(conn, &job, now)?;
        }
        let _ = crate::events::record_event(
            conn,
            &crate::events::NewEvent {
                severity: Some(crate::events::Severity::Info),
                app: if kind == "article" {
                    "intel".into()
                } else {
                    kind.into()
                },
                event_type: "insight.superseded".into(),
                message: format!("Superseded · replaced by {next_job} · {reason}"),
                details: detail.to_string(),
                job_id: job,
                task_id: task,
                resource_ref: owner.into(),
                ..Default::default()
            },
        );
    }
    Ok(())
}

/// Supersession record for a task: (replacement job, reason).
pub fn supersession(conn: &Connection, task_id: &str) -> Option<(String, String)> {
    conn.query_row(
        "SELECT replaced_by_job,reason FROM insight_supersessions WHERE task_id=?1",
        [task_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
    .ok()
    .flatten()
}

/// True when the owner's desired generation moved past `generation`.
pub fn is_superseded(conn: &Connection, input: &PipelineInput) -> bool {
    desired_generation(conn, &input.snapshot.owner_kind, &input.snapshot.owner_id)
        .is_ok_and(|g| g != input.snapshot.generation)
}

pub fn active_output(conn: &Connection, kind: &str, owner: &str) -> Result<Option<PipelineOutput>> {
    let head:Option<(String,String)>=conn.query_row("SELECT active_snapshot_id,lifecycle FROM insight_owner_state WHERE owner_kind=?1 AND owner_id=?2",params![kind,owner],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    let Some((snapshot, lifecycle)) = head.filter(|(id, _)| !id.is_empty()) else {
        return Ok(None);
    };
    let mut output = PipelineOutput::default();
    let entities: String = conn.query_row(
        "SELECT entities_json FROM insight_snapshots WHERE id=?1",
        [&snapshot],
        |r| r.get(0),
    )?;
    output.entities = serde_json::from_str(&entities)?;
    let readiness: String = conn.query_row(
        "SELECT readiness_json FROM insight_snapshots WHERE id=?1",
        [&snapshot],
        |r| r.get(0),
    )?;
    let stored: serde_json::Value = serde_json::from_str(&readiness)?;
    output.limit_reached = stored["limit_reached"].as_bool().unwrap_or(false);
    output.partial_enumeration = stored["partial_enumeration"].as_bool().unwrap_or(false);
    output.item_gaps = serde_json::from_value(stored["item_gaps"].clone()).unwrap_or_default();
    output.withheld_count = stored["withheld_count"].as_u64().unwrap_or(0) as usize;
    if let Some(value) = stored.get("readiness") {
        output.readiness = serde_json::from_value(value.clone())?;
    }
    output.readiness.input_eligible = lifecycle == "active";
    output.readiness.publication_eligible = lifecycle == "active";
    if lifecycle != "active" {
        output
            .readiness
            .gaps
            .push(format!("insight lifecycle: {lifecycle}"));
    }
    let mut stmt = conn.prepare(
        "SELECT insight_kind,item_json FROM insight_derivations WHERE snapshot_id=?1 AND status<>'deleted' ORDER BY id",
    )?;
    for row in stmt.query_map([&snapshot], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })? {
        let (kind, json) = row?;
        if kind == "claim" {
            output.claims.push(serde_json::from_str(&json)?);
        } else if kind == "inference" {
            output.inferences.push(serde_json::from_str(&json)?);
        }
    }
    let mut links = conn.prepare("SELECT id,left_kind,left_id,right_kind,right_id,relation,rationale,assessment_json,evidence_json FROM insight_derivation_links WHERE snapshot_id=?1 AND status='assessed' ORDER BY id")?;
    for row in links.query_map([&snapshot], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, String>(6)?,
            r.get::<_, String>(7)?,
            r.get::<_, String>(8)?,
        ))
    })? {
        let (id, lk, li, rk, ri, relation, rationale, assessment, evidence) = row?;
        output.links.push(super::insights::AssessedLink {
            id,
            typed_endpoints: vec![
                super::insights::TypedEndpoint { kind: lk, id: li },
                super::insights::TypedEndpoint { kind: rk, id: ri },
            ],
            relation,
            rationale,
            assessment: serde_json::from_str(&assessment)?,
            evidence_refs: serde_json::from_str(&evidence)?,
            snapshot_key: conn.query_row(
                "SELECT input_key FROM insight_snapshots WHERE id=?1",
                [&snapshot],
                |r| r.get(0),
            )?,
        });
    }
    Ok(Some(output))
}

pub fn article_claim_rows(
    conn: &Connection,
    article_id: &str,
) -> Result<Option<Vec<crate::store::AtlasArticleClaim>>> {
    owner_claim_rows(conn, "article", article_id)
}

pub fn atlas_claim_rows(
    conn: &Connection,
    article_id: &str,
) -> Result<Option<Vec<crate::store::AtlasArticleClaim>>> {
    owner_claim_rows(conn, "atlas", article_id)
}

fn owner_claim_rows(
    conn: &Connection,
    kind: &str,
    article_id: &str,
) -> Result<Option<Vec<crate::store::AtlasArticleClaim>>> {
    let Some(output) = active_output(conn, kind, article_id)? else {
        return Ok(None);
    };
    let lifecycle: String = conn.query_row(
        "SELECT lifecycle FROM insight_owner_state WHERE owner_kind=?2 AND owner_id=?1",
        params![article_id, kind],
        |r| r.get(0),
    )?;
    let mut rows = Vec::new();
    let mut push = |fp: String,
                    subject: &str,
                    predicate: &str,
                    object: &str,
                    text: &str,
                    kind: &str,
                    refs: &[super::insights::EvidenceRef],
                    assessment: &super::insights::Assessment|
     -> Result<()> {
        if !assessment.faithful() {
            return Ok(());
        }
        let memory:Option<(String,String)>=conn.query_row("SELECT m.id,m.text FROM insight_claims c JOIN memories m ON m.id=c.memory_id WHERE c.fingerprint=?1",[&fp],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        if memory.is_none() && crate::store::tombstone_runs(conn, &fp)?.is_some() {
            return Ok(());
        }
        let edited = memory.as_ref().is_some_and(|(_, current)| current != text);
        let text = memory
            .as_ref()
            .map(|(_, text)| text.as_str())
            .unwrap_or(text);
        let status = if edited {
            "user_edited · unassessed".into()
        } else if lifecycle == "pending" || lifecycle == "previous" {
            format!("previous · {lifecycle}")
        } else {
            assessment.relation.clone()
        };
        let entity = output
            .entities
            .iter()
            .find(|e| e.id == subject)
            .and_then(|e| e.original_forms.first())
            .cloned()
            .unwrap_or_else(|| subject.into());
        rows.push(crate::store::AtlasArticleClaim {
            fingerprint: fp,
            entity,
            predicate: predicate.into(),
            object: object.into(),
            topic: "investigation".into(),
            classification: kind.into(),
            confidence: 0.0,
            claim: format!("[{kind} · {}] {text}", status.replace('_', " ")),
            source_url: refs.first().map(|r| r.url.clone()).unwrap_or_default(),
            published_at: refs
                .first()
                .and_then(|r| r.published_at.clone())
                .unwrap_or_default(),
            article_id: article_id.into(),
            reliability: String::new(),
            info_credibility: 0,
            admiralty: String::new(),
            rsp_status: String::new(),
        });
        Ok(())
    };
    for c in &output.claims {
        push(
            super::insights::claim_fingerprint(c),
            &c.subject_ref,
            &c.predicate,
            &c.object,
            &c.text,
            "claim",
            &c.evidence_refs,
            &c.assessment,
        )?;
    }
    for i in &output.inferences {
        push(
            super::insights::inference_fingerprint(i),
            i.entity_refs.first().map(String::as_str).unwrap_or(""),
            "derived",
            &i.text,
            &i.text,
            "inference",
            &i.evidence_refs,
            &i.assessment,
        )?;
    }
    Ok(Some(rows))
}

pub fn enqueue_refresh(
    conn: &Connection,
    input: &mut PipelineInput,
    settings: &crate::provider::SettingsFile,
    now: &str,
) -> Result<String> {
    ensure!(
        input.surface != "atlas" && input.snapshot.owner_kind != "atlas",
        "Atlas cycles cannot enqueue insight refreshes"
    );
    input.snapshot.key = super::insights::input_key(input, settings);
    let job_id = format!(
        "insight-refresh:{}:{}:{}",
        input.snapshot.owner_kind, input.snapshot.owner_id, input.snapshot.generation
    );
    let task_id = format!("{job_id}:{}", input.snapshot.key);
    crate::tasks::enqueue_job_with(
        conn,
        &crate::tasks::NewJob {
            id: job_id.clone(),
            kind: "insight_refresh".into(),
            owner_scope: format!("{}:{}", input.snapshot.owner_kind, input.snapshot.owner_id),
            input_revision: input.snapshot.key.clone(),
            deadline_at: String::new(),
        },
        &crate::tasks::JobMeta {
            app: if input.snapshot.owner_kind == "article" {
                "intel".into()
            } else {
                input.snapshot.owner_kind.clone()
            },
            operation: "insight_refresh".into(),
            resource_ref: input.snapshot.owner_id.clone(),
            title: "Assess and refresh investigative insights".into(),
            ..Default::default()
        },
        now,
    )?;
    crate::tasks::enqueue_task(
        conn,
        &crate::tasks::NewTask {
            id: task_id.clone(),
            job_id: job_id.clone(),
            operation: "insight_refresh".into(),
            dedupe_key: task_id.clone(),
            priority: 20,
            input_ref: serde_json::to_string(input)?,
            input_hash: input.snapshot.key.clone(),
            source_revision: input.snapshot.generation.to_string(),
            // Capture analytical routes/preparation, never provider/tool keys.
            role_snapshot: serde_json::to_string(&crate::provider::SettingsFile {
                defaults: settings.defaults.clone(),
                nlp: settings.nlp.clone(),
                ..Default::default()
            })?,
            max_attempts: 1,
        },
        now,
    )?;
    conn.execute("UPDATE insight_owner_state SET refresh_job_id=?4 WHERE owner_kind=?1 AND owner_id=?2 AND desired_generation=?3",params![input.snapshot.owner_kind,input.snapshot.owner_id,input.snapshot.generation,job_id])?;
    // Explicit enqueue on resume may restart cancellation, but never an
    // exhausted provider chain. Startup recovery does not claim paused work.
    conn.execute("UPDATE argos_tasks SET state='queued',blocked_reason='',updated_at=?2 WHERE id=?1 AND operation='insight_refresh' AND state='paused' AND blocked_reason='cancelled; evidence dirty'",params![task_id,now])?;
    Ok(task_id)
}

/// Claims only our exact refresh task; it never consumes arbitrary LLM-pool work.
pub async fn run_refresh_slice(
    db_path: &std::path::Path,
    task_id: &str,
    auth: &crate::secrets::AuthFile,
    settings: &crate::provider::SettingsFile,
    cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<Option<PipelineOutput>> {
    let claimed = {
        let store = crate::store::Store::open(db_path)?;
        crate::tasks::retire_atlas_insight_work(&store.conn, &chrono::Utc::now().to_rfc3339())?;
        let is_refresh: bool = store.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM argos_tasks WHERE id=?1 AND operation='insight_refresh')",
            [task_id],
            |r| r.get(0),
        )?;
        ensure!(is_refresh, "not an insight refresh task");
        let Some(task) = crate::tasks::claim_task(
            &store.conn,
            task_id,
            &format!("insight-worker:{}", std::process::id()),
            120,
            &chrono::Utc::now().to_rfc3339(),
        )?
        else {
            return Ok(None);
        };
        task
    };
    let mut input: PipelineInput = serde_json::from_str(&claimed.input_ref)?;
    ensure!(
        input.surface != "atlas" && input.snapshot.owner_kind != "atlas",
        "Atlas insight refresh retired"
    );
    let admission = {
        let store = crate::store::Store::open(db_path)?;
        validate_live_evidence(&store.conn, &input.snapshot).and_then(|()| {
            ensure!(
                desired_generation(
                    &store.conn,
                    &input.snapshot.owner_kind,
                    &input.snapshot.owner_id
                )? == input.snapshot.generation,
                "refresh superseded"
            );
            ensure!(
                input.surface != "article" || article_details_eligible(&input),
                "waiting for complete article details"
            );
            Ok(())
        })
    };
    let result = if let Err(error) = admission {
        Err(error)
    } else if super::insights::input_key(&input, settings) != claimed.input_hash {
        Err(anyhow::anyhow!(
            "captured route/preparation contract changed"
        ))
    } else {
        let fence_owner = (
            input.snapshot.owner_kind.clone(),
            input.snapshot.owner_id.clone(),
            input.snapshot.generation,
        );
        let refresh = super::insight_execution::slice(super::insights::run_pipeline(
            &mut input, auth, settings, cancel, db_path,
        ));
        tokio::pin!(refresh);
        let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(30));
        let result = loop {
            tokio::select! {
                result=&mut refresh => break result,
                _=heartbeat.tick() => {
                    let store=crate::store::Store::open(db_path)?;
                    ensure!(crate::tasks::renew_lease(&store.conn,&claimed.id,&claimed.owner,claimed.epoch,120,&chrono::Utc::now().to_rfc3339())?,"refresh lease lost");
                    // Genuine replacement: drop the in-flight request now.
                    if desired_generation(&store.conn, &fence_owner.0, &fence_owner.1)? != fence_owner.2 {
                        break Err(anyhow::anyhow!("refresh superseded"));
                    }
                }
            }
        };
        result
    };
    let now = chrono::Utc::now().to_rfc3339();
    let mut store = crate::store::Store::open(db_path)?;
    let mut output = match result {
        Ok(output) => output,
        Err(error) => {
            if error.is::<super::insight_execution::SliceYield>() {
                let n=store.conn.execute("UPDATE argos_tasks SET state='queued',max_attempts=attempts+1,lease_owner='',lease_until='',updated_at=?4 WHERE id=?1 AND state='running' AND lease_owner=?2 AND lease_epoch=?3 AND julianday(lease_until)>julianday(?4)",params![claimed.id,claimed.owner,claimed.epoch,now])?;
                ensure!(n == 1, "refresh lease lost before continuation");
                store.conn.execute("UPDATE argos_attempts SET finished_at=?2,outcome='slice_complete' WHERE task_id=?1 AND finished_at=''",params![claimed.id,now])?;
                crate::tasks::refresh_job_state(&store.conn, &claimed.job_id, &now)?;
                return Ok(None);
            }
            // Supersession is intentional retirement, not Unknown/Timeout.
            if is_superseded(&store.conn, &input) {
                let message = match supersession(&store.conn, &claimed.id) {
                    Some((job, reason)) => format!("Superseded · replaced by {job} · {reason}"),
                    None => "Superseded · replaced by a newer generation".into(),
                };
                crate::tasks::supersede_claimed(&store.conn, &claimed, &message, &now)?;
                return Ok(None);
            }
            record_terminal_failure(
                &store.conn,
                &input.snapshot.owner_kind,
                &input.snapshot.owner_id,
                input.snapshot.generation,
                &error.to_string(),
                &now,
            )?;
            if cancel.load(std::sync::atomic::Ordering::Acquire) {
                store.conn.execute("UPDATE insight_owner_state SET lifecycle='pending',last_error='cancelled; evidence dirty' WHERE owner_kind=?1 AND owner_id=?2 AND desired_generation=?3",params![input.snapshot.owner_kind,input.snapshot.owner_id,input.snapshot.generation])?;
                crate::tasks::block_claimed(
                    &store.conn,
                    &claimed,
                    "cancelled; evidence dirty",
                    &now,
                )?;
                return Err(error);
            }
            crate::tasks::fail_claimed(
                &store.conn,
                &claimed,
                crate::tasks::OperationKind::OtherLlm,
                &crate::tasks::TaskError::new(
                    error
                        .downcast_ref::<crate::recon::model_exec::ModelOperationError>()
                        .and_then(|e| e.category)
                        .map(|c| c.task_category())
                        .unwrap_or(crate::tasks::ErrorCategory::Unknown),
                    error.to_string(),
                ),
                &now,
            )?;
            return Err(error);
        }
    };
    // Reduced and later repaired snapshots have distinct publication identities,
    // while stage checkpoint identity remains independent of execution epoch.
    input.snapshot.key = crate::nlp::cache::key(&serde_json::json!([input.snapshot.key, output]));
    for inference in &mut output.inferences {
        inference.snapshot_key = input.snapshot.key.clone();
    }
    for link in &mut output.links {
        link.snapshot_key = input.snapshot.key.clone();
    }
    if cancel.load(std::sync::atomic::Ordering::Acquire) {
        store.conn.execute("UPDATE insight_owner_state SET lifecycle='pending',last_error='cancelled; evidence dirty' WHERE owner_kind=?1 AND owner_id=?2 AND desired_generation=?3", params![input.snapshot.owner_kind,input.snapshot.owner_id,input.snapshot.generation])?;
        crate::tasks::block_claimed(&store.conn, &claimed, "cancelled; evidence dirty", &now)?;
        anyhow::bail!("cancelled before insight publication");
    }
    let lifecycle = if output.readiness.publication_eligible {
        "active"
    } else {
        "insufficient_evidence"
    };
    if !output.readiness.publication_eligible {
        let reason = output.readiness.gaps.join("; ");
        record_terminal_failure(
            &store.conn,
            &input.snapshot.owner_kind,
            &input.snapshot.owner_id,
            input.snapshot.generation,
            &reason,
            &now,
        )?;
        crate::tasks::fail_claimed(
            &store.conn,
            &claimed,
            crate::tasks::OperationKind::OtherLlm,
            &crate::tasks::TaskError::new(crate::tasks::ErrorCategory::Unknown, &reason),
            &now,
        )?;
        anyhow::bail!("replacement coverage incomplete: {reason}");
    }
    let activated = publish_snapshot(
        &mut store.conn,
        &input.snapshot,
        &PublicationFence {
            task: &claimed,
            now: &now,
            live_input_key: &input.snapshot.key,
        },
        lifecycle,
        |tx, snapshot_id| {
            validate_live_evidence(tx, &input.snapshot)?;
            write_derivations(tx, snapshot_id, &input, &output, &now)?;
            super::insight_execution::progress(
                tx,
                &input,
                "published",
                None,
                Some(output.withheld_count),
            )?;
            Ok(())
        },
    );
    match activated {
        Ok(true) => {}
        failure => {
            let error = match failure {
                Err(error) => error,
                _ => anyhow::anyhow!("stale insight snapshot; previous history retained"),
            };
            let live:bool=store.conn.query_row("SELECT EXISTS(SELECT 1 FROM argos_tasks WHERE id=?1 AND state='running' AND lease_owner=?2 AND lease_epoch=?3 AND julianday(lease_until)>julianday(?4))",params![claimed.id,claimed.owner,claimed.epoch,now],|r|r.get(0))?;
            if live {
                record_terminal_failure(
                    &store.conn,
                    &input.snapshot.owner_kind,
                    &input.snapshot.owner_id,
                    input.snapshot.generation,
                    &error.to_string(),
                    &now,
                )?;
                crate::tasks::fail_claimed(
                    &store.conn,
                    &claimed,
                    crate::tasks::OperationKind::OtherLlm,
                    &crate::tasks::TaskError::new(
                        error
                            .downcast_ref::<crate::recon::model_exec::ModelOperationError>()
                            .and_then(|e| e.category)
                            .map(|c| c.task_category())
                            .unwrap_or(crate::tasks::ErrorCategory::Unknown),
                        error.to_string(),
                    ),
                    &now,
                )?;
            }
            return Err(error);
        }
    }
    Ok(Some(output))
}

/// Recovery is deliberately restricted to insight_refresh tasks and their exact IDs.
pub async fn recover_refreshes(
    db_path: &std::path::Path,
    auth: &crate::secrets::AuthFile,
    settings: &crate::provider::SettingsFile,
    cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<usize> {
    let ids = {
        let store = crate::store::Store::open(db_path)?;
        let now = chrono::Utc::now().to_rfc3339();
        // Recover a visit whose body commit became durable before scheduling,
        // and completion receipts even when no TUI listener survived.
        let owners=store.conn.prepare("SELECT article_id FROM article_refresh_intents UNION SELECT article_id FROM intel_report_results WHERE completion_sequence>0")?.query_map([],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        for owner in owners {
            let run:Option<String>=store.conn.query_row("SELECT run_id FROM atlas_articles WHERE article_id=?1 ORDER BY seen_at DESC LIMIT 1",[&owner],|r|r.get(0)).optional()?;
            if let Some(article) =
                run.and_then(|run| store.atlas_article(&run, &owner).ok().flatten())
            {
                if let Err(error) = schedule_article_refresh(&store, &article, settings, false) {
                    let _ = crate::events::record_event(
                        &store.conn,
                        &crate::events::NewEvent {
                            app: "intel".into(),
                            event_type: "insight.recovery_error".into(),
                            message: error.to_string(),
                            resource_ref: owner,
                            ..Default::default()
                        },
                    );
                }
            }
        }
        crate::tasks::retire_atlas_insight_work(&store.conn, &now)?;
        // One recovery path for every operation: atomic CAS, epoch fencing,
        // worker recoveries counted separately from provider attempts.
        crate::tasks::interrupt_expired_leases(&store.conn, &now)?;
        let mut stmt=store.conn.prepare("SELECT id FROM argos_tasks WHERE operation='insight_refresh' AND state IN ('queued','retry_scheduled') ORDER BY created_at,id")?;
        let ids = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ids
    };
    let mut completed = 0;
    for id in ids {
        if cancel.load(std::sync::atomic::Ordering::Acquire) {
            break;
        }
        let captured = {
            let store = crate::store::Store::open(db_path)?;
            let json: String = store.conn.query_row(
                "SELECT role_snapshot FROM argos_tasks WHERE id=?1",
                [&id],
                |r| r.get(0),
            )?;
            serde_json::from_str::<crate::provider::SettingsFile>(&json)
                .unwrap_or_else(|_| settings.clone())
        };
        match run_refresh_slice(db_path, &id, auth, &captured, cancel).await {
            Ok(Some(_)) => completed += 1,
            Ok(None) => {}
            Err(error) => {
                let store = crate::store::Store::open(db_path)?;
                let _ = crate::events::record_event(
                    &store.conn,
                    &crate::events::NewEvent {
                        app: "intel".into(),
                        event_type: "insight.recovery_error".into(),
                        message: error.to_string(),
                        task_id: id,
                        ..Default::default()
                    },
                );
            }
        }
    }
    Ok(completed)
}

pub(super) fn validate_retry(conn: &Connection, input: &PipelineInput) -> Result<()> {
    validate_live_evidence(conn, &input.snapshot)?;
    ensure!(
        desired_generation(conn, &input.snapshot.owner_kind, &input.snapshot.owner_id)?
            == input.snapshot.generation,
        "refresh superseded; cannot retry"
    );
    Ok(())
}

pub(super) fn validate_dispatch(conn: &Connection, input: &PipelineInput) -> Result<()> {
    if input.snapshot.owner_kind != "article" {
        return Ok(());
    }
    validate_live_evidence(conn, &input.snapshot)?;
    ensure!(
        desired_generation(conn, "article", &input.snapshot.owner_id)? == input.snapshot.generation,
        "article refresh superseded before dispatch"
    );
    let job: String = conn.query_row(
        "SELECT refresh_job_id FROM insight_owner_state WHERE owner_kind='article' AND owner_id=?1",
        [&input.snapshot.owner_id],
        |r| r.get(0),
    )?;
    crate::job_cleanup::ensure_admissible(conn, &job, &chrono::Utc::now().to_rfc3339())?;
    let leased:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM argos_tasks WHERE job_id=?1 AND operation='insight_refresh' AND state='running' AND julianday(lease_until)>julianday('now'))",[job],|r|r.get(0))?;
    ensure!(leased, "article refresh lease lost before dispatch");
    Ok(())
}

fn validate_live_evidence(conn: &Connection, snapshot: &EvidenceSnapshot) -> Result<()> {
    match snapshot.owner_kind.as_str() {
        "article" | "atlas" => {
            let live: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM atlas_articles WHERE article_id=?1)",
                [&snapshot.owner_id],
                |r| r.get(0),
            )?;
            ensure!(live, "article owner no longer exists");
        }
        "recon" => {
            let live:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM recon_runs r JOIN recon_threads t ON t.id=r.thread_id WHERE r.id=?1 AND t.deleted=0 AND r.state<>'cancelled')",[&snapshot.owner_id],|r|r.get(0))?;
            ensure!(live, "Recon owner no longer active");
        }
        _ => anyhow::bail!("unsupported publication owner"),
    }
    for reference in &snapshot.source_refs {
        if let Some(id) = reference.source_id.strip_prefix("report:") {
            let live: Option<(String,i64)> = conn.query_row("SELECT source_hash,completion_sequence FROM intel_report_results WHERE article_id=?1 AND job_id=?2 AND completion_sequence=(SELECT MAX(completion_sequence) FROM intel_report_results WHERE article_id=?1)",params![snapshot.owner_id,id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
            ensure!(
                live.is_some_and(
                    |(hash, sequence)| reference.revision == format!("{sequence}:{hash}")
                ),
                "report authority changed"
            );
        } else if let Some(id) = reference.source_id.strip_prefix("call:") {
            let id = id.split("#page").next().unwrap_or(id);
            let json: Option<String> = conn
                .query_row(
                    "SELECT result_json FROM osint_calls WHERE id=?1 AND status='completed'",
                    [id],
                    |r| r.get(0),
                )
                .optional()?;
            ensure!(
                json.is_some_and(|s| crate::nlp::cache::hash(&s) == reference.revision),
                "tool evidence revision changed"
            );
        } else if let Some(id) = reference.source_id.strip_prefix("article:") {
            let row:Option<(String,String,String,String)>=conn.query_row("SELECT title,description,url,published_at FROM atlas_articles WHERE article_id=?1 ORDER BY seen_at DESC LIMIT 1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
            ensure!(
                row.is_some_and(
                    |(title, description, url, published)| crate::nlp::cache::hash(&format!(
                        "{title}\n{description}"
                    )) == reference.revision
                        && url == reference.url
                        && reference.published_at.as_ref() == Some(&published)
                ),
                "article metadata revision changed"
            );
        } else if let Some(id) = reference.source_id.strip_prefix("body:") {
            let report: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM intel_report_results WHERE article_id=?1)",
                [&snapshot.owner_id],
                |r| r.get(0),
            )?;
            ensure!(!report, "completed report supersedes article details");
            let version: Option<i64> = conn
                .query_row(
                    "SELECT body_version FROM article_bodies WHERE id=?1",
                    [id],
                    |r| r.get(0),
                )
                .optional()?;
            ensure!(
                version.is_some_and(|v| v.to_string() == reference.revision),
                "body evidence revision changed"
            );
        }
        let artifact: Option<String> = conn
            .query_row(
                "SELECT text FROM nlp_artifacts WHERE hash=?1",
                [&reference.artifact_hash],
                |r| r.get(0),
            )
            .optional()?;
        ensure!(
            artifact.is_some_and(|s| crate::nlp::cache::hash(&s) == reference.artifact_hash),
            "retained source artifact unavailable"
        );
    }
    for (id, revision) in &snapshot.memory_refs {
        let text: Option<String> = conn
            .query_row("SELECT text FROM memories WHERE id=?1", [id], |r| r.get(0))
            .optional()?;
        ensure!(
            text.is_some_and(|s| crate::store::memory_revision(&s) == *revision),
            "memory premise revision changed"
        );
    }
    Ok(())
}

/// Canonical derivation writer shared by every publication lane.
pub fn write_derivations(
    tx: &Connection,
    snapshot_id: &str,
    input: &PipelineInput,
    output: &PipelineOutput,
    now: &str,
) -> Result<()> {
    if input.snapshot.owner_kind == "article" {
        retire_article_associations(tx, &input.snapshot.owner_id, output)?;
    }
    let mut items = Vec::new();
    for claim in &output.claims {
        items.push((
            claim.id.clone(),
            "claim",
            super::insights::claim_fingerprint(claim),
            claim.text.clone(),
            serde_json::to_string(claim)?,
            serde_json::to_string(&claim.assessment)?,
            claim.evidence_refs.clone(),
            claim.subject_ref.clone(),
            claim.predicate.clone(),
            claim.object.clone(),
        ));
    }
    for inference in &output.inferences {
        items.push((
            inference.id.clone(),
            "inference",
            super::insights::inference_fingerprint(inference),
            inference.text.clone(),
            serde_json::to_string(inference)?,
            serde_json::to_string(&inference.assessment)?,
            inference.evidence_refs.clone(),
            inference.entity_refs.first().cloned().unwrap_or_default(),
            "derived".into(),
            inference.text.clone(),
        ));
    }
    let origin_run: Option<String> = if input.snapshot.owner_kind == "atlas" {
        tx.query_row(
            "SELECT run_id FROM atlas_articles WHERE article_id=?1 ORDER BY seen_at DESC LIMIT 1",
            [&input.snapshot.owner_id],
            |r| r.get(0),
        )
        .optional()?
    } else {
        None
    };
    let source = crate::brain::MemorySource {
        app: input.snapshot.owner_kind.clone(),
        conversation_id: origin_run
            .clone()
            .unwrap_or_else(|| input.snapshot.owner_id.clone()),
        message_id: None,
        reference: Some(snapshot_id.into()),
    };
    for (id, kind, fingerprint, text, item_json, assessment, refs, subject, predicate, object) in
        items
    {
        let mut memory_id = None;
        let faithful = serde_json::from_str::<super::insights::Assessment>(&assessment)?.faithful()
            && !refs.is_empty();
        if input.save_insights && faithful {
            let owned_tombstone = |owners: Vec<String>| {
                owners.is_empty()
                    || owners.contains(&input.snapshot.owner_id)
                    || origin_run.as_ref().is_some_and(|run| owners.contains(run))
            };
            let mut tombstoned =
                crate::store::tombstone_runs(tx, &fingerprint)?.is_some_and(&owned_tombstone);
            if kind == "claim" {
                if let Some(entity) = output.entities.iter().find(|e| e.id == subject) {
                    let namespace = match entity.type_hint.as_str() {
                        "person" => "person",
                        "place" => "place",
                        _ => "org",
                    };
                    for form in &entity.original_forms {
                        let legacy = crate::store::insight_fingerprint(
                            namespace,
                            &form.trim().to_lowercase(),
                            &predicate.trim().to_lowercase(),
                            &object.trim().to_lowercase(),
                        );
                        tombstoned |= crate::store::tombstone_runs(tx, &legacy)?
                            .is_some_and(&owned_tombstone);
                    }
                }
            }
            if !tombstoned {
                let existing: Option<String> = tx
                    .query_row(
                        "SELECT memory_id FROM insight_claims WHERE fingerprint=?1",
                        [&fingerprint],
                        |r| r.get(0),
                    )
                    .optional()?;
                let mid = existing.unwrap_or_else(|| format!("mem:{}", fingerprint));
                tx.execute("INSERT OR IGNORE INTO memories(id,text,category,pinned,created_at,source_json) VALUES(?1,?2,'investigation',0,?3,?4)",params![mid,text,now,serde_json::to_string(&source)?])?;
                if input.snapshot.owner_kind == "article" {
                    let updated=tx.execute("UPDATE memories SET text=?2,source_json=?3 WHERE id=?1 AND json_extract(source_json,'$.app') IN ('article','atlas') AND NOT EXISTS(SELECT 1 FROM insight_sources WHERE fingerprint=?4)",params![mid,text,serde_json::to_string(&source)?,fingerprint])?;
                    if updated > 0 {
                        tx.execute("DELETE FROM insight_user_edits WHERE memory_id=?1", [&mid])?;
                    }
                }
                if let Some(entity) = output.entities.iter().find(|e| e.id == subject) {
                    tx.execute("INSERT OR IGNORE INTO recon_entities(id,kind,namespace,canonical,label) VALUES(?1,?2,?1,?3,?4)",params![entity.id,entity.type_hint,entity.comparison_key,entity.original_forms.first().cloned().unwrap_or_default()])?;
                }
                tx.execute("INSERT OR IGNORE INTO insight_claims(fingerprint,memory_id,entity_id,predicate,object_value,topic,classification,confidence,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,'investigation',?6,0,?7,?7)",params![fingerprint,mid,subject,predicate,object,kind,now])?;
                crate::store::enqueue_memory_index_on(tx, std::slice::from_ref(&mid), None)?;
                for reference in &refs {
                    let thread_id = if input.snapshot.owner_kind == "recon" {
                        tx.query_row(
                            "SELECT thread_id FROM recon_runs WHERE id=?1",
                            [&input.snapshot.owner_id],
                            |r| r.get::<_, String>(0),
                        )
                        .optional()?
                    } else {
                        None
                    };
                    tx.execute("INSERT OR IGNORE INTO insight_sources(fingerprint,thread_id,run_id,answer_id,call_id,source_url,published_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![fingerprint,thread_id,origin_run.as_ref().unwrap_or(&input.snapshot.owner_id),snapshot_id,reference.source_id.strip_prefix("call:").unwrap_or(&reference.source_id),reference.url,reference.published_at.clone().unwrap_or_default()])?;
                }
                memory_id = Some(mid);
            }
        }
        let derivation_id = format!("{snapshot_id}:{id}");
        tx.execute("INSERT INTO insight_derivations(id,snapshot_id,memory_id,insight_kind,fingerprint,item_json,assessment_json,status,revision) VALUES(?1,?2,?3,?4,?5,?6,?7,?9,?8)",params![derivation_id,snapshot_id,memory_id,kind,fingerprint,item_json,assessment,crate::nlp::cache::hash(&text),if faithful {"assessed"} else {"rejected"}])?;
        for reference in refs {
            tx.execute("INSERT INTO insight_derivation_evidence(derivation_id,source_id,source_revision,evidence_json) VALUES(?1,?2,?3,?4)",params![derivation_id,reference.source_id,reference.revision,serde_json::to_string(&reference)?])?;
        }
    }
    for inference in &output.inferences {
        for premise in &inference.premise_claim_ids {
            let Some(claim) = output.claims.iter().find(|c| &c.id == premise) else {
                continue;
            };
            for reference in &claim.evidence_refs {
                tx.execute("INSERT OR IGNORE INTO insight_derivation_evidence(derivation_id,source_id,source_revision,premise_id,evidence_json) VALUES(?1,?2,?3,?4,?5)",params![format!("{snapshot_id}:{}",inference.id),reference.source_id,reference.revision,premise,serde_json::to_string(reference)?])?;
            }
        }
    }
    for inference in &output.inferences {
        for (memory_id, revision) in &input.snapshot.memory_refs {
            for reference in &inference.evidence_refs {
                tx.execute("INSERT OR IGNORE INTO insight_derivation_evidence(derivation_id,source_id,source_revision,memory_id,premise_id,evidence_json) VALUES(?1,?2,?3,?4,?5,?6)",params![format!("{snapshot_id}:{}",inference.id),reference.source_id,reference.revision,memory_id,format!("memory:{memory_id}@{revision}"),serde_json::to_string(reference)?])?;
            }
        }
    }
    let publisher = tx.query_row("SELECT metadata_json FROM atlas_article_metadata WHERE article_id=?1 ORDER BY rowid DESC LIMIT 1",[&input.snapshot.owner_id],|r|r.get::<_,String>(0)).optional()?.and_then(|json|serde_json::from_str::<crate::atlas_metadata::ArticleMetadata>(&json).ok()).and_then(|m|m.publisher_prior).and_then(|p|crate::osint::SourceReliability::parse(&p.observation.reliability)).unwrap_or(crate::osint::SourceReliability::F);
    let report = input.surface == "recon_report";
    let rating = crate::intel_recon::EvidenceConfidence::estimate_for_source(
        output,
        output.readiness.publication_eligible,
        publisher,
        report,
    );
    tx.execute("UPDATE insight_snapshots SET readiness_json=?2,model_versions_json=(SELECT json_group_array(json(route_json)) FROM insight_stage_checkpoints WHERE owner_kind=?3 AND owner_id=?4 AND generation=?5 AND state='completed') WHERE id=?1",params![snapshot_id,serde_json::json!({"readiness":output.readiness,"limit_reached":output.limit_reached,"partial_enumeration":output.partial_enumeration,"item_gaps":output.item_gaps,"withheld_count":output.withheld_count,"rating":{"policy":"brief-rating-v2","input_mode":input.surface,"source_hash":crate::nlp::cache::key(&serde_json::json!(input.snapshot.source_refs)),"estimate":rating}}).to_string(),input.snapshot.owner_kind,input.snapshot.owner_id,input.snapshot.generation])?;
    for link in &output.links {
        let left = &link.typed_endpoints[0];
        let right = &link.typed_endpoints[1];
        tx.execute("INSERT INTO insight_derivation_links(id,snapshot_id,left_kind,left_id,right_kind,right_id,relation,rationale,assessment_json,evidence_json,status,revision) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'assessed',?11)",params![format!("{snapshot_id}:{}",link.id),snapshot_id,left.kind,left.id,right.kind,right.id,link.relation,link.rationale,serde_json::to_string(&link.assessment)?,serde_json::to_string(&link.evidence_refs)?,crate::nlp::cache::hash(&serde_json::to_string(link)?)])?;
    }
    if input.snapshot.owner_kind == "article" {
        crate::job_cleanup::cleanup_batch(tx, chrono::Utc::now())?;
    }
    crate::store::bump_memories_changed(tx)?;
    Ok(())
}

/// Retire associations, not the historical snapshot. Only exclusively owned
/// canonical memories are removed; other articles, Recon and manual rows live.
fn retire_article_associations(
    conn: &Connection,
    article: &str,
    output: &PipelineOutput,
) -> Result<()> {
    let retained: std::collections::HashSet<String> = output
        .claims
        .iter()
        .filter(|claim| claim.assessment.faithful())
        .map(super::insights::claim_fingerprint)
        .chain(
            output
                .inferences
                .iter()
                .map(super::insights::inference_fingerprint),
        )
        .collect();
    let rows = conn.prepare("SELECT DISTINCT c.fingerprint,c.memory_id FROM insight_claims c WHERE c.fingerprint IN (SELECT d.fingerprint FROM insight_derivations d JOIN insight_snapshots s ON s.id=d.snapshot_id WHERE s.owner_kind IN ('article','atlas') AND s.owner_id=?1) OR c.fingerprint IN (SELECT fingerprint FROM insight_sources WHERE call_id=?1)")?
        .query_map([article],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    conn.execute("DELETE FROM insight_sources WHERE answer_id IN (SELECT id FROM insight_snapshots WHERE owner_kind IN ('article','atlas') AND owner_id=?1) OR call_id=?1",[article])?;
    conn.execute("UPDATE insight_derivation_links SET status='retired' WHERE snapshot_id IN (SELECT id FROM insight_snapshots WHERE owner_kind IN ('article','atlas') AND owner_id=?1)",[article])?;
    conn.execute("UPDATE insight_derivations SET status='retired' WHERE snapshot_id IN (SELECT id FROM insight_snapshots WHERE owner_kind IN ('article','atlas') AND owner_id=?1)",[article])?;
    for (fp, memory) in rows {
        let shared: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM insight_sources WHERE fingerprint=?1) OR EXISTS(SELECT 1 FROM insight_derivations d JOIN insight_owner_state h ON h.active_snapshot_id=d.snapshot_id WHERE d.memory_id=?2 AND d.status='assessed' AND h.owner_id<>?3)",params![fp,memory,article],|r|r.get(0))?;
        if shared {
            continue;
        }
        conn.execute(
            "DELETE FROM insight_relations WHERE left_fingerprint=?1 OR right_fingerprint=?1",
            [&fp],
        )?;
        if retained.contains(&fp) {
            continue;
        }
        let edited: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM insight_user_edits WHERE memory_id=?1)",
            [&memory],
            |r| r.get(0),
        )?;
        if edited {
            continue;
        }
        let owned: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM memories WHERE id=?1 AND json_extract(source_json,'$.app') IN ('article','atlas'))",[&memory],|r|r.get(0))?;
        if owned {
            conn.execute(
                "DELETE FROM insight_user_edits WHERE memory_id=?1",
                [&memory],
            )?;
            conn.execute("DELETE FROM insight_claims WHERE fingerprint=?1", [&fp])?;
            conn.execute(
                "DELETE FROM memory_graph_summaries WHERE memory_id=?1",
                [&memory],
            )?;
            conn.execute("DELETE FROM memories WHERE id=?1", [&memory])?;
            crate::store::enqueue_memory_index_on(conn, &[memory], None)?;
        }
    }
    Ok(())
}

pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(include_str!("../schema_insights.sql"))?;
    let columns = conn
        .prepare("PRAGMA table_info(extraction_jobs)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if !columns.is_empty() && !columns.iter().any(|s| s == "contract_version") {
        conn.execute_batch("ALTER TABLE extraction_jobs ADD COLUMN contract_version TEXT NOT NULL DEFAULT 'legacy-v1'")?;
    }
    Ok(())
}

/// Called inside deletion transactions. Historical evidence remains retained;
/// deleted canonical endpoints cannot remain active graph relationships.
pub fn reconcile_deleted_memories(conn: &Connection) -> Result<()> {
    conn.execute_batch("UPDATE insight_derivations SET status='deleted' WHERE memory_id IS NOT NULL AND memory_id NOT IN (SELECT id FROM memories);
        UPDATE insight_derivation_links SET status='deleted_endpoint' WHERE left_id IN (SELECT json_extract(item_json,'$.id') FROM insight_derivations WHERE status='deleted') OR right_id IN (SELECT json_extract(item_json,'$.id') FROM insight_derivations WHERE status='deleted');
        UPDATE insight_owner_state SET lifecycle='previous',last_error='memory endpoint removed' WHERE active_snapshot_id IN (SELECT snapshot_id FROM insight_derivations WHERE status='deleted');
        UPDATE insight_derivations SET memory_id=NULL WHERE status='deleted';
        UPDATE insight_derivation_evidence SET memory_id=NULL WHERE memory_id IS NOT NULL AND memory_id NOT IN (SELECT id FROM memories);")?;
    Ok(())
}

pub fn reconcile_deleted_articles(conn: &Connection) -> Result<()> {
    conn.execute_batch("UPDATE insight_owner_state SET desired_generation=desired_generation+1,lifecycle='previous',last_error='origin article removed' WHERE owner_kind IN ('article','atlas') AND lifecycle<>'previous' AND NOT EXISTS (SELECT 1 FROM atlas_articles a WHERE a.article_id=insight_owner_state.owner_id);
        UPDATE insight_derivation_links SET status='origin_deleted' WHERE snapshot_id IN (SELECT active_snapshot_id FROM insight_owner_state WHERE last_error='origin article removed');")?;
    Ok(())
}

pub fn desired_generation(conn: &Connection, kind: &str, owner: &str) -> Result<u32> {
    Ok(conn.query_row("SELECT desired_generation FROM insight_owner_state WHERE owner_kind=?1 AND owner_id=?2", params![kind, owner], |r| r.get(0)).optional()?.unwrap_or(0))
}

/// A new investigation keeps the active pointer and labels its history previous/pending.
pub fn advance_generation(conn: &Connection, kind: &str, owner: &str, now: &str) -> Result<u32> {
    Ok(conn.query_row(
        "INSERT INTO insight_owner_state(owner_kind,owner_id,desired_generation,lifecycle,updated_at)
         VALUES(?1,?2,1,'pending',?3) ON CONFLICT(owner_kind,owner_id) DO UPDATE SET
         desired_generation=desired_generation+1,lifecycle='pending',updated_at=excluded.updated_at
         RETURNING desired_generation", params![kind, owner, now], |r| r.get(0))?)
}

pub fn record_terminal_failure(
    conn: &Connection,
    kind: &str,
    owner: &str,
    generation: u32,
    error: &str,
    now: &str,
) -> Result<bool> {
    Ok(conn.execute("UPDATE insight_owner_state SET lifecycle='previous',last_error=?4,updated_at=?5 WHERE owner_kind=?1 AND owner_id=?2 AND desired_generation=?3", params![kind, owner, generation, error, now])? == 1)
}

pub struct PublicationFence<'a> {
    pub task: &'a ClaimedTask,
    pub now: &'a str,
    /// Live input key recomputed from source, premise, binding and memory revisions.
    pub live_input_key: &'a str,
}

/// Lane-independent canonical publication, run on the caller's transaction.
///
/// Checks the owner's desired generation and live input key, revalidates live
/// evidence, records the immutable snapshot, runs `publish` (canonical Brain
/// rows, evidence, links and index outbox) and activates the snapshot. Each lane
/// supplies its own fence first: the insight-refresh lease
/// ([`publish_snapshot`]) or the brief chain's fenced Stage-4 commit
/// (`intel_recon::brief_pipeline::commit_stage`). Returns the snapshot id, or
/// `None` without writing when the generation/key is stale.
pub fn publish_snapshot_on<F>(
    tx: &Connection,
    snapshot: &EvidenceSnapshot,
    live_input_key: &str,
    lifecycle: &str,
    now: &str,
    publish: F,
) -> Result<Option<String>>
where
    F: FnOnce(&Connection, &str) -> Result<()>,
{
    ensure!(
        snapshot.owner_kind != "atlas",
        "Atlas insight publication retired"
    );
    if desired_generation(tx, &snapshot.owner_kind, &snapshot.owner_id)? != snapshot.generation
        || snapshot.key != live_input_key
    {
        return Ok(None);
    }
    ensure!(
        ["active", "insufficient_evidence", "blocked"].contains(&lifecycle),
        "invalid snapshot lifecycle"
    );
    validate_live_evidence(tx, snapshot)?;
    let snapshot_id = format!(
        "isnap:{}:{}:{}:{}",
        snapshot.owner_kind, snapshot.owner_id, snapshot.generation, snapshot.key
    );
    let existing: Option<String> = tx.query_row("SELECT id FROM insight_snapshots WHERE owner_kind=?1 AND owner_id=?2 AND generation=?3 AND input_key=?4", params![snapshot.owner_kind, snapshot.owner_id, snapshot.generation, snapshot.key], |r| r.get(0)).optional()?;
    if existing.is_none() {
        tx.execute("INSERT INTO insight_snapshots(id,owner_kind,owner_id,generation,input_key,inputs_json,entities_json,coverage_json,readiness_json,contract_version,policy_version,created_at)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)", params![snapshot_id,snapshot.owner_kind,snapshot.owner_id,snapshot.generation,snapshot.key,serde_json::to_string(snapshot)?,serde_json::to_string(&snapshot.entities)?,serde_json::to_string(&snapshot.coverage_by_stage)?,serde_json::json!({"lifecycle":lifecycle}).to_string(),CONTRACT,READINESS_POLICY,now])?;
        publish(tx, &snapshot_id)?;
    }
    tx.execute("UPDATE insight_owner_state SET active_snapshot_id=?4,lifecycle=?5,last_error='',updated_at=?6 WHERE owner_kind=?1 AND owner_id=?2 AND desired_generation=?3", params![snapshot.owner_kind,snapshot.owner_id,snapshot.generation,snapshot_id,lifecycle,now])?;
    Ok(Some(snapshot_id))
}

/// Insight-refresh lane: the lease fence around [`publish_snapshot_on`], with
/// task acknowledgement in the same transaction.
pub fn publish_snapshot<F>(
    conn: &mut Connection,
    snapshot: &EvidenceSnapshot,
    fence: &PublicationFence<'_>,
    lifecycle: &str,
    publish: F,
) -> Result<bool>
where
    F: FnOnce(&Connection, &str) -> Result<()>,
{
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    crate::job_cleanup::ensure_admissible(&tx, &fence.task.job_id, fence.now)?;
    let leased = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM argos_tasks WHERE id=?1 AND job_id=?2 AND operation='insight_refresh'
         AND lease_owner=?3 AND lease_epoch=?4 AND state='running' AND julianday(lease_until)>julianday(?5))",
        params![fence.task.id, fence.task.job_id, fence.task.owner, fence.task.epoch, fence.now], |r| r.get::<_, bool>(0))?;
    if !leased {
        return Ok(false);
    }
    let Some(snapshot_id) = publish_snapshot_on(
        &tx,
        snapshot,
        fence.live_input_key,
        lifecycle,
        fence.now,
        publish,
    )?
    else {
        return Ok(false);
    };
    ensure!(
        crate::tasks::complete_claimed(&tx, fence.task, &snapshot_id, fence.now)?,
        "publication lease lost"
    );
    tx.commit()?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_waits_for_explicit_resume_and_exhaustion_does_not_restart() {
        let (conn, _, claimed) = leased_fixture();
        let now = "2026-10-11T00:00:10Z";
        let mut input: PipelineInput = serde_json::from_str(&claimed.input_ref).unwrap();
        let settings = crate::provider::SettingsFile::default();
        crate::tasks::block_claimed(&conn, &claimed, "cancelled; evidence dirty", now).unwrap();
        assert!(
            crate::tasks::claim_task(&conn, &claimed.id, "startup", 120, now)
                .unwrap()
                .is_none()
        );
        let resumed = enqueue_refresh(&conn, &mut input, &settings, now).unwrap();
        assert_eq!(resumed, claimed.id);
        let lease = crate::tasks::claim_task(&conn, &resumed, "explicit-resume", 120, now)
            .unwrap()
            .unwrap();
        assert!(lease.epoch > claimed.epoch);
        crate::tasks::fail_claimed(
            &conn,
            &lease,
            crate::tasks::OperationKind::OtherLlm,
            &crate::tasks::TaskError::new(
                crate::tasks::ErrorCategory::Unknown,
                "provider chain exhausted",
            ),
            now,
        )
        .unwrap();
        enqueue_refresh(&conn, &mut input, &settings, now).unwrap();
        assert!(crate::tasks::claim_task(&conn, &resumed, "retry", 120, now)
            .unwrap()
            .is_none());
    }

    #[test]
    fn article_readiness_requires_acquisition_coverage_even_when_display_is_complete() {
        let store = crate::store::Store::memory().unwrap();
        let article = crate::store::AtlasArticleRow {
            run_id: "run".into(),
            id: "article".into(),
            title: "Acme expansion".into(),
            description: String::new(),
            url: "https://example.test/story".into(),
            country: String::new(),
            source_name: String::new(),
            source_domain: "example.test".into(),
            published_at: String::new(),
            provider: String::new(),
            temperature: 0.0,
            category: String::new(),
            seen_at: String::new(),
            author: String::new(),
            image_url: String::new(),
        };
        let body = store
            .ensure_article_body(
                &article.id,
                &article.run_id,
                &article.url,
                &article.source_domain,
                "",
            )
            .unwrap();
        let raw=format!("Acme expansion\n\n{}", "The company announced an expansion and explained the project, timing, qualification and original source attribution in detail. ".repeat(30));
        assert_eq!(
            crate::intel_recon::validate_article_body(&raw, &article.title, &article.url).quality,
            crate::intel_recon::BodyQuality::Complete
        );
        store
            .commit_article_body(
                &body.id,
                "Model-written display body",
                "display",
                "complete",
                "display only",
                &article.url,
                "fixture",
                false,
            )
            .unwrap();
        let mut source =
            crate::nlp::Source::derived(&body.id, &body.body_version.to_string(), &raw);
        source.attempt_id = "attempt".into();
        source.raw_hash = Some(crate::nlp::cache::hash(&raw));
        crate::nlp::cache::register(
            &store.conn,
            "article",
            &article.id,
            &source,
            Some(&raw),
            "validated-baseline-v1",
        )
        .unwrap();
        assert!(
            !article_input(&store, &article, 1, "objective", false)
                .unwrap()
                .evidence[0]
                .complete_story,
            "legacy length/quality label certified acquisition"
        );
        for truncated in [true, false] {
            store.app_state_set("article-acquisition:attempt",&serde_json::json!({"artifact_hash":source.raw_hash,"complete_story":true,"known_truncation":truncated}).to_string()).unwrap();
            let input = article_input(&store, &article, 1, "objective", false).unwrap();
            assert_eq!(input.evidence[0].text, raw);
            assert_eq!(input.evidence[0].complete_story, !truncated);
        }
    }

    #[test]
    fn canonical_derivations_preserve_user_edits_and_scoped_tombstones() {
        use super::super::insights::{Assessment, EvidenceLocation, EvidenceRef, SourceClaim};
        let store = crate::store::Store::memory().unwrap();
        let text = "Acme denied opening in 2020.";
        let hash = crate::nlp::cache::artifact(&store.conn, text).unwrap();
        let reference = EvidenceRef {
            source_id: "fixture:source".into(),
            revision: "1".into(),
            artifact_hash: hash,
            acquisition_id: "fixture".into(),
            origin_root: "fixture-root".into(),
            url: "https://example.test".into(),
            published_at: None,
            retrieved_at: String::new(),
            location: EvidenceLocation::Range(crate::nlp::Span {
                start_byte: 0,
                end_byte: text.len(),
            }),
        };
        let entity = crate::nlp::EntityCandidate {
            id: "fixture:source:entity:acme".into(),
            type_hint: "org".into(),
            comparison_key: "acme".into(),
            original_forms: vec!["Acme".into()],
            mention_refs: vec![crate::nlp::Span {
                start_byte: 0,
                end_byte: 4,
            }],
        };
        let claim = SourceClaim {
            support_receipts: Vec::new(),
            id: "claim:fixture".into(),
            subject_ref: entity.id.clone(),
            predicate: "opened".into(),
            object: "office".into(),
            text: text.into(),
            polarity: "negative".into(),
            modality: "denied".into(),
            attributed_to: "Acme".into(),
            temporal_scope: "2020".into(),
            evidence_refs: vec![reference],
            extraction_scope: "original fixture".into(),
            assessment: Assessment {
                fidelity: "faithful".into(),
                relation: "insufficient".into(),
                threshold_passed: true,
                ..Default::default()
            },
        };
        let output = PipelineOutput {
            entities: vec![entity],
            claims: vec![claim],
            ..Default::default()
        };
        let mut input = PipelineInput {
            snapshot: super::super::insights::snapshot("fixture", "owner", 1, &[]),
            evidence: vec![],
            objective: "fixture".into(),
            surface: "atlas".into(),
            required_tasks_complete: true,
            save_insights: true,
            recall_enabled: false,
        };
        let publish = |id: &str, input: &PipelineInput| {
            let tx = store.conn.unchecked_transaction().unwrap();
            tx.execute("INSERT INTO insight_snapshots(id,owner_kind,owner_id,generation,input_key,inputs_json,entities_json,coverage_json,readiness_json,contract_version,policy_version,created_at) VALUES(?1,?2,?3,?4,?1,'{}','[]','[]','{}','fixture','fixture','fixture')",params![id,input.snapshot.owner_kind,input.snapshot.owner_id,input.snapshot.generation]).unwrap();
            write_derivations(&tx, id, input, &output, "fixture").unwrap();
            tx.commit().unwrap();
        };
        publish("first", &input);
        let fp = super::super::insights::claim_fingerprint(&output.claims[0]);
        let mid: String = store
            .conn
            .query_row(
                "SELECT memory_id FROM insight_claims WHERE fingerprint=?1",
                [&fp],
                |r| r.get(0),
            )
            .unwrap();
        store
            .update_memory(
                &mid,
                "User corrected the source assertion",
                "investigation",
                false,
            )
            .unwrap();
        input.snapshot.generation += 1;
        publish("edited", &input);
        assert_eq!(
            store.get_memory(&mid).unwrap().unwrap().text,
            "User corrected the source assertion"
        );
        store.delete_memory(&mid).unwrap();
        input.snapshot.generation += 1;
        publish("deleted", &input);
        assert!(
            store.get_memory(&mid).unwrap().is_none(),
            "same owner resurrected a tombstone"
        );
        input.snapshot.owner_id = "independent-owner".into();
        publish("shared-owner", &input);
        assert!(
            store.get_memory(&mid).unwrap().is_some(),
            "an independent owner was globally suppressed"
        );
    }

    #[test]
    fn report_replacement_retires_associations_but_preserves_removed_user_edits() {
        let store = crate::store::Store::memory().unwrap();
        store.conn.execute_batch("INSERT INTO memories(id,text,category,pinned,created_at,source_json) VALUES('edited','User correction','investigation',0,'fixture','{\"app\":\"article\",\"conversation_id\":\"owner\",\"message_id\":null,\"reference\":null}'); INSERT INTO insight_user_edits(memory_id) VALUES('edited'); INSERT INTO insight_claims(fingerprint,memory_id,entity_id,predicate,object_value,topic,classification,confidence,created_at,updated_at) VALUES('edit-fp','edited','Acme','announced','project','investigation','claim',0,'fixture','fixture');").unwrap();
        store.conn.execute("INSERT INTO insight_snapshots(id,owner_kind,owner_id,generation,input_key,inputs_json,entities_json,coverage_json,readiness_json,contract_version,policy_version,created_at) VALUES('previous','article','owner',1,'key','{}','[]','[]','{}','fixture','fixture','fixture')",[]).unwrap();
        store.conn.execute("INSERT INTO insight_derivations(id,snapshot_id,fingerprint,memory_id,insight_kind,status,revision,item_json,assessment_json) VALUES('derivation','previous','edit-fp','edited','claim','assessed','revision','{}','{}')",[]).unwrap();
        retire_article_associations(&store.conn, "owner", &PipelineOutput::default()).unwrap();
        assert_eq!(
            store.get_memory("edited").unwrap().unwrap().text,
            "User correction"
        );
        assert_eq!(
            store
                .conn
                .query_row(
                    "SELECT status FROM insight_derivations WHERE id='derivation'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "retired"
        );
        assert_eq!(
            store
                .conn
                .query_row("SELECT COUNT(*) FROM insight_user_edits", [], |r| r
                    .get::<_, usize>(0))
                .unwrap(),
            1
        );
    }

    fn leased_fixture() -> (Connection, EvidenceSnapshot, ClaimedTask) {
        let conn = Connection::open_in_memory().unwrap();
        crate::tasks::migrate_tables(&conn).unwrap();
        crate::tasks::migrate_additive(&conn).unwrap();
        migrate(&conn).unwrap();
        conn.execute_batch("CREATE TABLE publication_probe(value TEXT); CREATE TABLE atlas_articles(article_id TEXT); INSERT INTO atlas_articles VALUES('fixture')")
            .unwrap();
        let now = "2026-10-11T00:00:00Z";
        advance_generation(&conn, "article", "fixture", now).unwrap();
        let mut input = PipelineInput {
            snapshot: super::super::insights::snapshot("article", "fixture", 1, &[]),
            evidence: vec![],
            objective: "fixture".into(),
            surface: "article".into(),
            required_tasks_complete: true,
            save_insights: false,
            recall_enabled: false,
        };
        let task_id = enqueue_refresh(
            &conn,
            &mut input,
            &crate::provider::SettingsFile::default(),
            now,
        )
        .unwrap();
        let claimed = crate::tasks::claim_task(&conn, &task_id, "fixture-worker", 120, now)
            .unwrap()
            .unwrap();
        (conn, input.snapshot, claimed)
    }
    #[test]
    fn publication_fault_rolls_back_head_snapshot_and_canonical_writes() {
        let (mut conn, snapshot, task) = leased_fixture();
        let result = publish_snapshot(
            &mut conn,
            &snapshot,
            &PublicationFence {
                task: &task,
                now: "2026-10-11T00:00:10Z",
                live_input_key: &snapshot.key,
            },
            "insufficient_evidence",
            |tx, _| {
                tx.execute("INSERT INTO publication_probe VALUES('canonical')", [])?;
                anyhow::bail!("injected after canonical write")
            },
        );
        assert!(result.is_err());
        for table in ["publication_probe", "insight_snapshots"] {
            let count: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0);
        }
        let head: String = conn
            .query_row(
                "SELECT active_snapshot_id FROM insight_owner_state",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(head.is_empty());
        let state: String = conn
            .query_row(
                "SELECT state FROM argos_tasks WHERE id=?1",
                [&task.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state, "running");
    }
    #[test]
    fn stale_generation_epoch_expiry_and_input_cannot_mutate_active_rows() {
        for fault in ["generation", "epoch", "expiry", "input"] {
            let (mut conn, snapshot, task) = leased_fixture();
            if fault == "generation" {
                advance_generation(&conn, "article", "fixture", "2026-10-11T00:00:10Z").unwrap();
            }
            if fault == "epoch" {
                conn.execute(
                    "UPDATE argos_tasks SET lease_epoch=lease_epoch+1 WHERE id=?1",
                    [&task.id],
                )
                .unwrap();
            }
            let at = if fault == "expiry" {
                "2026-10-11T00:03:00Z"
            } else {
                "2026-10-11T00:00:10Z"
            };
            let live = if fault == "input" {
                "changed"
            } else {
                &snapshot.key
            };
            let published = publish_snapshot(
                &mut conn,
                &snapshot,
                &PublicationFence {
                    task: &task,
                    now: at,
                    live_input_key: live,
                },
                "active",
                |_, _| panic!("stale publication callback ran"),
            )
            .unwrap();
            assert!(!published, "{fault}");
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM insight_snapshots", [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0);
        }
    }
    #[test]
    fn valid_empty_refresh_commits_once_and_duplicate_terminal_event_is_inert() {
        let (mut conn, snapshot, task) = leased_fixture();
        assert!(publish_snapshot(
            &mut conn,
            &snapshot,
            &PublicationFence {
                task: &task,
                now: "2026-10-11T00:00:10Z",
                live_input_key: &snapshot.key
            },
            "insufficient_evidence",
            |_, _| Ok(())
        )
        .unwrap());
        assert!(!publish_snapshot(
            &mut conn,
            &snapshot,
            &PublicationFence {
                task: &task,
                now: "2026-10-11T00:00:11Z",
                live_input_key: &snapshot.key
            },
            "active",
            |_, _| panic!("duplicate callback")
        )
        .unwrap());
        let lifecycle: String = conn
            .query_row("SELECT lifecycle FROM insight_owner_state", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(lifecycle, "insufficient_evidence");
        let output = active_output(&conn, "article", "fixture").unwrap().unwrap();
        assert!(output.claims.is_empty());
        assert!(!output.readiness.publication_eligible);
    }
    #[test]
    fn pending_generation_preserves_history() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        assert_eq!(
            advance_generation(&conn, "article", "a", "2026-10-11").unwrap(),
            1
        );
        conn.execute(
            "UPDATE insight_owner_state SET active_snapshot_id='previous'",
            [],
        )
        .unwrap();
        assert_eq!(
            advance_generation(&conn, "article", "a", "2026-10-11").unwrap(),
            2
        );
        assert!(!record_terminal_failure(&conn, "article", "a", 1, "stale", "2026-10-11").unwrap());
        assert!(record_terminal_failure(&conn, "article", "a", 2, "failed", "2026-10-11").unwrap());
        let active: String = conn
            .query_row(
                "SELECT active_snapshot_id FROM insight_owner_state",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(active, "previous");
    }
}
