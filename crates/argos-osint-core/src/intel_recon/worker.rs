//! Bounded task-graph worker for Intel report jobs.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rusqlite::OptionalExtension;

use anyhow::{anyhow, Result};
use serde_json::json;

use crate::osint::ProviderKeys;
use crate::provider::SettingsFile;
use crate::secrets::ProviderSecret;
use crate::store::Store;

use super::body::{enqueue_article_body, fetch_article_body_with_runtime};
use super::jobs::{IntelReportEvent, ReportScope};
use super::ledger::coverage_complete;
use super::modes::ReportMode;
use crate::secrets::AuthFile;

use super::synthesize::{save_section, synthesize_section_with_runtime, SectionSynthInput};

#[derive(Clone)]
pub struct JobRuntime {
    pub db_path: PathBuf,
    pub job_id: String,
    pub article_title: String,
    pub article_url: String,
    pub article_preview: String,
    pub article_published: String,
    pub article_domain: String,
    pub run_id: String,
    pub keys: ProviderKeys,
    pub auth: AuthFile,
    pub synthesis_secret: Option<ProviderSecret>,
    pub classifier_secret: Option<ProviderSecret>,
    pub settings: SettingsFile,
    pub cancel: Arc<AtomicBool>,
}

pub async fn run_job_to_completion(
    runtime: JobRuntime,
    mut on_event: impl FnMut(IntelReportEvent) + Send,
) -> Result<()> {
    loop {
        if runtime.cancel.load(Ordering::Relaxed) {
            let store = Store::open(&runtime.db_path)?;
            let _ = cancel_if_needed(&store, &runtime.job_id);
            return Ok(());
        }
        let progressed = run_job_slice(runtime.clone(), &mut on_event).await?;
        if !progressed {
            break;
        }
    }
    Ok(())
}

/// Execute at most one ready task. Returns false when the job is terminal or paused.
pub async fn run_job_slice(
    runtime: JobRuntime,
    on_event: &mut (impl FnMut(IntelReportEvent) + Send),
) -> Result<bool> {
    let (job, task) = {
        let store = Store::open(&runtime.db_path)?;
        store.interrupt_expired_leases()?;
        let job = store
            .intel_report_job(&runtime.job_id)?
            .ok_or_else(|| anyhow!("job missing"))?;

        if matches!(
            job.state.as_str(),
            "completed" | "failed" | "cancelled" | "partial" | "paused"
        ) {
            return Ok(false);
        }

        store.update_report_job(
            &job.id,
            "running",
            &job.stage,
            job.sections_done,
            job.elements_done,
            &job.warning,
            &job.error,
        )?;

        let owner = format!("worker-{}-{}", std::process::id(), crate::recon::id("inst"));
        let task = store.claim_report_task(&job.id, &owner, 120)?;
        (job, task)
    };
    let Some(task) = task else {
        let terminal = {
            let store = Store::open(&runtime.db_path)?;
            !store
                .intel_report_tasks(&job.id)?
                .iter()
                .any(|t| matches!(t.status.as_str(), "pending" | "running" | "interrupted"))
        };
        if terminal {
            terminal_refresh(&runtime, &job, on_event).await?;
            let store = Store::open(&runtime.db_path)?;
            finalize_job_state(&store, &job.id, on_event)?;
        }
        return Ok(false);
    };

    on_event(IntelReportEvent::Stage {
        article_id: job.article_id.clone(),
        job_id: job.id.clone(),
        stage: format!("{} {}", task.task_type, task.section_key),
        generation: job.generation,
    });

    let renew_cancel = Arc::new(AtomicBool::new(false));
    let renew_cancel_clone = renew_cancel.clone();
    let db_path_clone = runtime.db_path.clone();
    let task_id_clone = task.id.clone();
    let lease_epoch = task.lease_epoch;
    let renew_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        interval.tick().await;
        while !renew_cancel_clone.load(Ordering::Relaxed) {
            interval.tick().await;
            if renew_cancel_clone.load(Ordering::Relaxed) {
                break;
            }
            if let Ok(store) = Store::open(&db_path_clone) {
                let _ = store.renew_report_task_lease(&task_id_clone, lease_epoch, 120);
            }
        }
    });

    let result = match task.task_type.as_str() {
        "acquire_body" => run_acquire_body(&runtime, &job.article_id).await,
        "inventory" => run_inventory(&runtime, &job).await,
        "collect" => run_collect(&runtime, &job, &task.id).await,
        "assess" => run_assess(&runtime, &job).await,
        "refresh_insights" => run_refresh(&runtime, &job, on_event).await,
        "synthesize" => run_synthesize(&runtime, &job, &task.section_key, on_event).await,
        "finalize" => {
            terminal_refresh(&runtime, &job, on_event).await?;
            let store = Store::open(&runtime.db_path)?;
            run_finalize(&store, &job, on_event)
        }
        other => Err(anyhow!("unknown task type {other}")),
    };

    renew_cancel.store(true, Ordering::Relaxed);
    renew_handle.abort();

    let store = Store::open(&runtime.db_path)?;
    match result {
        Ok(output_ref) => {
            let fenced_ok =
                store.complete_report_task_fenced(&task.id, &output_ref, lease_epoch)?;
            if !fenced_ok {
                return Ok(false);
            }
            let sections = store.intel_report_sections(&job.id)?;
            let done = sections.iter().filter(|s| s.status == "complete").count() as i64;
            let elements = store.intel_elements(&job.investigation_id)?;
            let el_done = elements
                .iter()
                .filter(|e| {
                    matches!(
                        e.status.as_str(),
                        "assessed" | "unresolved" | "superseded" | "excluded"
                    )
                })
                .count() as i64;
            store.update_report_job(
                &job.id,
                "running",
                &format!("done {}", task.task_type),
                done,
                el_done,
                &job.warning,
                "",
            )?;
            Ok(true)
        }
        Err(err) => {
            let retryable = task.attempts < task.max_attempts;
            let _ =
                store.fail_report_task_fenced(&task.id, &err.to_string(), retryable, lease_epoch);
            if !retryable {
                store.update_report_job(
                    &job.id,
                    "running",
                    "task failed",
                    job.sections_done,
                    job.elements_done,
                    &job.warning,
                    &err.to_string(),
                )?;
            }
            Ok(true)
        }
    }
}

async fn run_acquire_body(runtime: &JobRuntime, article_id: &str) -> Result<String> {
    let (article, enqueue) = {
        let store = Store::open(&runtime.db_path)?;
        let article = store
            .atlas_article(&runtime.run_id, article_id)?
            .or_else(|| {
                store
                    .atlas_recent_articles()
                    .ok()
                    .and_then(|rows| rows.into_iter().find(|a| a.id == article_id))
            });
        let Some(article) = article else {
            return Ok("no_article".into());
        };
        let enqueue = enqueue_article_body(&store, &article, false)?;
        (article, enqueue)
    };
    match enqueue {
        super::body::EnqueueOutcome::Cached(_) => Ok("cached".into()),
        super::body::EnqueueOutcome::AlreadyRunning { .. } => Ok("subscribed".into()),
        super::body::EnqueueOutcome::Cooldown { .. } => Ok("cooldown".into()),
        super::body::EnqueueOutcome::Start {
            body_id,
            force_refresh,
            ..
        } => {
            let ua = runtime.settings.osint_user_agent.clone();
            fetch_article_body_with_runtime(
                &runtime.db_path,
                &body_id,
                &article.title,
                &article.description,
                runtime.keys.clone(),
                runtime.synthesis_secret.clone(),
                runtime.classifier_secret.clone(),
                if ua.trim().is_empty() { None } else { Some(ua) },
                force_refresh,
                runtime.cancel.clone(),
                |_| {},
                Some(runtime),
            )
            .await?;
            Ok(body_id)
        }
    }
}

async fn run_inventory(
    runtime: &JobRuntime,
    job: &super::persist::IntelReportJobRow,
) -> Result<String> {
    let mut input = {
        let store = Store::open(&runtime.db_path)?;
        let article = store
            .atlas_article(&runtime.run_id, &job.article_id)?
            .ok_or_else(|| anyhow!("article missing"))?;
        let generation = crate::investigation::insight_persist::desired_generation(
            &store.conn,
            "article",
            &job.article_id,
        )?;
        crate::investigation::insight_persist::article_input(
            &store,
            &article,
            generation,
            &runtime.article_title,
            false,
        )?
    };
    input.surface = "atlas".into(); // Inventory stages claims while article Recon is active.
    input.snapshot.key = crate::investigation::insights::input_key(&input, &runtime.settings);
    let output = crate::investigation::insights::run_pipeline(
        &mut input,
        &runtime.auth,
        &runtime.settings,
        &runtime.cancel,
        &runtime.db_path,
    )
    .await;
    let store = Store::open(&runtime.db_path)?;
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            if runtime.cancel.load(Ordering::Acquire) {
                return Err(error);
            }
            let mut withheld = crate::investigation::insights::PipelineOutput::default();
            withheld
                .readiness
                .gaps
                .push(format!("Source inventory blocked: {error}"));
            store.update_shared_assessment(
                &job.investigation_id,
                &serde_json::to_string(&withheld)?,
            )?;
            return Ok("source_inventory_blocked; retained leads require assessment".into());
        }
    };
    for claim in &output.claims {
        let fingerprint = crate::investigation::insights::claim_fingerprint(claim);
        store.upsert_element(
            &job.investigation_id,
            &format!("claim:{fingerprint}"),
            &fingerprint,
            "claim",
            &claim.text,
            "claim_assessor",
            "pending",
        )?;
    }
    let elements = store.intel_elements(&job.investigation_id)?;
    Ok(format!("elements:{}", elements.len()))
}

async fn run_collect(
    runtime: &JobRuntime,
    job: &super::persist::IntelReportJobRow,
    task_id: &str,
) -> Result<String> {
    {
        let store = Store::open(&runtime.db_path)?;
        let body = store.article_body_for_article(&job.article_id)?;
        if let Some(body) = &body {
            if !body.body_markdown.is_empty() {
                let existing = store.intel_evidence_list(&job.investigation_id)?;
                if !existing.iter().any(|e| e.origin == "article") {
                    let excerpt: String = body.body_markdown.chars().take(2000).collect();
                    store.insert_evidence(
                        &job.investigation_id,
                        &runtime.article_url,
                        &runtime.article_domain,
                        &excerpt,
                        "article_body",
                        "[]",
                        &runtime.article_published,
                        "",
                        "support",
                        "article",
                        &body.fetch_tool,
                        &job.article_id,
                        "",
                    )?;
                }
            }
        }
    }

    let store = Store::open(&runtime.db_path)?;
    let existing = store.intel_evidence_list(&job.investigation_id)?;
    if existing.iter().any(|e| e.origin == "search") {
        store.update_report_job(
            &job.id,
            "running",
            "collected",
            job.sections_done,
            job.elements_done,
            &job.warning,
            "",
        )?;
        return Ok("collect:reused_existing".into());
    }
    drop(store);

    let (primary, fallback) = runtime.keys.pair("firecrawl");
    if primary.is_empty() && fallback.is_empty() {
        return Ok("collect:no_firecrawl".into());
    }

    let query = runtime.article_title.chars().take(80).collect::<String>();

    let attempt_id = {
        let store = Store::open(&runtime.db_path)?;
        store.insert_report_attempt(&job.id, job.generation, task_id, "firecrawl_search")?
    };

    let tool_runner = crate::recon::tool_runner::ToolRunner::new(
        &runtime.db_path,
        runtime.settings.clone(),
        runtime.keys.clone(),
    );
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let result = tool_runner
        .execute_tool(
            "firecrawl_search",
            json!({"query": query, "limit": 5}),
            false,
            &cancel,
        )
        .await;

    let store = Store::open(&runtime.db_path)?;
    let state = if matches!(&result, Ok(ref response) if response.error.is_none()) {
        "success"
    } else {
        "failed"
    };
    store.finish_report_attempt(&job.id, &attempt_id, state)?;

    match result {
        Ok(res) if res.error.is_none() => {
            let items = crate::osint::results::extract_search_results(&res.observations);
            for item in items.iter().take(5) {
                let url = item.url.as_str();
                let snippet = item.snippet.as_str();
                if snippet.is_empty() {
                    continue;
                }
                let stance = if url == runtime.article_url {
                    "mention"
                } else {
                    "support"
                };
                store.insert_evidence(
                    &job.investigation_id,
                    url,
                    "",
                    snippet,
                    "search",
                    "[]",
                    "",
                    "",
                    stance,
                    "firecrawl_search",
                    "firecrawl_search",
                    "",
                    "search snippet; not full article",
                )?;
            }
            store.update_report_job(
                &job.id,
                "running",
                "collected",
                job.sections_done,
                job.elements_done,
                &job.warning,
                "",
            )?;
            Ok("collect:search".into())
        }
        Ok(res) => Ok(format!(
            "collect:search_error:{}",
            res.error.unwrap_or_default()
        )),
        Err(err) => Ok(format!("collect:search_failed:{err}")),
    }
}

fn claim_relevant_evidence<'a>(
    claim_id: &str,
    claim_text: &str,
    evidence: &'a [super::persist::IntelEvidenceRow],
) -> Vec<&'a super::persist::IntelEvidenceRow> {
    let claim_lower = claim_text.to_ascii_lowercase();
    let words: Vec<&str> = claim_lower
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|w| w.len() >= 4)
        .collect();

    evidence
        .iter()
        .filter(|ev| {
            if !ev.claim_ids_json.is_empty()
                && ev.claim_ids_json != "[]"
                && ev.claim_ids_json.contains(claim_id)
            {
                return true;
            }
            if words.is_empty() {
                return false;
            }
            let excerpt_lower = ev.excerpt.to_ascii_lowercase();
            let matches = words.iter().filter(|&&w| excerpt_lower.contains(w)).count();
            matches >= 2.min(words.len())
        })
        .collect()
}

async fn run_assess(
    runtime: &JobRuntime,
    job: &super::persist::IntelReportJobRow,
) -> Result<String> {
    use crate::investigation::decisions::{
        templates::template_claim_relation, DecisionPolicy, DecisionService, DecisionState,
    };
    let (evidence, elements) = {
        let store = Store::open(&runtime.db_path)?;
        (
            store.intel_evidence_list(&job.investigation_id)?,
            store.intel_elements(&job.investigation_id)?,
        )
    };
    for element in &elements {
        let matching = claim_relevant_evidence(&element.id, &element.original_text, &evidence);
        let mut state = DecisionState::new().with_candidate(json!({"claim":element.original_text}));
        for item in &matching {
            state = state.with_evidence_passage(
                &item.id,
                &item.source_url,
                &item.source_domain,
                &item.excerpt,
                Some("Lexical overlap is a candidate, not support. Assess independently.".into()),
            );
        }
        let id = format!("{}:{}:assess:{}", job.id, job.generation, element.id);
        let scope = crate::recon::model_exec::OperationScope::new(&id, "claim_assessor")
            .with_generation(job.generation as u32);
        let result = DecisionService::new()
            .evaluate_shared(
                &runtime.auth,
                &runtime.settings,
                &scope,
                &template_claim_relation(),
                &state,
                &DecisionPolicy::default(),
                &runtime.cancel,
                Some(&runtime.db_path),
            )
            .await;
        let (stance,rationale)=match result {
            Ok((result,outcomes)) if outcomes.get("claim_relation").is_some_and(|o|o.is_approved()) => match result.answers.get("claim_relation").map(|a|a.label.as_str()) {
                Some("supports") => ("supported","Semantic assessment of retained evidence supports the qualified assertion."),
                Some("contradicts") => ("disputed","Semantic assessment identifies counterevidence to the qualified assertion."),
                Some("mentions_only") => ("mention","Evidence mentions the subject without establishing the assertion."),
                _ => ("unresolved","Semantic assessment supplies no conclusive corroboration."),
            },
            _ => ("unresolved","Semantic assessment unavailable or uncertain; assertion remains a claim."),
        };
        let ids: Vec<_> = matching.iter().map(|e| &e.id).collect();
        let store = Store::open(&runtime.db_path)?;
        store.update_element_assessment(
            &element.id,
            if stance == "unresolved" || stance == "mention" {
                "unresolved"
            } else {
                "assessed"
            },
            stance,
            rationale,
            "",
            &json!(ids).to_string(),
        )?;
    }
    let store = Store::open(&runtime.db_path)?;
    store.update_shared_assessment(&job.investigation_id,&json!({"elements_assessed":elements.len(),"evidence":evidence.len(),"readiness":"refresh_pending"}).to_string())?;
    Ok(format!("assessed:{}", elements.len()))
}

async fn terminal_refresh(
    runtime: &JobRuntime,
    job: &super::persist::IntelReportJobRow,
    on_event: &mut impl FnMut(IntelReportEvent),
) -> Result<()> {
    let changed = {
        let store = Store::open(&runtime.db_path)?;
        let article = store
            .atlas_article(&runtime.run_id, &job.article_id)?
            .ok_or_else(|| anyhow!("article missing"))?;
        let generation = crate::investigation::insight_persist::desired_generation(
            &store.conn,
            "article",
            &job.article_id,
        )?;
        let current = crate::investigation::insight_persist::article_input(
            &store,
            &article,
            generation,
            &runtime.article_title,
            true,
        )?;
        let old:Option<String>=store.conn.query_row("SELECT s.inputs_json FROM insight_owner_state h JOIN insight_snapshots s ON s.id=h.active_snapshot_id WHERE h.owner_kind='article' AND h.owner_id=?1",[&job.article_id],|r|r.get(0)).optional()?;
        match old {
            Some(old) => {
                let previous: crate::investigation::insights::EvidenceSnapshot =
                    serde_json::from_str(&old)?;
                current
                    .snapshot
                    .source_refs
                    .iter()
                    .any(|source| !previous.source_refs.contains(source))
            }
            None => !current.evidence.is_empty(),
        }
    };
    if changed && !runtime.cancel.load(Ordering::Acquire) {
        run_refresh(runtime, job, on_event).await?;
    } else {
        let store = Store::open(&runtime.db_path)?;
        let failed = store
            .intel_report_tasks(&job.id)?
            .iter()
            .any(|t| t.status == "failed");
        if failed {
            let generation = serde_json::from_str::<serde_json::Value>(&job.budget_json)?
                .get("insight_generation")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0) as u32;
            crate::investigation::insight_persist::record_terminal_failure(
                &store.conn,
                "article",
                &job.article_id,
                generation,
                "investigation finished with unchanged evidence and failed tasks",
                &chrono::Utc::now().to_rfc3339(),
            )?;
        }
    }
    Ok(())
}

async fn run_refresh(
    runtime: &JobRuntime,
    job: &super::persist::IntelReportJobRow,
    on_event: &mut impl FnMut(IntelReportEvent),
) -> Result<String> {
    let task = {
        let store = Store::open(&runtime.db_path)?;
        let article = store
            .atlas_article(&runtime.run_id, &job.article_id)?
            .ok_or_else(|| anyhow!("article missing"))?;
        let generation = crate::investigation::insight_persist::desired_generation(
            &store.conn,
            "article",
            &job.article_id,
        )?;
        let captured = serde_json::from_str::<serde_json::Value>(&job.budget_json)?
            .get("insight_generation")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(generation as u64) as u32;
        if captured != generation {
            return Ok("superseded insight generation".into());
        }
        let mut input = crate::investigation::insight_persist::article_input(
            &store,
            &article,
            generation,
            &runtime.article_title,
            true,
        )?;
        let collections_complete = store
            .intel_report_tasks(&job.id)?
            .iter()
            .filter(|t| t.task_type == "collect")
            .all(|t| t.status == "completed");
        input.required_tasks_complete = collections_complete
            && store
                .intel_elements(&job.investigation_id)?
                .iter()
                .all(|e| {
                    matches!(
                        e.status.as_str(),
                        "assessed" | "unresolved" | "superseded" | "excluded"
                    )
                });
        crate::investigation::insight_persist::enqueue_refresh(
            &store.conn,
            &mut input,
            &runtime.settings,
            &chrono::Utc::now().to_rfc3339(),
        )?
    };
    let result = crate::investigation::insight_persist::run_refresh_slice(
        &runtime.db_path,
        &task,
        &runtime.auth,
        &runtime.settings,
        &runtime.cancel,
    )
    .await;
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            if runtime.cancel.load(Ordering::Acquire) {
                return Err(error);
            }
            let store = Store::open(&runtime.db_path)?;
            let mut withheld = crate::investigation::insights::PipelineOutput::default();
            withheld
                .readiness
                .gaps
                .push(format!("Insight refresh blocked: {error}"));
            store.update_shared_assessment(
                &job.investigation_id,
                &serde_json::to_string(&withheld)?,
            )?;
            return Ok("insights_blocked; prior snapshot retained".into());
        }
    };
    if result.is_some() {
        on_event(IntelReportEvent::InsightsUpdated {
            article_id: job.article_id.clone(),
            job_id: job.id.clone(),
            generation: job.generation,
        });
    }
    let store = Store::open(&runtime.db_path)?;
    let output = crate::investigation::insight_persist::active_output(
        &store.conn,
        "article",
        &job.article_id,
    )?
    .unwrap_or_default();
    store.update_shared_assessment(&job.investigation_id, &serde_json::to_string(&output)?)?;
    Ok(if output.readiness.publication_eligible {
        "insights_updated"
    } else {
        "insufficient_evidence"
    }
    .into())
}

async fn run_synthesize(
    runtime: &JobRuntime,
    job: &super::persist::IntelReportJobRow,
    section_key: &str,
    on_event: &mut impl FnMut(IntelReportEvent),
) -> Result<String> {
    let input = {
        let store = Store::open(&runtime.db_path)?;
        let mode = ReportMode::parse(&job.mode).unwrap_or(ReportMode::Verify);
        let section = store
            .intel_report_section(&job.id, section_key)?
            .ok_or_else(|| anyhow!("section {section_key} missing"))?;
        store.upsert_report_section_markdown(
            &job.id,
            section_key,
            &section.markdown,
            "running",
            &section.evidence_ids_json,
            &section.judgment_json,
            section.assessment_version,
            "",
        )?;
        on_event(IntelReportEvent::Section {
            article_id: job.article_id.clone(),
            job_id: job.id.clone(),
            section_key: section_key.into(),
            status: "running".into(),
            generation: job.generation,
        });

        let elements = store.intel_elements(&job.investigation_id)?;
        let evidence = store.intel_evidence_list(&job.investigation_id)?;
        let inv = store
            .intel_investigation_by_id(&job.investigation_id)?
            .ok_or_else(|| anyhow!("investigation missing"))?;
        let body = store.article_body_for_article(&job.article_id)?;
        let body_excerpt = body
            .as_ref()
            .map(|b| b.body_markdown.clone())
            .unwrap_or_default();
        let settings: ReportScope = serde_json::from_str(&job.settings_json).unwrap_or_default();
        let upstream = if section_key == "bluf" {
            store
                .intel_report_sections(&job.id)?
                .into_iter()
                .filter(|s| s.section_key != "bluf" && s.status == "complete")
                .map(|s| (s.title, s.markdown))
                .collect()
        } else {
            Vec::new()
        };

        (
            SectionSynthInput {
                mode,
                section_key: section_key.into(),
                section_title: section.title.clone(),
                objective: {
                    let guideline =
                        super::modes::section_guideline(mode, section_key).unwrap_or("");
                    format!(
                        "Write the '{section_key}' section (## {}) for a {} report.\n\
Writing target, structure, and style:\n{}\n\
Mode style guide:\n{}\n\
The writing targets are recommendations—soft limits that should expand when necessary to preserve material evidence or uncertainty.",
                        section.title,
                        mode.title(),
                        guideline,
                        mode.style_guide()
                    )
                },
                article_title: runtime.article_title.clone(),
                article_url: runtime.article_url.clone(),
                preview: runtime.article_preview.clone(),
                body_excerpt,
                elements,
                evidence,
                shared_assessment: inv.shared_assessment_json,
                upstream_summaries: upstream,
                outlook_horizon_days: settings.outlook_horizon_days,
            },
            section,
            inv.shared_assessment_version,
        )
    };

    let (input, section, assessment_version) = input;
    let operation_id = format!("{}:{}:section:{}", job.id, job.generation, section_key);
    let output = synthesize_section_with_runtime(runtime, &input, &operation_id).await?;
    {
        let store = Store::open(&runtime.db_path)?;
        save_section(&store, &section, &output, assessment_version)?;
    }
    on_event(IntelReportEvent::Section {
        article_id: job.article_id.clone(),
        job_id: job.id.clone(),
        section_key: section_key.into(),
        status: "complete".into(),
        generation: job.generation,
    });
    Ok(section_key.into())
}

fn run_finalize(
    store: &Store,
    job: &super::persist::IntelReportJobRow,
    on_event: &mut impl FnMut(IntelReportEvent),
) -> Result<String> {
    finalize_job_state(store, &job.id, on_event)?;
    Ok("finalized".into())
}

fn finalize_job_state(
    store: &Store,
    job_id: &str,
    on_event: &mut impl FnMut(IntelReportEvent),
) -> Result<()> {
    let job = store
        .intel_report_job(job_id)?
        .ok_or_else(|| anyhow!("job missing"))?;
    let sections = store.intel_report_sections(job_id)?;
    let elements = store.intel_elements(&job.investigation_id)?;
    let sections_done = sections.iter().filter(|s| s.status == "complete").count() as i64;
    let el_done = elements
        .iter()
        .filter(|e| {
            matches!(
                e.status.as_str(),
                "assessed" | "unresolved" | "superseded" | "excluded"
            )
        })
        .count() as i64;
    let failed_tasks = store
        .intel_report_tasks(job_id)?
        .into_iter()
        .any(|t| t.status == "failed");
    let drafts_done = sections
        .iter()
        .filter(|s| s.status == "draft" && !s.markdown.is_empty())
        .count() as i64;
    let state = if sections_done == job.sections_total && coverage_complete(&elements) {
        "completed"
    } else if failed_tasks || sections_done > 0 || drafts_done > 0 {
        "partial"
    } else {
        "failed"
    };
    store.update_report_job(
        job_id,
        state,
        "finished",
        sections_done,
        el_done,
        &job.warning,
        &job.error,
    )?;
    on_event(IntelReportEvent::JobDone {
        article_id: job.article_id,
        job_id: job.id,
        state: state.into(),
        generation: job.generation,
    });
    Ok(())
}

fn cancel_if_needed(store: &Store, job_id: &str) -> Result<()> {
    if let Some(job) = store.intel_report_job(job_id)? {
        if matches!(job.state.as_str(), "queued" | "running" | "waiting") {
            store.update_report_job(
                job_id,
                "cancelled",
                "cancelled",
                job.sections_done,
                job.elements_done,
                &job.warning,
                "cancelled",
            )?;
        }
    }
    Ok(())
}
