//! Scoped, persistent asynchronous research jobs. Results are observations, not conclusions.
use crate::{
    evidence::{Basis, EvidenceRef, Observation},
    search::{SearchHit, SourcePlan},
    store::{new_id, Store},
};
use anyhow::{bail, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::{Mutex, Semaphore};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Readiness {
    NotInstalled,
    Installing,
    Configured,
    Ready,
    MissingCredentials,
    UnsupportedVersion,
    RateLimited,
    Degraded,
    Failed,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    NativeHttp,
    LocalExecutable,
    Container,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ResearchConfig {
    pub enabled: bool,
    pub mode: ExecutionMode,
    pub endpoint: String,
    pub secret_ref: String,
    pub executable: String,
    pub container: String,
    pub config_path: String,
    pub dataset_path: String,
    pub supported_version: String,
    pub detected_version: String,
    pub readiness: Readiness,
    pub concurrency: usize,
    pub rate_interval_ms: u64,
    pub timeout_secs: u64,
    pub result_limit: usize,
    pub cache_secs: u64,
    pub retries: usize,
    pub depth: usize,
    pub duration_secs: u64,
    pub allowed_hosts: Vec<String>,
    pub selected_modules: Vec<String>,
    pub selected_sites: Vec<String>,
    pub scan_profile: String,
    pub allow_sensitive: bool,
    pub allow_active: bool,
    pub verified_domains: Vec<String>,
    pub dataset_version: String,
    pub refresh_days: u64,
    pub account_capabilities: String,
    pub quota: String,
}
impl Default for ResearchConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: ExecutionMode::NativeHttp,
            endpoint: String::new(),
            secret_ref: String::new(),
            executable: String::new(),
            container: String::new(),
            config_path: String::new(),
            dataset_path: String::new(),
            supported_version: String::new(),
            detected_version: String::new(),
            readiness: Readiness::Configured,
            concurrency: 1,
            rate_interval_ms: 1000,
            timeout_secs: 30,
            result_limit: 20,
            cache_secs: 86400,
            retries: 1,
            depth: 1,
            duration_secs: 30,
            allowed_hosts: vec![],
            selected_modules: vec![],
            selected_sites: vec![],
            scan_profile: "passive".into(),
            allow_sensitive: false,
            allow_active: false,
            verified_domains: vec![],
            dataset_version: String::new(),
            refresh_days: 30,
            account_capabilities: "unverified".into(),
            quota: "unknown".into(),
        }
    }
}
pub fn defaults() -> std::collections::BTreeMap<String, ResearchConfig> {
    let mut map = std::collections::BTreeMap::new();
    for name in [
        "search",
        "identity",
        "contacts",
        "domain",
        "internetdb",
        "leakcheck",
        "shodan",
        "katana",
        "mosint",
        "whatsmyname",
        "maigret",
        "spiderfoot",
        "xposedornot",
    ] {
        let mut config = ResearchConfig::default();
        config.enabled = matches!(
            name,
            "search" | "identity" | "domain" | "internetdb" | "leakcheck"
        );
        match name {
            "katana" => {
                config.mode = ExecutionMode::LocalExecutable;
                config.supported_version = "1.7.0".into();
                config.readiness = Readiness::NotInstalled;
            }
            "spiderfoot" => {
                config.mode = ExecutionMode::LocalExecutable;
                config.supported_version = "4.0".into();
                config.readiness = Readiness::NotInstalled;
            }
            "maigret" => {
                config.mode = ExecutionMode::LocalExecutable;
                config.supported_version = "0.6.6".into();
                config.readiness = Readiness::NotInstalled;
            }
            "mosint" => {
                config.mode = ExecutionMode::LocalExecutable;
                config.supported_version = "3".into();
                config.readiness = Readiness::NotInstalled;
            }
            "shodan" => {
                config.endpoint = "https://api.shodan.io".into();
                config.secret_ref = "shodan".into();
                config.readiness = Readiness::MissingCredentials;
            }
            "xposedornot" => {
                config.endpoint = "https://api.xposedornot.com".into();
                // The public analytics endpoint also has a daily cap. A conservative
                // 15-minute provider-wide interval stays below 100 requests/day.
                config.rate_interval_ms = 900000;
            }
            "whatsmyname" => {
                config.readiness = Readiness::NotInstalled;
            }
            _ => {}
        }
        map.insert(name.into(), config);
    }
    map
}
pub fn capabilities(name: &str) -> &'static str {
    match name {"search"=>"topic/org: public documents, candidate domains; search operators are provider-dependent", "contacts"=>"domain: published contacts on an exact page; source-backed addresses", "identity"=>"selected email/handle: GitHub profile references, candidate matches", "domain"=>"domain: DNS, RDAP, certificates, Wayback", "internetdb"=>"IP: previously observed ports, CPEs, vulnerabilities (keyless)","leakcheck"=>"email/handle: public breach sources; attribution: LeakCheck", "shodan"=>"IP: observed services; host lookup requires account entitlement", "katana"=>"domain: scoped crawling, JSONL; active collection", "mosint"=>"email: contract verification required; no guaranteed mailbox verification", "whatsmyname"=>"handle: provisional platform matches from a versioned dataset", "maigret"=>"handle: selected profile enrichment; provisional", "spiderfoot"=>"optional passive collection; module provider overlap must be reviewed", "xposedornot"=>"email: breach names/analytics; domain monitoring requires verified ownership", _=>"unsupported"}
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    TopicOrganization,
    InfrastructureIp,
    DomainEmail,
    IdentityAliases,
    BreachExposure,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResearchInput {
    pub case_id: Option<String>,
    pub report_id: Option<String>,
    pub entity_id: String,
    pub label: String,
    pub action: String,
    pub depth: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResearchRun {
    pub id: String,
    pub input: ResearchInput,
    pub jobs: Vec<String>,
    pub max_depth: usize,
    pub max_entities: usize,
    pub timeout_secs: u64,
    pub request_budget: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Completed,
    Partial,
    Cancelled,
    Failed,
    RateLimited,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResearchJob {
    #[serde(default)]
    pub provider_version: Option<String>,
    #[serde(default)]
    pub metadata: Vec<ResultMetadata>,
    pub id: String,
    pub run_id: String,
    pub input: ResearchInput,
    pub provider: String,
    pub stage: Stage,
    pub state: JobState,
    pub progress: String,
    pub created_at: String,
    pub finished_at: Option<String>,
    pub elapsed_ms: u64,
    pub error: Option<String>,
    pub hits: Vec<SearchHit>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ResultMetadata {
    pub fields: serde_json::Value,
    pub event_time: Option<String>,
    pub event_uncertainty: Option<String>,
}
struct AdapterOutput {
    hits: Vec<SearchHit>,
    metadata: Vec<ResultMetadata>,
    warnings: Vec<String>,
}
pub fn stage_for(provider: &str) -> Stage {
    match provider {
        "domain" | "internetdb" | "shodan" | "spiderfoot" => Stage::InfrastructureIp,
        "contacts" | "katana" | "mosint" => Stage::DomainEmail,
        "identity" | "whatsmyname" | "maigret" => Stage::IdentityAliases,
        "leakcheck" | "xposedornot" => Stage::BreachExposure,
        _ => Stage::TopicOrganization,
    }
}
impl Store {
    pub fn research_job_is_current(&self, job: &ResearchJob) -> Result<bool> {
        Ok(job
            .input
            .case_id
            .as_ref()
            .is_none_or(|id| self.check_case_write(id, Some(&job.created_at)).is_ok()))
    }
    pub fn save_job(&self, key: &str, job: &ResearchJob) -> Result<()> {
        if let Some(id) = &job.input.case_id {
            self.check_case_write(id, Some(&job.created_at))?;
        }
        self.conn.execute("INSERT INTO research_jobs VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET state=excluded.state,body=excluded.body,updated_at=excluded.updated_at",params![job.id,key,serde_json::to_string(&job.state)?,serde_json::to_string(job)?,chrono::Utc::now().to_rfc3339()])?;
        Ok(())
    }
    pub fn jobs_for_case(&self, case_id: &str) -> Result<Vec<ResearchJob>> {
        let mut stmt=self.conn.prepare("SELECT body FROM research_jobs WHERE json_extract(body,'$.input.case_id')=?1 ORDER BY updated_at DESC")?;
        let bodies = stmt
            .query_map([case_id], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        bodies
            .into_iter()
            .map(|b| Ok(serde_json::from_str(&b)?))
            .collect()
    }
    pub fn jobs(&self) -> Result<Vec<ResearchJob>> {
        let mut s = self
            .conn
            .prepare("SELECT body FROM research_jobs ORDER BY updated_at DESC LIMIT 200")?;
        let bodies = s
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        bodies
            .iter()
            .map(|b| Ok(serde_json::from_str(b)?))
            .collect()
    }
    pub fn recover_jobs(&self) -> Result<()> {
        for mut job in self.jobs()? {
            if matches!(job.state, JobState::Queued | JobState::Running) {
                job.state = JobState::Partial;
                job.error=Some("Interrupted by application restart; completed requests were not replayed. Review before resuming.".into());
                self.conn.execute(
                    "UPDATE research_jobs SET state=?1,body=?2 WHERE id=?3",
                    params![
                        serde_json::to_string(&job.state)?,
                        serde_json::to_string(&job)?,
                        job.id
                    ],
                )?;
            }
        }
        Ok(())
    }
    fn cached_job(&self, key: &str, freshness: u64) -> Result<Option<ResearchJob>> {
        let body:Option<String>=self.conn.query_row("SELECT body FROM research_jobs WHERE dedup_key=?1 AND state='\"completed\"' ORDER BY updated_at DESC LIMIT 1",[key],|r|r.get(0)).optional()?;
        let job: Option<ResearchJob> = body.map(|b| serde_json::from_str(&b)).transpose()?;
        Ok(job.filter(|j| {
            j.finished_at
                .as_ref()
                .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                .is_some_and(|t| {
                    chrono::Utc::now()
                        .signed_duration_since(t)
                        .num_seconds()
                        .max(0)
                        < freshness as i64
                })
        }))
    }
}
#[derive(Clone)]
pub struct ResearchQueue {
    events: tokio::sync::broadcast::Sender<ResearchJob>,
    db: PathBuf,
    global: Arc<Semaphore>,
    providers: Arc<Mutex<HashMap<String, Arc<Semaphore>>>>,
    rates: Arc<Mutex<HashMap<String, Arc<Mutex<std::time::Instant>>>>>,
    dedup: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
    pub cancel: Arc<AtomicBool>,
}
impl ResearchQueue {
    pub fn new(db: PathBuf, limit: usize) -> Self {
        Self {
            events: tokio::sync::broadcast::channel(64).0,
            db,
            global: Arc::new(Semaphore::new(limit.clamp(1, 8))),
            providers: Default::default(),
            rates: Default::default(),
            dedup: Default::default(),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<ResearchJob> {
        self.events.subscribe()
    }
    pub async fn recover(&self) -> Result<()> {
        let db = self.db.clone();
        crate::workers::spawn_blocking(move || Store::open(&db)?.recover_jobs()).await??;
        Ok(())
    }
    pub async fn execute(
        &self,
        input: ResearchInput,
        provider: &str,
        config: ResearchConfig,
        plan: SourcePlan,
        secret: Option<String>,
    ) -> ResearchJob {
        let started = std::time::Instant::now();
        let mut job = ResearchJob {
            provider_version: if config.detected_version.is_empty() {
                None
            } else {
                Some(config.detected_version.clone())
            },
            metadata: vec![],
            id: new_id("job"),
            run_id: new_id("run"),
            input: input.clone(),
            provider: provider.into(),
            stage: stage_for(provider),
            state: JobState::Queued,
            progress: "queued".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
            finished_at: None,
            elapsed_ms: 0,
            error: None,
            hits: vec![],
        };
        // Scope and configuration participate in deduplication. Secret values never enter the DB.
        use sha2::{Digest, Sha256};
        let secret_fingerprint = format!(
            "{:x}",
            Sha256::digest(secret.as_deref().unwrap_or("").as_bytes())
        );
        let fingerprint = serde_json::to_string(&(
            provider,
            &input.entity_id,
            &input.label,
            &input.action,
            &config,
            &plan.searx_url,
            &plan.facts,
            &plan.web,
            &plan.news,
            &plan.social,
            &plan.brave_key,
            &plan.tavily_key,
            &plan.youtube_key,
            &plan.github_token,
            &secret_fingerprint,
        ))
        .unwrap_or_default();
        let key = format!("{:x}", Sha256::digest(fingerprint.as_bytes()));
        let lock = {
            self.dedup
                .lock()
                .await
                .entry(key.clone())
                .or_default()
                .clone()
        };
        if let Err(err) = self.persist(&key, &job).await {
            job.state = JobState::Failed;
            job.error = Some(err.to_string());
            return job;
        }
        let _equivalent = tokio::select! {guard=lock.lock()=>guard,_=wait_cancel(self.cancel.clone())=>{job.state=JobState::Cancelled;job.error=Some("cancelled while queued".into());job.finished_at=Some(chrono::Utc::now().to_rfc3339());let _=self.persist(&key,&job).await;return job;}};
        if let Some(case_id) = input.case_id.clone() {
            let db = self.db.clone();
            let provider_name = provider.to_string();
            let allowed = crate::workers::spawn_blocking(move || -> Result<bool> {
                let store = Store::open(&db)?;
                Ok(store
                    .records::<crate::investigation::InvestigationScope>(
                        "investigation_scope",
                        None,
                        Some(&case_id),
                    )?
                    .last()
                    .is_none_or(|s| s.allowed_actions.contains(&provider_name)))
            })
            .await;
            if !matches!(allowed, Ok(Ok(true))) {
                job.state = JobState::Failed;
                job.error = Some(
                    "Provider action outside saved investigation scope or scope unavailable".into(),
                );
                job.finished_at = Some(chrono::Utc::now().to_rfc3339());
                let _ = self.persist(&key, &job).await;
                return job;
            }
        }
        let db = self.db.clone();
        let cache_key = key.clone();
        let freshness = config.cache_secs;
        if let Ok(Ok(Some(mut cached))) = crate::workers::spawn_blocking(move || {
            Store::open(&db)?.cached_job(&cache_key, freshness)
        })
        .await
        {
            let original = cached.id.clone();
            // The queued request has its own terminal job record. Reuse source data,
            // retain the original job/history, and do not leave an orphan queued job.
            cached.progress = format!("cached evidence reused from {original}");
            cached.id = job.id.clone();
            cached.input = input.clone();
            cached.created_at = job.created_at.clone();
            if let Err(error) = self.persist(&key, &cached).await {
                cached.state = JobState::Partial;
                cached.error = Some(format!(
                    "Cached evidence reused but result persistence failed: {error}"
                ));
            }
            return cached;
        }
        if let Err(err) = self.persist(&key, &job).await {
            job.state = JobState::Failed;
            job.error = Some(err.to_string());
            return job;
        }
        let limits = {
            self.providers
                .lock()
                .await
                .entry(provider.into())
                .or_insert_with(|| Arc::new(Semaphore::new(config.concurrency.clamp(1, 4))))
                .clone()
        };
        let permits = async {
            let global = self.global.acquire().await.map_err(|e| e.to_string())?;
            let per = limits.acquire().await.map_err(|e| e.to_string())?;
            Ok::<_, String>((global, per))
        };
        let work = async {
            let _permits = permits.await?;
            if !config.enabled {
                return Err("Integration disabled. Enable it under Providers → Research.".into());
            }
            job.state = JobState::Running;
            job.progress = "collecting scoped public evidence".into();
            self.persist(&key, &job).await.map_err(|e| e.to_string())?;
            let rate = {
                self.rates
                    .lock()
                    .await
                    .entry(provider.into())
                    .or_insert_with(|| {
                        Arc::new(Mutex::new(
                            std::time::Instant::now() - Duration::from_secs(3600),
                        ))
                    })
                    .clone()
            };
            let mut last = rate.lock().await;
            let due = *last + Duration::from_millis(config.rate_interval_ms.min(3600000));
            if let Some(wait) = due.checked_duration_since(std::time::Instant::now()) {
                if wait >= Duration::from_secs(config.timeout_secs.clamp(1, 600)) {
                    return Err(format!(
                        "429 local provider limit; retry after {} seconds",
                        wait.as_secs() + 1
                    ));
                }
                tokio::time::sleep(wait).await;
            }
            *last = std::time::Instant::now();
            drop(last);
            crate::search::with_http_policy(
                crate::search::HttpPolicy {
                    timeout_secs: config.timeout_secs,
                    retries: config.retries,
                    rate_interval_ms: config.rate_interval_ms,
                    cache_secs: config.cache_secs,
                },
                run_adapter(provider, &input, &config, &plan, secret.as_deref()),
            )
            .await
        };
        let result = tokio::select! {
            result=tokio::time::timeout(Duration::from_secs(config.timeout_secs.clamp(1,600)),work)=>result.unwrap_or_else(|_|Err("Job timed out; subprocess group cleaned up".into())),
            _=wait_cancel(self.cancel.clone())=>Err("cancelled".into()),
        };
        match result {
            Ok(output) => {
                job.metadata = output
                    .metadata
                    .into_iter()
                    .map(|mut m| {
                        redact_value(&mut m.fields, secret.as_deref());
                        m
                    })
                    .collect();
                job.hits = output
                    .hits
                    .into_iter()
                    .take(config.result_limit.min(200))
                    .map(|mut h| {
                        h.title = redact(&h.title, secret.as_deref());
                        h.url = redact(&h.url, secret.as_deref());
                        h.snippet = redact(&h.snippet, secret.as_deref());
                        h
                    })
                    .collect();
                job.state = if output.warnings.is_empty() {
                    JobState::Completed
                } else {
                    job.error = Some(redact(&output.warnings.join("; "), secret.as_deref()));
                    JobState::Partial
                };
                job.progress = format!("{} observations available for review", job.hits.len());
            }
            Err(err) => {
                job.state = if err == "cancelled" {
                    JobState::Cancelled
                } else if err.contains("429") {
                    JobState::RateLimited
                } else {
                    JobState::Failed
                };
                job.error = Some(redact(&err, secret.as_deref()));
                job.progress = "Review job details".into();
            }
        }
        job.elapsed_ms = started.elapsed().as_millis() as u64;
        job.finished_at = Some(chrono::Utc::now().to_rfc3339());
        if let Err(err) = self.persist(&key, &job).await {
            job.error = Some(format!("Could not persist results: {err}"));
            job.state = JobState::Partial;
        }
        job
    }
    async fn persist(&self, key: &str, job: &ResearchJob) -> Result<()> {
        let db = self.db.clone();
        let key = key.to_string();
        let job = job.clone();
        let job_for_event = job.clone();
        crate::workers::spawn_blocking(move|| {
            let store=Store::open(&db)?;
            let transaction=rusqlite::Transaction::new_unchecked(&store.conn,rusqlite::TransactionBehavior::Immediate)?;
            store.save_job(&key,&job)?;
            if job.input.case_id.is_some() && matches!(job.state,JobState::Queued|JobState::Completed|JobState::Partial) {
                use crate::evidence::EntityType;
                let kind=match job.provider.as_str(){"domain"|"contacts"=>EntityType::Domain,"internetdb"|"shodan"=>EntityType::Ip,"xposedornot"=>EntityType::Email,"identity"|"whatsmyname"=>if job.input.label.contains('@'){EntityType::Email}else{EntityType::Account},_=>EntityType::Theme};
                let existing=store.records::<crate::evidence::Entity>("entity",job.input.report_id.as_deref(),job.input.case_id.as_deref())?.iter().any(|e|e.id==job.input.entity_id);
                if !existing {if let Ok(mut entity)=crate::investigation::normalize_entity(&job.input.label,kind){entity.id=job.input.entity_id.clone();store.put_record(&entity.id,"entity",job.input.case_id.as_deref(),job.input.report_id.as_deref(),&entity)?;}}
            }
            if matches!(job.state,JobState::Completed|JobState::Partial) {
                for (n,hit) in job.hits.iter().enumerate() {
                    let observation=Observation{id:format!("{}:o{n}",job.id),case_id:job.input.case_id.clone(),report_id:job.input.report_id.clone(),entity_id:job.input.entity_id.clone(),job_id:job.id.clone(),provider:hit.title.strip_prefix('[').and_then(|s|s.split_once(']')).map(|(p,_)|p.to_string()).unwrap_or_else(||job.provider.clone()),provider_version:job.provider_version.clone(),retrieved_at:job.finished_at.clone().unwrap_or_else(||job.created_at.clone()),event_time:job.metadata.get(n).and_then(|m|m.event_time.clone()).or_else(||if job.provider=="leakcheck" {hit.snippet.strip_prefix("date: ").and_then(|s|s.split_whitespace().next()).filter(|date|date.len()>=4).map(str::to_string)}else{None}),event_uncertainty:job.metadata.get(n).and_then(|m|m.event_uncertainty.clone()).or_else(||Some("Event time not supplied; collection time is not event time".into())),basis:Basis::Observed,statement:hit.snippet.clone(),attribution:hit.title.clone(),evidence:vec![EvidenceRef{source_url:Some(hit.url.clone()),artifact_id:Some(format!("{}:o{n}:artifact",job.id)),..Default::default()}],fields:job.metadata.get(n).map(|m|m.fields.clone()).unwrap_or_else(||serde_json::json!({"title":hit.title,"url":hit.url,"snippet":hit.snippet}))};
                    store.put_record(&observation.id,"observation",observation.case_id.as_deref(),observation.report_id.as_deref(),&observation)?;
                    use crate::evidence::{Entity,EntityType,Relationship,RelationshipType,Artifact};
                    let artifact=Artifact{id:format!("{}:artifact",observation.id),source_url:Some(hit.url.clone()),retrieved_at:observation.retrieved_at.clone(),media_type:"application/json+argos-evidence".into(),body:serde_json::to_string(&serde_json::json!({"source":hit,"normalized":observation.fields}))?};
                    store.put_record(&artifact.id,"artifact",observation.case_id.as_deref(),observation.report_id.as_deref(),&artifact)?;
                    let mut relations=vec![];
                    let candidates=crate::search::TextQuery::extract(&hit.snippet);
                    for (label,kind) in candidates.domains.into_iter().map(|s|(s,EntityType::Domain)).chain(candidates.emails.into_iter().map(|s|(s,EntityType::Email))).take(32) {
                        match crate::investigation::normalize_entity(&label,kind) {
                            Ok(entity)=>{store.put_record(&entity.id,"entity",observation.case_id.as_deref(),observation.report_id.as_deref(),&entity)?;
                                if let Some(start)=hit.snippet.to_ascii_lowercase().find(&label.to_ascii_lowercase()){let mention=crate::evidence::Mention{entity_id:entity.id,original:hit.snippet[start..start+label.len()].into(),start,end:start+label.len(),evidence:observation.evidence[0].clone(),speaker:Some(hit.title.clone()),negated:false,speculative:true};store.put_record(&format!("{}:mention:{start}",observation.id),"mention",observation.case_id.as_deref(),observation.report_id.as_deref(),&mention)?;}
                            },
                            Err(error)=>store.put_record(&format!("{}:rejection:{label}",observation.id),"label_rejection",observation.case_id.as_deref(),observation.report_id.as_deref(),&serde_json::json!({"original":label,"reason":error.to_string(),"evidence":observation.evidence}))?,
                        }
                    }
                    if job.provider=="identity" && hit.url.starts_with("https://github.com/") {
                        if let Ok(url)=url::Url::parse(&hit.url){let label=url.path().trim_matches('/');if !label.is_empty()&&!label.contains('/') {
                            let id=format!("account:github:{label}");let entity=Entity{id:id.clone(),kind:EntityType::Account,label:label.into(),canonical:id.clone(),platform:Some("github".into()),aliases:vec![]};store.put_record(&id,"entity",observation.case_id.as_deref(),observation.report_id.as_deref(),&entity)?;
                            relations.push(Relationship{from:job.input.entity_id.clone(),to:id,kind:RelationshipType::CandidateIdentityAssociation,basis:Basis::Inferred,evidence:observation.evidence.clone(),uncertainty:"GitHub profile reference; corroboration required before identity resolution".into()});
                        }}
                    }

                    if job.provider=="contacts" {
                        if let Some(email)=observation.fields["email"].as_str() {
                            let id=format!("email:{}",email.to_ascii_lowercase());
                            let entity=Entity{id:id.clone(),kind:EntityType::Email,label:email.into(),canonical:id.clone(),platform:None,aliases:vec![email.into()]};
                            store.put_record(&id,"entity",observation.case_id.as_deref(),observation.report_id.as_deref(),&entity)?;
                            relations.push(Relationship{from:job.input.entity_id.clone(),to:id,kind:RelationshipType::AddressPublishedOnPage,basis:Basis::Observed,evidence:observation.evidence.clone(),uncertainty:"Publication establishes a page reference, not ownership or mailbox verification".into()});
                        }
                    }
                    if observation.provider=="doh" && hit.url.starts_with("https://cloudflare-dns.com/dns-query?") {
                        for answer in hit.snippet.split(';') {let fields=answer.split_whitespace().collect::<Vec<_>>();if fields.len()==2 && matches!(fields[0],"A"|"AAAA") {if let Ok(ip)=fields[1].parse::<std::net::IpAddr>() {if !crate::search::ip_blocked(ip){relations.push(Relationship{from:format!("domain:{}",job.input.label),to:format!("ip:{ip}"),kind:RelationshipType::DomainResolvesToIp,basis:Basis::Observed,evidence:observation.evidence.clone(),uncertainty:"Resolution observed at retrieval time; shared hosting does not establish common ownership".into()});}}}}
                    }
                    if matches!(job.provider.as_str(),"leakcheck"|"xposedornot") {relations.push(Relationship{from:job.input.entity_id.clone(),to:format!("breach:{}",hit.title),kind:RelationshipType::IdentifierReportedInBreach,basis:Basis::Observed,evidence:observation.evidence.clone(),uncertainty:"Provider-reported exposure; no credential testing or retrieval".into()});}
                    if job.provider=="whatsmyname" {
                        let platform=url::Url::parse(&hit.url).ok().and_then(|u|u.host_str().map(str::to_string)).unwrap_or_else(||"unknown".into());
                        let id=format!("account:{platform}:{}",job.input.label.to_ascii_lowercase());
                        let entity=Entity{id:id.clone(),kind:EntityType::Account,label:job.input.label.clone(),canonical:id.clone(),platform:Some(platform),aliases:vec![]};store.put_record(&id,"entity",observation.case_id.as_deref(),observation.report_id.as_deref(),&entity)?;
                        relations.push(Relationship{from:job.input.entity_id.clone(),to:id,kind:RelationshipType::CandidateIdentityAssociation,basis:Basis::Inferred,evidence:observation.evidence.clone(),uncertainty:"Matching usernames and response signatures do not establish verified identity".into()});
                    }
                    for relationship in &relations {
                        for endpoint in [&relationship.from, &relationship.to] {
                            if let Some((prefix,label)) = endpoint.split_once(':') {
                                let kind = match prefix { "domain"=>EntityType::Domain,"ip"=>EntityType::Ip,"email"=>EntityType::Email,"breach"=>EntityType::BreachEvent,_=>continue };
                                match crate::investigation::normalize_entity(label,kind) {
                                    Ok(mut entity)=>{entity.id=endpoint.clone();store.put_record(endpoint,"entity",observation.case_id.as_deref(),observation.report_id.as_deref(),&entity)?;},
                                    Err(e)=>store.put_record(&format!("{}:rejection:{endpoint}",observation.id),"label_rejection",observation.case_id.as_deref(),observation.report_id.as_deref(),&serde_json::json!({"original":endpoint,"reason":e.to_string(),"evidence":observation.evidence}))?,
                                }
                            }
                        }
                    }
                    for (index,relationship) in relations.iter().enumerate(){store.put_record(&format!("{}:relationship:{index}",observation.id),"relationship",observation.case_id.as_deref(),observation.report_id.as_deref(),relationship)?;}

                }
            }store.record_job_gap(&job)?;transaction.commit()?;Ok::<(), anyhow::Error>(())
        }).await??;
        let _ = self.events.send(job_for_event);
        Ok(())
    }
}
async fn wait_cancel(cancel: Arc<AtomicBool>) {
    loop {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
fn redact(text: &str, secret: Option<&str>) -> String {
    match secret.filter(|s| !s.is_empty()) {
        Some(s) => text.replace(s, "[redacted]"),
        None => text.into(),
    }
}
fn redact_value(value: &mut serde_json::Value, secret: Option<&str>) {
    match value {
        serde_json::Value::String(text) => *text = redact(text, secret),
        serde_json::Value::Array(items) => {
            for item in items {
                redact_value(item, secret);
            }
        }
        serde_json::Value::Object(fields) => {
            for item in fields.values_mut() {
                redact_value(item, secret);
            }
        }
        _ => {}
    }
}
pub(crate) fn valid_input(provider: &str, label: &str) -> Result<()> {
    if label.is_empty()
        || label.len() > 512
        || label.starts_with('-')
        || label.chars().any(char::is_control)
    {
        bail!("Invalid research input");
    }
    match provider {
        "contacts" | "domain" | "katana" => {
            if crate::search::query::normalize_domain(label).as_deref() != Some(label) {
                bail!("Select an exact public domain");
            }
        }
        "internetdb" | "shodan" => {
            let ip = label.parse::<std::net::IpAddr>()?;
            if crate::search::ip_blocked(ip) {
                bail!("Private and reserved addresses are refused");
            }
        }
        "xposedornot" | "mosint" => {
            if crate::search::TextQuery::extract(label).emails != vec![label.to_string()] {
                bail!("Select an exact email address");
            }
        }
        "whatsmyname" | "maigret" => {
            if !label
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
            {
                bail!("Select an exact handle");
            }
        }
        _ => {}
    }
    Ok(())
}
async fn run_adapter(
    provider: &str,
    input: &ResearchInput,
    c: &ResearchConfig,
    plan: &SourcePlan,
    secret: Option<&str>,
) -> std::result::Result<AdapterOutput, String> {
    let mut metadata = Vec::new();
    let mut warnings = Vec::new();
    eligible_action(provider, &input.label, c).map_err(|e| e.to_string())?;
    if input.depth > c.depth.min(3) {
        return Err("Pivot depth limit reached".into());
    }
    let label = &input.label;
    let hits=match provider {
        "search"=>{let mut p=plan.clone();p.domain=false;p.identity=false;crate::search::research(label,&p).await.map(|r|{warnings.extend(r.adapters.iter().filter_map(|a|if let crate::search::AdapterState::Error{message}=&a.state{Some(format!("{}: {message}",a.name))}else{None}));r.hits})},
        "identity"=>crate::search::identity::gather_profiles(vec![label.clone()],plan.github_token.clone()).await.map(|r|{warnings.extend(r.adapters.iter().filter_map(|a|if let crate::search::AdapterState::Error{message}=&a.state{Some(format!("{}: {message}",a.name))}else{None}));r.hits}),
        "contacts"=>{
            if !c.allow_active || !c.allowed_hosts.iter().any(|h|h==label){return Err("Published contact lookup requires an allowed host and active HTTP opt-in".into());}
            let url=format!("https://{label}/");let page=crate::search::fetch_page(&url).await?;
            let query=crate::search::TextQuery::extract(&page);
            Ok(query.emails.into_iter().map(|email|{
                let offset=page.to_ascii_lowercase().find(&email.to_ascii_lowercase()).unwrap_or(0);
                let start=page[..offset].char_indices().rev().nth(120).map(|(i,_)|i).unwrap_or(0);
                let end=page[offset..].char_indices().nth(email.len()+120).map(|(i,_)|offset+i).unwrap_or(page.len());
                metadata.push(ResultMetadata{fields:serde_json::json!({"email":email,"page":url,"supporting_passage":&page[start..end],"start":offset}),event_time:None,event_uncertainty:None});
                SearchHit {title:format!("Published email {email}"),url:url.clone(),snippet:format!("Address {email} published on exact page {url}. Supporting passage: {}",&page[start..end])}
            }).collect())
        },
        "domain"=>crate::search::domain::gather(vec![label.clone()]).await.map(|r|{warnings.extend(r.adapters.iter().filter_map(|a|if let crate::search::AdapterState::Error{message}=&a.state{Some(format!("{}: {message}",a.name))}else{None}));r.hits}),
        "internetdb"=>crate::search::domain::internetdb(label).await,
        "leakcheck"=>{if !c.allow_sensitive{return Err("Exposure lookup requires explicit privacy opt-in (allow_sensitive)".into());}crate::search::identity::leakcheck_one(label).await},
        "shodan"=>{
            let key=secret.ok_or("Missing Shodan secret reference")?;
            let url=format!("{}/shodan/host/{}?key={}&minify=true",endpoint(c,"https://api.shodan.io")?,label,urlencoding::encode(key));
            let value=crate::search::get_json(&url).await.map_err(|e|redact(&e,Some(key)))?;
            metadata.push(ResultMetadata{fields:serde_json::json!({"ports":value["ports"],"org":value["org"],"ip":label,"last_update":value["last_update"]}),event_time:None,event_uncertainty:Some("Provider last_update is an update time, not verification of current exposure".into())});
            parse_shodan(label,&value)
        },
        "xposedornot"=>{
            if !c.allow_sensitive{return Err("Email exposure lookup requires explicit privacy opt-in (allow_sensitive)".into());}
            let url=format!("{}/v1/breach-analytics?email={}",endpoint(c,"https://api.xposedornot.com")?,urlencoding::encode(label));
            let value=crate::search::get_json(&url).await?;let (hits,details)=parse_xon_analytics(label,&value)?;metadata=details;Ok(hits)
        },
        "whatsmyname"=>whatsmyname(label,c).await,
        // These tools may make hidden requests to arbitrary providers/targets. Fail closed until
        // an enforceable network policy and installed-version contract are available.
        "katana"|"maigret"|"spiderfoot"=>Err("Collection is unavailable: external-tool network isolation and this installed version's output contract have not been verified. Native scoped actions remain available.".into()),
        "mosint"=>Err("Mosint collection is unavailable: verify the installed version's YAML and structured output contract and disable credential/password services. Mailbox verification is not guaranteed.".into()),
        _=>Err("Unsupported research integration".into())
    }?;
    Ok(AdapterOutput {
        hits,
        metadata,
        warnings,
    })
}
fn endpoint<'a>(c: &'a ResearchConfig, official: &'a str) -> std::result::Result<&'a str, String> {
    if c.endpoint.is_empty() || c.endpoint == official {
        Ok(official)
    } else {
        Err("Endpoint must match the documented provider host; credentials cannot be sent elsewhere".into())
    }
}
pub fn parse_shodan(
    ip: &str,
    v: &serde_json::Value,
) -> std::result::Result<Vec<SearchHit>, String> {
    if let Some(error) = v.get("error").and_then(|v| v.as_str()) {
        return Err(format!("Shodan capability error: {error}"));
    }
    if v.get("ports").and_then(|v| v.as_array()).is_none() {
        return Err("Unsupported Shodan response: missing ports".into());
    }
    Ok(vec![SearchHit{title:"Shodan historical service observation".into(),url:format!("https://www.shodan.io/host/{ip}"),snippet:format!("Previously observed ports: {}. Hosting metadata: {}. This does not verify current exposure or common ownership.",v["ports"],v.get("org").unwrap_or(&serde_json::Value::Null))}])
}
pub fn parse_xon(
    email: &str,
    v: &serde_json::Value,
) -> std::result::Result<Vec<SearchHit>, String> {
    if v.get("Error").and_then(|v| v.as_str()) == Some("Not found") {
        return Ok(vec![]);
    }
    let rows = v
        .get("breaches")
        .and_then(|v| v.as_array())
        .ok_or("Unsupported XposedOrNot response: missing breaches")?;
    Ok(rows.iter().flat_map(|row|row.as_array().into_iter().flatten()).filter_map(|v|v.as_str()).map(|name|SearchHit{title:format!("XposedOrNot · {name}"),url:"https://xposedornot.com".into(),snippet:format!("Identifier {email} reported in breach {name}. Breach date and exposed-data categories unavailable in basic lookup; no credentials retrieved.")}).collect())
}
async fn whatsmyname(
    handle: &str,
    c: &ResearchConfig,
) -> std::result::Result<Vec<SearchHit>, String> {
    if c.selected_sites.is_empty() {
        return Err("Select a bounded set of sites before checking profiles".into());
    }
    if c.dataset_version.is_empty() {
        return Err(
            "Record the dataset version before checking profiles; refresh is manual".into(),
        );
    }
    let dataset = load_dataset(c.dataset_path.clone()).await?;
    let sites = dataset
        .get("sites")
        .and_then(|v| v.as_array())
        .ok_or("WhatsMyName dataset must contain sites")?;
    let mut hits = vec![];
    if c.selected_sites
        .iter()
        .any(|name| !sites.iter().any(|site| site["name"].as_str() == Some(name)))
    {
        return Err("A selected site is absent from this dataset version".into());
    }
    for site in sites
        .iter()
        .filter(|s| {
            s["name"]
                .as_str()
                .is_some_and(|n| c.selected_sites.iter().any(|s| s == n))
        })
        .take(c.result_limit.min(20))
    {
        if site["headers"]
            .as_object()
            .is_some_and(|headers| !headers.is_empty())
            || site["post_body"]
                .as_str()
                .is_some_and(|body| !body.is_empty())
        {
            return Err("Selected site requires headers or POST handling unsupported by this checker; select a compatible GET site".into());
        }
        let url = site["uri_check"]
            .as_str()
            .ok_or("Dataset uri_check missing")?
            .replace("{account}", &urlencoding::encode(handle));
        let raw =
            crate::search::request(reqwest::Method::GET, &url, None, "text/html", &[]).await?;
        let body = String::from_utf8_lossy(&raw.bytes);
        let code = site["e_code"].as_u64().ok_or("Dataset e_code missing")?;
        let exists = site["e_string"]
            .as_str()
            .ok_or("Dataset e_string missing")?;
        let missing = site["m_string"].as_str().unwrap_or("");
        if raw.status as u64 == code
            && body.contains(exists)
            && (missing.is_empty() || !body.contains(missing))
        {
            let platform = site["name"].as_str().unwrap_or("unknown");
            hits.push(SearchHit{title:format!("WhatsMyName provisional {platform}:{handle}"),url,snippet:format!("Profile response matched dataset {} on {platform}. Candidate username association only; corroboration required.",c.dataset_version)});
        }
    }
    Ok(hits)
}
async fn load_dataset(path: String) -> std::result::Result<serde_json::Value, String> {
    crate::workers::spawn_blocking(move || -> Result<serde_json::Value> {
        use std::io::Read;
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(8_000_001)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 8_000_000 {
            bail!("Dataset too large; maximum 8 MB");
        }
        Ok(serde_json::from_slice(&bytes)?)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}
/// Version/help checks do not perform collection or installation.
pub async fn test_configuration(
    name: &str,
    c: &ResearchConfig,
    has_secret: bool,
) -> (Readiness, String) {
    if name == "shodan" && !has_secret {
        return (
            Readiness::MissingCredentials,
            "Store a Shodan key; host lookup entitlement and quota remain unverified".into(),
        );
    }
    if name == "whatsmyname" {
        return match load_dataset(c.dataset_path.clone()).await {
            Ok(dataset) if dataset["sites"].as_array().is_some() => {
                if c.selected_sites.is_empty() || c.dataset_version.is_empty() {
                    (Readiness::Configured,"Dataset parsed. Set its version and select sites before checking provisional profiles".into())
                } else {
                    (Readiness::Ready,"Dataset parsed; selected-site HTTP checking configured. Profile matches remain provisional; no live check performed".into())
                }
            }
            Ok(_) => (
                Readiness::Failed,
                "Invalid dataset contract: sites array missing".into(),
            ),
            Err(error) => (
                Readiness::NotInstalled,
                format!("Configure a supported dataset path: {error}"),
            ),
        };
    }
    if c.mode == ExecutionMode::NativeHttp {
        return (Readiness::Configured,"Configuration checked offline; execution has not been live-verified; no quota consumed".into());
    }
    if c.mode == ExecutionMode::Container {
        return (Readiness::Degraded,"Container network isolation and pinned output contract are not yet verified; runtime is never silently installed".into());
    }
    if c.executable.is_empty() {
        return (
            Readiness::NotInstalled,
            "Set the executable path or choose a supported managed installation".into(),
        );
    }
    let args = if name == "katana" {
        vec!["-version".into()]
    } else {
        vec!["--version".into()]
    };
    match run_process(
        Path::new(&c.executable),
        &args,
        Duration::from_secs(10),
        16384,
        &[],
    )
    .await
    {
        Ok(output) => {
            let text = String::from_utf8_lossy(&output.stdout).to_string()
                + &String::from_utf8_lossy(&output.stderr);
            if !text.contains(&c.supported_version) {
                (Readiness::UnsupportedVersion,"Detected version does not match supported version; review the official contract".into())
            } else {
                (Readiness::Degraded,format!("Version matched {}. Collection output and network isolation require verification",c.supported_version))
            }
        }
        Err(e) => (
            Readiness::Failed,
            format!("Executable verification failed: {e}"),
        ),
    }
}
#[derive(Debug)]
pub struct ProcessOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}
#[cfg(unix)]
struct ProcessGroup(i32);
#[cfg(unix)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        unsafe {
            libc::kill(-self.0, libc::SIGKILL);
        }
    }
}
/// Argument arrays, bounded concurrent pipe draining, kill-on-drop and process-group cleanup.
pub async fn run_process(
    executable: &Path,
    args: &[String],
    timeout: Duration,
    limit: usize,
    env: &[(String, String)],
) -> Result<ProcessOutput> {
    use tokio::io::AsyncReadExt;
    let mut command = tokio::process::Command::new(executable);
    command
        .args(args)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("LANG", "C.UTF-8")
        .envs(env.iter().cloned())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
    }
    #[cfg(not(unix))]
    {
        bail!("Process-tree cleanup is not supported on this platform");
    }
    let mut child = command.spawn()?;
    #[cfg(unix)]
    let _group = ProcessGroup(
        child
            .id()
            .ok_or_else(|| anyhow::anyhow!("Child PID unavailable"))? as i32,
    );
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("stdout unavailable"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("stderr unavailable"))?;
    let read = |stream: Box<dyn tokio::io::AsyncRead + Unpin + Send>| async move {
        let mut bytes = Vec::new();
        stream
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .await?;
        if bytes.len() > limit {
            bail!("Tool output exceeded limit");
        }
        Ok::<_, anyhow::Error>(bytes)
    };
    let (stdout, stderr, status) = tokio::time::timeout(timeout, async {
        tokio::try_join!(read(Box::new(stdout)), read(Box::new(stderr)), async {
            Ok::<_, anyhow::Error>(child.wait().await?)
        })
    })
    .await
    .map_err(|_| anyhow::anyhow!("Process timed out"))??;
    if !status.success() {
        bail!("Tool exited unsuccessfully ({status}); diagnostics withheld to prevent credential leakage");
    }
    Ok(ProcessOutput { stdout, stderr })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn output_contracts_and_private_scope() {
        assert!(
            parse_shodan("8.8.8.8", &serde_json::json!({"ports":[443],"org":"Acme"})).unwrap()[0]
                .snippet
                .contains("does not verify")
        );
        assert!(parse_shodan(
            "8.8.8.8",
            &serde_json::json!({"error":"requires membership"})
        )
        .is_err());
        assert_eq!(
            parse_xon(
                "a@example.com",
                &serde_json::json!({"breaches":[["Adobe"]]})
            )
            .unwrap()
            .len(),
            1
        );
        assert!(valid_input("internetdb", "127.0.0.1").is_err());
        assert!(valid_input("maigret", "--help").is_err());
        let mut fields =
            serde_json::json!({"nested":[{"url":"https://example.com/key/secret-value"}]});
        redact_value(&mut fields, Some("secret-value"));
        assert!(!fields.to_string().contains("secret-value"));
    }
    #[tokio::test]
    async fn process_drains_both_pipes_and_bounds_output() {
        let out = run_process(
            Path::new("/bin/sh"),
            &[
                "-c".into(),
                "printf 'result'; printf 'diagnostic' >&2".into(),
            ],
            Duration::from_secs(2),
            100,
            &[],
        )
        .await
        .unwrap();
        assert_eq!(out.stdout, b"result");
        assert_eq!(out.stderr, b"diagnostic");
        assert!(run_process(
            Path::new("/bin/sh"),
            &["-c".into(), "while :; do printf x; done".into()],
            Duration::from_secs(2),
            100,
            &[]
        )
        .await
        .is_err());
    }
    #[tokio::test]
    async fn timeout_and_recovery() {
        assert!(run_process(
            Path::new("/bin/sleep"),
            &["5".into()],
            Duration::from_millis(20),
            100,
            &[]
        )
        .await
        .is_err());
        let dir = tempfile::tempdir().unwrap();
        let q = ResearchQueue::new(dir.path().join("state.db"), 2);
        let input = ResearchInput {
            case_id: Some("c".into()),
            report_id: Some("r".into()),
            entity_id: "ip:127.0.0.1".into(),
            label: "127.0.0.1".into(),
            action: "internetdb".into(),
            depth: 0,
        };
        let job = q
            .execute(
                input,
                "internetdb",
                ResearchConfig {
                    enabled: true,
                    ..Default::default()
                },
                SourcePlan::default(),
                None,
            )
            .await;
        assert_eq!(job.state, JobState::Failed);
        let s = Store::open(&q.db).unwrap();
        let mut interrupted = job;
        interrupted.state = JobState::Running;
        s.save_job("key", &interrupted).unwrap();
        s.recover_jobs().unwrap();
        assert_eq!(s.jobs().unwrap()[0].state, JobState::Partial);
    }
    #[tokio::test]
    async fn provider_rate_gate_returns_retry_without_network() {
        let dir = tempfile::tempdir().unwrap();
        let queue = ResearchQueue::new(dir.path().join("state.db"), 2);
        queue.rates.lock().await.insert(
            "xposedornot".into(),
            Arc::new(Mutex::new(std::time::Instant::now())),
        );
        let input = ResearchInput {
            case_id: None,
            report_id: None,
            entity_id: "email:alice@example.com".into(),
            label: "alice@example.com".into(),
            action: "xposedornot".into(),
            depth: 0,
        };
        let config = ResearchConfig {
            enabled: true,
            allow_sensitive: true,
            rate_interval_ms: 900000,
            timeout_secs: 1,
            ..Default::default()
        };
        let job = queue
            .execute(input, "xposedornot", config, SourcePlan::default(), None)
            .await;
        assert_eq!(job.state, JobState::RateLimited);
        assert!(job.error.unwrap().contains("retry after"));
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn cancellation_cleans_process_tree() {
        let directory = tempfile::tempdir().unwrap();
        let pid_file = directory.path().join("pids");
        let output_path = pid_file.clone();
        let task = tokio::spawn(async move {
            run_process(
                Path::new("/bin/sh"),
                &[
                    "-c".into(),
                    "echo $$ > \"$1\"; sleep 30 & echo $! >> \"$1\"; wait".into(),
                    "fixture".into(),
                    output_path.to_string_lossy().into(),
                ],
                Duration::from_secs(35),
                100,
                &[],
            )
            .await
        });
        let pids = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(text) = tokio::fs::read_to_string(&pid_file).await {
                    let pids = text
                        .lines()
                        .filter_map(|s| s.parse::<i32>().ok())
                        .collect::<Vec<_>>();
                    if pids.len() == 2 {
                        break pids;
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        task.abort();
        let _ = task.await;
        tokio::time::timeout(Duration::from_secs(2), async {
            while pids.iter().any(|pid| unsafe { libc::kill(*pid, 0) } == 0) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("cancelled process tree still exists");
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StageOutput {
    pub stage: Stage,
    pub jobs: Vec<ResearchJob>,
    pub skipped: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkflowOutput {
    pub run: ResearchRun,
    pub stages: Vec<StageOutput>,
    pub hits: Vec<SearchHit>,
}
impl ResearchQueue {
    /// Bounded dependency-aware execution. Only exact inputs or analyst-selected pivots are
    /// automatically eligible; candidate domains from search are offered for review.
    pub async fn workflow(
        &self,
        input: ResearchInput,
        configs: std::collections::BTreeMap<String, ResearchConfig>,
        plan: SourcePlan,
        secrets: std::collections::BTreeMap<String, String>,
    ) -> WorkflowOutput {
        let mut run = ResearchRun {
            id: new_id("run"),
            input: input.clone(),
            jobs: vec![],
            max_depth: 1,
            max_entities: 4,
            timeout_secs: 120,
            request_budget: 8,
        };
        let parsed = crate::search::TextQuery::extract(&input.label);
        let exact_ip = input
            .label
            .parse::<std::net::IpAddr>()
            .ok()
            .filter(|ip| !crate::search::ip_blocked(*ip));
        let exact_domain = crate::search::query::normalize_domain(&input.label).as_deref()
            == Some(input.label.as_str());
        let exact_email = parsed.emails.first().is_some_and(|e| e == &input.label);
        let exact_handle = input.label.starts_with('@') && parsed.handles.len() == 1;
        let eligible: Vec<(&str, String)> = if exact_ip.is_some() {
            vec![("internetdb", input.label.clone())]
        } else if exact_domain {
            vec![("domain", input.label.clone())]
        } else if exact_email {
            vec![
                ("identity", input.label.clone()),
                ("leakcheck", input.label.clone()),
                ("xposedornot", input.label.clone()),
            ]
        } else if exact_handle {
            vec![
                ("identity", parsed.handles[0].clone()),
                ("whatsmyname", parsed.handles[0].clone()),
            ]
        } else {
            vec![("search", input.label.clone())]
        };
        let mut planned = Vec::new();
        let mut skipped = Vec::new();
        for (provider, label) in eligible.into_iter().take(run.request_budget) {
            match configs.get(provider).cloned().filter(|c| {
                c.enabled && (!matches!(provider, "leakcheck" | "xposedornot") || c.allow_sensitive)
            }) {
                Some(config) => {
                    let mut job_input = input.clone();
                    job_input.label = label;
                    job_input.action = provider.into();
                    let secret = secrets.get(&config.secret_ref).cloned();
                    let q = self.clone();
                    let p = plan.clone();
                    planned.push(
                        async move { q.execute(job_input, provider, config, p, secret).await },
                    );
                }
                None => skipped.push(format!("{provider}: disabled or privacy opt-in missing")),
            }
        }
        let jobs = futures_util::future::join_all(planned).await;
        let hits = jobs.iter().flat_map(|j| j.hits.clone()).collect::<Vec<_>>();
        let mut stages = vec![];
        for stage in [
            Stage::TopicOrganization,
            Stage::InfrastructureIp,
            Stage::DomainEmail,
            Stage::IdentityAliases,
            Stage::BreachExposure,
        ] {
            let jobs_for_stage = jobs
                .iter()
                .filter(|j| j.stage == stage)
                .cloned()
                .collect::<Vec<_>>();
            run.jobs.extend(jobs_for_stage.iter().map(|j| j.id.clone()));
            stages.push(StageOutput {
                stage,
                jobs: jobs_for_stage,
                skipped: if skipped.is_empty() {
                    vec![
                        "No suitable validated input or explicitly selected pivot for this stage"
                            .into(),
                    ]
                } else {
                    skipped.clone()
                },
            });
        }
        let db = self.db.clone();
        let persisted_run = run.clone();
        let _ = crate::workers::spawn_blocking(move || {
            Store::open(&db)?.put_record(
                &persisted_run.id,
                "research_run",
                persisted_run.input.case_id.as_deref(),
                persisted_run.input.report_id.as_deref(),
                &persisted_run,
            )
        })
        .await;
        WorkflowOutput { run, stages, hits }
    }
}

pub fn parse_xon_analytics(
    email: &str,
    value: &serde_json::Value,
) -> std::result::Result<(Vec<SearchHit>, Vec<ResultMetadata>), String> {
    if value.get("ExposedBreaches") == Some(&serde_json::Value::Null)
        && value.get("BreachesSummary").is_some()
    {
        return Ok((vec![], vec![]));
    }
    let rows = value
        .pointer("/ExposedBreaches/breaches_details")
        .and_then(|v| v.as_array())
        .ok_or("Unsupported XposedOrNot analytics contract")?;
    let mut hits = vec![];
    let mut metadata = vec![];
    for row in rows {
        let name = row["breach"].as_str().ok_or("Breach name missing")?;
        let date = row["xposed_date"].as_str().map(str::to_string);
        let categories = row["xposed_data"].as_str().unwrap_or("unavailable");
        let references = row["references"].as_str().unwrap_or("");
        let source = if references.starts_with("https://") {
            references.to_string()
        } else {
            "https://xposedornot.com".into()
        };
        hits.push(SearchHit{title:format!("XposedOrNot · {name}"),url:source.clone(),snippet:format!("Identifier {email} reported in breach {name}. Event date: {} (provider precision). Exposed-data categories: {categories}. Credentials were not retrieved.",date.as_deref().unwrap_or("undated"))});
        metadata.push(ResultMetadata{fields:serde_json::json!({"breach":name,"email":email,"domain":row["domain"],"event_date":date,"exposed_categories":categories,"reference":source}),event_time:date,event_uncertainty:Some("Date precision is exactly as supplied by the provider; no discovery date substituted".into())});
    }
    Ok((hits, metadata))
}
impl ResearchQueue {
    pub async fn manage_tool(
        &self,
        name: &str,
        action: &str,
        plan: Option<crate::tool_manager::InstallPlan>,
        executable: Option<PathBuf>,
    ) -> (ResearchJob, std::result::Result<Option<String>, String>) {
        let mut job = ResearchJob {
            id: new_id("setup"),
            run_id: new_id("run"),
            provider: name.into(),
            provider_version: plan.as_ref().map(|p| p.version.clone()),
            metadata: vec![],
            stage: stage_for(name),
            state: JobState::Queued,
            progress: format!("{action} queued"),
            created_at: chrono::Utc::now().to_rfc3339(),
            finished_at: None,
            elapsed_ms: 0,
            error: None,
            hits: vec![],
            input: ResearchInput {
                case_id: None,
                report_id: None,
                entity_id: name.into(),
                label: name.into(),
                action: action.into(),
                depth: 0,
            },
        };
        let key = job.id.clone();
        let start = std::time::Instant::now();
        let _ = self.persist(&key, &job).await;
        let work = async {
            let _permit = self
                .global
                .acquire()
                .await
                .map_err(|_| "Research queue closed")?;
            job.state = JobState::Running;
            job.progress =
                format!("{action}: downloading / checking / verifying managed installation");
            self.persist(&key, &job).await.map_err(|e| e.to_string())?;
            if action == "remove" {
                let executable = executable.ok_or("Managed executable path required")?;
                crate::tool_manager::remove(&crate::paths::home_dir().join("tools"), &executable)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(None)
            } else {
                let plan = plan.ok_or("Review a pinned install plan first")?;
                let binary = crate::tool_manager::install(&plan)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(Some(binary.to_string_lossy().into()))
            }
        };
        let result = tokio::select! {result=tokio::time::timeout(Duration::from_secs(600),work)=>result.unwrap_or_else(|_|Err("Installer timed out; previous installation retained".into())),_=wait_cancel(self.cancel.clone())=>Err("cancelled".into())};
        job.state = match &result {
            Ok(_) => JobState::Completed,
            Err(e) if e == "cancelled" => JobState::Cancelled,
            _ => JobState::Failed,
        };
        job.error = result.as_ref().err().cloned();
        job.elapsed_ms = start.elapsed().as_millis() as u64;
        job.finished_at = Some(chrono::Utc::now().to_rfc3339());
        job.progress = format!("{action}: {:?}", job.state);
        let _ = self.persist(&key, &job).await;
        (job, result)
    }
}

/// Configuration tabs are capability groups, never an automatic execution sequence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResearchPhase {
    #[default]
    Discovery,
    Infrastructure,
    Identity,
    Exposure,
    Analysis,
}
impl ResearchPhase {
    pub const ALL: [Self; 5] = [
        Self::Discovery,
        Self::Infrastructure,
        Self::Identity,
        Self::Exposure,
        Self::Analysis,
    ];
    pub fn title(self) -> &'static str {
        match self {
            Self::Discovery => "Discovery",
            Self::Infrastructure => "Infrastructure",
            Self::Identity => "Identity & contacts",
            Self::Exposure => "Exposure",
            Self::Analysis => "Analysis & output",
        }
    }
    pub fn providers(self) -> &'static [&'static str] {
        match self {
            Self::Discovery => &["search"],
            Self::Infrastructure => &["domain", "internetdb", "shodan", "katana", "spiderfoot"],
            Self::Identity => &["identity", "contacts", "whatsmyname", "maigret", "mosint"],
            Self::Exposure => &["leakcheck", "xposedornot"],
            Self::Analysis => &[],
        }
    }
}
pub fn collection_available(name: &str) -> bool {
    matches!(
        name,
        "search"
            | "domain"
            | "internetdb"
            | "shodan"
            | "identity"
            | "contacts"
            | "whatsmyname"
            | "leakcheck"
            | "xposedornot"
    )
}
/// Same input/privacy checks used to present and submit a focused action.
/// Input compatibility only: availability and saved scope are checked before submission.
pub fn input_matches_action(name: &str, label: &str) -> Result<()> {
    valid_input(name, label)?;
    if matches!(name, "identity" | "leakcheck") {
        let email = crate::search::TextQuery::extract(label).emails == vec![label.to_string()];
        let handle = label
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-');
        if !email && !handle {
            bail!("Select an exact email or handle");
        }
    }
    Ok(())
}
pub fn eligible_action(name: &str, label: &str, config: &ResearchConfig) -> Result<()> {
    if !config.enabled {
        bail!("Integration disabled");
    }
    if !collection_available(name) || config.mode != ExecutionMode::NativeHttp {
        bail!("Collection unavailable: output and scope contracts unverified");
    }
    if !matches!(
        config.readiness,
        Readiness::Ready | Readiness::Configured | Readiness::Degraded
    ) {
        bail!("Configuration is not ready: {:?}", config.readiness);
    }
    valid_input(name, label)?;
    if matches!(name, "identity" | "leakcheck") {
        let email = crate::search::TextQuery::extract(label).emails == vec![label.to_string()];
        let handle = label
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-');
        if !email && !handle {
            bail!("Select an exact email or handle");
        }
    }

    if matches!(name, "leakcheck" | "xposedornot") && !config.allow_sensitive {
        bail!("Exposure lookup requires sensitive lookup opt-in");
    }
    if name == "contacts"
        && (!config.allow_active || !config.allowed_hosts.iter().any(|h| h == label))
    {
        bail!("Exact allowed host and active HTTP opt-in required");
    }
    if name == "whatsmyname" && (config.dataset_path.is_empty() || config.selected_sites.is_empty())
    {
        bail!("Versioned dataset and selected sites required");
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AnalysisConfig {
    pub report_mode: String,
    pub lead_limit: usize,
}
impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            report_mode: "final".into(),
            lead_limit: 5,
        }
    }
}
pub fn display_name(name: &str) -> &str {
    match name {
        "identity" => "GitHub identity",
        "contacts" => "Published contacts",
        "leakcheck" => "LeakCheck Public",
        "xposedornot" => "XposedOrNot",
        "whatsmyname" => "WhatsMyName",
        "internetdb" => "InternetDB",
        "katana" => "Katana",
        "shodan" => "Shodan",
        "maigret" => "Maigret",
        "mosint" => "Mosint",
        "spiderfoot" => "SpiderFoot",
        "domain" => "DNS / RDAP / certificates / Wayback",
        "search" => "Search & public documents",
        _ => name,
    }
}

#[cfg(test)]
mod case_job_tests {
    use super::*;
    #[tokio::test]
    async fn queued_cancellation_is_persisted_and_partial_results_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("jobs.sqlite");
        let store = Store::open(&db).unwrap();
        let case = store.create_case("DNS").unwrap();
        let queue = ResearchQueue::new(db.clone(), 1);
        let _permit = queue.global.acquire().await.unwrap();
        queue.cancel.store(true, Ordering::Relaxed);
        let input = ResearchInput {
            case_id: Some(case.id.clone()),
            report_id: None,
            entity_id: "domain:harbor.example".into(),
            label: "harbor.example".into(),
            action: "domain".into(),
            depth: 0,
        };
        let cancelled = queue
            .execute(
                input.clone(),
                "domain",
                defaults()["domain"].clone(),
                SourcePlan::default(),
                None,
            )
            .await;
        assert_eq!(cancelled.state, JobState::Cancelled);
        assert!(store
            .jobs()
            .unwrap()
            .iter()
            .any(|j| j.id == cancelled.id && j.state == JobState::Cancelled));
        let partial = ResearchJob {
            provider_version: None,
            metadata: vec![],
            id: "partial-job".into(),
            run_id: "run".into(),
            input,
            provider: "domain".into(),
            stage: Stage::InfrastructureIp,
            state: JobState::Partial,
            progress: "One successful source; another failed".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
            finished_at: Some(chrono::Utc::now().to_rfc3339()),
            elapsed_ms: 1,
            error: Some("RDAP failed".into()),
            hits: vec![SearchHit {
                title: "[doh] DNS".into(),
                url: "https://cloudflare-dns.com/dns-query?name=harbor.example".into(),
                snippet: "A 8.8.8.8".into(),
            }],
        };
        queue.persist("partial", &partial).await.unwrap();
        drop(store);
        let store = Store::open(&db).unwrap();
        store.recover_jobs().unwrap();
        let p = store.case_projection(&case.id).unwrap();
        assert_eq!(p.findings.len(), 1);
        assert_eq!(p.links.len(), 1);
        assert!(p.links[0].candidate);
        assert!(p.entities.iter().any(|e| e.id == "ip:8.8.8.8"));
        let o = &p.findings[0].observation;
        assert!(store
            .resolve_evidence(
                &o.evidence[0],
                &crate::evidence::EvidenceScope::Case(case.id.clone())
            )
            .unwrap()
            .contains("A 8.8.8.8"));
        store
            .review_finding(
                &o.id,
                crate::evidence::ReviewDecision::Accept,
                "DNS source checked",
            )
            .unwrap();
        assert!(!store.case_projection(&case.id).unwrap().links[0].candidate);
        assert_eq!(
            store
                .jobs()
                .unwrap()
                .iter()
                .find(|j| j.id == partial.id)
                .unwrap()
                .state,
            JobState::Partial
        );
        queue.persist("partial", &partial).await.unwrap();
        assert_eq!(store.case_projection(&case.id).unwrap().findings.len(), 1);
    }
    #[test]
    fn eligibility_refuses_unavailable_collection_and_privacy_or_input_violations() {
        let mut katana = defaults()["katana"].clone();
        katana.enabled = true;
        katana.readiness = Readiness::Ready;
        assert!(eligible_action("katana", "harbor.example", &katana).is_err());
        let config = defaults()["domain"].clone();
        assert!(eligible_action("domain", "harbor.example", &config).is_ok());
        assert!(eligible_action("domain", "query with domain", &config).is_err());
        assert!(eligible_action(
            "leakcheck",
            "contact@harbor.example",
            &defaults()["leakcheck"]
        )
        .is_err());
        assert!(eligible_action("internetdb", "127.0.0.1", &defaults()["internetdb"]).is_err());
    }
}

#[cfg(test)]
mod investigation_scope_tests {
    use super::*;
    #[tokio::test]
    async fn explicit_job_cannot_bypass_saved_case_source_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("scope.sqlite");
        let store = Store::open(&db).unwrap();
        let case = store.create_case("Passive only").unwrap();
        let scope = crate::investigation::InvestigationScope {
            question: "DNS only".into(),
            allowed_actions: vec!["domain".into()],
            ..Default::default()
        };
        store
            .put_record("scope", "investigation_scope", Some(&case.id), None, &scope)
            .unwrap();
        let queue = ResearchQueue::new(db, 1);
        let mut config = defaults()["leakcheck"].clone();
        config.allow_sensitive = true;
        let job = queue
            .execute(
                ResearchInput {
                    case_id: Some(case.id),
                    report_id: None,
                    entity_id: "email:contact@harbor.example".into(),
                    label: "contact@harbor.example".into(),
                    action: "leakcheck".into(),
                    depth: 0,
                },
                "leakcheck",
                config,
                SourcePlan::default(),
                None,
            )
            .await;
        assert_eq!(job.state, JobState::Failed);
        assert!(job
            .error
            .unwrap()
            .contains("outside saved investigation scope"));
        assert!(job.hits.is_empty());
        assert_eq!(store.jobs().unwrap()[0].state, JobState::Failed);
    }
}

#[cfg(test)]
mod cache_restart_tests {
    use super::*;
    #[tokio::test]
    async fn explicit_cached_request_after_restart_finishes_its_queued_record_without_network() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("cache.sqlite");
        let store = Store::open(&db).unwrap();
        let case = store.create_case("Cache").unwrap();
        let queue = ResearchQueue::new(db.clone(), 1);
        queue.cancel.store(true, Ordering::Relaxed);
        let input = ResearchInput {
            case_id: Some(case.id),
            report_id: None,
            entity_id: "domain:harbor.example".into(),
            label: "harbor.example".into(),
            action: "domain".into(),
            depth: 0,
        };
        let config = defaults()["domain"].clone();
        let mut fixture = queue
            .execute(
                input.clone(),
                "domain",
                config.clone(),
                SourcePlan::default(),
                None,
            )
            .await;
        assert_eq!(fixture.state, JobState::Cancelled);
        let key: String = store
            .conn
            .query_row(
                "SELECT dedup_key FROM research_jobs WHERE id=?1",
                [&fixture.id],
                |r| r.get(0),
            )
            .unwrap();
        fixture.state = JobState::Completed;
        fixture.finished_at = Some(chrono::Utc::now().to_rfc3339());
        fixture.hits = vec![SearchHit {
            title: "[doh] DNS".into(),
            url: "https://cloudflare-dns.com/dns-query?name=harbor.example".into(),
            snippet: "A 8.8.8.8".into(),
        }];
        queue.persist(&key, &fixture).await.unwrap();
        let restarted = ResearchQueue::new(db, 1);
        let _permit = restarted.global.acquire().await.unwrap();
        assert_eq!(store.jobs().unwrap().len(), 1);
        let cached = tokio::time::timeout(
            Duration::from_secs(2),
            restarted.execute(input, "domain", config, SourcePlan::default(), None),
        )
        .await
        .unwrap();
        assert_eq!(cached.state, JobState::Completed);
        assert!(cached.progress.contains("cached evidence reused"));
        assert_ne!(cached.id, fixture.id);
        let jobs = store.jobs().unwrap();
        assert_eq!(jobs.len(), 2);
        assert!(jobs.iter().all(|j| j.state == JobState::Completed));
        assert_eq!(cached.hits.len(), 1);
        let projection = store
            .case_projection(cached.input.case_id.as_deref().unwrap())
            .unwrap();
        assert_eq!(projection.links.len(), 1);
        assert_eq!(projection.links[0].observations.len(), 2);
        assert_eq!(projection.links[0].relationship.evidence.len(), 2);
        assert_eq!(
            crate::investigation::distinct_findings(projection.findings.iter()),
            1
        );
    }
}
