//! App state and keyboard routing for the Argos terminal shell.

use anyhow::Result;
use argos_osint_core::brain::{Memory, MemorySource, ScoredMemory};
use argos_osint_core::hardware::{self, HardwareProfile};
use argos_osint_core::paths;
use argos_osint_core::provider::{self, SettingsFile};
use argos_osint_core::secrets::{AuthFile, ProviderSecret};
use argos_osint_core::store::Store;
use argos_osint_core::{osint, recon};
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};
use ratatui::layout::Rect;
use ratatui::Terminal;
use serde_json::Value;
use std::collections::HashMap;
use std::io::Stdout;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModuleId {
    Recon,
    Brain,
    Osint,
    Providers,
    System,
}
impl ModuleId {
    pub const ALL: [Self; 5] = [
        Self::Recon,
        Self::Brain,
        Self::Osint,
        Self::Providers,
        Self::System,
    ];
    pub fn title(self) -> &'static str {
        match self {
            Self::Recon => "Recon",
            Self::Brain => "Brain",
            Self::Osint => "OSINT",
            Self::Providers => "Providers",
            Self::System => "System",
        }
    }
    pub fn blurb(self) -> &'static str {
        match self {
            Self::Recon => "Investigate with evidence",
            Self::Brain => "Recall insights from conversations",
            Self::Osint => "Public lookup tools",
            Self::Providers => "Accounts and Defaults",
            Self::System => "Hardware and settings",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderPage {
    Grok,
    OpenAI,
    OpenRouter,
    Defaults,
}
impl ProviderPage {
    pub const ALL: [Self; 4] = [Self::Grok, Self::OpenAI, Self::OpenRouter, Self::Defaults];
    pub fn title(self) -> &'static str {
        match self {
            Self::Grok => "Grok",
            Self::OpenAI => "OpenAI",
            Self::OpenRouter => "OpenRouter",
            Self::Defaults => "Defaults",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldId {
    BrainApp,
    BrainConversation,
    BrainInsight,
    BrainQuery,
    ReconSearch,
    OsintSearch,
    OsintInput,
    ReconProvider,
    ReconModel,
    SynthesisProvider,
    SynthesisModel,
    RouterKey,
    RouterEndpoint,
    Composer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonId {
    Send,
    Add,
    Recall,
    Pin,
    Delete,
    SaveRecon,
    SaveSynthesis,
    ToggleDefaultRole,
    RefreshModels,
    NewThread,
    DeleteThread,
    CancelRun,
    ResumeRun,
    RetryInsights,
    OsintRun,
    OsintAttach,
    OsintStartRecon,
    OsintCancel,
    OsintToggle,
    OsintRaw,
    GrokSignIn,
    GrokCheck,
    OpenAISignIn,
    OpenAICheck,
    RouterSave,
    RouterVerify,
    RouterAdvanced,
    RefreshHardware,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    App(usize),
    ProviderTab(ProviderPage),
    Memory(usize),
    Thread(usize),
    Tool(usize),
    Field(FieldId),
    Button(ButtonId),
}

#[derive(Debug)]
enum ProviderEvent {
    Progress {
        page: ProviderPage,
        line: String,
    },
    Finished {
        page: ProviderPage,
        result: Result<String, String>,
    },
}

#[derive(Debug)]
enum WorkEvent {
    ReconStage {
        thread_id: String,
        stage: String,
    },
    ReconDone {
        thread_id: String,
        outcome: Result<(), String>,
    },
    OsintDone {
        outcome: Result<(String, osint::ToolResult), String>,
    },
    CatalogDone {
        synthesis: bool,
        outcome: Result<Vec<String>, String>,
    },
    InsightDone {
        thread_id: String,
        outcome: Result<(), String>,
    },
}

pub struct App {
    pub module: Option<ModuleId>,
    pub launcher_sel: usize,
    pub provider_page: ProviderPage,
    pub input: String,
    pub brain_app: String,
    pub brain_conversation: String,
    pub brain_insight: String,
    pub brain_query: String,
    pub recon_search: String,
    pub osint_search: String,
    pub osint_input: String,
    osint_inputs: HashMap<String, String>,
    pub selected_thread: Option<String>,
    pub threads: Vec<recon::Thread>,
    pub thread_states: HashMap<String, String>,
    pub messages: Vec<recon::Message>,
    pub calls: Vec<recon::Call>,
    pub thread_sel: usize,
    pub tool_sel: usize,
    pub tool_enabled: Vec<bool>,
    pub osint_result: Option<(String, osint::ToolResult)>,
    pub osint_raw: bool,
    pub manual_runs: Vec<recon::Call>,
    pub manual_run_pos: usize,
    pub recon_stage: String,
    pub recon_scroll: u16,
    pub thread_history: Vec<String>,
    pub history_pos: usize,
    running: HashMap<String, Arc<AtomicBool>>,
    osint_cancel: Option<Arc<AtomicBool>>,
    pub recon_provider: String,
    pub recon_model: String,
    pub synthesis_provider: String,
    pub synthesis_model: String,
    pub defaults_synthesis: bool,
    pub model_catalog: Vec<String>,
    pub router_key: String,
    pub router_endpoint: String,
    pub router_advanced: bool,
    pub grok_status: String,
    pub openai_status: String,
    pub router_status: String,
    pub provider_progress: Vec<String>,
    pub provider_progress_page: Option<ProviderPage>,
    pub provider_pending: Option<ProviderPage>,
    pub focus: Target,
    pub cursor: usize,
    pub screen: Rect,
    pub status: String,
    pub memories: Vec<Memory>,
    pub selected_insight: Option<recon::InsightView>,
    pub memory_sel: usize,
    pub hits: Vec<ScoredMemory>,
    pub auth: AuthFile,
    pub settings: SettingsFile,
    pub hardware: HardwareProfile,
    auth_path: PathBuf,
    store: Store,
    provider_tx: UnboundedSender<ProviderEvent>,
    provider_rx: UnboundedReceiver<ProviderEvent>,
    work_tx: UnboundedSender<WorkEvent>,
    work_rx: UnboundedReceiver<WorkEvent>,
}

impl App {
    pub fn boot() -> Result<Self> {
        paths::ensure_home()?;
        let store = Store::open(&paths::db_path())?;
        store.recover_runs()?;
        let threads = store.list_threads("")?;
        let thread_states = threads
            .iter()
            .map(|t| {
                Ok((
                    t.id.clone(),
                    store.latest_run_state(&t.id)?.unwrap_or_default(),
                ))
            })
            .collect::<Result<HashMap<_, _>>>()?;
        let selected_thread = store
            .last_thread()?
            .or_else(|| threads.first().map(|t| t.id.clone()));
        let messages = selected_thread
            .as_ref()
            .map(|id| store.list_messages(id))
            .transpose()?
            .unwrap_or_default();
        let draft = selected_thread
            .as_ref()
            .and_then(|id| store.get_thread(id).ok().flatten())
            .map(|t| t.draft)
            .unwrap_or_default();
        let recon_scroll = selected_thread
            .as_ref()
            .and_then(|id| store.get_thread(id).ok().flatten())
            .map(|t| t.scroll.clamp(0, i64::from(u16::MAX)) as u16)
            .unwrap_or_default();
        let calls = selected_thread
            .as_ref()
            .map(|id| store.all_calls_for_thread(id))
            .transpose()?
            .unwrap_or_default();
        let manual_runs = store.manual_calls()?;
        let manual_run_pos = manual_runs.len().saturating_sub(1);
        let osint_result = manual_runs
            .get(manual_run_pos)
            .and_then(|call| call.result.clone().map(|result| (call.id.clone(), result)));
        let tool_enabled = osint::registry()
            .iter()
            .map(|tool| store.tool_enabled(tool.id))
            .collect::<Result<Vec<_>>>()?;
        let memories = store.list_memories()?;
        let auth = AuthFile::load()?;
        let settings = SettingsFile::load()?;
        let recon_default = provider::role_secret(&auth, &settings, "recon")?;
        let synthesis_default = provider::role_secret(&auth, &settings, "synthesis")?;
        let router = provider::account_secret(&auth, "openrouter");
        let (provider_tx, provider_rx) = unbounded_channel();
        let (work_tx, work_rx) = unbounded_channel();
        Ok(Self {
            module: Some(ModuleId::Recon),
            launcher_sel: 0,
            provider_page: ProviderPage::Grok,
            input: draft,
            brain_app: String::new(),
            brain_conversation: String::new(),
            brain_insight: String::new(),
            brain_query: String::new(),
            recon_search: String::new(),
            osint_search: String::new(),
            osint_input: osint::registry()
                .first()
                .map(|t| t.example_input().to_string())
                .unwrap_or_else(|| "{}".into()),
            osint_inputs: HashMap::new(),
            selected_thread,
            threads,
            thread_states,
            messages,
            calls,
            thread_sel: 0,
            tool_sel: 0,
            tool_enabled,
            osint_result,
            osint_raw: false,
            manual_runs,
            manual_run_pos,
            recon_stage: "ready".into(),
            recon_scroll,
            thread_history: Vec::new(),
            history_pos: 0,
            running: HashMap::new(),
            osint_cancel: None,
            recon_provider: provider::effective_kind(&recon_default),
            recon_model: recon_default.model,
            synthesis_provider: provider::effective_kind(&synthesis_default),
            synthesis_model: synthesis_default.model,
            defaults_synthesis: false,
            model_catalog: Vec::new(),
            router_key: router.api_key.unwrap_or_default(),
            router_endpoint: router.base_url,
            router_advanced: false,
            grok_status: "Not checked · Sign in or check existing login".into(),
            openai_status: "Not checked · Sign in or check existing login".into(),
            router_status: "Enter a key, then verify or save".into(),
            provider_progress: Vec::new(),
            provider_progress_page: None,
            provider_pending: None,
            focus: Target::App(0),
            cursor: 0,
            screen: Rect::default(),
            status: "ready".into(),
            memories,
            selected_insight: None,
            memory_sel: 0,
            hits: Vec::new(),
            auth,
            settings,
            hardware: hardware::profile_cached(false),
            auth_path: paths::auth_path(),
            store,
            provider_tx,
            provider_rx,
            work_tx,
            work_rx,
        })
    }

    fn select(&mut self, index: usize) {
        if self.module == Some(ModuleId::Recon) {
            if let Some(id) = &self.selected_thread {
                let _ = self.store.save_draft(id, &self.input, 0);
            }
        }
        self.launcher_sel = index;
        self.module = Some(ModuleId::ALL[index]);
        if self.module == Some(ModuleId::Recon) {
            self.input = self
                .selected_thread
                .as_ref()
                .and_then(|id| self.store.get_thread(id).ok().flatten())
                .map(|t| t.draft)
                .unwrap_or_default();
        } else {
            self.input.clear();
        }
        self.hits.clear();
        self.status = format!("{} open", ModuleId::ALL[index].title());
        self.set_focus(match self.module {
            Some(ModuleId::Recon) => Target::Field(FieldId::Composer),
            Some(ModuleId::Brain) => Target::Field(FieldId::BrainApp),
            Some(ModuleId::Osint) => Target::Field(FieldId::OsintSearch),
            Some(ModuleId::Providers) => Target::ProviderTab(self.provider_page),
            _ => Target::Button(ButtonId::RefreshHardware),
        });
    }

    pub fn field(&self, field: FieldId) -> &str {
        match field {
            FieldId::BrainApp => &self.brain_app,
            FieldId::BrainConversation => &self.brain_conversation,
            FieldId::BrainInsight => &self.brain_insight,
            FieldId::BrainQuery => &self.brain_query,
            FieldId::ReconSearch => &self.recon_search,
            FieldId::OsintSearch => &self.osint_search,
            FieldId::OsintInput => &self.osint_input,
            FieldId::ReconProvider => &self.recon_provider,
            FieldId::ReconModel => &self.recon_model,
            FieldId::SynthesisProvider => &self.synthesis_provider,
            FieldId::SynthesisModel => &self.synthesis_model,
            FieldId::RouterKey => &self.router_key,
            FieldId::RouterEndpoint => &self.router_endpoint,
            FieldId::Composer => &self.input,
        }
    }

    fn field_mut(&mut self, field: FieldId) -> &mut String {
        match field {
            FieldId::BrainApp => &mut self.brain_app,
            FieldId::BrainConversation => &mut self.brain_conversation,
            FieldId::BrainInsight => &mut self.brain_insight,
            FieldId::BrainQuery => &mut self.brain_query,
            FieldId::ReconSearch => &mut self.recon_search,
            FieldId::OsintSearch => &mut self.osint_search,
            FieldId::OsintInput => &mut self.osint_input,
            FieldId::ReconProvider => &mut self.recon_provider,
            FieldId::ReconModel => &mut self.recon_model,
            FieldId::SynthesisProvider => &mut self.synthesis_provider,
            FieldId::SynthesisModel => &mut self.synthesis_model,
            FieldId::RouterKey => &mut self.router_key,
            FieldId::RouterEndpoint => &mut self.router_endpoint,
            FieldId::Composer => &mut self.input,
        }
    }

    fn set_focus(&mut self, target: Target) {
        self.focus = target;
        self.cursor = match target {
            Target::Field(field) => self.field(field).chars().count(),
            _ => 0,
        };
    }

    fn refresh_threads(&mut self) -> Result<()> {
        self.threads = self.store.list_threads(&self.recon_search)?;
        self.thread_states = self
            .threads
            .iter()
            .map(|t| {
                Ok((
                    t.id.clone(),
                    self.store.latest_run_state(&t.id)?.unwrap_or_default(),
                ))
            })
            .collect::<Result<HashMap<_, _>>>()?;
        self.thread_sel = self
            .selected_thread
            .as_ref()
            .and_then(|id| self.threads.iter().position(|t| &t.id == id))
            .unwrap_or(0);
        Ok(())
    }

    fn filtered_tool_ids(&self) -> Vec<usize> {
        let q = self.osint_search.trim().to_ascii_lowercase();
        osint::registry()
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                q.is_empty()
                    || t.name.to_ascii_lowercase().contains(&q)
                    || t.category.to_ascii_lowercase().contains(&q)
                    || t.id.contains(&q)
            })
            .map(|(i, _)| i)
            .collect()
    }

    fn select_tool(&mut self, index: usize) {
        if let Some(old) = osint::registry().get(self.tool_sel) {
            self.osint_inputs
                .insert(old.id.into(), self.osint_input.clone());
        }
        if let Some(tool) = osint::registry().get(index) {
            self.tool_sel = index;
            self.osint_input = self
                .osint_inputs
                .get(tool.id)
                .cloned()
                .unwrap_or_else(|| tool.example_input().to_string());
            if let Some(pos) = self
                .manual_runs
                .iter()
                .rposition(|call| call.tool_id == tool.id && call.result.is_some())
            {
                self.manual_run_pos = pos;
                self.osint_result = self.manual_runs[pos]
                    .result
                    .clone()
                    .map(|r| (self.manual_runs[pos].id.clone(), r));
            } else {
                self.osint_result = None;
            }
            self.set_focus(Target::Tool(index));
        }
    }

    fn open_thread(&mut self, id: &str) -> Result<()> {
        self.open_thread_with_history(id, true)
    }

    fn open_thread_with_history(&mut self, id: &str, record: bool) -> Result<()> {
        if self.module == Some(ModuleId::Recon) {
            if let Some(previous) = &self.selected_thread {
                self.store
                    .save_draft(previous, &self.input, i64::from(self.recon_scroll))?;
            }
        }
        let thread = self
            .store
            .get_thread(id)?
            .ok_or_else(|| anyhow::anyhow!("Thread not found"))?;
        self.store.select_thread(id)?;
        self.selected_thread = Some(id.into());
        self.messages = self.store.list_messages(id)?;
        self.refresh_selected()?;
        self.input = thread.draft;
        self.recon_scroll = thread.scroll.clamp(0, i64::from(u16::MAX)) as u16;
        self.recon_stage = "ready".into();
        self.refresh_threads()?;
        if record && self.thread_history.last().map(String::as_str) != Some(id) {
            self.thread_history
                .truncate(self.history_pos.saturating_add(1));
            self.thread_history.push(id.into());
            self.history_pos = self.thread_history.len().saturating_sub(1);
        }
        Ok(())
    }

    fn new_thread(&mut self) -> Result<()> {
        let thread = self.store.new_thread("New investigation")?;
        self.open_thread(&thread.id)?;
        self.input.clear();
        self.set_focus(Target::Field(FieldId::Composer));
        Ok(())
    }

    fn send_recon(&mut self) -> Result<()> {
        let question = self.input.trim().to_string();
        if question.is_empty() {
            return Ok(());
        }
        if self.selected_thread.is_none() {
            self.new_thread()?;
        }
        let tid = self.selected_thread.clone().unwrap();
        anyhow::ensure!(
            !self.running.contains_key(&tid),
            "This thread already has a running turn"
        );
        self.input.clear();
        self.store
            .save_draft(&tid, "", i64::from(self.recon_scroll))?;
        let service =
            recon::Service::new(&paths::db_path(), self.auth.clone(), self.settings.clone())?;
        let tx = self.work_tx.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        self.running.insert(tid.clone(), cancel.clone());
        self.recon_stage = "starting".into();
        tokio::spawn(async move {
            let progress_tx = tx.clone();
            let thread_id = tid.clone();
            let outcome = service
                .ask(&tid, &question, cancel, move |stage| {
                    let _ = progress_tx.send(WorkEvent::ReconStage {
                        thread_id: thread_id.clone(),
                        stage: stage.into(),
                    });
                })
                .await
                .map(|_| ())
                .map_err(|e| e.to_string());
            let _ = tx.send(WorkEvent::ReconDone {
                thread_id: tid,
                outcome,
            });
        });
        Ok(())
    }

    fn resume_recon(&mut self) -> Result<()> {
        let tid = self
            .selected_thread
            .clone()
            .ok_or_else(|| anyhow::anyhow!("No thread selected"))?;
        anyhow::ensure!(
            !self.running.contains_key(&tid),
            "This thread already has a running turn"
        );
        let run = self
            .store
            .latest_resumable_run(&tid)?
            .ok_or_else(|| anyhow::anyhow!("No interrupted or failed run to resume"))?;
        let service =
            recon::Service::new(&paths::db_path(), self.auth.clone(), self.settings.clone())?;
        let tx = self.work_tx.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        self.running.insert(tid.clone(), cancel.clone());
        tokio::spawn(async move {
            let progress_tx = tx.clone();
            let thread_id = tid.clone();
            let outcome = service
                .resume(&run.id, cancel, move |stage| {
                    let _ = progress_tx.send(WorkEvent::ReconStage {
                        thread_id: thread_id.clone(),
                        stage: stage.into(),
                    });
                })
                .await
                .map(|_| ())
                .map_err(|e| e.to_string());
            let _ = tx.send(WorkEvent::ReconDone {
                thread_id: tid,
                outcome,
            });
        });
        Ok(())
    }

    fn retry_insights(&mut self) -> Result<()> {
        let tid = self
            .selected_thread
            .clone()
            .ok_or_else(|| anyhow::anyhow!("No thread selected"))?;
        let answer_id = self
            .store
            .latest_retryable_insight_job(&tid)?
            .ok_or_else(|| anyhow::anyhow!("No retryable insight job"))?;
        let service =
            recon::Service::new(&paths::db_path(), self.auth.clone(), self.settings.clone())?;
        let tx = self.work_tx.clone();
        tokio::spawn(async move {
            let outcome = service
                .retry_insights(&answer_id)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(WorkEvent::InsightDone {
                thread_id: tid,
                outcome,
            });
        });
        Ok(())
    }

    fn run_osint(&mut self) -> Result<()> {
        anyhow::ensure!(
            self.osint_cancel.is_none(),
            "An OSINT tool is already running"
        );
        let tool = osint::registry()
            .get(self.tool_sel)
            .ok_or_else(|| anyhow::anyhow!("No tool selected"))?;
        let input: Value = serde_json::from_str(&self.osint_input)?;
        osint::validate(tool.id, &input)?;
        let service =
            recon::Service::new(&paths::db_path(), self.auth.clone(), self.settings.clone())?;
        let tx = self.work_tx.clone();
        let tool_id = tool.id.to_string();
        let cancel = Arc::new(AtomicBool::new(false));
        self.osint_cancel = Some(cancel.clone());
        self.status = format!("Running {}", tool.name);
        tokio::spawn(async move {
            let outcome = service
                .manual_with_cancel(&tool_id, input, cancel)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(WorkEvent::OsintDone { outcome });
        });
        Ok(())
    }

    fn on_work_event(&mut self, event: WorkEvent) {
        match event {
            WorkEvent::ReconStage { thread_id, stage } => {
                if self.selected_thread.as_deref() == Some(&thread_id) {
                    self.recon_stage = stage;
                    let _ = self.refresh_selected();
                }
            }
            WorkEvent::ReconDone { thread_id, outcome } => {
                self.running.remove(&thread_id);
                if self.selected_thread.as_deref() == Some(&thread_id) {
                    self.recon_stage = outcome
                        .as_ref()
                        .map(|_| "complete".to_string())
                        .unwrap_or_else(|e| format!("failed: {e}"));
                    let _ = self.refresh_selected();
                }
                let _ = self.refresh_threads();
                if let Ok(memories) = self.store.list_memories() {
                    self.memories = memories;
                }
            }
            WorkEvent::OsintDone { outcome } => {
                self.osint_cancel = None;
                match outcome {
                    Ok(value) => {
                        self.status = format!("{}: {}", value.1.tool_id, value.1.status);
                        self.osint_result = Some(value);
                        if let Ok(runs) = self.store.manual_calls() {
                            self.manual_runs = runs;
                            self.manual_run_pos = self.manual_runs.len().saturating_sub(1);
                        }
                    }
                    Err(err) => self.status = err,
                }
            }
            WorkEvent::CatalogDone { synthesis, outcome } => {
                if self.defaults_synthesis == synthesis {
                    match outcome {
                        Ok(models) => {
                            self.status = format!("{} models available", models.len());
                            self.model_catalog = models;
                        }
                        Err(err) => self.status = err,
                    }
                }
            }
            WorkEvent::InsightDone { thread_id, outcome } => {
                if self.selected_thread.as_deref() == Some(&thread_id) {
                    self.status = outcome
                        .map(|_| "Insights saved".into())
                        .unwrap_or_else(|e| format!("Insight retry failed: {e}"));
                    if let Ok(memories) = self.store.list_memories() {
                        self.memories = memories;
                    }
                }
            }
        }
    }

    fn refresh_catalog(&mut self) {
        let synthesis = self.defaults_synthesis;
        let kind = if synthesis {
            &self.synthesis_provider
        } else {
            &self.recon_provider
        };
        let secret = provider::account_secret(&self.auth, kind);
        let tx = self.work_tx.clone();
        self.status = "Loading models".into();
        tokio::spawn(async move {
            let outcome = provider::list_models(&secret)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(WorkEvent::CatalogDone { synthesis, outcome });
        });
    }

    fn refresh_selected(&mut self) -> Result<()> {
        if let Some(tid) = &self.selected_thread {
            self.messages = self.store.list_messages(tid)?;
            self.calls = self.store.all_calls_for_thread(tid)?;
        }
        Ok(())
    }

    fn save_insight(&mut self) -> Result<String> {
        let (category, text) = argos_osint_core::brain::parse_typed_memory(&self.brain_insight);
        self.store.add_memory(
            &text,
            category,
            false,
            MemorySource {
                app: self.brain_app.trim().into(),
                conversation_id: self.brain_conversation.trim().into(),
                message_id: None,
                reference: None,
            },
        )?;
        self.memories = self.store.list_memories()?;
        self.brain_insight.clear();
        Ok("Insight saved with source".into())
    }

    fn activate_button(&mut self, button: ButtonId) {
        let result = match button {
            ButtonId::Send => {
                self.submit();
                return;
            }
            ButtonId::Add => self.save_insight(),
            ButtonId::Recall => self.store.recall(&self.brain_query, 8).map(|hits| {
                self.hits = hits;
                format!("{} relevant memories", self.hits.len())
            }),
            ButtonId::Pin => {
                let Some(memory) = self.memories.get(self.memory_sel) else {
                    self.status = "No memory selected".into();
                    return;
                };
                self.store
                    .update_memory(&memory.id, &memory.text, &memory.category, !memory.pinned)
                    .and_then(|_| self.store.list_memories())
                    .map(|memories| {
                        self.memories = memories;
                        "Memory pin updated".into()
                    })
            }
            ButtonId::Delete => {
                let Some(memory) = self.memories.get(self.memory_sel) else {
                    self.status = "No memory selected".into();
                    return;
                };
                self.store
                    .delete_memory(&memory.id)
                    .and_then(|_| self.store.list_memories())
                    .map(|memories| {
                        self.memories = memories;
                        self.memory_sel =
                            self.memory_sel.min(self.memories.len().saturating_sub(1));
                        "Memory deleted".into()
                    })
            }
            ButtonId::NewThread => self.new_thread().map(|_| "New investigation".into()),
            ButtonId::DeleteThread => {
                let Some(id) = self.selected_thread.clone() else {
                    self.status = "No thread selected".into();
                    return;
                };
                self.running
                    .remove(&id)
                    .inspect(|cancel| cancel.store(true, Ordering::Relaxed));
                self.store.delete_thread(&id, false).and_then(|_| {
                    self.selected_thread = None;
                    self.messages.clear();
                    self.input.clear();
                    self.refresh_threads()?;
                    if let Some(next) = self.threads.first().map(|t| t.id.clone()) {
                        self.open_thread(&next)?;
                    }
                    Ok("Conversation deleted; contributed insights retained".into())
                })
            }
            ButtonId::CancelRun => {
                if let Some(id) = &self.selected_thread {
                    if let Some(cancel) = self.running.get(id) {
                        cancel.store(true, Ordering::Relaxed);
                    }
                }
                Ok("Cancellation requested".into())
            }
            ButtonId::ResumeRun => self.resume_recon().map(|_| "Resuming run".into()),
            ButtonId::RetryInsights => self
                .retry_insights()
                .map(|_| "Retrying insight extraction".into()),
            ButtonId::OsintRun => self.run_osint().map(|_| "Tool started".into()),
            ButtonId::OsintCancel => {
                if let Some(cancel) = &self.osint_cancel {
                    cancel.store(true, Ordering::Relaxed);
                }
                Ok("Tool cancellation requested".into())
            }
            ButtonId::OsintToggle => {
                let tool = osint::registry()
                    .get(self.tool_sel)
                    .ok_or_else(|| anyhow::anyhow!("No tool selected"));
                match tool {
                    Ok(tool) => {
                        let enabled = !self
                            .tool_enabled
                            .get(self.tool_sel)
                            .copied()
                            .unwrap_or(true);
                        self.store.set_tool_enabled(tool.id, enabled).map(|_| {
                            if let Some(state) = self.tool_enabled.get_mut(self.tool_sel) {
                                *state = enabled;
                            }
                            format!(
                                "{} {}",
                                tool.name,
                                if enabled { "enabled" } else { "disabled" }
                            )
                        })
                    }
                    Err(err) => Err(err),
                }
            }
            ButtonId::OsintRaw => {
                self.osint_raw = !self.osint_raw;
                Ok(if self.osint_raw {
                    "Raw response"
                } else {
                    "Parsed result"
                }
                .into())
            }
            ButtonId::OsintAttach => {
                let Some((call_id, _)) = &self.osint_result else {
                    self.status = "No result selected".into();
                    return;
                };
                let Some(tid) = &self.selected_thread else {
                    self.status = "Open or create a Recon thread first".into();
                    return;
                };
                self.store
                    .attach_call(call_id, tid)
                    .map(|_| "Result attached to thread".into())
            }
            ButtonId::OsintStartRecon => {
                let Some((call_id, result)) = &self.osint_result else {
                    self.status = "No result selected".into();
                    return;
                };
                let call_id = call_id.clone();
                let title = format!("Investigate {}", result.tool_id);
                self.store.new_thread(&title).and_then(|thread| {
                    self.store.attach_call(&call_id, &thread.id)?;
                    self.open_thread(&thread.id)?;
                    self.module = Some(ModuleId::Recon);
                    self.set_focus(Target::Field(FieldId::Composer));
                    Ok("Recon started from result".into())
                })
            }
            ButtonId::SaveRecon => {
                let provider = self.recon_provider.trim();
                let model = self.recon_model.trim();
                if provider.is_empty() || model.is_empty() {
                    Err(anyhow::anyhow!("Provider and model are required"))
                } else {
                    let kind = provider::normalize_kind(provider);
                    let kind = if kind == "openai" {
                        "openai-chatgpt".into()
                    } else {
                        kind
                    };
                    if !matches!(
                        kind.as_str(),
                        "grok" | "openai-chatgpt" | "openrouter" | "local"
                    ) {
                        self.status = "Choose Grok, OpenAI, OpenRouter, or local".into();
                        return;
                    }
                    self.settings.defaults.recon.provider = kind;
                    self.settings.defaults.recon.model = model.into();
                    self.settings.save().map(|_| {
                        format!(
                            "Recon: {} / {}",
                            self.settings.defaults.recon.provider, model
                        )
                    })
                }
            }
            ButtonId::SaveSynthesis => {
                let provider = self.synthesis_provider.trim();
                let model = self.synthesis_model.trim();
                if provider.is_empty() || model.is_empty() {
                    Err(anyhow::anyhow!("Provider and model are required"))
                } else {
                    let kind = provider::normalize_kind(provider);
                    self.settings.defaults.synthesis.provider = kind;
                    self.settings.defaults.synthesis.model = model.into();
                    self.settings
                        .save()
                        .map(|_| "Synthesis default saved".into())
                }
            }
            ButtonId::ToggleDefaultRole => {
                self.defaults_synthesis = !self.defaults_synthesis;
                self.model_catalog.clear();
                Ok(format!(
                    "{} default",
                    if self.defaults_synthesis {
                        "Synthesis"
                    } else {
                        "Recon"
                    }
                ))
            }
            ButtonId::RefreshModels => {
                self.refresh_catalog();
                return;
            }
            ButtonId::GrokSignIn => {
                self.start_subscription(ProviderPage::Grok, true);
                return;
            }
            ButtonId::GrokCheck => {
                self.start_subscription(ProviderPage::Grok, false);
                return;
            }
            ButtonId::OpenAISignIn => {
                self.start_subscription(ProviderPage::OpenAI, true);
                return;
            }
            ButtonId::OpenAICheck => {
                self.start_subscription(ProviderPage::OpenAI, false);
                return;
            }
            ButtonId::RouterSave => self.save_router(),
            ButtonId::RouterVerify => {
                self.verify_router();
                return;
            }
            ButtonId::RouterAdvanced => {
                self.router_advanced = !self.router_advanced;
                Ok(if self.router_advanced {
                    "Advanced endpoint shown"
                } else {
                    "Advanced endpoint hidden"
                }
                .into())
            }
            ButtonId::RefreshHardware => {
                self.hardware = hardware::profile_cached(true);
                Ok("Hardware refreshed".into())
            }
        };
        self.status = result.unwrap_or_else(|err| err.to_string());
    }

    fn router_draft(&self) -> Result<ProviderSecret> {
        let mut secret = provider::account_secret(&self.auth, "openrouter");
        secret.api_key = Some(self.router_key.trim().to_string()).filter(|key| !key.is_empty());
        secret.base_url = provider::normalize_base(self.router_endpoint.trim());
        let endpoint = url::Url::parse(&secret.base_url)
            .map_err(|_| anyhow::anyhow!("Enter a valid HTTPS API endpoint"))?;
        anyhow::ensure!(
            endpoint.scheme() == "https"
                && endpoint.host_str().is_some()
                && endpoint.username().is_empty()
                && endpoint.password().is_none()
                && endpoint.query().is_none()
                && endpoint.fragment().is_none(),
            "Use an HTTPS endpoint without credentials, query, or fragment"
        );
        Ok(secret)
    }

    fn save_router(&mut self) -> Result<String> {
        let secret = self.router_draft()?;
        let mut auth = self.auth.clone();
        auth.set_account(secret);
        auth.save_to(&self.auth_path)?;
        self.auth = auth;
        self.router_status = "OpenRouter account saved · verify to test access".into();
        Ok(self.router_status.clone())
    }

    fn verify_router(&mut self) {
        if self.provider_pending.is_some() {
            self.status = "Another provider check is running".into();
            return;
        }
        let secret = match self.router_draft() {
            Ok(secret) => secret,
            Err(err) => {
                self.status = err.to_string();
                return;
            }
        };
        let saved = provider::account_secret(&self.auth, "openrouter");
        let draft = secret.api_key != saved.api_key || secret.base_url != saved.base_url;
        self.provider_pending = Some(ProviderPage::OpenRouter);
        self.router_status = "Verifying OpenRouter connection…".into();
        self.status = self.router_status.clone();
        let tx = self.provider_tx.clone();
        tokio::spawn(async move {
            let result = provider::verified_catalog(&secret)
                .await
                .map(|models| {
                    format!(
                        "OpenRouter verified · {} models{}",
                        models.len(),
                        if draft { " · Save to use" } else { "" }
                    )
                })
                .map_err(|err| err.to_string());
            let _ = tx.send(ProviderEvent::Finished {
                page: ProviderPage::OpenRouter,
                result,
            });
        });
    }

    fn start_subscription(&mut self, page: ProviderPage, login: bool) {
        if self.provider_pending.is_some() {
            self.status = "Another provider sign-in is running".into();
            return;
        }
        self.provider_pending = Some(page);
        self.provider_progress.clear();
        self.provider_progress_page = Some(page);
        let label = if page == ProviderPage::Grok {
            "Grok"
        } else {
            "ChatGPT"
        };
        let status = format!("{} {}…", if login { "Starting" } else { "Checking" }, label);
        if page == ProviderPage::Grok {
            self.grok_status = status.clone();
        } else {
            self.openai_status = status.clone();
        }
        self.status = status;
        let tx = self.provider_tx.clone();
        tokio::spawn(async move {
            let result = if page == ProviderPage::Grok {
                let outcome = if login {
                    let progress_tx = tx.clone();
                    argos_osint_core::grok_oauth::login(move |line| {
                        let _ = progress_tx.send(ProviderEvent::Progress {
                            page,
                            line: line.into(),
                        });
                    })
                    .await
                } else {
                    argos_osint_core::grok_oauth::check_login().await
                };
                match outcome {
                    Ok(_) => {
                        let secret = provider::account_secret(&AuthFile::default(), "grok");
                        provider::verified_catalog(&secret)
                            .await
                            .map(|models| {
                                format!("Grok subscription connected · {} models", models.len())
                            })
                            .map_err(|err| err.to_string())
                    }
                    Err(err) => Err(err.to_string()),
                }
            } else {
                let outcome = if login {
                    let progress_tx = tx.clone();
                    argos_osint_core::subscription::login(move |line| {
                        let _ = progress_tx.send(ProviderEvent::Progress {
                            page,
                            line: line.into(),
                        });
                    })
                    .await
                } else {
                    argos_osint_core::subscription::check_login().await
                };
                outcome.map_err(|err| err.to_string())
            };
            let _ = tx.send(ProviderEvent::Finished { page, result });
        });
    }

    fn on_provider_event(&mut self, event: ProviderEvent) {
        match event {
            ProviderEvent::Progress { page, line } => {
                if self.provider_pending == Some(page) {
                    self.provider_progress.push(line);
                    if self.provider_progress.len() > 6 {
                        self.provider_progress.remove(0);
                    }
                }
            }
            ProviderEvent::Finished { page, result } => {
                if self.provider_pending != Some(page) {
                    return;
                }
                self.provider_pending = None;
                let message = result.unwrap_or_else(|err| err);
                match page {
                    ProviderPage::Grok => self.grok_status = message.clone(),
                    ProviderPage::OpenAI => self.openai_status = message.clone(),
                    ProviderPage::OpenRouter => self.router_status = message.clone(),
                    ProviderPage::Defaults => {}
                }
                self.status = message;
            }
        }
    }

    fn activate_target(&mut self, target: Target) {
        match target {
            Target::App(index) => self.select(index),
            Target::ProviderTab(page) => {
                self.provider_page = page;
                self.set_focus(target);
            }
            Target::Memory(index) => {
                self.memory_sel = index;
                self.selected_insight = self
                    .memories
                    .get(index)
                    .and_then(|m| self.store.insight_for_memory(&m.id).ok().flatten());
                self.set_focus(target);
            }
            Target::Thread(index) => {
                if let Some(id) = self.threads.get(index).map(|t| t.id.clone()) {
                    self.status = self
                        .open_thread(&id)
                        .map(|_| "Thread opened".into())
                        .unwrap_or_else(|e| e.to_string());
                    self.set_focus(Target::Thread(index));
                }
            }
            Target::Tool(index) => self.select_tool(index),
            Target::Field(_) => self.set_focus(target),
            Target::Button(button) => {
                self.set_focus(target);
                self.activate_button(button);
            }
        }
    }

    fn edit_char(&mut self, character: char) {
        let Target::Field(field) = self.focus else {
            return;
        };
        let cursor = self.cursor;
        let value = self.field_mut(field);
        let byte = value
            .char_indices()
            .nth(cursor)
            .map(|(byte, _)| byte)
            .unwrap_or(value.len());
        value.insert(byte, character);
        self.cursor += 1;
    }

    fn edit_backspace(&mut self) {
        let Target::Field(field) = self.focus else {
            return;
        };
        if self.cursor == 0 {
            return;
        }
        let cursor = self.cursor;
        let value = self.field_mut(field);
        let start = value
            .char_indices()
            .nth(cursor - 1)
            .map(|(byte, _)| byte)
            .unwrap_or(0);
        let end = value
            .char_indices()
            .nth(cursor)
            .map(|(byte, _)| byte)
            .unwrap_or(value.len());
        value.replace_range(start..end, "");
        self.cursor -= 1;
    }

    fn edit_delete(&mut self) {
        let Target::Field(field) = self.focus else {
            return;
        };
        let cursor = self.cursor;
        let value = self.field_mut(field);
        let start = value
            .char_indices()
            .nth(cursor)
            .map(|(byte, _)| byte)
            .unwrap_or(value.len());
        let end = value
            .char_indices()
            .nth(cursor + 1)
            .map(|(byte, _)| byte)
            .unwrap_or(value.len());
        value.replace_range(start..end, "");
    }

    fn submit(&mut self) {
        let input = std::mem::take(&mut self.input);
        let input = input.trim();
        if input.is_empty() {
            return;
        }
        let result = match self.module {
            Some(ModuleId::Recon) => {
                self.input = input.into();
                self.recon_command(input)
            }
            Some(ModuleId::Brain) => self.brain_command(input),
            Some(ModuleId::Osint) => self.osint_command(input),
            Some(ModuleId::Providers) => self.provider_command(input),
            Some(ModuleId::System) if input == "refresh" => {
                self.hardware = hardware::profile_cached(true);
                Ok("Hardware refreshed".into())
            }
            _ => Err(anyhow::anyhow!(
                "Open Brain or Providers to use the composer"
            )),
        };
        self.status = match result {
            Ok(message) => message,
            Err(err) => format!("{err}"),
        };
    }

    fn osint_command(&mut self, input: &str) -> Result<String> {
        if matches!(input, ":prev" | ":next") {
            let tool = osint::registry()
                .get(self.tool_sel)
                .ok_or_else(|| anyhow::anyhow!("No tool selected"))?;
            let positions: Vec<_> = self
                .manual_runs
                .iter()
                .enumerate()
                .filter(|(_, call)| call.tool_id == tool.id && call.result.is_some())
                .map(|(i, _)| i)
                .collect();
            anyhow::ensure!(!positions.is_empty(), "No previous runs for this tool");
            let current = positions
                .iter()
                .position(|p| *p == self.manual_run_pos)
                .unwrap_or(positions.len() - 1);
            let next = if input == ":prev" {
                current.saturating_sub(1)
            } else {
                (current + 1).min(positions.len() - 1)
            };
            self.manual_run_pos = positions[next];
            let call = &self.manual_runs[self.manual_run_pos];
            self.osint_result = call.result.clone().map(|r| (call.id.clone(), r));
            Ok(format!("Manual result {} of {}", next + 1, positions.len()))
        } else {
            self.run_osint()?;
            Ok("Tool started".into())
        }
    }

    fn recon_command(&mut self, input: &str) -> Result<String> {
        if let Some(title) = input.strip_prefix(":rename ") {
            let id = self
                .selected_thread
                .clone()
                .ok_or_else(|| anyhow::anyhow!("No thread selected"))?;
            self.store.rename_thread(&id, title)?;
            self.refresh_threads()?;
            self.input.clear();
            return Ok("Thread renamed".into());
        }
        if input == ":delete" || input == ":delete-with-insights" {
            let id = self
                .selected_thread
                .clone()
                .ok_or_else(|| anyhow::anyhow!("No thread selected"))?;
            self.running
                .remove(&id)
                .inspect(|c| c.store(true, Ordering::Relaxed));
            let with_insights = input.ends_with("insights");
            let retained = if with_insights {
                self.store.deletion_consequences(&id)?.len()
            } else {
                0
            };
            self.store.delete_thread(&id, with_insights)?;
            self.selected_thread = None;
            self.messages.clear();
            self.refresh_threads()?;
            if let Some(next) = self.threads.first().map(|t| t.id.clone()) {
                self.open_thread(&next)?;
            }
            self.input.clear();
            return Ok(format!("Thread deleted; {retained} pinned or edited insights retained without their deleted source"));
        }
        if input == ":cancel" {
            if let Some(id) = &self.selected_thread {
                if let Some(cancel) = self.running.get(id) {
                    cancel.store(true, Ordering::Relaxed);
                }
            }
            self.input.clear();
            return Ok("Cancellation requested".into());
        }
        self.send_recon()?;
        Ok("Recon started".into())
    }

    fn brain_command(&mut self, input: &str) -> Result<String> {
        if input == "source" {
            self.open_insight_source()?;
            return Ok("Source thread opened".into());
        }
        if let Some(query) = input.strip_prefix("recall ") {
            self.hits = self.store.recall(query, 8)?;
            return Ok(format!("{} relevant memories", self.hits.len()));
        }
        if let Some(rest) = input.strip_prefix("add ") {
            let (source, text) = rest
                .split_once('|')
                .ok_or_else(|| anyhow::anyhow!("Use: add <app> <conversation-id> | <insight>"))?;
            let mut parts = source.split_whitespace();
            let app = parts.next().unwrap_or_default();
            let conversation_id = parts.next().unwrap_or_default();
            anyhow::ensure!(
                parts.next().is_none(),
                "Use one app and one conversation ID"
            );
            let (category, text) = argos_osint_core::brain::parse_typed_memory(text);
            self.store.add_memory(
                &text,
                category,
                false,
                MemorySource {
                    app: app.into(),
                    conversation_id: conversation_id.into(),
                    message_id: None,
                    reference: None,
                },
            )?;
            self.memories = self.store.list_memories()?;
            return Ok("Insight saved with source".into());
        }
        if input == "pin" {
            let memory = self
                .memories
                .get(self.memory_sel)
                .ok_or_else(|| anyhow::anyhow!("No memory selected"))?;
            self.store
                .update_memory(&memory.id, &memory.text, &memory.category, !memory.pinned)?;
            self.memories = self.store.list_memories()?;
            return Ok("Memory pin updated".into());
        }
        if input == "delete" {
            let memory = self
                .memories
                .get(self.memory_sel)
                .ok_or_else(|| anyhow::anyhow!("No memory selected"))?;
            self.store.delete_memory(&memory.id)?;
            self.memories = self.store.list_memories()?;
            self.memory_sel = self.memory_sel.min(self.memories.len().saturating_sub(1));
            return Ok("Memory deleted".into());
        }
        Err(anyhow::anyhow!("Use add, recall, pin, or delete"))
    }

    fn open_insight_source(&mut self) -> Result<()> {
        let insight = self
            .selected_insight
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Select an investigation insight"))?;
        let id = insight
            .sources
            .iter()
            .filter_map(|s| s.thread_id.as_ref())
            .find(|id| self.store.get_thread(id).ok().flatten().is_some())
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("No surviving source thread"))?;
        self.open_thread(&id)?;
        self.module = Some(ModuleId::Recon);
        self.set_focus(Target::Field(FieldId::Composer));
        Ok(())
    }

    fn provider_command(&mut self, input: &str) -> Result<String> {
        if let Some(rest) = input.strip_prefix("recon ") {
            let mut parts = rest.split_whitespace();
            let provider = parts
                .next()
                .ok_or_else(|| anyhow::anyhow!("Use: recon <provider> <model>"))?;
            let model = parts
                .next()
                .ok_or_else(|| anyhow::anyhow!("Use: recon <provider> <model>"))?;
            anyhow::ensure!(parts.next().is_none(), "Use one provider and one model ID");
            let kind = provider::normalize_kind(provider);
            let kind = if kind == "openai" {
                "openai-chatgpt".into()
            } else {
                kind
            };
            anyhow::ensure!(
                matches!(
                    kind.as_str(),
                    "grok" | "openai-chatgpt" | "openrouter" | "local"
                ),
                "Choose Grok, OpenAI, OpenRouter, or local"
            );
            self.settings.defaults.recon.provider = kind;
            self.settings.defaults.recon.model = model.into();
            self.settings.save()?;
            self.recon_provider = self.settings.defaults.recon.provider.clone();
            self.recon_model = self.settings.defaults.recon.model.clone();
            return Ok(format!(
                "Recon: {} / {}",
                self.settings.defaults.recon.provider, model
            ));
        }
        Err(anyhow::anyhow!(
            "Use Defaults controls to set Recon and Synthesis models"
        ))
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return false;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('n') {
            self.status = self
                .new_thread()
                .map(|_| "New investigation".into())
                .unwrap_or_else(|e| e.to_string());
            self.module = Some(ModuleId::Recon);
            return true;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && key.code == KeyCode::Char('o')
            && self.module == Some(ModuleId::Brain)
        {
            self.status = self
                .open_insight_source()
                .map(|_| "Source thread opened".into())
                .unwrap_or_else(|e| e.to_string());
            return true;
        }
        if key.modifiers.contains(KeyModifiers::ALT)
            && matches!(key.code, KeyCode::Left | KeyCode::Right)
        {
            let next = if key.code == KeyCode::Left {
                self.history_pos.saturating_sub(1)
            } else {
                (self.history_pos + 1).min(self.thread_history.len().saturating_sub(1))
            };
            if let Some(id) = self.thread_history.get(next).cloned() {
                self.history_pos = next;
                let _ = self.open_thread_with_history(&id, false);
                self.module = Some(ModuleId::Recon);
            }
            return true;
        }
        if key.modifiers.contains(KeyModifiers::SHIFT)
            && key.code == KeyCode::Enter
            && self.focus == Target::Field(FieldId::Composer)
        {
            self.edit_char('\n');
            return true;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('a') {
            if let Target::Field(field) = self.focus {
                self.field_mut(field).clear();
                self.cursor = 0;
            }
            return true;
        }
        match key.code {
            KeyCode::Esc => {
                self.set_focus(Target::Field(FieldId::Composer));
            }
            KeyCode::Enter => match self.focus {
                Target::Field(FieldId::Composer) => self.submit(),
                Target::Field(_) => self.focus_next(false),
                target => self.activate_target(target),
            },
            KeyCode::Backspace => self.edit_backspace(),
            KeyCode::Delete => self.edit_delete(),
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => {
                if let Target::Field(field) = self.focus {
                    self.cursor = (self.cursor + 1).min(self.field(field).chars().count());
                }
            }
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => {
                if let Target::Field(field) = self.focus {
                    self.cursor = self.field(field).chars().count();
                }
            }
            KeyCode::Char(c) => self.edit_char(c),
            KeyCode::Up if self.module == Some(ModuleId::Recon) => {
                self.thread_sel = self.thread_sel.saturating_sub(1);
                self.set_focus(Target::Thread(self.thread_sel));
            }
            KeyCode::Down if self.module == Some(ModuleId::Recon) => {
                self.thread_sel = (self.thread_sel + 1).min(self.threads.len().saturating_sub(1));
                self.set_focus(Target::Thread(self.thread_sel));
            }
            KeyCode::Up if self.module == Some(ModuleId::Osint) => {
                let ids = self.filtered_tool_ids();
                let pos = ids.iter().position(|id| *id == self.tool_sel).unwrap_or(0);
                let next = *ids.get(pos.saturating_sub(1)).unwrap_or(&self.tool_sel);
                self.select_tool(next);
            }
            KeyCode::Down if self.module == Some(ModuleId::Osint) => {
                let ids = self.filtered_tool_ids();
                let pos = ids.iter().position(|id| *id == self.tool_sel).unwrap_or(0);
                let next = *ids
                    .get((pos + 1).min(ids.len().saturating_sub(1)))
                    .unwrap_or(&self.tool_sel);
                self.select_tool(next);
            }
            KeyCode::Up if self.module == Some(ModuleId::Brain) => {
                self.memory_sel = self.memory_sel.saturating_sub(1);
                self.set_focus(Target::Memory(self.memory_sel));
            }
            KeyCode::Down if self.module == Some(ModuleId::Brain) => {
                self.memory_sel = (self.memory_sel + 1).min(self.memories.len().saturating_sub(1));
                self.set_focus(Target::Memory(self.memory_sel));
            }
            KeyCode::Up => self.launcher_sel = self.launcher_sel.saturating_sub(1),
            KeyCode::Down => {
                self.launcher_sel = (self.launcher_sel + 1).min(ModuleId::ALL.len() - 1)
            }
            KeyCode::Tab => self.focus_next(key.modifiers.contains(KeyModifiers::SHIFT)),
            KeyCode::F(n) if (1..=ModuleId::ALL.len() as u8).contains(&n) => {
                self.select(n as usize - 1)
            }
            _ => {}
        }
        if self.module == Some(ModuleId::Recon) && self.focus == Target::Field(FieldId::ReconSearch)
        {
            let _ = self.refresh_threads();
        }
        if self.module == Some(ModuleId::Osint) && self.focus == Target::Field(FieldId::OsintSearch)
        {
            if let Some(id) = self.filtered_tool_ids().first() {
                let next = *id;
                self.select_tool(next);
                self.set_focus(Target::Field(FieldId::OsintSearch));
            }
        }
        if self.module == Some(ModuleId::Recon) && self.focus == Target::Field(FieldId::Composer) {
            if let Some(id) = &self.selected_thread {
                let _ = self
                    .store
                    .save_draft(id, &self.input, i64::from(self.recon_scroll));
            }
        }
        true
    }

    fn focus_next(&mut self, reverse: bool) {
        let order = super::ui::focus_order(self);
        if order.is_empty() {
            return;
        }
        let current = order
            .iter()
            .position(|target| *target == self.focus)
            .unwrap_or(0);
        let next = if reverse {
            (current + order.len() - 1) % order.len()
        } else {
            (current + 1) % order.len()
        };
        self.set_focus(order[next]);
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(target) = super::ui::hit_test(self, mouse.column, mouse.row) {
                    self.activate_target(target);
                    if let Target::Field(field) = target {
                        self.cursor = super::ui::cursor_at(self, field, mouse.column);
                    }
                }
            }
            MouseEventKind::ScrollDown if self.module == Some(ModuleId::Brain) => {
                self.memory_sel = (self.memory_sel + 1).min(self.memories.len().saturating_sub(1));
            }
            MouseEventKind::ScrollUp if self.module == Some(ModuleId::Brain) => {
                self.memory_sel = self.memory_sel.saturating_sub(1);
            }
            MouseEventKind::ScrollDown if self.module == Some(ModuleId::Recon) => {
                self.thread_sel = (self.thread_sel + 1).min(self.threads.len().saturating_sub(1));
            }
            MouseEventKind::ScrollUp if self.module == Some(ModuleId::Recon) => {
                self.thread_sel = self.thread_sel.saturating_sub(1);
            }
            MouseEventKind::ScrollDown if self.module == Some(ModuleId::Osint) => {
                self.tool_sel = (self.tool_sel + 1).min(osint::registry().len().saturating_sub(1));
            }
            MouseEventKind::ScrollUp if self.module == Some(ModuleId::Osint) => {
                self.tool_sel = self.tool_sel.saturating_sub(1);
            }
            _ => {}
        }
    }
}

pub async fn run(mut app: App) -> Result<()> {
    use crossterm::{
        execute,
        terminal::{enable_raw_mode, EnterAlternateScreen},
    };
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = crossterm::terminal::disable_raw_mode();
            let _ = crossterm::execute!(
                std::io::stdout(),
                crossterm::event::DisableMouseCapture,
                crossterm::terminal::LeaveAlternateScreen
            );
        }
    }
    enable_raw_mode()?;
    let _restore = Restore;
    let mut stdout = std::io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;
    let mut terminal: Terminal<ratatui::backend::CrosstermBackend<Stdout>> =
        Terminal::new(ratatui::backend::CrosstermBackend::new(stdout))?;
    loop {
        while let Ok(message) = app.provider_rx.try_recv() {
            app.on_provider_event(message);
        }
        while let Ok(message) = app.work_rx.try_recv() {
            app.on_work_event(message);
        }
        terminal.draw(|frame| {
            app.screen = frame.area();
            super::ui::draw(frame, &app)
        })?;
        let next = if event::poll(Duration::from_millis(150))? {
            Some(event::read()?)
        } else {
            None
        };
        match next {
            Some(Event::Key(key)) if key.kind == KeyEventKind::Press && !app.handle_key(key) => {
                break
            }
            Some(Event::Key(_)) => {}
            Some(Event::Mouse(mouse)) => app.handle_mouse(mouse),
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let (provider_tx, provider_rx) = unbounded_channel();
        let (work_tx, work_rx) = unbounded_channel();
        App {
            module: None,
            launcher_sel: 0,
            provider_page: ProviderPage::Grok,
            input: String::new(),
            brain_app: String::new(),
            brain_conversation: String::new(),
            brain_insight: String::new(),
            brain_query: String::new(),
            recon_search: String::new(),
            osint_search: String::new(),
            osint_input: osint::registry()[0].example_input().to_string(),
            osint_inputs: HashMap::new(),
            selected_thread: None,
            threads: Vec::new(),
            thread_states: HashMap::new(),
            messages: Vec::new(),
            calls: Vec::new(),
            thread_sel: 0,
            tool_sel: 0,
            tool_enabled: vec![true; osint::registry().len()],
            osint_result: None,
            osint_raw: false,
            manual_runs: Vec::new(),
            manual_run_pos: 0,
            recon_stage: "ready".into(),
            recon_scroll: 0,
            thread_history: Vec::new(),
            history_pos: 0,
            running: HashMap::new(),
            osint_cancel: None,
            recon_provider: String::new(),
            recon_model: String::new(),
            synthesis_provider: String::new(),
            synthesis_model: String::new(),
            defaults_synthesis: false,
            model_catalog: Vec::new(),
            router_key: String::new(),
            router_endpoint: "https://openrouter.ai/api/v1".into(),
            router_advanced: false,
            grok_status: "Not checked".into(),
            openai_status: "Not checked".into(),
            router_status: "Not checked".into(),
            provider_progress: Vec::new(),
            provider_progress_page: None,
            provider_pending: None,
            focus: Target::App(0),
            cursor: 0,
            screen: Rect::new(0, 0, 100, 34),
            status: String::new(),
            memories: Vec::new(),
            selected_insight: None,
            memory_sel: 0,
            hits: Vec::new(),
            auth: AuthFile::default(),
            settings: SettingsFile::default(),
            hardware: HardwareProfile::unknown(),
            auth_path: PathBuf::new(),
            store: Store::memory().unwrap(),
            provider_tx,
            provider_rx,
            work_tx,
            work_rx,
        }
    }

    fn click(app: &mut App, target: Target) {
        let position = (0..app.screen.height)
            .flat_map(|y| (0..app.screen.width).map(move |x| (x, y)))
            .find(|(x, y)| super::super::ui::hit_test(app, *x, *y) == Some(target))
            .expect("visible click target");
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: position.0,
            row: position.1,
            modifiers: KeyModifiers::NONE,
        });
    }

    fn type_text(app: &mut App, text: &str) {
        for character in text.chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }
    }

    #[test]
    fn brain_tab_still_edits_and_recalls_sourced_memories() {
        let mut app = app();
        click(&mut app, Target::App(1));
        click(&mut app, Target::Field(FieldId::BrainApp));
        type_text(&mut app, "chat");
        click(&mut app, Target::Field(FieldId::BrainConversation));
        type_text(&mut app, "thread-1");
        click(&mut app, Target::Field(FieldId::BrainInsight));
        type_text(&mut app, "project: Atlas launch");
        click(&mut app, Target::Button(ButtonId::Add));
        assert_eq!(app.memories[0].source.conversation_id, "thread-1");
        click(&mut app, Target::Field(FieldId::BrainQuery));
        type_text(&mut app, "Atlas");
        click(&mut app, Target::Button(ButtonId::Recall));
        assert_eq!(app.hits.len(), 1);
    }

    #[test]
    fn provider_auth_tabs_and_router_form_are_clickable() {
        let mut app = app();
        click(&mut app, Target::App(3));
        for page in ProviderPage::ALL {
            click(&mut app, Target::ProviderTab(page));
            assert_eq!(app.provider_page, page);
        }
        click(&mut app, Target::ProviderTab(ProviderPage::OpenRouter));
        click(&mut app, Target::Field(FieldId::RouterKey));
        type_text(&mut app, "secret");
        assert_eq!(app.router_key, "secret");
        click(&mut app, Target::Button(ButtonId::RouterAdvanced));
        assert!(app.router_advanced);
        click(&mut app, Target::Field(FieldId::RouterEndpoint));
        app.handle_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        type_text(&mut app, "invalid-");
        click(&mut app, Target::Button(ButtonId::RouterSave));
        assert!(app.status.contains("HTTPS"));
        assert!(app.auth.account("openrouter").is_none());
    }

    #[test]
    fn openrouter_save_keeps_other_accounts_and_recon_routing() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app();
        app.auth_path = dir.path().join("auth.json");
        let mut other = provider::account_secret(&app.auth, "local");
        other.api_key = Some("other-secret".into());
        app.auth.set_account(other);
        app.settings.defaults.recon.provider = "grok".into();
        click(&mut app, Target::App(3));
        click(&mut app, Target::ProviderTab(ProviderPage::OpenRouter));
        click(&mut app, Target::Field(FieldId::RouterKey));
        type_text(&mut app, "router-secret");
        click(&mut app, Target::Button(ButtonId::RouterSave));
        assert_eq!(
            app.auth.account("openrouter").unwrap().api_key.as_deref(),
            Some("router-secret")
        );
        assert_eq!(
            app.auth.account("local").unwrap().api_key.as_deref(),
            Some("other-secret")
        );
        assert_eq!(app.settings.defaults.recon.provider, "grok");
        let saved = std::fs::read_to_string(&app.auth_path).unwrap();
        assert!(saved.contains("router-secret"));
    }

    #[test]
    fn subscription_progress_and_result_update_the_correct_page() {
        let mut app = app();
        app.provider_pending = Some(ProviderPage::Grok);
        app.on_provider_event(ProviderEvent::Progress {
            page: ProviderPage::Grok,
            line: "Open browser".into(),
        });
        assert_eq!(app.provider_progress, ["Open browser"]);
        app.on_provider_event(ProviderEvent::Finished {
            page: ProviderPage::Grok,
            result: Ok("Grok subscription connected · 2 models".into()),
        });
        assert!(app.grok_status.contains("connected"));
        assert_eq!(app.openai_status, "Not checked");
        assert_eq!(app.provider_pending, None);
    }

    #[test]
    fn all_provider_actions_render_with_hit_areas_at_80x24() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 80, 24);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        app.select(3);
        for (page, target) in [
            (ProviderPage::Grok, Target::Button(ButtonId::GrokSignIn)),
            (ProviderPage::OpenAI, Target::Button(ButtonId::OpenAISignIn)),
            (
                ProviderPage::OpenRouter,
                Target::Button(ButtonId::RouterVerify),
            ),
            (ProviderPage::Defaults, Target::Button(ButtonId::SaveRecon)),
        ] {
            app.provider_page = page;
            terminal
                .draw(|frame| super::super::ui::draw(frame, &app))
                .unwrap();
            assert!((0..24)
                .flat_map(|y| (0..80).map(move |x| (x, y)))
                .any(|(x, y)| super::super::ui::hit_test(&app, x, y) == Some(target)));
        }
        app.provider_page = ProviderPage::Defaults;
        app.defaults_synthesis = true;
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();
        assert!((0..24)
            .flat_map(|y| (0..80).map(move |x| (x, y)))
            .any(|(x, y)| super::super::ui::hit_test(&app, x, y)
                == Some(Target::Button(ButtonId::SaveSynthesis))));
    }

    #[test]
    fn recon_and_osint_controls_are_clickable_at_80x24() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 80, 24);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        app.select(0);
        terminal.draw(|f| super::super::ui::draw(f, &app)).unwrap();
        for target in [
            Target::Button(ButtonId::NewThread),
            Target::Button(ButtonId::DeleteThread),
            Target::Button(ButtonId::CancelRun),
            Target::Button(ButtonId::ResumeRun),
            Target::Button(ButtonId::RetryInsights),
            Target::Button(ButtonId::Send),
        ] {
            assert!(
                (0..24)
                    .flat_map(|y| (0..80).map(move |x| (x, y)))
                    .any(|(x, y)| super::super::ui::hit_test(&app, x, y) == Some(target)),
                "missing {target:?}"
            );
        }
        app.select(2);
        terminal.draw(|f| super::super::ui::draw(f, &app)).unwrap();
        for target in [
            Target::Button(ButtonId::OsintRun),
            Target::Button(ButtonId::OsintCancel),
            Target::Button(ButtonId::OsintToggle),
            Target::Button(ButtonId::OsintRaw),
            Target::Button(ButtonId::OsintAttach),
            Target::Button(ButtonId::OsintStartRecon),
        ] {
            assert!(
                (0..24)
                    .flat_map(|y| (0..80).map(move |x| (x, y)))
                    .any(|(x, y)| super::super::ui::hit_test(&app, x, y) == Some(target)),
                "missing {target:?}"
            );
        }
    }
}
