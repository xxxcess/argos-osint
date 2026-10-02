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
    pub id: u64,
    pub at: String,
    pub level: String,
    pub text: String,
    /// Full tool result. Empty lines stay a single row.
    pub detail: String,
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
pub enum BrainPage {
    Memories,
    Graph,
}
impl BrainPage {
    pub const ALL: [Self; 2] = [Self::Memories, Self::Graph];
    pub fn title(self) -> &'static str {
        match self {
            Self::Memories => "Memories",
            Self::Graph => "Graph",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrainListMode {
    List,
    Create,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphView {
    Force,
    Directive,
    Path,
}
impl GraphView {
    pub const ALL: [Self; 3] = [Self::Force, Self::Directive, Self::Path];
    pub fn title(self) -> &'static str {
        match self {
            Self::Force => "Force",
            Self::Directive => "Directive",
            Self::Path => "Path",
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
    NewsApiKey,
    CourtListenerKey,
    ReconProvider,
    ReconModel,
    PickerProvider,
    PickerModel,
    SynthesisProvider,
    SynthesisModel,
    RouterKey,
    RouterEndpoint,
    Composer,
}

/// The model role the Defaults tab is editing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefaultsRole {
    Recon,
    ToolPicker,
    Synthesis,
}

impl DefaultsRole {
    pub const ALL: [DefaultsRole; 3] = [
        DefaultsRole::Recon,
        DefaultsRole::ToolPicker,
        DefaultsRole::Synthesis,
    ];

    pub fn label(self) -> &'static str {
        match self {
            DefaultsRole::Recon => "Recon",
            DefaultsRole::ToolPicker => "Tool picker",
            DefaultsRole::Synthesis => "Synthesis",
        }
    }

    /// The settings key the System event log names when this role changes.
    pub fn settings_key(self) -> &'static str {
        match self {
            DefaultsRole::Recon => "defaults.recon",
            DefaultsRole::ToolPicker => "defaults.tool_picker",
            DefaultsRole::Synthesis => "defaults.synthesis",
        }
    }

    pub fn provider_field(self) -> FieldId {
        match self {
            DefaultsRole::Recon => FieldId::ReconProvider,
            DefaultsRole::ToolPicker => FieldId::PickerProvider,
            DefaultsRole::Synthesis => FieldId::SynthesisProvider,
        }
    }

    pub fn model_field(self) -> FieldId {
        match self {
            DefaultsRole::Recon => FieldId::ReconModel,
            DefaultsRole::ToolPicker => FieldId::PickerModel,
            DefaultsRole::Synthesis => FieldId::SynthesisModel,
        }
    }

    pub fn save_button(self) -> ButtonId {
        match self {
            DefaultsRole::Recon => ButtonId::SaveRecon,
            DefaultsRole::ToolPicker => ButtonId::SavePicker,
            DefaultsRole::Synthesis => ButtonId::SaveSynthesis,
        }
    }

    fn of_field(field: FieldId) -> Option<DefaultsRole> {
        match field {
            FieldId::ReconProvider | FieldId::ReconModel => Some(DefaultsRole::Recon),
            FieldId::PickerProvider | FieldId::PickerModel => Some(DefaultsRole::ToolPicker),
            FieldId::SynthesisProvider | FieldId::SynthesisModel => Some(DefaultsRole::Synthesis),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonId {
    Send,
    Add,
    Recall,
    Pin,
    Delete,
    SaveRecon,
    SavePicker,
    SaveSynthesis,
    DefaultRole(DefaultsRole),
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
    SaveNewsApiKey,
    SaveCourtListenerKey,
    OpenSource,
    CreateMemory,
    BrainBack,
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
    BrainTab(BrainPage),
    GraphView(GraphView),
    GraphNode(usize),
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
        role: DefaultsRole,
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
    AnswerDelta {
        thread_id: String,
        text: String,
    },
    AnswerNote {
        thread_id: String,
        text: String,
    },
    Deadline {
        thread_id: String,
        label: String,
    },
}

/// Synthesis text for a thread that is still streaming. Deltas for a thread that is not
/// open stay here until the turn finishes; they are not dropped.
#[derive(Default)]
struct LiveAnswer {
    text: String,
    shown: String,
    note: String,
    painted: Option<Instant>,
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
    pub newsapi_key: String,
    pub courtlistener_key: String,
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
    /// Latest stage for a thread that is running, including one that is not selected.
    recon_stages: HashMap<String, String>,
    live_answers: HashMap<String, LiveAnswer>,
    deadlines: HashMap<String, String>,
    /// False on the investigation list. True when a transcript fills the screen.
    pub recon_chat: bool,
    pub scrolls: Scrolls,
    pub expanded: HashSet<String>,
    pub chat_sel: usize,
    pub chat_follow: bool,
    pub overlay: Overlay,
    pub log: Vec<LogLine>,
    pub log_sel: usize,
    pub log_open: HashSet<u64>,
    pub log_browsing: bool,
    log_seq: u64,
    logged_calls: HashSet<String>,
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
    pub picker_provider: String,
    pub picker_model: String,
    pub synthesis_provider: String,
    pub synthesis_model: String,
    pub defaults_role: DefaultsRole,
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
    pub brain_page: BrainPage,
    pub brain_list_mode: BrainListMode,
    pub graph_view: GraphView,
    pub brain_graph: recon::MemoryGraph,
    brain_graph_for: Option<String>,
    pub graph_node: usize,
    pub hits: Vec<ScoredMemory>,
    pub auth: AuthFile,
    pub settings: SettingsFile,
    pub hardware: HardwareProfile,
    auth_path: PathBuf,
    settings_path: PathBuf,
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
        let picker_default = provider::role_secret(&auth, &settings, "tool-picker")?;
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
            newsapi_key: settings.newsapi_api_key.clone(),
            courtlistener_key: settings.courtlistener_api_token.clone(),
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
            recon_stages: HashMap::new(),
            live_answers: HashMap::new(),
            deadlines: HashMap::new(),
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
            log_sel: 0,
            log_open: HashSet::new(),
            log_browsing: false,
            log_seq: 0,
            logged_calls: HashSet::new(),
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
            picker_provider: provider::effective_kind(&picker_default),
            picker_model: picker_default.model,
            synthesis_provider: provider::effective_kind(&synthesis_default),
            synthesis_model: synthesis_default.model,
            defaults_role: DefaultsRole::Recon,
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
            brain_page: BrainPage::Memories,
            brain_list_mode: BrainListMode::List,
            graph_view: GraphView::Force,
            brain_graph: recon::MemoryGraph::default(),
            brain_graph_for: None,
            graph_node: 0,
            hits: Vec::new(),
            auth,
            settings,
            hardware: hardware::profile_cached(false),
            auth_path: paths::auth_path(),
            settings_path: paths::config_path(),
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
        self.push_log_detail(level, text, "");
    }

    fn push_log_detail(&mut self, level: &str, text: impl Into<String>, detail: impl Into<String>) {
        let at_end = self.log.is_empty() || self.log_sel + 1 >= self.log.len();
        self.log_seq = self.log_seq.saturating_add(1);
        self.log.push(LogLine {
            id: self.log_seq,
            at: log_stamp(),
            level: level.into(),
            text: text.into(),
            detail: detail.into(),
        });
        if self.log.len() > 400 {
            let extra = self.log.len() - 400;
            for line in self.log.drain(0..extra) {
                self.log_open.remove(&line.id);
            }
        }
        if at_end {
            self.log_sel = self.log.len().saturating_sub(1);
        } else if self.log_sel >= self.log.len() {
            self.log_sel = self.log.len().saturating_sub(1);
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
            Some(ModuleId::Brain) => Target::Button(ButtonId::CreateMemory),
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
            FieldId::NewsApiKey => &self.newsapi_key,
            FieldId::CourtListenerKey => &self.courtlistener_key,
            FieldId::ReconProvider => &self.recon_provider,
            FieldId::ReconModel => &self.recon_model,
            FieldId::PickerProvider => &self.picker_provider,
            FieldId::PickerModel => &self.picker_model,
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
            FieldId::NewsApiKey => &mut self.newsapi_key,
            FieldId::CourtListenerKey => &mut self.courtlistener_key,
            FieldId::ReconProvider => &mut self.recon_provider,
            FieldId::ReconModel => &mut self.recon_model,
            FieldId::PickerProvider => &mut self.picker_provider,
            FieldId::PickerModel => &mut self.picker_model,
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
        self.log_browsing = false;
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
        self.recon_stage = self
            .recon_stages
            .get(id)
            .cloned()
            .unwrap_or_else(|| "ready".into());
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
                .ask(&tid, &question, cancel, move |event| {
                    let _ = progress_tx.send(work_event(&thread_id, event));
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
                .resume(&run.id, cancel, move |event| {
                    let _ = progress_tx.send(work_event(&thread_id, event));
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
        self.save_settings()?;
        Ok("Firecrawl API key saved".into())
    }

    fn remember_hunter_key(&mut self) -> Result<String> {
        let key = self.hunter_key.trim().to_string();
        anyhow::ensure!(!key.is_empty(), "Enter a Hunter API key");
        self.settings.hunter_api_key = key;
        self.save_settings()?;
        Ok("Hunter API key saved".into())
    }

    fn remember_sociavault_key(&mut self) -> Result<String> {
        let key = self.sociavault_key.trim().to_string();
        anyhow::ensure!(!key.is_empty(), "Enter a SociaVault API key");
        self.settings.sociavault_api_key = key;
        self.save_settings()?;
        Ok("SociaVault API key saved".into())
    }

    fn remember_newsapi_key(&mut self) -> Result<String> {
        let key = self.newsapi_key.trim().to_string();
        anyhow::ensure!(!key.is_empty(), "Enter a NewsAPI key");
        self.settings.newsapi_api_key = key;
        self.save_settings()?;
        Ok("NewsAPI key saved".into())
    }

    fn remember_courtlistener_key(&mut self) -> Result<String> {
        let key = self.courtlistener_key.trim().to_string();
        anyhow::ensure!(!key.is_empty(), "Enter a CourtListener API token");
        self.settings.courtlistener_api_token = key;
        self.save_settings()?;
        Ok("CourtListener API token saved".into())
    }

    /// The tool's provider has no key saved and none in its environment variable.
    pub fn tool_needs_key(&self, id: &str) -> bool {
        self.tool_needs_key_with(id, |name| std::env::var(name).ok())
    }

    pub fn tool_needs_key_with(&self, id: &str, env: impl Fn(&str) -> Option<String>) -> bool {
        osint::endpoint_cost(id).is_some_and(|cost| {
            self.settings
                .provider_key_with(cost.provider, env)
                .is_empty()
        })
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
        if tool.id.starts_with("firecrawl_") {
            self.remember_firecrawl_key()?;
        } else if tool.id.starts_with("hunter_") {
            self.remember_hunter_key()?;
        } else if tool.id.starts_with("sociavault_") {
            self.remember_sociavault_key()?;
        } else if tool.id.starts_with("newsapi_") && !self.newsapi_key.trim().is_empty() {
            // An empty field falls back to NEWSAPI_API_KEY.
            self.remember_newsapi_key()?;
        } else if tool.id.starts_with("courtlistener_") && !self.courtlistener_key.trim().is_empty()
        {
            self.remember_courtlistener_key()?;
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

    fn on_work_event(&mut self, event: WorkEvent) -> bool {
        match event {
            WorkEvent::ReconStage { thread_id, stage } => {
                self.push_log("info", format!("Recon {stage}"));
                self.recon_stages.insert(thread_id.clone(), stage.clone());
                if self.selected_thread.as_deref() == Some(&thread_id) {
                    self.recon_stage = stage;
                    let _ = self.refresh_selected();
                    let _ = self.refresh_threads();
                }
                true
            }
            WorkEvent::AnswerDelta { thread_id, text } => self.note_delta(&thread_id, &text),
            WorkEvent::AnswerNote { thread_id, text } => {
                self.push_log("info", text.clone());
                self.live_answers.entry(thread_id.clone()).or_default().note = text;
                self.selected_thread.as_deref() == Some(&thread_id)
            }
            WorkEvent::Deadline { thread_id, label } => {
                self.push_log("info", label.clone());
                self.deadlines.insert(thread_id.clone(), label);
                self.selected_thread.as_deref() == Some(&thread_id)
            }
            WorkEvent::ReconDone { thread_id, outcome } => {
                self.running.remove(&thread_id);
                self.live_answers.remove(&thread_id);
                self.deadlines.remove(&thread_id);
                self.recon_stages.remove(&thread_id);
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
                true
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
                true
            }
            WorkEvent::CatalogDone {
                role,
                provider,
                outcome,
            } => {
                self.finish_catalog(role, provider, outcome);
                true
            }
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
                true
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
                true
            }
        }
    }

    pub(crate) fn stage_label(&self, thread_id: &str) -> String {
        self.recon_stages
            .get(thread_id)
            .cloned()
            .unwrap_or_else(|| self.recon_stage.clone())
    }

    pub(crate) fn deadline_label(&self, thread_id: &str) -> String {
        self.deadlines.get(thread_id).cloned().unwrap_or_default()
    }

    /// Title and visible text of the streaming answer bubble, when there is something to show.
    pub(crate) fn live_bubble(&self, thread_id: &str) -> Option<(String, String)> {
        let live = self.live_answers.get(thread_id)?;
        if live.shown.is_empty() && live.note.is_empty() {
            return None;
        }
        let title = if live.note.is_empty() {
            "Recon · streaming".to_string()
        } else {
            live.note.clone()
        };
        Some((title, live.shown.clone()))
    }

    /// Appends a synthesis delta. The first one shows immediately; later ones wait 50ms
    /// so the transcript does not redraw on every token. A thread that is not open still
    /// keeps the text.
    fn note_delta(&mut self, thread_id: &str, text: &str) -> bool {
        let live = self.live_answers.entry(thread_id.to_string()).or_default();
        live.text.push_str(text);
        let due = live.shown.is_empty()
            || live
                .painted
                .is_none_or(|painted| painted.elapsed() >= Duration::from_millis(50));
        if !due {
            return false;
        }
        live.shown.clone_from(&live.text);
        live.painted = Some(Instant::now());
        self.selected_thread.as_deref() == Some(thread_id)
    }

    pub fn role_provider(&self) -> String {
        let raw = self.field(self.defaults_role.provider_field());
        match provider::normalize_kind(raw).as_str() {
            "openai" => "openai-chatgpt".into(),
            "grok-subscription" => "grok".into(),
            other => other.to_string(),
        }
    }

    fn role_model(&self) -> String {
        self.field(self.defaults_role.model_field()).to_string()
    }

    fn set_role_provider(&mut self, id: &str) {
        *self.field_mut(self.defaults_role.provider_field()) = id.to_string();
    }

    fn set_role_model(&mut self, id: &str) {
        *self.field_mut(self.defaults_role.model_field()) = id.to_string();
    }

    pub fn field_display(&self, field: FieldId) -> String {
        match field {
            FieldId::ReconProvider | FieldId::PickerProvider | FieldId::SynthesisProvider => {
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
        let role = self.defaults_role;
        let provider = self.role_provider();
        if provider.is_empty() {
            self.status = "Choose a provider first".into();
            return;
        }
        if provider == "openai-chatgpt" {
            self.finish_catalog(role, provider, Ok(codex_models()));
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
                role,
                provider,
                outcome,
            });
        });
    }

    fn finish_catalog(
        &mut self,
        role: DefaultsRole,
        provider: String,
        outcome: Result<Vec<ListedModel>, String>,
    ) {
        if self.defaults_role != role || self.role_provider() != provider {
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
        if DefaultsRole::of_field(field) != Some(self.defaults_role) {
            return;
        }
        if field == self.defaults_role.provider_field() {
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
            self.finish_catalog(self.defaults_role, provider, Ok(codex_models()));
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
        let items = role_model_items(
            self.defaults_role,
            &self.role_provider(),
            &self.model_catalog,
        );
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
            self.note_finished_calls();
        }
        Ok(())
    }

    /// Appends one collapsible System log entry the first time a call has a result.
    fn note_finished_calls(&mut self) {
        let pending: Vec<recon::Call> = self
            .calls
            .iter()
            .filter(|call| call.result.is_some() && !self.logged_calls.contains(&call.id))
            .cloned()
            .collect();
        for call in pending {
            self.logged_calls.insert(call.id.clone());
            if let Some(entry) = super::ui::tool_result_log(&call) {
                self.push_log_detail(entry.level, entry.summary, entry.detail);
            }
        }
    }

    fn save_insight(&mut self) -> Result<String> {
        let (category, text) = argos_osint_core::brain::parse_typed_memory(&self.brain_insight);
        let memory = self.store.add_memory(
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
        self.memory_sel = self
            .memories
            .iter()
            .position(|item| item.id == memory.id)
            .unwrap_or(0);
        self.selected_insight = self.store.insight_for_memory(&memory.id).ok().flatten();
        self.brain_insight.clear();
        self.brain_list_mode = BrainListMode::List;
        self.brain_graph_for = None;
        self.sync_graph();
        self.set_focus(Target::Memory(self.memory_sel));
        Ok("Insight saved with source".into())
    }

    fn sync_graph(&mut self) {
        let id = self
            .memories
            .get(self.memory_sel)
            .map(|memory| memory.id.clone());
        if self.brain_graph_for == id {
            return;
        }
        self.brain_graph_for = id.clone();
        self.graph_node = 0;
        self.brain_graph = match &id {
            Some(id) => self.store.graph_for_memory(id).unwrap_or_default(),
            None => recon::MemoryGraph::default(),
        };
    }

    fn move_graph_node(&mut self, delta: i32) {
        let count = super::graph::selectable(&self.brain_graph, self.graph_view).len();
        if count == 0 {
            return;
        }
        let next = (self.graph_node as i32 + delta).clamp(0, count as i32 - 1) as usize;
        self.graph_node = next;
        self.set_focus(Target::GraphNode(next));
    }

    fn activate_button(&mut self, button: ButtonId) {
        let result = match button {
            ButtonId::Send => {
                self.submit();
                return;
            }
            ButtonId::CreateMemory => {
                self.brain_page = BrainPage::Memories;
                self.brain_list_mode = BrainListMode::Create;
                self.set_focus(Target::Field(FieldId::BrainApp));
                self.cursor = self.brain_app.chars().count();
                self.status = "New memory".into();
                return;
            }
            ButtonId::BrainBack => {
                self.brain_list_mode = BrainListMode::List;
                self.set_focus(Target::Button(ButtonId::CreateMemory));
                self.status = "Memories".into();
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
                        self.brain_graph_for = None;
                        self.sync_graph();
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
            ButtonId::SaveNewsApiKey => self.remember_newsapi_key(),
            ButtonId::SaveCourtListenerKey => self.remember_courtlistener_key(),
            ButtonId::OpenSource => self
                .open_insight_source()
                .map(|_| "Source thread opened".into()),
            ButtonId::ClearLog => {
                self.log.clear();
                self.log_open.clear();
                self.log_sel = 0;
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
            ButtonId::SaveRecon => self.save_role(DefaultsRole::Recon),
            ButtonId::SavePicker => self.save_role(DefaultsRole::ToolPicker),
            ButtonId::SaveSynthesis => self.save_role(DefaultsRole::Synthesis),
            ButtonId::DefaultRole(role) => {
                if self.defaults_role != role {
                    self.defaults_role = role;
                    self.model_catalog.clear();
                    self.catalog_for.clear();
                }
                Ok(format!("{} default", role.label()))
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

    /// Saves one role's provider and model. Only `settings.toml` changes; credentials
    /// stay where they are. The change is recorded in the System event log.
    fn save_role(&mut self, role: DefaultsRole) -> Result<String> {
        let provider = self.field(role.provider_field()).trim().to_string();
        let model = self.field(role.model_field()).trim().to_string();
        if provider.is_empty() || model.is_empty() {
            anyhow::bail!("Provider and model are required");
        }
        let kind = provider::normalize_kind(&provider);
        let kind = if kind == "openai" && role != DefaultsRole::Synthesis {
            "openai-chatgpt".into()
        } else {
            kind
        };
        if role != DefaultsRole::Synthesis
            && !matches!(
                kind.as_str(),
                "grok" | "openai-chatgpt" | "openrouter" | "local"
            )
        {
            anyhow::bail!("Choose Grok, OpenAI, OpenRouter, or local");
        }
        let assignment = self
            .settings
            .defaults
            .role_mut(match role {
                DefaultsRole::Recon => "recon",
                DefaultsRole::ToolPicker => "tool-picker",
                DefaultsRole::Synthesis => "synthesis",
            })
            .ok_or_else(|| anyhow::anyhow!("unknown role"))?;
        let before = format!("{} / {}", assignment.provider, assignment.model);
        assignment.provider = kind.clone();
        assignment.model = model.clone();
        self.save_settings()?;
        let after = format!("{kind} / {model}");
        if before != after {
            self.push_log(
                "info",
                format!("{}: {before} -> {after}", role.settings_key()),
            );
        }
        Ok(match role {
            DefaultsRole::Recon => format!("Recon: {after}"),
            DefaultsRole::ToolPicker => format!(
                "Tool picker: {after} ({})",
                provider::picker_transport(&model)
            ),
            DefaultsRole::Synthesis => "Synthesis default saved".into(),
        })
    }

    /// Writes `config.toml`. Tests point `settings_path` at a temp file.
    fn save_settings(&self) -> Result<()> {
        if self.settings_path == paths::config_path() {
            self.settings.save()
        } else {
            self.settings.save_to(&self.settings_path)
        }
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
            Target::BrainTab(page) => {
                self.brain_page = page;
                if page == BrainPage::Graph {
                    self.sync_graph();
                    let count = super::graph::selectable(&self.brain_graph, self.graph_view).len();
                    if count == 0 {
                        self.set_focus(Target::BrainTab(BrainPage::Graph));
                    } else {
                        self.graph_node = self.graph_node.min(count - 1);
                        self.set_focus(Target::GraphNode(self.graph_node));
                    }
                } else {
                    self.set_focus(target);
                }
            }
            Target::GraphView(view) => {
                self.graph_view = view;
                self.graph_node = 0;
                self.set_focus(
                    if super::graph::selectable(&self.brain_graph, view).is_empty() {
                        Target::GraphView(view)
                    } else {
                        Target::GraphNode(0)
                    },
                );
            }
            Target::GraphNode(index) => {
                self.graph_node = index;
                self.set_focus(target);
            }
            Target::Memory(index) => {
                self.memory_sel = index;
                self.selected_insight = self
                    .memories
                    .get(index)
                    .and_then(|m| self.store.insight_for_memory(&m.id).ok().flatten());
                self.sync_graph();
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
            KeyCode::Char(c)
                if self.module == Some(ModuleId::Brain)
                    && self.brain_page == BrainPage::Graph
                    && !self.field_focused()
                    && matches!(c, '1' | '2' | '3') =>
            {
                self.activate_target(Target::GraphView(match c {
                    '1' => GraphView::Force,
                    '2' => GraphView::Directive,
                    _ => GraphView::Path,
                }));
            }
            KeyCode::Left | KeyCode::Char('h')
                if self.module == Some(ModuleId::Brain)
                    && self.brain_page == BrainPage::Graph
                    && !self.field_focused() =>
            {
                self.move_graph_node(-1);
            }
            KeyCode::Right | KeyCode::Char('l')
                if self.module == Some(ModuleId::Brain)
                    && self.brain_page == BrainPage::Graph
                    && !self.field_focused() =>
            {
                self.move_graph_node(1);
            }
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
        if self.module == Some(ModuleId::Brain) && self.brain_list_mode == BrainListMode::Create {
            self.brain_list_mode = BrainListMode::List;
            self.set_focus(Target::Button(ButtonId::CreateMemory));
            self.status = "Memories".into();
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

    fn toggle_log(&mut self) {
        let Some(line) = self.log.get(self.log_sel) else {
            return;
        };
        if line.detail.is_empty() {
            return;
        }
        let id = line.id;
        if !self.log_open.insert(id) {
            self.log_open.remove(&id);
        }
        super::ui::reveal_log(self);
    }

    fn on_enter(&mut self) {
        if self.module == Some(ModuleId::System) && self.log_browsing {
            self.toggle_log();
            return;
        }
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
            Target::GraphNode(_) => self.move_graph_node(delta),
            Target::Tool(_) => self.move_tool(delta),
            _ => match self.module {
                None => self.move_home(delta),
                Some(ModuleId::Brain) if self.brain_page == BrainPage::Graph => {
                    self.move_graph_node(delta);
                }
                Some(ModuleId::Brain) if self.brain_list_mode == BrainListMode::Create => {}
                Some(ModuleId::Brain) => self.move_memory(delta),
                Some(ModuleId::Osint) => self.move_tool(delta),
                Some(ModuleId::System) => {
                    super::ui::move_system_log(self, delta);
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
        self.sync_graph();
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
            | FieldId::PickerProvider
            | FieldId::PickerModel
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

/// Model choices for a role. The tool picker on OpenRouter always offers the decisions
/// models first, even when `GET /api/v1/models` omits them.
fn role_model_items(
    role: DefaultsRole,
    provider: &str,
    catalog: &[ListedModel],
) -> Vec<ChoiceItem> {
    let mut items = Vec::new();
    let picker = role == DefaultsRole::ToolPicker && provider == "openrouter";
    if picker {
        for (id, label) in provider::DECISIONS_MODELS {
            items.push(ChoiceItem {
                id: id.to_string(),
                label: format!("{label} · {id}"),
            });
        }
    }
    for model in catalog {
        if picker && items.iter().any(|item| item.id == model.id) {
            continue;
        }
        items.push(ChoiceItem {
            id: model.id.clone(),
            label: model_label(model),
        });
    }
    items
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

fn work_event(thread_id: &str, event: recon::TurnEvent) -> WorkEvent {
    let thread_id = thread_id.to_string();
    match event {
        recon::TurnEvent::Stage(stage) => WorkEvent::ReconStage { thread_id, stage },
        recon::TurnEvent::AnswerDelta(text) => WorkEvent::AnswerDelta { thread_id, text },
        recon::TurnEvent::AnswerNote(text) => WorkEvent::AnswerNote { thread_id, text },
        recon::TurnEvent::Deadline(label) => WorkEvent::Deadline { thread_id, label },
    }
}

fn pump(app: &mut App) -> bool {
    let mut dirty = false;
    while let Ok(message) = app.provider_rx.try_recv() {
        app.on_provider_event(message);
        dirty = true;
    }
    while let Ok(message) = app.work_rx.try_recv() {
        dirty |= app.on_work_event(message);
    }
    dirty |= flush_streams(app);
    dirty
}

/// Copies buffered synthesis text into the bubble once 50ms have passed since the last paint.
fn flush_streams(app: &mut App) -> bool {
    let now = Instant::now();
    let selected = app.selected_thread.clone();
    let mut dirty = false;
    for (id, live) in &mut app.live_answers {
        if live.shown == live.text {
            continue;
        }
        let due = live
            .painted
            .is_none_or(|painted| now.duration_since(painted) >= Duration::from_millis(50));
        if !due {
            continue;
        }
        live.shown.clone_from(&live.text);
        live.painted = Some(now);
        if selected.as_deref() == Some(id.as_str()) {
            dirty = true;
        }
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
            newsapi_key: String::new(),
            courtlistener_key: String::new(),
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
            recon_stages: HashMap::new(),
            live_answers: HashMap::new(),
            deadlines: HashMap::new(),
            recon_chat: false,
            scrolls: Scrolls::default(),
            expanded: HashSet::new(),
            chat_sel: 0,
            chat_follow: true,
            overlay: Overlay::None,
            log: Vec::new(),
            log_sel: 0,
            log_open: HashSet::new(),
            log_browsing: false,
            log_seq: 0,
            logged_calls: HashSet::new(),
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
            picker_provider: String::new(),
            picker_model: String::new(),
            synthesis_provider: String::new(),
            synthesis_model: String::new(),
            defaults_role: DefaultsRole::Recon,
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
            brain_page: BrainPage::Memories,
            brain_list_mode: BrainListMode::List,
            graph_view: GraphView::Force,
            brain_graph: recon::MemoryGraph::default(),
            brain_graph_for: None,
            graph_node: 0,
            hits: Vec::new(),
            auth: AuthFile::default(),
            settings: SettingsFile::default(),
            hardware: HardwareProfile::unknown(),
            auth_path: PathBuf::new(),
            settings_path: PathBuf::new(),
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
        let listed = super::super::ui::focus_order(&app);
        assert!(listed.contains(&Target::Button(ButtonId::CreateMemory)));
        assert!(listed.contains(&Target::Memory(0)) || app.memories.is_empty());
        assert!(!listed
            .iter()
            .any(|target| matches!(target, Target::Field(FieldId::BrainApp))));
        click(&mut app, Target::Button(ButtonId::CreateMemory));
        assert_eq!(app.brain_list_mode, BrainListMode::Create);
        assert_eq!(app.focus, Target::Field(FieldId::BrainApp));
        let forming = super::super::ui::focus_order(&app);
        assert!(forming.contains(&Target::Field(FieldId::BrainApp)));
        assert!(forming.contains(&Target::Button(ButtonId::BrainBack)));
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.brain_list_mode, BrainListMode::List);
        assert!(app.memories.is_empty());
        click(&mut app, Target::Button(ButtonId::CreateMemory));
        click(&mut app, Target::Field(FieldId::BrainApp));
        type_text(&mut app, "chat");
        click(&mut app, Target::Field(FieldId::BrainConversation));
        type_text(&mut app, "thread-1");
        click(&mut app, Target::Field(FieldId::BrainInsight));
        type_text(&mut app, "project: Atlas launch");
        click(&mut app, Target::Button(ButtonId::Add));
        assert_eq!(app.brain_list_mode, BrainListMode::List);
        assert_eq!(app.memories[0].source.conversation_id, "thread-1");
        let listed = super::super::ui::focus_order(&app);
        assert!(listed.contains(&Target::Button(ButtonId::CreateMemory)));
        assert!(listed.contains(&Target::Memory(app.memory_sel)));
        assert!(!listed
            .iter()
            .any(|target| matches!(target, Target::Field(FieldId::BrainInsight))));
        click(&mut app, Target::BrainTab(BrainPage::Graph));
        assert_eq!(app.brain_page, BrainPage::Graph);
        assert!(app.brain_graph.is_empty());
        click(&mut app, Target::BrainTab(BrainPage::Memories));
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
    fn defaults_tool_picker_saves_only_its_role() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app();
        app.settings_path = dir.path().join("config.toml");
        app.auth_path = dir.path().join("auth.json");
        let mut router = provider::account_secret(&app.auth, "openrouter");
        router.api_key = Some("router-key".into());
        app.auth.set_account(router);
        app.settings.defaults.recon.provider = "grok".into();
        app.settings.defaults.recon.model = "grok-4.6".into();
        app.settings.defaults.synthesis.provider = "openrouter".into();
        app.settings.defaults.synthesis.model = "openrouter/free".into();
        app.recon_provider = "grok".into();
        app.recon_model = "grok-4.6".into();
        app.synthesis_provider = "openrouter".into();
        app.synthesis_model = "openrouter/free".into();
        let auth_before = serde_json::to_string(&app.auth).unwrap();

        click(&mut app, Target::App(3));
        click(&mut app, Target::ProviderTab(ProviderPage::Defaults));
        click(
            &mut app,
            Target::Button(ButtonId::DefaultRole(DefaultsRole::ToolPicker)),
        );
        assert_eq!(app.defaults_role, DefaultsRole::ToolPicker);
        click(&mut app, Target::Field(FieldId::PickerProvider));
        assert!(matches!(app.overlay, Overlay::Choice(ChoiceKind::Provider)));
        let openrouter = app
            .choice_items
            .iter()
            .position(|item| item.id == "openrouter")
            .unwrap();
        click(&mut app, Target::Choice(openrouter));
        assert_eq!(app.picker_provider, "openrouter");

        // The account catalog omits Jev; the picker list still offers it first.
        app.on_work_event(WorkEvent::CatalogDone {
            role: DefaultsRole::ToolPicker,
            provider: "openrouter".into(),
            outcome: Ok(vec![ListedModel {
                id: "openai/gpt-5-mini".into(),
                name: "GPT-5 mini".into(),
                free: false,
            }]),
        });
        click(&mut app, Target::Field(FieldId::PickerModel));
        assert!(matches!(app.overlay, Overlay::Choice(ChoiceKind::Model)));
        assert_eq!(app.choice_items[0].id, "typesafe/jev-1.13");
        assert!(app.choice_items[0].label.contains("Jev 1.13 (decisions)"));
        assert_eq!(app.choice_items[1].id, "openai/gpt-5-mini");
        click(&mut app, Target::Choice(0));
        assert_eq!(app.picker_model, "typesafe/jev-1.13");

        click(&mut app, Target::Button(ButtonId::SavePicker));
        assert!(app.status.contains("decisions"), "{}", app.status);
        assert_eq!(app.settings.defaults.tool_picker.provider, "openrouter");
        assert_eq!(app.settings.defaults.tool_picker.model, "typesafe/jev-1.13");
        assert_eq!(app.settings.defaults.recon.provider, "grok");
        assert_eq!(app.settings.defaults.recon.model, "grok-4.6");
        assert_eq!(app.settings.defaults.synthesis.provider, "openrouter");
        assert_eq!(app.settings.defaults.synthesis.model, "openrouter/free");
        assert_eq!(app.recon_model, "grok-4.6");
        assert_eq!(app.synthesis_model, "openrouter/free");
        assert!(app
            .log
            .iter()
            .any(|line| line.text.starts_with("defaults.tool_picker:")));
        let saved = std::fs::read_to_string(&app.settings_path).unwrap();
        assert!(saved.contains("[defaults.tool_picker]"), "{saved}");
        assert!(saved.contains("typesafe/jev-1.13"));
        assert!(!saved.contains("router-key"));
        assert!(!app.auth_path.exists());
        assert_eq!(serde_json::to_string(&app.auth).unwrap(), auth_before);
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
        app.defaults_role = DefaultsRole::Synthesis;
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
            role: DefaultsRole::Recon,
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

        click(
            &mut app,
            Target::Button(ButtonId::DefaultRole(DefaultsRole::Synthesis)),
        );
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
            role: DefaultsRole::Synthesis,
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
                ..recon::PlanCall::default()
            }],
            unresolved_inputs: Vec::new(),
            stop_condition: "A current observation is in hand".into(),
            planning_mode: "json".into(),
            ..recon::Plan::default()
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
    fn a_finished_tool_call_is_logged_once_and_the_transcript_keeps_a_summary() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 100, 40);
        app.calls.push(recon::Call {
            id: "call-s1".into(),
            tool_id: "firecrawl_search".into(),
            run_id: None,
            thread_id: None,
            turn_id: None,
            origin: "recon".into(),
            inputs: serde_json::json!({"query": "Jane Roe"}),
            status: "completed".into(),
            attempts: 1,
            result: Some(osint::ToolResult {
                tool_id: "firecrawl_search".into(),
                inputs: serde_json::json!({"query": "Jane Roe"}),
                status: "completed".into(),
                source_url: "https://example.test/jane".into(),
                retrieved_at: String::new(),
                observations: serde_json::json!({"results": [{"title": "Jane Roe role"}, {"title": "Jane Roe site"}]}),
                raw: String::new(),
                error: None,
                cached: true,
                truncated: false,
                credits_charged: 0,
                credits_reported: None,
            }),
            started_at: String::new(),
            completed_at: Some(String::new()),
        });
        app.note_finished_calls();
        app.note_finished_calls();
        let logged: Vec<_> = app
            .log
            .iter()
            .filter(|line| !line.detail.is_empty())
            .collect();
        assert_eq!(logged.len(), 1);
        assert!(logged[0].text.contains("cache"));
        assert!(logged[0].text.contains("2 results"));
        assert!(logged[0].detail.contains("Jane Roe role"));
        app.module = Some(ModuleId::Recon);
        app.recon_chat = true;
        app.expanded.insert("tool:call-s1".into());
        let body = super::super::ui::chat_blocks(&app)
            .into_iter()
            .find(|block| block.key == "tool:call-s1")
            .expect("tool row")
            .body;
        assert!(body.contains("System event log"));
        assert!(body.contains("cache"));
        assert!(!body.contains("Jane Roe role"));
        app.select(4);
        let index = app
            .log
            .iter()
            .position(|line| !line.detail.is_empty())
            .unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.log_sel = index;
        let id = app.log[index].id;
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.log_open.contains(&id));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(!app.log_open.contains(&id));
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
        // Every tool of a provider shares its key row.
        for (id, field) in [
            ("firecrawl_map", FieldId::FirecrawlKey),
            ("sociavault_google_search", FieldId::SociaVaultKey),
            ("hunter_company_enrichment", FieldId::HunterKey),
        ] {
            app.tool_sel = osint::registry()
                .iter()
                .position(|tool| tool.id == id)
                .unwrap();
            assert!(hit(&app, Target::Field(field)), "{id}");
        }
        app.tool_sel = 0;
        assert!(!hit(&app, Target::Field(FieldId::FirecrawlKey)));
        assert!(!hit(&app, Target::Field(FieldId::HunterKey)));
        assert!(!hit(&app, Target::Field(FieldId::SociaVaultKey)));
    }

    /// AC2: NewsAPI and CourtListener tools each show a masked key row with a Save
    /// button; saving stores the key for every tool of the provider, and a tool without
    /// a saved or environment key shows "needs key".
    #[test]
    fn news_and_legal_key_fields_save_and_show_needs_key() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app();
        app.settings_path = dir.path().join("config.toml");
        app.screen = Rect::new(0, 0, 100, 36);
        app.select(2);
        let select = |app: &mut App, id: &str| {
            app.tool_sel = osint::registry()
                .iter()
                .position(|tool| tool.id == id)
                .unwrap()
        };
        for id in ["newsapi_search", "newsapi_headlines"] {
            select(&mut app, id);
            assert!(hit(&app, Target::Field(FieldId::NewsApiKey)), "{id}");
            assert!(hit(&app, Target::Button(ButtonId::SaveNewsApiKey)), "{id}");
            assert!(!hit(&app, Target::Field(FieldId::CourtListenerKey)), "{id}");
        }
        for id in [
            "courtlistener_case_search",
            "courtlistener_docket_search",
            "courtlistener_judge_search",
        ] {
            select(&mut app, id);
            assert!(hit(&app, Target::Field(FieldId::CourtListenerKey)), "{id}");
            assert!(
                hit(&app, Target::Button(ButtonId::SaveCourtListenerKey)),
                "{id}"
            );
        }
        // Without a saved or environment key every tool of the provider needs one.
        let no_env = |_: &str| None;
        for id in osint::NEWS_TOOLS.iter().chain(osint::LEGAL_TOOLS) {
            assert!(app.tool_needs_key_with(id, no_env), "{id}");
        }
        assert!(
            !app.tool_needs_key_with("crtsh_certificates", no_env),
            "keyless tools never need one"
        );
        let env_court =
            |name: &str| (name == "COURTLISTENER_API_TOKEN").then(|| "env-token".to_string());
        assert!(
            !app.tool_needs_key_with("courtlistener_judge_search", env_court),
            "the env fallback counts"
        );
        // Saving from the key row stores it for the provider.
        select(&mut app, "newsapi_headlines");
        app.newsapi_key = "news-secret-29".into();
        click(&mut app, Target::Button(ButtonId::SaveNewsApiKey));
        assert_eq!(app.status, "NewsAPI key saved");
        select(&mut app, "courtlistener_case_search");
        app.courtlistener_key = "court-secret-29".into();
        click(&mut app, Target::Button(ButtonId::SaveCourtListenerKey));
        assert_eq!(app.status, "CourtListener API token saved");
        assert_eq!(
            (
                app.settings.newsapi_api_key.as_str(),
                app.settings.courtlistener_api_token.as_str()
            ),
            ("news-secret-29", "court-secret-29")
        );
        for id in osint::NEWS_TOOLS.iter().chain(osint::LEGAL_TOOLS) {
            assert!(
                !app.tool_needs_key_with(id, no_env),
                "{id}: one key enables every tool of the provider"
            );
        }
        let saved = std::fs::read_to_string(&app.settings_path).unwrap();
        assert!(
            saved.contains("newsapi_api_key") && saved.contains("courtlistener_api_token"),
            "{saved}"
        );
        // The key field is masked on screen and "needs key" shows on unkeyed tools.
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 36)).unwrap();
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();
        let text = screen_text(&terminal);
        assert!(
            !text.contains("court-secret-29") && text.contains("•••••"),
            "masked key field"
        );
        app.settings.courtlistener_api_token.clear();
        if std::env::var("COURTLISTENER_API_TOKEN").is_err() {
            terminal
                .draw(|frame| super::super::ui::draw(frame, &app))
                .unwrap();
            assert!(screen_text(&terminal).contains("needs key"));
        }
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

    #[test]
    fn synthesis_deltas_fill_the_live_bubble_and_the_saved_answer_replaces_them() {
        let mut app = app();
        app.module = Some(ModuleId::Recon);
        app.recon_chat = true;
        app.screen = Rect::new(0, 0, 100, 40);
        app.selected_thread = Some("t-open".into());
        app.running
            .insert("t-open".into(), Arc::new(AtomicBool::new(false)));
        app.messages = vec![recon::Message {
            id: "m1".into(),
            thread_id: "t-open".into(),
            sequence: 1,
            role: "user".into(),
            content: "who?".into(),
            run_id: None,
            created_at: String::new(),
        }];
        app.runs = vec![recon::Run {
            id: "run-1".into(),
            thread_id: "t-open".into(),
            turn_id: "m1".into(),
            state: "running".into(),
            stage: "synthesizing".into(),
            recon_model: String::new(),
            synthesis_model: String::new(),
            tool_picker_model: String::new(),
            max_rounds: 1,
            max_calls: 4,
            turn_seconds: 300,
            plan_json: None,
            error: None,
            created_at: String::new(),
            updated_at: String::new(),
        }];
        let label = "Deadline 6m 10s: 11 calls, ~52k chars evidence";
        app.recon_stage = "synthesizing".into();
        app.recon_stages
            .insert("t-open".into(), "synthesizing".into());
        app.on_work_event(WorkEvent::Deadline {
            thread_id: "t-open".into(),
            label: label.into(),
        });
        assert!(app.on_work_event(WorkEvent::AnswerDelta {
            thread_id: "t-open".into(),
            text: "Hel".into()
        }));
        assert!(!app.on_work_event(WorkEvent::AnswerDelta {
            thread_id: "t-open".into(),
            text: "lo".into()
        }));
        // A thread that is not on screen keeps every token.
        app.on_work_event(WorkEvent::AnswerDelta {
            thread_id: "t-hidden".into(),
            text: "Hid".into(),
        });
        app.on_work_event(WorkEvent::AnswerDelta {
            thread_id: "t-hidden".into(),
            text: "den".into(),
        });
        assert_eq!(app.live_answers["t-hidden"].text, "Hidden");
        let blocks = super::super::ui::chat_blocks(&app);
        let stream = blocks
            .iter()
            .find(|block| block.key == "stream:run-1")
            .unwrap();
        assert_eq!(stream.title, "Recon · streaming");
        assert_eq!(stream.body, "Hel");
        assert!(blocks
            .iter()
            .any(|block| block.key == "status:run-1" && block.title.contains(label)));
        assert!(!blocks.iter().any(|block| block.body.contains("Hidden")));
        app.live_answers.get_mut("t-open").unwrap().painted =
            Some(Instant::now() - Duration::from_millis(50));
        assert!(flush_streams(&mut app));
        let blocks = super::super::ui::chat_blocks(&app);
        assert_eq!(
            blocks
                .iter()
                .find(|block| block.key == "stream:run-1")
                .unwrap()
                .body,
            "Hello"
        );
        app.on_work_event(WorkEvent::AnswerNote {
            thread_id: "t-open".into(),
            text: "fixing citations…".into(),
        });
        let blocks = super::super::ui::chat_blocks(&app);
        assert_eq!(
            blocks
                .iter()
                .find(|block| block.key == "stream:run-1")
                .unwrap()
                .title,
            "fixing citations…"
        );
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 40)).unwrap();
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();
        let painted = screen_text(&terminal);
        assert!(painted.contains("Hello"), "{painted}");
        assert!(painted.contains("fixing citations"), "{painted}");
        app.on_work_event(WorkEvent::ReconDone {
            thread_id: "t-open".into(),
            outcome: Ok(()),
        });
        assert!(!app.live_answers.contains_key("t-open"));
        app.messages = vec![
            recon::Message {
                id: "m1".into(),
                thread_id: "t-open".into(),
                sequence: 1,
                role: "user".into(),
                content: "who?".into(),
                run_id: None,
                created_at: String::new(),
            },
            recon::Message {
                id: "m2".into(),
                thread_id: "t-open".into(),
                sequence: 2,
                role: "assistant".into(),
                content: "Repaired answer [call-1].".into(),
                run_id: Some("run-1".into()),
                created_at: String::new(),
            },
        ];
        let blocks = super::super::ui::chat_blocks(&app);
        assert!(blocks.iter().all(|block| !block.key.starts_with("stream:")));
        assert!(blocks
            .iter()
            .any(|block| block.body == "Repaired answer [call-1]."));
    }

    fn hit(app: &App, target: Target) -> bool {
        (0..app.screen.height)
            .flat_map(|y| (0..app.screen.width).map(move |x| (x, y)))
            .any(|(x, y)| super::super::ui::hit_test(app, x, y) == Some(target))
    }
}
