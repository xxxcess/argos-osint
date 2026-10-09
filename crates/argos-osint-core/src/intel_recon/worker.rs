//! Bounded task-graph worker for Intel report jobs.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use serde_json::json;

use crate::osint::ProviderKeys;
use crate::provider::SettingsFile;
use crate::secrets::ProviderSecret;
use crate::store::Store;

use super::body::{enqueue_article_body, fetch_article_body};
use super::brain::{upsert_recon_insights, ReconInsightUpdate};
use super::jobs::{IntelReportEvent, ReportScope};
use super::ledger::coverage_complete;
use super::modes::ReportMode;
use super::synthesize::{save_section, synthesize_section, SectionJudgment, SectionSynthInput};

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

        let owner = format!("worker-{}", std::process::id());
        let task = store.claim_report_task(&job.id, &owner, 120)?;
        if task.is_none() {
            let tasks = store.intel_report_tasks(&job.id)?;
            let pending = tasks
                .iter()
                .any(|t| matches!(t.status.as_str(), "pending" | "running" | "interrupted"));
            if !pending {
                finalize_job_state(&store, &job.id, on_event)?;
            }
            return Ok(false);
        }
        (job, task.unwrap())
    };

    on_event(IntelReportEvent::Stage {
        article_id: job.article_id.clone(),
        job_id: job.id.clone(),
        stage: format!("{} {}", task.task_type, task.section_key),
        generation: job.generation,
    });

    let result = match task.task_type.as_str() {
        "acquire_body" => run_acquire_body(&runtime, &job.article_id).await,
        "inventory" => {
            let store = Store::open(&runtime.db_path)?;
            run_inventory(&store, &job.investigation_id)
        }
        "collect" => run_collect(&runtime, &job, &task.id).await,
        "assess" => {
            let store = Store::open(&runtime.db_path)?;
            run_assess(&runtime, &store, &job, on_event)
        }
        "synthesize" => run_synthesize(&runtime, &job, &task.section_key, on_event).await,
        "finalize" => {
            let store = Store::open(&runtime.db_path)?;
            run_finalize(&store, &job, on_event)
        }
        other => Err(anyhow!("unknown task type {other}")),
    };

    let store = Store::open(&runtime.db_path)?;
    match result {
        Ok(output_ref) => {
            store.complete_report_task(&task.id, &output_ref)?;
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
            store.fail_report_task(&task.id, &err.to_string(), retryable)?;
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
            fetch_article_body(
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
            )
            .await?;
            Ok(body_id)
        }
    }
}

fn run_inventory(store: &Store, investigation_id: &str) -> Result<String> {
    let elements = store.intel_elements(investigation_id)?;
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

    let (primary, fallback) = runtime.keys.pair("firecrawl");
    if primary.is_empty() && fallback.is_empty() {
        return Ok("collect:no_firecrawl".into());
    }

    let executor = crate::osint::Executor::new()?;
    let query = runtime.article_title.chars().take(80).collect::<String>();
    let ua = runtime.settings.osint_user_agent.clone();

    let attempt_id = {
        let store = Store::open(&runtime.db_path)?;
        store.insert_report_attempt(&job.id, job.generation, task_id, "firecrawl_search")?
    };

    let result = executor
        .run_configured(
            "firecrawl_search",
            json!({"query": query, "limit": 5}),
            if ua.trim().is_empty() {
                None
            } else {
                Some(ua.as_str())
            },
            &runtime.keys,
        )
        .await;

    let store = Store::open(&runtime.db_path)?;
    let state = if matches!(&result, Ok(response) if response.error.is_none()) {
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

fn run_assess(
    runtime: &JobRuntime,
    store: &Store,
    job: &super::persist::IntelReportJobRow,
    on_event: &mut impl FnMut(IntelReportEvent),
) -> Result<String> {
    let elements = store.intel_elements(&job.investigation_id)?;
    let evidence = store.intel_evidence_list(&job.investigation_id)?;
    let mut updates = Vec::new();

    for el in &elements {
        let matching = claim_relevant_evidence(&el.id, &el.original_text, &evidence);
        let stance = if matching.is_empty() {
            "unresolved"
        } else if matching.iter().any(|e| e.stance == "contradict") {
            "disputed"
        } else if matching.iter().any(|e| e.stance == "support") {
            "supported"
        } else {
            "unresolved"
        };
        let rationale = match stance {
            "supported" => "Corroborating evidence present in retained sources for this claim.",
            "disputed" => "Contrary evidence present; not treated as confirmed.",
            _ => "No independent corroboration located for this specific claim; unresolved.",
        };
        let matching_ids: Vec<&str> = matching.iter().map(|e| e.id.as_str()).collect();
        store.update_element_assessment(
            &el.id,
            if stance == "unresolved" {
                "unresolved"
            } else {
                "assessed"
            },
            stance,
            rationale,
            "",
            &json!(matching_ids).to_string(),
        )?;

        let (entity, predicate, object) = split_claim_triple(&el.original_text, &el.fingerprint);
        updates.push(ReconInsightUpdate {
            entity,
            namespace: "news".into(),
            predicate,
            object,
            topic: el.element_type.clone(),
            claim: el.original_text.clone(),
            classification: if stance == "supported" {
                "fact".into()
            } else {
                "inference".into()
            },
            confidence: match stance {
                "supported" => 0.72,
                "disputed" => 0.45,
                _ => 0.35,
            },
            article_id: job.article_id.clone(),
            source_url: runtime.article_url.clone(),
            published_at: runtime.article_published.clone(),
            reliability: String::new(),
            info_credibility: 0,
            admiralty: String::new(),
            rsp_status: String::new(),
            element_id: Some(el.id.clone()),
            investigation_id: Some(job.investigation_id.clone()),
            stance: stance.into(),
            rationale: rationale.into(),
        });
    }

    let n = upsert_recon_insights(store, &runtime.run_id, &updates, &[])?;
    on_event(IntelReportEvent::InsightsUpdated {
        article_id: job.article_id.clone(),
        job_id: job.id.clone(),
        generation: job.generation,
    });

    let shared = json!({
        "elements_assessed": elements.len(),
        "evidence": evidence.len(),
        "as_of": chrono::Utc::now().to_rfc3339(),
    });
    store.update_shared_assessment(&job.investigation_id, &shared.to_string())?;
    Ok(format!("assessed:{n}"))
}

fn split_claim_triple(text: &str, fingerprint: &str) -> (String, String, String) {
    if let Ok(parts) = serde_json::from_str::<(String, String, String, String)>(fingerprint) {
        return (parts.1, parts.2, parts.3);
    }
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() >= 3 {
        (
            words[0].trim_matches(|c: char| !c.is_alphanumeric()).into(),
            words[1].trim_matches(|c: char| !c.is_alphanumeric()).into(),
            words[2..]
                .join(" ")
                .trim_matches(|c: char| c == '.' || c == ',')
                .chars()
                .take(80)
                .collect(),
        )
    } else {
        (
            text.chars().take(40).collect(),
            "states".into(),
            "claim".into(),
        )
    }
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
            .map(|b| b.body_markdown.chars().take(2500).collect::<String>())
            .unwrap_or_default();
        let settings: ReportScope = serde_json::from_str(&job.settings_json).unwrap_or_default();
        let upstream = if section_key == "bluf" {
            store
                .intel_report_sections(&job.id)?
                .into_iter()
                .filter(|s| s.section_key != "bluf" && s.status == "complete")
                .map(|s| {
                    let summary = serde_json::from_str::<SectionJudgment>(&s.judgment_json)
                        .map(|j| j.summary)
                        .unwrap_or_else(|_| s.markdown.chars().take(400).collect());
                    (s.title, summary)
                })
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
    let output = synthesize_section(runtime.synthesis_secret.as_ref(), &input).await?;
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
    let state = if sections_done == job.sections_total && coverage_complete(&elements) {
        "completed"
    } else if failed_tasks {
        "partial"
    } else if sections_done > 0 {
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
