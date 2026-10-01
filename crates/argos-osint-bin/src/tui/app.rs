//! App state and keyboard routing for the Argos terminal shell.

use anyhow::Result;
use argos_osint_core::brain::{Memory, MemorySource, ScoredMemory};
use argos_osint_core::hardware::{self, HardwareProfile};
use argos_osint_core::paths;
use argos_osint_core::provider::{self, ListedModel, SettingsFile};
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
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io::Stdout;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
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
            Self::Brain => "Recall and manage insights",
            Self::Osint => "Configure public lookup tools",
            Self::Providers => "Accounts and model defaults",
            Self::System => "Hardware, paths, and event log",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Scrolls {
    pub chat: u16,
    pub threads: u16,
    pub memories: u16,
    pub tools: u16,
    pub detail: u16,
    pub log: u16,
    pub popup: u16,
    pub recall: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChoiceKind {
    Provider,
    Model,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChoiceItem {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Overlay {
    None,
    Help,
    Memories { message_id: String },
    Block { title: String, body: String },
    Choice(ChoiceKind),
}

#[derive(Clone, Debug)]
pub struct LogLine {
    pub at: String,
    pub level: String,
    pub text: String,
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
    FirecrawlKey,
    HunterKey,
    SociaVaultKey,
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
    OsintPrev,
    OsintNext,
    SaveFirecrawlKey,
    SaveHunterKey,
    SaveSociaVaultKey,
    OpenSource,
    ClearLog,
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
    Home,
    ProviderTab(ProviderPage),
    Memory(usize),
    Thread(usize),
    Tool(usize),
    Field(FieldId),
    Button(ButtonId),
    Transcript,
    ChatHeader(usize),
    ChatBody(usize),
    BrainMark(usize),
    Choice(usize),
    CloseOverlay,
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
        provider: String,
        outcome: Result<Vec<ListedModel>, String>,
    },
    Access {
        grok: bool,
        openai: bool,
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
    pub firecrawl_key: String,
    pub hunter_key: String,
    pub sociavault_key: String,
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
    /// False on the investigation list. True when a transcript fills the screen.
    pub recon_chat: bool,
    pub scrolls: Scrolls,
    pub expanded: HashSet<String>,
    pub chat_sel: usize,
    pub chat_follow: bool,
    pub overlay: Overlay,
    pub log: Vec<LogLine>,
    pub runs: Vec<recon::Run>,
    pub answer_memories: HashMap<String, Vec<Memory>>,
    quit_arm: Option<Instant>,
    esc_arm: Option<Instant>,
    press: Option<(u16, u16, Option<Target>)>,
    draft_dirty: bool,
    pub frame: RefCell<super::ui::FrameCache>,
    pub thread_history: Vec<String>,
    pub history_pos: usize,
    running: HashMap<String, Arc<AtomicBool>>,
    osint_cancel: Option<Arc<AtomicBool>>,
    pub recon_provider: String,
    pub recon_model: String,
    pub synthesis_provider: String,
    pub synthesis_model: String,
    pub defaults_synthesis: bool,
    pub model_catalog: Vec<ListedModel>,
    pub catalog_for: String,
    pub choice_items: Vec<ChoiceItem>,
    pub choice_sel: usize,
    pub choice_note: String,
    pub grok_signed_in: bool,
    pub openai_signed_in: bool,
    access_probe: bool,
    access_checked: bool,
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
        let chat_scroll = selected_thread
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
        let mut app = Self {
            module: None,
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
            firecrawl_key: settings.firecrawl_api_key.clone(),
            hunter_key: settings.hunter_api_key.clone(),
            sociavault_key: settings.sociavault_api_key.clone(),
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
            recon_chat: false,
            scrolls: Scrolls {
                chat: chat_scroll,
                ..Scrolls::default()
            },
            expanded: HashSet::new(),
            chat_sel: 0,
            chat_follow: chat_scroll == 0,
            overlay: Overlay::None,
            log: Vec::new(),
            runs: Vec::new(),
            answer_memories: HashMap::new(),
            quit_arm: None,
            esc_arm: None,
            press: None,
            draft_dirty: false,
            frame: RefCell::new(super::ui::FrameCache::default()),
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
            catalog_for: String::new(),
            choice_items: Vec::new(),
            choice_sel: 0,
            choice_note: String::new(),
            grok_signed_in: argos_osint_core::grok_oauth::login_present(),
            openai_signed_in: false,
            access_probe: false,
            access_checked: false,
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
        };
        app.push_log("info", "Argos ready");
        if app.selected_thread.is_some() {
            let _ = app.refresh_selected();
        }
        Ok(app)
    }

    pub fn error_count(&self) -> usize {
        self.log.iter().filter(|line| line.level == "error").count()
    }

    pub fn running_thread(&self, id: &str) -> bool {
        self.running.contains_key(id)
    }

    pub fn insight_summary(&self, memory_id: &str) -> Option<String> {
        let insight = self.store.insight_for_memory(memory_id).ok().flatten()?;
        let mut lines = vec![format!(
            "{} · {} → {} · {} · {:.0}%",
            insight.entity,
            insight.predicate,
            insight.object_value,
            insight.classification,
            insight.confidence * 100.0
        )];
        lines.push(format!(
            "{} evidence link{}",
            insight.sources.len(),
            if insight.sources.len() == 1 { "" } else { "s" }
        ));
        if !insight.related.is_empty() {
            lines.push(format!("Related: {}", insight.related.join(", ")));
        }
        Some(lines.join("\n"))
    }

    fn push_log(&mut self, level: &str, text: impl Into<String>) {
        self.log.push(LogLine {
            at: log_stamp(),
            level: level.into(),
            text: text.into(),
        });
        if self.log.len() > 400 {
            let extra = self.log.len() - 400;
            self.log.drain(0..extra);
        }
    }

    fn go_home(&mut self) {
        self.flush_draft();
        self.overlay = Overlay::None;
        self.module = None;
        self.set_focus(Target::App(self.launcher_sel));
        self.status = "Home".into();
    }

    fn select(&mut self, index: usize) {
        self.flush_draft();
        self.launcher_sel = index;
        self.module = Some(ModuleId::ALL[index]);
        if self.module == Some(ModuleId::Recon) {
            self.recon_chat = false;
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
            Some(ModuleId::Recon) if self.threads.is_empty() => Target::Field(FieldId::ReconSearch),
            Some(ModuleId::Recon) => Target::Thread(self.thread_sel),
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
            FieldId::FirecrawlKey => &self.firecrawl_key,
            FieldId::HunterKey => &self.hunter_key,
            FieldId::SociaVaultKey => &self.sociavault_key,
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
            FieldId::FirecrawlKey => &mut self.firecrawl_key,
            FieldId::HunterKey => &mut self.hunter_key,
            FieldId::SociaVaultKey => &mut self.sociavault_key,
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
        if self.focus == Target::Field(FieldId::Composer)
            && target != Target::Field(FieldId::Composer)
        {
            self.flush_draft();
        }
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

    fn enter_investigation(&mut self, id: &str) -> Result<()> {
        self.open_thread(id)?;
        self.recon_chat = true;
        self.module = Some(ModuleId::Recon);
        self.set_focus(Target::Field(FieldId::Composer));
        Ok(())
    }

    fn open_thread_with_history(&mut self, id: &str, record: bool) -> Result<()> {
        if self.module == Some(ModuleId::Recon) {
            if let Some(previous) = &self.selected_thread {
                self.store
                    .save_draft(previous, &self.input, i64::from(self.scrolls.chat))?;
                self.draft_dirty = false;
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
        self.scrolls.chat = thread.scroll.clamp(0, i64::from(u16::MAX)) as u16;
        self.chat_follow = self.scrolls.chat == 0;
        self.chat_sel = usize::MAX;
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
        self.enter_investigation(&thread.id)?;
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
            .save_draft(&tid, "", i64::from(self.scrolls.chat))?;
        self.chat_follow = true;
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

    fn remember_firecrawl_key(&mut self) -> Result<String> {
        let key = self.firecrawl_key.trim().to_string();
        anyhow::ensure!(!key.is_empty(), "Enter a Firecrawl API key");
        self.settings.firecrawl_api_key = key;
        self.settings.save()?;
        Ok("Firecrawl API key saved".into())
    }

    fn remember_hunter_key(&mut self) -> Result<String> {
        let key = self.hunter_key.trim().to_string();
        anyhow::ensure!(!key.is_empty(), "Enter a Hunter API key");
        self.settings.hunter_api_key = key;
        self.settings.save()?;
        Ok("Hunter API key saved".into())
    }

    fn remember_sociavault_key(&mut self) -> Result<String> {
        let key = self.sociavault_key.trim().to_string();
        anyhow::ensure!(!key.is_empty(), "Enter a SociaVault API key");
        self.settings.sociavault_api_key = key;
        self.settings.save()?;
        Ok("SociaVault API key saved".into())
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
        if tool.id == "firecrawl_search" {
            self.remember_firecrawl_key()?;
        } else if tool.id.starts_with("hunter_") {
            self.remember_hunter_key()?;
        } else if tool.id == "sociavault_profile" {
            self.remember_sociavault_key()?;
        }
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
                self.push_log("info", format!("Recon {stage}"));
                if self.selected_thread.as_deref() == Some(&thread_id) {
                    self.recon_stage = stage;
                    let _ = self.refresh_selected();
                    let _ = self.refresh_threads();
                }
            }
            WorkEvent::ReconDone { thread_id, outcome } => {
                self.running.remove(&thread_id);
                match &outcome {
                    Ok(()) => self.push_log("info", "Recon turn complete"),
                    Err(err) => self.push_log("error", format!("Recon failed: {err}")),
                }
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
                        if value.1.status == "failed" {
                            self.push_log(
                                "error",
                                format!(
                                    "{} failed: {}",
                                    value.1.tool_id,
                                    value.1.error.as_deref().unwrap_or("unknown error")
                                ),
                            );
                        } else {
                            self.push_log(
                                "info",
                                format!("{} {}", value.1.tool_id, value.1.status),
                            );
                        }
                        self.osint_result = Some(value);
                        if let Ok(runs) = self.store.manual_calls() {
                            self.manual_runs = runs;
                            self.manual_run_pos = self.manual_runs.len().saturating_sub(1);
                        }
                    }
                    Err(err) => {
                        self.push_log("error", format!("OSINT failed: {err}"));
                        self.status = err;
                    }
                }
            }
            WorkEvent::CatalogDone {
                synthesis,
                provider,
                outcome,
            } => self.finish_catalog(synthesis, provider, outcome),
            WorkEvent::Access { grok, openai } => {
                if grok {
                    self.grok_signed_in = true;
                }
                if openai {
                    self.openai_signed_in = true;
                }
                self.access_probe = false;
                self.access_checked = true;
                if matches!(self.overlay, Overlay::Choice(ChoiceKind::Provider)) {
                    self.rebuild_provider_choices();
                }
            }
            WorkEvent::InsightDone { thread_id, outcome } => {
                if self.selected_thread.as_deref() == Some(&thread_id) {
                    self.status = match outcome {
                        Ok(()) => "Insights saved".into(),
                        Err(err) => {
                            let text = format!("Insight retry failed: {err}");
                            self.push_log("error", text.clone());
                            text
                        }
                    };
                    if let Ok(memories) = self.store.list_memories() {
                        self.memories = memories;
                    }
                }
            }
        }
    }

    pub fn role_provider(&self) -> String {
        let raw = if self.defaults_synthesis {
            &self.synthesis_provider
        } else {
            &self.recon_provider
        };
        match provider::normalize_kind(raw).as_str() {
            "openai" => "openai-chatgpt".into(),
            "grok-subscription" => "grok".into(),
            other => other.to_string(),
        }
    }

    fn role_model(&self) -> String {
        if self.defaults_synthesis {
            self.synthesis_model.clone()
        } else {
            self.recon_model.clone()
        }
    }

    fn set_role_provider(&mut self, id: &str) {
        if self.defaults_synthesis {
            self.synthesis_provider = id.to_string();
        } else {
            self.recon_provider = id.to_string();
        }
    }

    fn set_role_model(&mut self, id: &str) {
        if self.defaults_synthesis {
            self.synthesis_model = id.to_string();
        } else {
            self.recon_model = id.to_string();
        }
    }

    pub fn field_display(&self, field: FieldId) -> String {
        match field {
            FieldId::ReconProvider | FieldId::SynthesisProvider => {
                let kind = self.field(field);
                if kind.is_empty() {
                    String::new()
                } else {
                    provider_label(kind).to_string()
                }
            }
            _ => self.field(field).to_string(),
        }
    }

    fn refresh_catalog(&mut self) {
        let synthesis = self.defaults_synthesis;
        let provider = self.role_provider();
        if provider.is_empty() {
            self.status = "Choose a provider first".into();
            return;
        }
        if provider == "openai-chatgpt" {
            self.finish_catalog(synthesis, provider, Ok(codex_models()));
            return;
        }
        self.status = "Loading models this account can call".into();
        if matches!(self.overlay, Overlay::Choice(ChoiceKind::Model)) {
            self.choice_note = "Loading models this account can call…".into();
        }
        let secret = provider::account_secret(&self.auth, &provider);
        let tx = self.work_tx.clone();
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        tokio::spawn(async move {
            let outcome = provider::verified_catalog(&secret)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(WorkEvent::CatalogDone {
                synthesis,
                provider,
                outcome,
            });
        });
    }

    fn finish_catalog(
        &mut self,
        synthesis: bool,
        provider: String,
        outcome: Result<Vec<ListedModel>, String>,
    ) {
        if self.defaults_synthesis != synthesis || self.role_provider() != provider {
            return;
        }
        match outcome {
            Ok(models) => {
                let count = models.len();
                self.model_catalog = models;
                self.catalog_for = provider;
                self.status = format!("{count} models this account can call");
                if matches!(self.overlay, Overlay::Choice(ChoiceKind::Model)) {
                    self.choice_note = if count == 0 {
                        "This account returned no models.".into()
                    } else {
                        "Models this account can call.".into()
                    };
                    self.rebuild_model_choices();
                }
            }
            Err(err) => {
                self.push_log("error", format!("Model catalog failed: {err}"));
                self.status = err.clone();
                if matches!(self.overlay, Overlay::Choice(ChoiceKind::Model)) {
                    self.choice_note = err;
                    if self.catalog_for != self.role_provider() {
                        self.choice_items.clear();
                        self.choice_sel = 0;
                    }
                }
            }
        }
    }

    fn open_default_picker(&mut self, field: FieldId) {
        let synthesis = matches!(field, FieldId::SynthesisProvider | FieldId::SynthesisModel);
        if synthesis != self.defaults_synthesis {
            return;
        }
        if matches!(field, FieldId::ReconProvider | FieldId::SynthesisProvider) {
            self.open_provider_picker();
        } else {
            self.open_model_picker();
        }
    }

    fn open_provider_picker(&mut self) {
        self.scrolls.popup = 0;
        self.overlay = Overlay::Choice(ChoiceKind::Provider);
        self.choice_note = "Connected accounts. Sign in on the other tabs to add one.".into();
        self.rebuild_provider_choices();
        self.probe_access();
    }

    fn open_model_picker(&mut self) {
        let provider = self.role_provider();
        if provider.is_empty() {
            self.status = "Choose a provider first".into();
            return;
        }
        self.scrolls.popup = 0;
        self.overlay = Overlay::Choice(ChoiceKind::Model);
        if provider == "openai-chatgpt" {
            self.finish_catalog(self.defaults_synthesis, provider, Ok(codex_models()));
            self.choice_note = "ChatGPT subscription exposes the Codex default.".into();
            self.status = "ChatGPT subscription uses the Codex default".into();
            return;
        }
        if self.catalog_for != provider {
            self.model_catalog.clear();
            self.choice_items.clear();
            self.choice_sel = 0;
            self.choice_note = "Loading models this account can call…".into();
        } else {
            self.choice_note = "Models this account can call.".into();
            self.rebuild_model_choices();
        }
        self.refresh_catalog();
    }

    fn rebuild_provider_choices(&mut self) {
        let mut items = Vec::new();
        if self.grok_signed_in {
            items.push(ChoiceItem {
                id: "grok".into(),
                label: "Grok · subscription models".into(),
            });
        }
        if self.openai_signed_in {
            items.push(ChoiceItem {
                id: "openai-chatgpt".into(),
                label: "OpenAI · ChatGPT subscription".into(),
            });
        }
        if openrouter_ready(&self.auth) {
            items.push(ChoiceItem {
                id: "openrouter".into(),
                label: "OpenRouter · models this key can call".into(),
            });
        }
        items.push(ChoiceItem {
            id: "local".into(),
            label: "Local · models on this machine".into(),
        });
        let current = self.role_provider();
        self.set_choices(items, &current);
    }

    fn rebuild_model_choices(&mut self) {
        let current = self.role_model();
        let items = self
            .model_catalog
            .iter()
            .map(|model| ChoiceItem {
                id: model.id.clone(),
                label: model_label(model),
            })
            .collect();
        self.set_choices(items, &current);
    }

    fn set_choices(&mut self, items: Vec<ChoiceItem>, current: &str) {
        let sel = items
            .iter()
            .position(|item| item.id == current)
            .unwrap_or(0);
        self.choice_items = items;
        self.choice_sel = if self.choice_items.is_empty() {
            0
        } else {
            sel.min(self.choice_items.len() - 1)
        };
        let room = super::ui::choice_list_room(self).max(1);
        super::ui::reveal_index(&mut self.scrolls.popup, self.choice_sel, room);
    }

    pub fn move_choice(&mut self, delta: i32) {
        if self.choice_items.is_empty() {
            return;
        }
        let last = self.choice_items.len() as i32 - 1;
        self.choice_sel = (self.choice_sel as i32 + delta).clamp(0, last) as usize;
        let room = super::ui::choice_list_room(self).max(1);
        super::ui::reveal_index(&mut self.scrolls.popup, self.choice_sel, room);
    }

    fn apply_choice(&mut self, index: usize) {
        let Some(item) = self.choice_items.get(index).cloned() else {
            return;
        };
        match self.overlay {
            Overlay::Choice(ChoiceKind::Provider) => {
                let changed = self.role_provider() != item.id;
                self.set_role_provider(&item.id);
                if changed {
                    self.set_role_model("");
                    self.model_catalog.clear();
                    self.catalog_for.clear();
                }
                self.status = format!("Provider {}", provider_label(&item.id));
            }
            Overlay::Choice(ChoiceKind::Model) => {
                self.set_role_model(&item.id);
                self.status = format!("Model {}", item.id);
            }
            _ => return,
        }
        self.overlay = Overlay::None;
        self.scrolls.popup = 0;
    }

    fn probe_access(&mut self) {
        if self.access_checked || self.access_probe {
            return;
        }
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        self.access_probe = true;
        let tx = self.work_tx.clone();
        tokio::spawn(async move {
            let grok = argos_osint_core::grok_oauth::check_login().await.is_ok();
            let openai = argos_osint_core::subscription::check_login().await.is_ok();
            let _ = tx.send(WorkEvent::Access { grok, openai });
        });
    }

    fn refresh_selected(&mut self) -> Result<()> {
        if let Some(tid) = &self.selected_thread {
            self.messages = self.store.list_messages(tid)?;
            self.calls = self.store.all_calls_for_thread(tid)?;
            self.runs = self.store.runs_for_thread(tid)?;
            self.answer_memories = self.store.answer_memories(tid)?;
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
                    self.recon_chat = false;
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
            ButtonId::OsintPrev => self.shift_manual(false),
            ButtonId::OsintNext => self.shift_manual(true),
            ButtonId::SaveFirecrawlKey => self.remember_firecrawl_key(),
            ButtonId::SaveHunterKey => self.remember_hunter_key(),
            ButtonId::SaveSociaVaultKey => self.remember_sociavault_key(),
            ButtonId::OpenSource => self
                .open_insight_source()
                .map(|_| "Source thread opened".into()),
            ButtonId::ClearLog => {
                self.log.clear();
                self.push_log("info", "Event log cleared");
                Ok("Event log cleared".into())
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
                    self.enter_investigation(&thread.id)?;
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
                self.catalog_for.clear();
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
        self.report(result);
    }

    fn report(&mut self, result: Result<String>) {
        self.status = match result {
            Ok(message) => message,
            Err(err) => {
                let text = err.to_string();
                self.push_log("error", text.clone());
                text
            }
        };
    }

    fn shift_manual(&mut self, forward: bool) -> Result<String> {
        let tool = osint::registry()
            .get(self.tool_sel)
            .ok_or_else(|| anyhow::anyhow!("No tool selected"))?;
        let positions: Vec<_> = self
            .manual_runs
            .iter()
            .enumerate()
            .filter(|(_, call)| call.tool_id == tool.id && call.result.is_some())
            .map(|(index, _)| index)
            .collect();
        anyhow::ensure!(!positions.is_empty(), "No previous runs for this tool");
        let current = positions
            .iter()
            .position(|pos| *pos == self.manual_run_pos)
            .unwrap_or(positions.len() - 1);
        let next = if forward {
            (current + 1).min(positions.len() - 1)
        } else {
            current.saturating_sub(1)
        };
        self.manual_run_pos = positions[next];
        let call = &self.manual_runs[self.manual_run_pos];
        self.osint_result = call.result.clone().map(|result| (call.id.clone(), result));
        Ok(format!("Manual result {} of {}", next + 1, positions.len()))
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
                let lower = message.to_ascii_lowercase();
                match page {
                    ProviderPage::Grok => {
                        self.grok_status = message.clone();
                        if lower.contains("ready")
                            || lower.contains("connected")
                            || lower.contains("signed in")
                        {
                            self.grok_signed_in = true;
                        } else if lower.contains("sign-in required") {
                            self.grok_signed_in = false;
                        }
                    }
                    ProviderPage::OpenAI => {
                        self.openai_status = message.clone();
                        if lower.contains("signed in") {
                            self.openai_signed_in = true;
                        } else if lower.contains("sign-in required") {
                            self.openai_signed_in = false;
                        }
                    }
                    ProviderPage::OpenRouter => self.router_status = message.clone(),
                    ProviderPage::Defaults => {}
                }
                if message.to_ascii_lowercase().contains("fail")
                    || message.to_ascii_lowercase().contains("error")
                {
                    self.push_log("error", message.clone());
                } else {
                    self.push_log("info", message.clone());
                }
                self.status = message;
            }
        }
    }

    fn activate_target(&mut self, target: Target) {
        match target {
            Target::App(index) => self.select(index),
            Target::Home => self.go_home(),
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
                        .enter_investigation(&id)
                        .map(|_| "Investigation opened".into())
                        .unwrap_or_else(|e| e.to_string());
                }
            }
            Target::Tool(index) => self.select_tool(index),
            Target::Field(field) if is_picker_field(field) => {
                self.set_focus(target);
                self.open_default_picker(field);
            }
            Target::Field(_) => self.set_focus(target),
            Target::Button(button) => {
                self.set_focus(target);
                self.activate_button(button);
            }
            Target::Transcript => self.set_focus(Target::Transcript),
            Target::ChatHeader(index) => {
                self.chat_sel = index;
                self.chat_follow = false;
                self.set_focus(Target::Transcript);
                super::ui::toggle_chat(self);
            }
            Target::ChatBody(index) => {
                self.chat_sel = index;
                self.chat_follow = false;
                self.set_focus(Target::Transcript);
            }
            Target::BrainMark(index) => {
                self.chat_sel = index;
                self.set_focus(Target::Transcript);
                super::ui::open_memory(self, index);
            }
            Target::Choice(index) => self.apply_choice(index),
            Target::CloseOverlay => {
                self.overlay = Overlay::None;
                self.scrolls.popup = 0;
            }
        }
    }

    fn edit_char(&mut self, character: char) {
        let Target::Field(field) = self.focus else {
            return;
        };
        if is_picker_field(field) {
            return;
        }
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
        if is_picker_field(field) {
            return;
        }
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
        if is_picker_field(field) {
            return;
        }
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
        if self.module != Some(ModuleId::Recon) {
            return;
        }
        let input = self.input.trim().to_string();
        if input.is_empty() {
            return;
        }
        let result = self.recon_command(&input);
        self.report(result);
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
            self.recon_chat = false;
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
        self.enter_investigation(&id)?;
        Ok(())
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C')) {
            return self.on_interrupt();
        }
        if ctrl && matches!(key.code, KeyCode::Char('q') | KeyCode::Char('Q')) {
            return self.arm_quit();
        }
        if let Overlay::Choice(_) = self.overlay {
            if ctrl && matches!(key.code, KeyCode::Char('u') | KeyCode::Char('d')) {
                let room = super::ui::choice_list_room(self).max(1) as i32;
                self.move_choice(if key.code == KeyCode::Char('d') {
                    room
                } else {
                    -room
                });
                return true;
            }
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.activate_target(Target::CloseOverlay),
                KeyCode::Up | KeyCode::Char('k') => self.move_choice(-1),
                KeyCode::Down | KeyCode::Char('j') => self.move_choice(1),
                KeyCode::Enter => self.apply_choice(self.choice_sel),
                _ => {}
            }
            return true;
        }
        if self.overlay != Overlay::None {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.activate_target(Target::CloseOverlay),
                KeyCode::Up => {
                    self.scrolls.popup = self.scrolls.popup.saturating_sub(1);
                }
                KeyCode::Down => self.scrolls.popup = self.scrolls.popup.saturating_add(1),
                _ => {}
            }
            if ctrl && matches!(key.code, KeyCode::Char('u') | KeyCode::Char('d')) {
                super::ui::page(
                    self,
                    if key.code == KeyCode::Char('d') {
                        1
                    } else {
                        -1
                    },
                );
            }
            return true;
        }
        if ctrl && matches!(key.code, KeyCode::Char('u') | KeyCode::Char('d')) {
            let down = key.code == KeyCode::Char('d');
            if self.field_focused() {
                if !down {
                    if let Target::Field(field) = self.focus {
                        if !is_picker_field(field) {
                            self.field_mut(field).clear();
                            self.cursor = 0;
                            self.persist_draft();
                        }
                    }
                }
            } else {
                super::ui::page(self, if down { 1 } else { -1 });
            }
            return true;
        }
        if ctrl && matches!(key.code, KeyCode::Char('\\') | KeyCode::Char('\u{1c}')) {
            self.go_home();
            return true;
        }
        if ctrl && matches!(key.code, KeyCode::Char('n') | KeyCode::Char('N')) {
            let created = self.new_thread().map(|_| "New investigation".into());
            self.report(created);
            self.module = Some(ModuleId::Recon);
            self.launcher_sel = 0;
            return true;
        }
        if ctrl && key.code == KeyCode::Char('o') && self.module == Some(ModuleId::Brain) {
            let opened = self
                .open_insight_source()
                .map(|_| "Source thread opened".into());
            self.report(opened);
            return true;
        }
        if key.modifiers.contains(KeyModifiers::ALT)
            && matches!(key.code, KeyCode::Left | KeyCode::Right)
        {
            if self.field_focused() {
                self.move_word(if key.code == KeyCode::Left { -1 } else { 1 });
            } else if self.module == Some(ModuleId::Recon) {
                let next = if key.code == KeyCode::Left {
                    self.history_pos.saturating_sub(1)
                } else {
                    (self.history_pos + 1).min(self.thread_history.len().saturating_sub(1))
                };
                if let Some(id) = self.thread_history.get(next).cloned() {
                    self.history_pos = next;
                    let _ = self.open_thread_with_history(&id, false);
                }
            }
            return true;
        }
        if key.modifiers.contains(KeyModifiers::SHIFT)
            && key.code == KeyCode::Enter
            && self.focus == Target::Field(FieldId::Composer)
        {
            self.edit_char('\n');
            self.persist_draft();
            return true;
        }
        if ctrl && key.code == KeyCode::Char('a') {
            if let Target::Field(field) = self.focus {
                if !is_picker_field(field) {
                    self.field_mut(field).clear();
                    self.cursor = 0;
                }
            }
            return true;
        }
        if matches!(key.code, KeyCode::Char('?')) && !self.field_focused() {
            self.overlay = if self.overlay == Overlay::Help {
                Overlay::None
            } else {
                Overlay::Help
            };
            self.scrolls.popup = 0;
            return true;
        }
        match key.code {
            KeyCode::Esc => self.on_esc(),
            KeyCode::Enter => self.on_enter(),
            KeyCode::Backspace => self.edit_backspace(),
            KeyCode::Delete => self.edit_delete(),
            KeyCode::Left if self.field_focused() => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right if self.field_focused() => {
                if let Target::Field(field) = self.focus {
                    self.cursor = (self.cursor + 1).min(self.field(field).chars().count());
                }
            }
            KeyCode::Left | KeyCode::Char('h') if self.transcript_focused() => {
                super::ui::fold_chat(self, false);
            }
            KeyCode::Right | KeyCode::Char('l') if self.transcript_focused() => {
                super::ui::fold_chat(self, true);
            }
            KeyCode::Char('e') if self.transcript_focused() => super::ui::toggle_chat(self),
            KeyCode::Char('f') if self.transcript_focused() => {
                super::ui::open_block(self, self.chat_sel);
            }
            KeyCode::Char(' ') if self.transcript_focused() => {
                self.set_focus(Target::Field(FieldId::Composer));
            }
            KeyCode::Home if self.field_focused() => self.cursor = 0,
            KeyCode::End if self.field_focused() => {
                if let Target::Field(field) = self.focus {
                    self.cursor = self.field(field).chars().count();
                }
            }
            KeyCode::Home if self.transcript_focused() => super::ui::move_chat(self, -10_000),
            KeyCode::End if self.transcript_focused() => {
                self.chat_follow = true;
                super::ui::normalize(self);
            }
            KeyCode::Char(c) if self.field_focused() => self.edit_char(c),
            KeyCode::Char(c)
                if self.module.is_none()
                    && key.modifiers.is_empty()
                    && matches!(c, '1' | '2' | '3' | '4' | '5') =>
            {
                self.select((c as u8 - b'1') as usize);
            }
            KeyCode::Char(c) if self.module.is_none() && matches!(c, 'j' | 'k') => {
                self.move_vertical(if c == 'j' { 1 } else { -1 });
            }
            KeyCode::Char(c)
                if !self.field_focused()
                    && self.module != Some(ModuleId::Recon)
                    && matches!(c, 'j' | 'k') =>
            {
                self.move_vertical(if c == 'j' { 1 } else { -1 });
            }
            KeyCode::Char(c)
                if self.module == Some(ModuleId::Recon)
                    && !self.field_focused()
                    && !c.is_control() =>
            {
                self.set_focus(if self.recon_chat {
                    Target::Field(FieldId::Composer)
                } else {
                    Target::Field(FieldId::ReconSearch)
                });
                self.edit_char(c);
            }
            KeyCode::Up => self.move_vertical(-1),
            KeyCode::Down => self.move_vertical(1),
            KeyCode::Tab => self.focus_next(key.modifiers.contains(KeyModifiers::SHIFT)),
            _ => {}
        }
        if matches!(key.code, KeyCode::Char(_))
            && self.module == Some(ModuleId::Recon)
            && self.focus == Target::Field(FieldId::ReconSearch)
        {
            let _ = self.refresh_threads();
        }
        if matches!(key.code, KeyCode::Char(_))
            && self.module == Some(ModuleId::Osint)
            && self.focus == Target::Field(FieldId::OsintSearch)
        {
            if let Some(id) = self.filtered_tool_ids().first().copied() {
                self.select_tool(id);
                self.set_focus(Target::Field(FieldId::OsintSearch));
            }
        }
        self.persist_draft();
        super::ui::normalize(self);
        true
    }

    fn on_interrupt(&mut self) -> bool {
        if self.overlay != Overlay::None {
            self.activate_target(Target::CloseOverlay);
            return true;
        }
        if let Target::Field(field) = self.focus {
            if !self.field(field).is_empty() {
                self.field_mut(field).clear();
                self.cursor = 0;
                self.status = "Cleared".into();
                self.persist_draft();
                return true;
            }
        }
        if self.module == Some(ModuleId::Recon) {
            if let Some(id) = self.selected_thread.clone() {
                if let Some(cancel) = self.running.get(&id) {
                    cancel.store(true, Ordering::Relaxed);
                    self.status = "Cancellation requested".into();
                    self.push_log("info", "Recon cancellation requested");
                    return true;
                }
            }
        }
        self.arm_quit()
    }

    fn arm_quit(&mut self) -> bool {
        let now = Instant::now();
        if self
            .quit_arm
            .is_some_and(|armed| now.duration_since(armed) < Duration::from_secs(1))
        {
            return false;
        }
        self.quit_arm = Some(now);
        self.status = "Press Ctrl+C or Ctrl+Q again to quit".into();
        true
    }

    fn on_esc(&mut self) {
        if self.focus == Target::Field(FieldId::Composer) && !self.input.trim().is_empty() {
            let now = Instant::now();
            if self
                .esc_arm
                .is_some_and(|armed| now.duration_since(armed) < Duration::from_millis(800))
            {
                self.input.clear();
                self.cursor = 0;
                self.esc_arm = None;
                self.status = "Draft cleared".into();
                self.persist_draft();
            } else {
                self.esc_arm = Some(now);
                self.status = "Press Esc again to clear the draft".into();
            }
            return;
        }
        if self.module == Some(ModuleId::Recon) && self.recon_chat {
            self.recon_chat = false;
            self.set_focus(if self.threads.is_empty() {
                Target::Field(FieldId::ReconSearch)
            } else {
                Target::Thread(self.thread_sel)
            });
            self.status = "Investigations".into();
            return;
        }
        if self.module.is_some() {
            self.go_home();
        }
    }

    fn on_enter(&mut self) {
        match self.focus {
            Target::Field(FieldId::Composer) => self.submit(),
            Target::Field(field) if is_picker_field(field) => self.open_default_picker(field),
            Target::Field(_) => self.focus_next(false),
            Target::Transcript => self.enter_chat(),
            target => self.activate_target(target),
        }
    }

    fn enter_chat(&mut self) {
        let blocks = super::ui::chat_blocks(self);
        let Some(block) = blocks.get(self.chat_sel) else {
            return;
        };
        if block.collapsible {
            super::ui::toggle_chat(self);
        } else if block.has_memory {
            super::ui::open_memory(self, self.chat_sel);
        } else {
            super::ui::open_block(self, self.chat_sel);
        }
    }

    fn field_focused(&self) -> bool {
        matches!(self.focus, Target::Field(_))
    }

    fn transcript_focused(&self) -> bool {
        matches!(
            self.focus,
            Target::Transcript | Target::ChatHeader(_) | Target::ChatBody(_)
        )
    }

    fn move_vertical(&mut self, delta: i32) {
        match self.focus {
            Target::Field(FieldId::Composer) => self.move_composer_line(delta),
            Target::Field(FieldId::ReconSearch) | Target::Thread(_) => self.move_thread(delta),
            Target::Field(_) => {}
            Target::Transcript | Target::ChatHeader(_) | Target::ChatBody(_) => {
                super::ui::move_chat(self, delta);
            }
            Target::Memory(_) => self.move_memory(delta),
            Target::Tool(_) => self.move_tool(delta),
            _ => match self.module {
                None => self.move_home(delta),
                Some(ModuleId::Brain) => self.move_memory(delta),
                Some(ModuleId::Osint) => self.move_tool(delta),
                Some(ModuleId::System) => {
                    self.scrolls.log = add_scroll(self.scrolls.log, delta);
                }
                Some(ModuleId::Recon) if self.recon_chat => super::ui::move_chat(self, delta),
                Some(ModuleId::Recon) => self.move_thread(delta),
                Some(ModuleId::Providers) => {
                    self.scrolls.detail = add_scroll(self.scrolls.detail, delta * 3);
                }
            },
        }
    }

    fn move_home(&mut self, delta: i32) {
        let next = (self.launcher_sel as i32 + delta).clamp(0, ModuleId::ALL.len() as i32 - 1);
        self.launcher_sel = next as usize;
        self.set_focus(Target::App(self.launcher_sel));
    }

    fn move_thread(&mut self, delta: i32) {
        if self.threads.is_empty() {
            return;
        }
        let next =
            (self.thread_sel as i32 + delta).clamp(0, self.threads.len() as i32 - 1) as usize;
        self.thread_sel = next;
        let room = super::ui::thread_room_for(self);
        super::ui::reveal_index(&mut self.scrolls.threads, next, room);
        self.set_focus(Target::Thread(next));
    }

    fn move_memory(&mut self, delta: i32) {
        if self.memories.is_empty() {
            return;
        }
        let next =
            (self.memory_sel as i32 + delta).clamp(0, self.memories.len() as i32 - 1) as usize;
        self.memory_sel = next;
        self.selected_insight = self
            .memories
            .get(next)
            .and_then(|memory| self.store.insight_for_memory(&memory.id).ok().flatten());
        let room = super::ui::memory_room_for(self);
        super::ui::reveal_index(&mut self.scrolls.memories, next, room);
        self.set_focus(Target::Memory(next));
    }

    fn move_tool(&mut self, delta: i32) {
        let ids = self.filtered_tool_ids();
        if ids.is_empty() {
            return;
        }
        let pos = ids.iter().position(|id| *id == self.tool_sel).unwrap_or(0) as i32;
        let next = (pos + delta).clamp(0, ids.len() as i32 - 1) as usize;
        self.select_tool(ids[next]);
        let room = super::ui::tool_room_for(self);
        super::ui::reveal_index(&mut self.scrolls.tools, next, room);
    }

    fn move_composer_line(&mut self, delta: i32) {
        let value = self.input.clone();
        if !value.contains('\n') {
            return;
        }
        let chars: Vec<char> = value.chars().collect();
        let cursor = self.cursor.min(chars.len());
        let mut lines = vec![0usize];
        for (index, ch) in chars.iter().enumerate() {
            if *ch == '\n' {
                lines.push(index + 1);
            }
        }
        let current = lines
            .iter()
            .rposition(|start| *start <= cursor)
            .unwrap_or(0);
        let col = cursor - lines[current];
        let next = current as i32 + delta;
        if next < 0 || next as usize >= lines.len() {
            return;
        }
        let start = lines[next as usize];
        let end = if next as usize + 1 < lines.len() {
            lines[next as usize + 1] - 1
        } else {
            chars.len()
        };
        self.cursor = start + col.min(end.saturating_sub(start));
    }

    fn move_word(&mut self, delta: i32) {
        let Target::Field(field) = self.focus else {
            return;
        };
        let chars: Vec<char> = self.field(field).chars().collect();
        let mut cursor = self.cursor.min(chars.len());
        if delta < 0 {
            while cursor > 0 && chars[cursor - 1].is_whitespace() {
                cursor -= 1;
            }
            while cursor > 0 && !chars[cursor - 1].is_whitespace() {
                cursor -= 1;
            }
        } else {
            while cursor < chars.len() && !chars[cursor].is_whitespace() {
                cursor += 1;
            }
            while cursor < chars.len() && chars[cursor].is_whitespace() {
                cursor += 1;
            }
        }
        self.cursor = cursor;
    }

    fn persist_draft(&mut self) {
        if self.module == Some(ModuleId::Recon) && self.focus == Target::Field(FieldId::Composer) {
            self.draft_dirty = true;
        }
    }

    fn flush_draft(&mut self) {
        if !self.draft_dirty {
            return;
        }
        self.draft_dirty = false;
        if let Some(id) = &self.selected_thread {
            let _ = self
                .store
                .save_draft(id, &self.input, i64::from(self.scrolls.chat));
        }
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
                let target = super::ui::hit_test(self, mouse.column, mouse.row);
                self.press = Some((mouse.column, mouse.row, target));
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some((x, y, target)) = self.press.as_mut() {
                    if mouse.column.abs_diff(*x) > 1 || mouse.row.abs_diff(*y) > 1 {
                        *target = None;
                    }
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                let Some((x, y, target)) = self.press.take() else {
                    return;
                };
                if mouse.column.abs_diff(x) > 1 || mouse.row.abs_diff(y) > 1 {
                    return;
                }
                if let Some(target) = target {
                    self.activate_target(target);
                    if let Target::Field(field) = self.focus {
                        self.cursor = super::ui::cursor_at(self, field, x);
                    }
                }
            }
            MouseEventKind::ScrollDown => super::ui::scroll_at(self, mouse.column, mouse.row, 1),
            MouseEventKind::ScrollUp => super::ui::scroll_at(self, mouse.column, mouse.row, -1),
            _ => {}
        }
    }
}

pub fn is_picker_field(field: FieldId) -> bool {
    matches!(
        field,
        FieldId::ReconProvider
            | FieldId::ReconModel
            | FieldId::SynthesisProvider
            | FieldId::SynthesisModel
    )
}

fn provider_label(kind: &str) -> &'static str {
    match provider::normalize_kind(kind).as_str() {
        "grok" | "grok-subscription" => "Grok",
        "openai" | "openai-chatgpt" => "OpenAI",
        "openrouter" => "OpenRouter",
        "local" => "Local",
        _ => "Provider",
    }
}

fn model_label(model: &ListedModel) -> String {
    let name = model.name.trim();
    if name.is_empty() || name.eq_ignore_ascii_case(&model.id) {
        model.id.clone()
    } else {
        format!("{name} · {}", model.id)
    }
}

fn codex_models() -> Vec<ListedModel> {
    vec![ListedModel {
        id: "codex-default".into(),
        name: "Codex default".into(),
        free: false,
    }]
}

fn openrouter_ready(auth: &AuthFile) -> bool {
    provider::resolved_key(&provider::account_secret(auth, "openrouter")).is_some()
}

fn add_scroll(value: u16, delta: i32) -> u16 {
    (i32::from(value) + delta).clamp(0, i32::from(u16::MAX)) as u16
}

fn log_stamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    format!(
        "{:02}:{:02}:{:02}Z",
        (secs / 3600) % 24,
        (secs / 60) % 60,
        secs % 60
    )
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
    let mut dirty = true;
    loop {
        if pump(&mut app) {
            dirty = true;
        }
        if dirty {
            super::ui::normalize(&mut app);
            terminal.draw(|frame| {
                app.screen = frame.area();
                super::ui::draw(frame, &app)
            })?;
            dirty = false;
        }
        let busy =
            !app.running.is_empty() || app.osint_cancel.is_some() || app.provider_pending.is_some();
        let wait = Duration::from_millis(if busy { 80 } else { 400 });
        if !event::poll(wait)? {
            if app.draft_dirty {
                app.flush_draft();
            }
            continue;
        }
        loop {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if !app.handle_key(key) {
                        app.flush_draft();
                        return Ok(());
                    }
                    dirty = true;
                }
                Event::Mouse(mouse) => {
                    app.handle_mouse(mouse);
                    dirty |= mouse_dirties(mouse.kind);
                }
                Event::Resize(_, _) => dirty = true,
                _ => {}
            }
            if !event::poll(Duration::ZERO)? {
                break;
            }
        }
    }
}

fn pump(app: &mut App) -> bool {
    let mut dirty = false;
    while let Ok(message) = app.provider_rx.try_recv() {
        app.on_provider_event(message);
        dirty = true;
    }
    while let Ok(message) = app.work_rx.try_recv() {
        app.on_work_event(message);
        dirty = true;
    }
    dirty
}

fn mouse_dirties(kind: MouseEventKind) -> bool {
    matches!(
        kind,
        MouseEventKind::Up(MouseButton::Left)
            | MouseEventKind::ScrollUp
            | MouseEventKind::ScrollDown
            | MouseEventKind::ScrollLeft
            | MouseEventKind::ScrollRight
    )
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
            firecrawl_key: String::new(),
            hunter_key: String::new(),
            sociavault_key: String::new(),
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
            recon_chat: false,
            scrolls: Scrolls::default(),
            expanded: HashSet::new(),
            chat_sel: 0,
            chat_follow: true,
            overlay: Overlay::None,
            log: Vec::new(),
            runs: Vec::new(),
            answer_memories: HashMap::new(),
            quit_arm: None,
            esc_arm: None,
            press: None,
            draft_dirty: false,
            frame: RefCell::new(super::super::ui::FrameCache::default()),
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
            catalog_for: String::new(),
            choice_items: Vec::new(),
            choice_sel: 0,
            choice_note: String::new(),
            grok_signed_in: false,
            openai_signed_in: false,
            access_probe: false,
            access_checked: false,
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
        let down = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: position.0,
            row: position.1,
            modifiers: KeyModifiers::NONE,
        };
        app.handle_mouse(down);
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            ..down
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
    fn defaults_pick_provider_and_model_from_account_access() {
        let mut app = app();
        app.grok_signed_in = true;
        app.openai_signed_in = true;
        let mut router = provider::account_secret(&app.auth, "openrouter");
        router.api_key = Some("router-key".into());
        app.auth.set_account(router);
        click(&mut app, Target::App(3));
        click(&mut app, Target::ProviderTab(ProviderPage::Defaults));
        click(&mut app, Target::Field(FieldId::ReconProvider));
        assert!(matches!(app.overlay, Overlay::Choice(ChoiceKind::Provider)));
        let ids: Vec<_> = app
            .choice_items
            .iter()
            .map(|item| item.id.as_str())
            .collect();
        assert_eq!(ids, ["grok", "openai-chatgpt", "openrouter", "local"]);
        let openrouter = ids.iter().position(|id| *id == "openrouter").unwrap();
        click(&mut app, Target::Choice(openrouter));
        assert_eq!(app.recon_provider, "openrouter");
        assert!(app.recon_model.is_empty());
        assert!(app.model_catalog.is_empty());
        assert!(matches!(app.overlay, Overlay::None));

        click(&mut app, Target::Field(FieldId::ReconProvider));
        type_text(&mut app, "nope");
        assert_eq!(app.recon_provider, "openrouter");
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        type_text(&mut app, "still-nope");
        assert_eq!(app.recon_provider, "openrouter");

        app.model_catalog = vec![ListedModel {
            id: "grok-4.6".into(),
            name: "Grok 4.6".into(),
            free: false,
        }];
        app.catalog_for = "grok".into();
        app.on_work_event(WorkEvent::CatalogDone {
            synthesis: false,
            provider: "openrouter".into(),
            outcome: Ok(vec![
                ListedModel {
                    id: "alpha".into(),
                    name: "Alpha".into(),
                    free: false,
                },
                ListedModel {
                    id: "beta".into(),
                    name: "Beta".into(),
                    free: true,
                },
            ]),
        });
        assert_eq!(app.catalog_for, "openrouter");
        click(&mut app, Target::Field(FieldId::ReconModel));
        assert!(matches!(app.overlay, Overlay::Choice(ChoiceKind::Model)));
        let models: Vec<_> = app
            .choice_items
            .iter()
            .map(|item| item.id.as_str())
            .collect();
        assert_eq!(models, ["alpha", "beta"]);
        click(&mut app, Target::Choice(1));
        assert_eq!(app.recon_model, "beta");
        assert!(matches!(app.overlay, Overlay::None));

        click(&mut app, Target::Button(ButtonId::ToggleDefaultRole));
        click(&mut app, Target::Field(FieldId::SynthesisProvider));
        assert!(matches!(app.overlay, Overlay::Choice(ChoiceKind::Provider)));
        assert_eq!(app.choice_items[0].id, "grok");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.synthesis_provider, "grok");
        assert!(app.synthesis_model.is_empty());
        assert_eq!(app.recon_provider, "openrouter");
        assert_eq!(app.recon_model, "beta");

        click(&mut app, Target::Field(FieldId::SynthesisProvider));
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.synthesis_provider, "openai-chatgpt");
        click(&mut app, Target::Field(FieldId::SynthesisModel));
        assert!(matches!(app.overlay, Overlay::Choice(ChoiceKind::Model)));
        assert_eq!(
            app.choice_items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["codex-default"]
        );
        click(&mut app, Target::Choice(0));
        assert_eq!(app.synthesis_model, "codex-default");
        app.on_work_event(WorkEvent::CatalogDone {
            synthesis: true,
            provider: "grok".into(),
            outcome: Ok(vec![ListedModel {
                id: "not-allowed".into(),
                name: "Not allowed".into(),
                free: false,
            }]),
        });
        assert_eq!(app.catalog_for, "openai-chatgpt");
        assert_eq!(app.model_catalog[0].id, "codex-default");
        assert_eq!(app.recon_model, "beta");

        app.grok_signed_in = false;
        app.openai_signed_in = false;
        app.auth = AuthFile::default();
        click(&mut app, Target::Field(FieldId::SynthesisProvider));
        let available: Vec<_> = app
            .choice_items
            .iter()
            .map(|item| item.id.as_str())
            .collect();
        assert!(available.contains(&"local"));
        assert!(!available.contains(&"grok"));
        assert!(!available.contains(&"openai-chatgpt"));
        assert_eq!(
            available.contains(&"openrouter"),
            openrouter_ready(&app.auth)
        );
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
            Target::Field(FieldId::ReconSearch),
            Target::Button(ButtonId::NewThread),
            Target::Button(ButtonId::DeleteThread),
        ] {
            assert!(
                (0..24)
                    .flat_map(|y| (0..80).map(move |x| (x, y)))
                    .any(|(x, y)| super::super::ui::hit_test(&app, x, y) == Some(target)),
                "dashboard is missing {target:?}"
            );
        }
        assert!(!hit(&app, Target::Button(ButtonId::Send)));
        app.recon_chat = true;
        terminal.draw(|f| super::super::ui::draw(f, &app)).unwrap();
        for target in [
            Target::Button(ButtonId::CancelRun),
            Target::Button(ButtonId::ResumeRun),
            Target::Button(ButtonId::RetryInsights),
            Target::Button(ButtonId::Send),
        ] {
            assert!(
                (0..24)
                    .flat_map(|y| (0..80).map(move |x| (x, y)))
                    .any(|(x, y)| super::super::ui::hit_test(&app, x, y) == Some(target)),
                "chat is missing {target:?}"
            );
        }
        assert!(!hit(&app, Target::Button(ButtonId::NewThread)));
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

    #[test]
    fn home_offers_recon_and_brain_and_only_recon_has_chat() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 80, 24);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();
        for target in [Target::App(0), Target::App(1), Target::App(4)] {
            assert!(hit(&app, target), "home is missing {target:?}");
        }
        assert!(!hit(&app, Target::Field(FieldId::Composer)));
        app.select(1);
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();
        assert!(!hit(&app, Target::Field(FieldId::Composer)));
        assert!(!super::super::ui::focus_order(&app).contains(&Target::Field(FieldId::Composer)));
        app.select(0);
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();
        assert!(hit(&app, Target::Field(FieldId::ReconSearch)));
        assert!(hit(&app, Target::Button(ButtonId::NewThread)));
        assert!(!hit(&app, Target::Field(FieldId::Composer)));
        app.recon_chat = true;
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();
        assert!(hit(&app, Target::Field(FieldId::Composer)));
        assert!(hit(&app, Target::Button(ButtonId::Send)));
        assert!(!hit(&app, Target::Button(ButtonId::NewThread)));
    }

    #[test]
    fn recon_chat_folds_decisions_and_opens_synthesis_memory() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 100, 36);
        let thread = app.store.new_thread("example.org").unwrap();
        let user = app
            .store
            .add_message(&thread.id, "user", "What is known about example.org?", None)
            .unwrap();
        let run = app
            .store
            .new_run(&thread.id, &user.id, "grok / recon", "grok / synthesis")
            .unwrap();
        let tool = osint::registry()[0].id;
        let plan = recon::Plan {
            objective: "Resolve example.org".into(),
            calls: vec![recon::PlanCall {
                step_id: "a".into(),
                tool_id: tool.into(),
                arguments: serde_json::json!({"domain": "example.org"}),
                depends_on: Vec::new(),
                reason: "Need the current public record".into(),
            }],
            unresolved_inputs: Vec::new(),
            stop_condition: "A current observation is in hand".into(),
            planning_mode: "json".into(),
        };
        app.store
            .set_run(&run.id, "running", "synthesizing", Some(&plan), None)
            .unwrap();
        app.store
            .queue_call(
                tool,
                &serde_json::json!({"domain": "example.org"}),
                "recon",
                Some(&run.id),
                Some(&thread.id),
                Some(&user.id),
            )
            .unwrap();
        let memory = app
            .store
            .add_memory(
                "example.org previously resolved to a mail host",
                "investigation",
                true,
                MemorySource {
                    app: "recon".into(),
                    conversation_id: thread.id.clone(),
                    message_id: None,
                    reference: None,
                },
            )
            .unwrap();
        app.store
            .add_answer(
                &thread.id,
                &run.id,
                "The saved memory still names a mail host.",
                &[],
                std::slice::from_ref(&memory.id),
            )
            .unwrap();
        app.selected_thread = Some(thread.id);
        app.module = Some(ModuleId::Recon);
        app.recon_chat = true;
        app.refresh_selected().unwrap();
        app.chat_follow = false;
        app.scrolls.chat = 0;
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 36)).unwrap();
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();
        let drawn = screen_text(&terminal);
        assert!(
            drawn.contains('❯'),
            "user prompt should keep the prompt arrow"
        );
        assert!(
            !drawn.contains("You ·"),
            "user turns should not use a You header"
        );
        assert!(
            !drawn.contains("Synthesis ·"),
            "answers should not use a Synthesis header"
        );
        let brain = (0..36)
            .flat_map(|y| (0..100).map(move |x| (x, y)))
            .find(|(x, y)| {
                matches!(
                    super::super::ui::hit_test(&app, *x, *y),
                    Some(Target::BrainMark(_))
                )
            });
        let (x, y) = brain.expect("synthesis answer should show a brain mark");
        let down = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        };
        app.handle_mouse(down);
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            ..down
        });
        assert!(matches!(app.overlay, Overlay::Memories { .. }));
        app.overlay = Overlay::None;
        let tool_block = super::super::ui::chat_blocks(&app)
            .iter()
            .position(|block| block.key.starts_with("tool:"))
            .expect("tool log");
        app.chat_follow = false;
        app.chat_sel = tool_block;
        app.set_focus(Target::Transcript);
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        let key = super::super::ui::chat_blocks(&app)[tool_block].key.clone();
        assert!(app.expanded.contains(&key));
    }

    #[test]
    fn system_log_records_errors_and_scrolls() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 80, 24);
        for index in 0..40 {
            app.push_log("error", format!("lookup failed {index}"));
        }
        app.select(4);
        assert_eq!(app.error_count(), 40);
        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert!(app.scrolls.log > 0);
    }

    #[test]
    fn pointer_and_chords_do_not_switch_apps_on_their_own() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 80, 24);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();
        let (x, y) = (0..24)
            .flat_map(|row| (0..80).map(move |column| (column, row)))
            .find(|(column, row)| {
                super::super::ui::hit_test(&app, *column, *row) == Some(Target::App(0))
            })
            .expect("recon row");
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        });
        assert!(app.module.is_none());
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: x,
            row: y.saturating_add(4),
            modifiers: KeyModifiers::NONE,
        });
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: x,
            row: y.saturating_add(4),
            modifiers: KeyModifiers::NONE,
        });
        assert!(app.module.is_none());
        app.select(1);
        app.set_focus(Target::Field(FieldId::BrainInsight));
        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::ALT));
        app.handle_key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE));
        assert_eq!(app.module, Some(ModuleId::Brain));
    }

    #[test]
    fn firecrawl_key_field_is_on_the_osint_tool() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 100, 36);
        app.select(2);
        app.tool_sel = osint::registry()
            .iter()
            .position(|tool| tool.id == "firecrawl_search")
            .unwrap();
        assert!(hit(&app, Target::Field(FieldId::FirecrawlKey)));
        assert!(hit(&app, Target::Button(ButtonId::SaveFirecrawlKey)));
        app.tool_sel = osint::registry()
            .iter()
            .position(|tool| tool.id == "hunter_domain_search")
            .unwrap();
        assert!(hit(&app, Target::Field(FieldId::HunterKey)));
        assert!(hit(&app, Target::Button(ButtonId::SaveHunterKey)));
        assert!(!hit(&app, Target::Field(FieldId::FirecrawlKey)));
        app.tool_sel = osint::registry()
            .iter()
            .position(|tool| tool.id == "sociavault_profile")
            .unwrap();
        assert!(hit(&app, Target::Field(FieldId::SociaVaultKey)));
        assert!(hit(&app, Target::Button(ButtonId::SaveSociaVaultKey)));
        app.tool_sel = 0;
        assert!(!hit(&app, Target::Field(FieldId::FirecrawlKey)));
        assert!(!hit(&app, Target::Field(FieldId::HunterKey)));
        assert!(!hit(&app, Target::Field(FieldId::SociaVaultKey)));
    }

    fn screen_text(terminal: &ratatui::Terminal<ratatui::backend::TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    fn hit(app: &App, target: Target) -> bool {
        (0..app.screen.height)
            .flat_map(|y| (0..app.screen.width).map(move |x| (x, y)))
            .any(|(x, y)| super::super::ui::hit_test(app, x, y) == Some(target))
    }
}
